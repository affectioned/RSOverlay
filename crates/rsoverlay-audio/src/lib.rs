//! System volume mixer — master endpoint + per-application sessions.
//!
//! All WASAPI objects live on one dedicated MTA worker thread; the UI
//! thread never touches COM. The UI reads a [`Snapshot`] (refreshed every
//! [`REFRESH`]) and sends commands over a channel. Commands are also
//! applied optimistically to the snapshot, so a dragged slider doesn't
//! snap back while the worker catches up.

pub mod alerts;
pub mod mic;
pub mod typing;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REFRESH: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    /// 0.0..=1.0 scalar (perceptual, same as the Windows volume flyout).
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppSession {
    /// 0 = the system-sounds session.
    pub pid: u32,
    pub name: String,
    pub level: Level,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// `None` when there is no default render endpoint (no audio device).
    pub master: Option<Level>,
    /// Sorted by name; one entry per process even when the process owns
    /// several sessions (browsers do).
    pub apps: Vec<AppSession>,
}

#[derive(Clone, Copy, Debug)]
enum Cmd {
    MasterVolume(f32),
    MasterMute(bool),
    AppVolume(u32, f32),
    AppMute(u32, bool),
}

pub struct Mixer {
    snapshot: Arc<Mutex<Snapshot>>,
    tx: mpsc::Sender<Cmd>,
}

impl Mixer {
    /// Spawn the worker. The thread exits when the `Mixer` is dropped.
    pub fn spawn() -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (tx, rx) = mpsc::channel();
        let shared = snapshot.clone();
        let spawned = std::thread::Builder::new()
            .name("rsoverlay-audio".into())
            .spawn(move || imp::worker(rx, shared));
        if let Err(e) = spawned {
            tracing::warn!("audio mixer thread failed to start: {e}");
        }
        Self { snapshot, tx }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_master_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.apply(Cmd::MasterVolume(v), |s| {
            if let Some(m) = &mut s.master {
                m.volume = v;
            }
        });
    }

    pub fn set_master_mute(&self, muted: bool) {
        self.apply(Cmd::MasterMute(muted), |s| {
            if let Some(m) = &mut s.master {
                m.muted = muted;
            }
        });
    }

    pub fn set_app_volume(&self, pid: u32, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.apply(Cmd::AppVolume(pid, v), |s| {
            s.apps.iter_mut().filter(|a| a.pid == pid).for_each(|a| a.level.volume = v);
        });
    }

    pub fn set_app_mute(&self, pid: u32, muted: bool) {
        self.apply(Cmd::AppMute(pid, muted), |s| {
            s.apps.iter_mut().filter(|a| a.pid == pid).for_each(|a| a.level.muted = muted);
        });
    }

    fn apply(&self, cmd: Cmd, optimistic: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.snapshot.lock() {
            optimistic(&mut s);
        }
        let _ = self.tx.send(cmd);
    }
}

/// The mixer session of the app a media session belongs to. `source` is
/// the media session's app id: `Spotify.exe`, `chrome`, or a Store-app
/// id like `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`; app names are
/// process names (`Spotify`). Matches when one contains the other after
/// lower-casing and dropping punctuation / `.exe`; the longest name wins.
/// None when nothing matches (the caller falls back to the master
/// volume). Pure.
pub fn match_app<'a>(source: &str, apps: &'a [AppSession]) -> Option<&'a AppSession> {
    let norm = |s: &str| -> String {
        let s = s.to_lowercase();
        let s = s.strip_suffix(".exe").unwrap_or(&s);
        s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    };
    let src = norm(source);
    if src.is_empty() {
        return None;
    }
    apps.iter()
        .filter(|a| a.pid != 0)
        .filter(|a| {
            let n = norm(&a.name);
            n.len() >= 3 && (src.contains(&n) || n.contains(&src))
        })
        .max_by_key(|a| norm(&a.name).len())
}

/// `C:\Program Files\Spotify\Spotify.exe` → `Spotify`. Pure so it's
/// testable off-Windows.
pub fn app_label(image_path: &str) -> String {
    let file = image_path.rsplit(['\\', '/']).next().unwrap_or(image_path);
    let stem = match file.rfind('.') {
        Some(i) if i > 0 => &file[..i],
        _ => file,
    };
    stem.to_string()
}

/// Collapse per-session rows into one row per pid (first session's
/// level wins — Windows keeps a process's sessions in lockstep when
/// changed through the mixer) and sort for a stable UI order.
fn dedupe(mut sessions: Vec<AppSession>) -> Vec<AppSession> {
    let mut seen = std::collections::HashSet::new();
    sessions.retain(|s| seen.insert(s.pid));
    sessions.sort_by_key(|s| (s.name.to_lowercase(), s.pid));
    // Several processes with one name (Discord, browsers): number them
    // so two identical rows don't look like a bug.
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for s in &mut sessions {
        let n = count.entry(s.name.to_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            s.name = format!("{} {}", s.name, n);
        }
    }
    sessions
}

