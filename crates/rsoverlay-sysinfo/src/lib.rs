//! System load sampler — CPU, GPU, RAM, VRAM. No kernel driver.
//!
//! Temperatures are deliberately absent: reading them needs a ring-0
//! driver (WinRing0-class), which is a security liability we won't ship.
//! Everything here comes from documented user-mode APIs:
//!
//! - CPU: `GetSystemTimes` deltas between samples.
//! - RAM: `GlobalMemoryStatusEx`.
//! - GPU: PDH `\GPU Engine(*engtype_3D)\Utilization Percentage`, summed
//!   across instances — the same source Task Manager's 3D graph uses.
//! - VRAM: PDH `\GPU Adapter Memory(*)\Dedicated Usage` (system-wide —
//!   DXGI's QueryVideoMemoryInfo is per-process) over the largest
//!   adapter's dedicated memory from DXGI.
//!
//! One worker thread samples every [`INTERVAL`]; the UI reads snapshots.

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// 0..=100. `None` until two samples exist.
    pub cpu_pct: Option<f32>,
    pub gpu_pct: Option<f32>,
    pub ram_used: u64,
    pub ram_total: u64,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
}

pub struct Sampler {
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Sampler {
    pub fn spawn() -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s, st) = (snapshot.clone(), stop.clone());
        if let Err(e) = std::thread::Builder::new().name("rsoverlay-sysinfo".into()).spawn(move || imp::worker(s, st)) {
            tracing::warn!("sysinfo thread failed to start: {e}");
        }
        Self { snapshot, stop }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| *s).unwrap_or_default()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// CPU busy % from two `(idle, kernel, user)` samples in 100 ns ticks.
/// Kernel time *includes* idle time on Windows, hence `kernel + user`
/// as the total.
pub fn cpu_pct(prev: (u64, u64, u64), now: (u64, u64, u64)) -> Option<f32> {
    let idle = now.0.checked_sub(prev.0)?;
    let total = now.1.checked_sub(prev.1)? + now.2.checked_sub(prev.2)?;
    if total == 0 {
        return None;
    }
    Some((100.0 * (1.0 - idle as f64 / total as f64)).clamp(0.0, 100.0) as f32)
}

/// Bytes → "12.3 GB" (1024-based, one decimal).
pub fn format_bytes(b: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    const MB: f64 = (1u64 << 20) as f64;
    let b = b as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

#[cfg(windows)]
mod imp {
    use super::{cpu_pct, Snapshot, INTERVAL};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE};
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
        PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA,
    };
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::GetSystemTimes;

    pub(super) fn worker(shared: Arc<Mutex<Snapshot>>, stop: Arc<AtomicBool>) {
        let pdh = Pdh::open();
        let vram_total = largest_dedicated_vram();
        let mut prev = system_times();
        // First reading after 250 ms so the card isn't blank for a full
        // interval; steady-state cadence after that.
        let mut wait = std::time::Duration::from_millis(250);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(wait);
            wait = INTERVAL;
            let now = system_times();
            let mut snap = Snapshot {
                cpu_pct: prev.zip(now).and_then(|(p, n)| cpu_pct(p, n)),
                vram_total,
                ..Default::default()
            };
            prev = now;
            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if unsafe { GlobalMemoryStatusEx(&mut ms) }.is_ok() {
                snap.ram_total = ms.ullTotalPhys;
                snap.ram_used = ms.ullTotalPhys - ms.ullAvailPhys;
            }
            if let Some(p) = &pdh {
                if unsafe { PdhCollectQueryData(p.query) } == 0 {
                    snap.gpu_pct = sum_counter(p.gpu).map(|v| v.clamp(0.0, 100.0) as f32);
                    snap.vram_used = sum_counter(p.vram).map(|v| v as u64);
                }
            }
            if let Ok(mut s) = shared.lock() {
                *s = snap;
            }
        }
    }

    fn system_times() -> Option<(u64, u64, u64)> {
        let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)) }.ok()?;
        let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
        Some((t(i), t(k), t(u)))
    }

    struct Pdh {
        query: isize,
        gpu: isize,
        vram: isize,
    }

    impl Pdh {
        fn open() -> Option<Self> {
            unsafe {
                let mut query = 0isize;
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }
                let add = |path: PCWSTR| {
                    let mut c = 0isize;
                    (PdhAddEnglishCounterW(query, path, 0, &mut c) == 0).then_some(c)
                };
                let gpu = add(w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"));
                let vram = add(w!("\\GPU Adapter Memory(*)\\Dedicated Usage"));
                let (Some(gpu), Some(vram)) = (gpu, vram) else {
                    tracing::info!("sysinfo: GPU PDH counters unavailable; GPU/VRAM hidden");
                    PdhCloseQuery(query);
                    return None;
                };
                // Rate counters need a baseline sample before the first read.
                PdhCollectQueryData(query);
                Some(Self { query, gpu, vram })
            }
        }
    }

    impl Drop for Pdh {
        fn drop(&mut self) {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }

    /// Sum every wildcard instance of a counter (two-call size/fill).
    fn sum_counter(counter: isize) -> Option<f64> {
        unsafe {
            let (mut bytes, mut count) = (0u32, 0u32);
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, None) != PDH_MORE_DATA {
                return None;
            }
            // u64-aligned backing store: items hold f64s, and PDH packs
            // the instance-name strings after the item array.
            let mut buf = vec![0u64; (bytes as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, Some(items)) != 0 {
                return None;
            }
            let items = std::slice::from_raw_parts(items, count as usize);
            Some(
                items
                    .iter()
                    .filter(|it| it.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA)
                    .map(|it| it.FmtValue.Anonymous.doubleValue)
                    .sum(),
            )
        }
    }

    /// Dedicated VRAM of the biggest hardware adapter (the one games run
    /// on in every hybrid-graphics setup we care about).
    fn largest_dedicated_vram() -> Option<u64> {
        unsafe {
            let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let mut best = None;
            let mut i = 0;
            while let Ok(a) = f.EnumAdapters1(i) {
                i += 1;
                let mut d = DXGI_ADAPTER_DESC1::default();
                if a.GetDesc1(&mut d).is_err() || d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                best = best.max(Some(d.DedicatedVideoMemory as u64));
            }
            best.filter(|&b| b > 0)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Snapshot;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    pub(super) fn worker(_shared: Arc<Mutex<Snapshot>>, _stop: Arc<AtomicBool>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_from_deltas() {
        // 1000 ticks elapsed (kernel 600 incl. idle 250, user 400): busy 75 %.
        assert_eq!(cpu_pct((0, 0, 0), (250, 600, 400)), Some(75.0));
        assert_eq!(cpu_pct((5, 5, 5), (5, 5, 5)), None);
        // Counter went backwards (shouldn't happen) → no reading, no panic.
        assert_eq!(cpu_pct((10, 10, 10), (5, 5, 5)), None);
    }

    #[test]
    fn bytes_format() {
        assert_eq!(format_bytes(512 << 20), "512 MB");
        assert_eq!(format_bytes(16 << 30), "16.0 GB");
        assert_eq!(format_bytes((3 << 30) / 2), "1.5 GB");
    }

    /// `cargo test -p rsoverlay-sysinfo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_snapshot() {
        let s = Sampler::spawn();
        std::thread::sleep(INTERVAL * 2 + Duration::from_millis(300));
        let snap = s.snapshot();
        println!("{snap:#?}");
        assert!(snap.ram_total > 0);
        assert!(snap.cpu_pct.is_some());
    }
}