#[cfg(windows)]
mod imp {
    use super::{app_label, dedupe, AppSession, Cmd, Level, Snapshot, REFRESH};
    use std::sync::{mpsc, Arc, Mutex};
    use windows::core::{ComInterface, Result, GUID};
    use windows::Win32::Foundation::{CloseHandle, S_OK};
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
        IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub(super) fn worker(rx: mpsc::Receiver<Cmd>, shared: Arc<Mutex<Snapshot>>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("audio: MMDeviceEnumerator unavailable: {e}");
                    return;
                }
            };
        loop {
            // Default device can change (headset plugged in) — resolve it
            // every refresh rather than caching.
            let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok();
            // Apps on every active output, not just the default: with
            // Voicemeeter etc. a player often outputs elsewhere (tester:
            // Apple Music's volume didn't change from the wrist).
            let all = active_outputs(&enumerator);
            let snap = read_snapshot(device.as_ref(), &all);
            if let Ok(mut s) = shared.lock() {
                *s = snap;
            }
            match rx.recv_timeout(REFRESH) {
                Ok(cmd) => {
                    // Drain the burst a slider drag produces; only the
                    // newest value per target matters, but applying all
                    // is cheap and keeps ordering obvious.
                    let _ = run(device.as_ref(), &all, cmd);
                    while let Ok(cmd) = rx.try_recv() {
                        let _ = run(device.as_ref(), &all, cmd);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Every active render endpoint.
    fn active_outputs(enumerator: &IMMDeviceEnumerator) -> Vec<IMMDevice> {
        let Ok(list) = (unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }) else {
            return Vec::new();
        };
        let n = unsafe { list.GetCount() }.unwrap_or(0);
        (0..n).filter_map(|i| unsafe { list.Item(i) }.ok()).collect()
    }

    /// Master from the default output; apps from every active output.
    fn read_snapshot(device: Option<&IMMDevice>, all: &[IMMDevice]) -> Snapshot {
        let master = device.and_then(|d| endpoint(d).ok()).and_then(|ep| unsafe {
            Some(Level {
                volume: ep.GetMasterVolumeLevelScalar().ok()?,
                muted: ep.GetMute().ok()?.as_bool(),
            })
        });
        // Default device first, so its level wins for apps on several.
        let ordered = device.into_iter().chain(all.iter());
        let apps = dedupe(ordered.flat_map(|d| sessions(d).unwrap_or_default().into_iter().map(|(s, _)| s)).collect());
        Snapshot { master, apps }
    }

    fn endpoint(device: &IMMDevice) -> Result<IAudioEndpointVolume> {
        unsafe { device.Activate(CLSCTX_ALL, None) }
    }

    /// Live (non-expired) sessions with their volume control.
    fn sessions(device: &IMMDevice) -> Result<Vec<(AppSession, ISimpleAudioVolume)>> {
        let mgr: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let list = unsafe { mgr.GetSessionEnumerator()? };
        let n = unsafe { list.GetCount()? };
        let mut out = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let Ok(ctl) = (unsafe { list.GetSession(i) }) else { continue };
            if unsafe { ctl.GetState() }.is_ok_and(|s| s == AudioSessionStateExpired) {
                continue;
            }
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else { continue };
            let pid = unsafe { ctl2.GetProcessId() }.unwrap_or(0);
            let system = unsafe { ctl2.IsSystemSoundsSession() } == S_OK;
            let name = if system { "System sounds".to_string() } else { process_name(pid) };
            let level = unsafe {
                Level {
                    volume: vol.GetMasterVolume().unwrap_or(0.0),
                    muted: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                }
            };
            out.push((AppSession { pid: if system { 0 } else { pid }, name, level }, vol));
        }
        Ok(out)
    }

    fn process_name(pid: u32) -> String {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return format!("pid {pid}");
            };
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(h);
            if ok {
                app_label(&String::from_utf16_lossy(&buf[..len as usize]))
            } else {
                format!("pid {pid}")
            }
        }
    }

    fn run(device: Option<&IMMDevice>, all: &[IMMDevice], cmd: Cmd) -> Result<()> {
        let ctx: *const GUID = std::ptr::null();
        unsafe {
            match cmd {
                Cmd::MasterVolume(v) => match device {
                    Some(d) => endpoint(d)?.SetMasterVolumeLevelScalar(v, ctx),
                    None => Ok(()),
                },
                Cmd::MasterMute(m) => match device {
                    Some(d) => endpoint(d)?.SetMute(m, ctx),
                    None => Ok(()),
                },
                // On whichever outputs the app plays to.
                Cmd::AppVolume(pid, v) => all.iter().try_for_each(|d| for_pid(d, pid, |vol| vol.SetMasterVolume(v, ctx))),
                Cmd::AppMute(pid, m) => all.iter().try_for_each(|d| for_pid(d, pid, |vol| vol.SetMute(m, ctx))),
            }
        }
    }

    /// Apply to every session the process owns — a browser with three
    /// tabs playing audio has three sessions under one pid.
    fn for_pid(device: &IMMDevice, pid: u32, f: impl Fn(&ISimpleAudioVolume) -> Result<()>) -> Result<()> {
        for (s, vol) in sessions(device)? {
            if s.pid == pid {
                f(&vol)?;
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Cmd, Snapshot};
    use std::sync::{mpsc, Arc, Mutex};

    pub(super) fn worker(rx: mpsc::Receiver<Cmd>, _shared: Arc<Mutex<Snapshot>>) {
        // No backend: park until the Mixer is dropped.
        while rx.recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_from_image_paths() {
        assert_eq!(app_label(r"C:\Program Files\Spotify\Spotify.exe"), "Spotify");
        assert_eq!(app_label("/usr/bin/firefox"), "firefox");
        assert_eq!(app_label("vrchat.v2.exe"), "vrchat.v2");
        assert_eq!(app_label(".hidden"), ".hidden");
    }

    #[test]
    fn dedupe_keeps_first_per_pid_and_sorts() {
        let s = |pid, name: &str, v| AppSession { pid, name: name.into(), level: Level { volume: v, muted: false } };
        let out = dedupe(vec![s(7, "zoom", 0.3), s(3, "Chrome", 0.9), s(3, "Chrome", 0.1), s(0, "System sounds", 1.0)]);
        let names: Vec<_> = out.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Chrome", "System sounds", "zoom"]);
        assert_eq!(out[0].level.volume, 0.9);
    }

    #[test]
    fn same_named_apps_are_numbered() {
        let s = |pid, name: &str| AppSession { pid, name: name.into(), level: Level { volume: 1.0, muted: false } };
        let out = dedupe(vec![s(20, "Discord"), s(10, "Discord"), s(5, "Chrome")]);
        let names: Vec<_> = out.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Chrome", "Discord", "Discord 2"]);
    }

    /// Read-only smoke test against the real default device:
    /// `cargo test -p rsoverlay-audio -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_snapshot() {
        let m = Mixer::spawn();
        std::thread::sleep(REFRESH + Duration::from_millis(300));
        let s = m.snapshot();
        println!("master: {:?}", s.master);
        for a in &s.apps {
            println!("  {:>6} {:<24} {:.2} muted={}", a.pid, a.name, a.level.volume, a.level.muted);
        }
        assert!(s.master.is_some(), "no default render endpoint");
    }

    #[test]
    fn optimistic_updates_clamp() {
        let m = Mixer {
            snapshot: Arc::new(Mutex::new(Snapshot {
                master: Some(Level { volume: 0.5, muted: false }),
                apps: vec![AppSession { pid: 4, name: "a".into(), level: Level::default() }],
            })),
            tx: mpsc::channel().0,
        };
        m.set_master_volume(1.7);
        m.set_app_mute(4, true);
        let s = m.snapshot();
        assert_eq!(s.master.unwrap().volume, 1.0);
        assert!(s.apps[0].level.muted);
    }

    #[test]
    fn media_app_finds_its_mixer_session() {
        let app = |pid, name: &str| AppSession { pid, name: name.into(), level: Level::default() };
        let apps = vec![app(0, "System sounds"), app(10, "Spotify"), app(11, "chrome"), app(12, "Discord"), app(13, "msedge")];
        let pid = |s: &str| match_app(s, &apps).map(|a| a.pid);
        assert_eq!(pid("Spotify.exe"), Some(10));
        assert_eq!(pid("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), Some(10));
        assert_eq!(pid("Chrome"), Some(11));
        assert_eq!(pid("MSEdge"), Some(13));
        // Apple Music (Store app id) → its AppleMusic process.
        let mut with_am = apps.clone();
        with_am.push(app(14, "AppleMusic"));
        assert_eq!(match_app("AppleInc.AppleMusicWin_nzyj5cx40ttqa!App", &with_am).map(|a| a.pid), Some(14));
        // Firefox reports an opaque id: no match → master volume.
        assert_eq!(pid("308046B0AF4A39CB"), None);
        assert_eq!(pid(""), None);
        // Never the system-sounds session.
        assert_eq!(pid("System sounds"), None);
    }
}
