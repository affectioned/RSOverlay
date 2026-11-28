//! Microphone recording for voice-to-text: WASAPI shared-mode capture,
//! converted by Windows to 16 kHz mono float (what Whisper wants).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Sample rate of [`Recorder::finish`]'s output.
pub const RATE: u32 = 16_000;
/// Recording stops by itself after this long.
pub const MAX_SECONDS: u32 = 60;

/// A capture endpoint (Settings › Keyboard › Microphone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicDevice {
    /// Endpoint id (stable across reboots).
    pub id: String,
    pub name: String,
}

/// Every active microphone, by friendly name.
pub fn devices() -> Vec<MicDevice> {
    #[cfg(windows)]
    {
        imp::devices().unwrap_or_else(|e| {
            tracing::warn!("mic: listing devices: {e}");
            Vec::new()
        })
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// A recording in progress. Dropping it stops and discards.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    /// Peak of the last ~50 ms, f32 bits (UI level meter).
    level: Arc<AtomicU32>,
    /// Set once the capture thread has stopped on its own (time limit
    /// or a device error).
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<Vec<f32>, String>>>,
}

impl Recorder {
    /// Start recording from `device` (an id from [`devices`]; `None` or
    /// an id that's gone = the Windows default communications mic).
    pub fn start(device: Option<String>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (s, l, d) = (stop.clone(), level.clone(), done.clone());
        let thread = std::thread::Builder::new()
            .name("mic".into())
            .spawn(move || {
                #[cfg(windows)]
                let r = imp::record(device.as_deref(), &s, &l).map_err(|e| e.to_string());
                #[cfg(not(windows))]
                let r = {
                    let _ = (device, &s, &l);
                    Err("microphone capture is Windows-only".to_string())
                };
                d.store(true, Ordering::Release);
                r
            })
            .ok();
        Self { stop, level, done, thread }
    }

    /// Peak level 0..1 of the last few milliseconds.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// The capture stopped by itself (time limit, device unplugged).
    pub fn ended(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// Stop and return the audio: 16 kHz mono, -1..1.
    pub fn finish(mut self) -> Result<Vec<f32>, String> {
        self.stop.store(true, Ordering::Release);
        match self.thread.take() {
            Some(t) => t.join().map_err(|_| "mic thread panicked".to_string())?,
            None => Err("couldn't start the mic thread".into()),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Peak absolute value.
pub(crate) fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[cfg(windows)]
mod imp {
    use super::{peak, MicDevice, MAX_SECONDS, RATE};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use windows::core::{Result, HSTRING, PCWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

    fn enumerator() -> Result<IMMDeviceEnumerator> {
        // SAFETY: COM init on this thread (S_FALSE/RPC_E_CHANGED_MODE are
        // fine: COM is usable either way), then a plain CoCreateInstance.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        }
    }

    fn name(d: &IMMDevice) -> Option<String> {
        // SAFETY: property store read; the PROPVARIANT holds a VT_LPWSTR
        // for the friendly name, read before it is dropped.
        unsafe {
            let store = d.OpenPropertyStore(STGM_READ).ok()?;
            let v = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
            let p = v.Anonymous.Anonymous.Anonymous.pwszVal;
            (!p.is_null()).then(|| p.to_string().ok()).flatten()
        }
    }

    pub(super) fn devices() -> Result<Vec<MicDevice>> {
        let e = enumerator()?;
        // SAFETY: COM calls on live interfaces.
        unsafe {
            let list = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
            let mut out = Vec::new();
            for i in 0..list.GetCount()? {
                let d = list.Item(i)?;
                let id = d.GetId()?;
                let id_s = id.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
                out.push(MicDevice { name: name(&d).unwrap_or_else(|| id_s.clone()), id: id_s });
            }
            out.sort_by_key(|a| a.name.to_lowercase());
            Ok(out)
        }
    }

    pub(super) fn record(device: Option<&str>, stop: &AtomicBool, level: &AtomicU32) -> Result<Vec<f32>> {
        let e = enumerator()?;
        // SAFETY: WASAPI shared-mode capture; every buffer from GetBuffer
        // is read within its frame count and released before the next.
        unsafe {
            let dev = match device.and_then(|id| e.GetDevice(PCWSTR(HSTRING::from(id).as_ptr())).ok()) {
                Some(d) => d,
                None => e.GetDefaultAudioEndpoint(eCapture, eCommunications)?,
            };
            tracing::info!("mic: recording from {}", name(&dev).unwrap_or_default());
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
            // Windows converts whatever the mic delivers to this.
            let fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 4,
                nBlockAlign: 4,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            // 200 ms buffer (100-ns units).
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                2_000_000,
                0,
                &fmt,
                None,
            )?;
            let cap: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            let max = (RATE * MAX_SECONDS) as usize;
            let mut out: Vec<f32> = Vec::with_capacity(RATE as usize * 10);
            while !stop.load(Ordering::Acquire) && out.len() < max {
                std::thread::sleep(std::time::Duration::from_millis(15));
                loop {
                    let n = cap.GetNextPacketSize()?;
                    if n == 0 {
                        break;
                    }
                    let mut data = std::ptr::null_mut();
                    let (mut frames, mut flags) = (0u32, 0u32);
                    cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                    let start = out.len();
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        out.resize(start + frames as usize, 0.0);
                    } else {
                        out.extend_from_slice(std::slice::from_raw_parts(data as *const f32, frames as usize));
                    }
                    cap.ReleaseBuffer(frames)?;
                    level.store(peak(&out[start..]).min(1.0).to_bits(), Ordering::Relaxed);
                }
            }
            let _ = client.Stop();
            out.truncate(max);
            tracing::info!("mic: {:.1} s recorded", out.len() as f32 / RATE as f32);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn peak_is_the_loudest_sample() {
        assert_eq!(super::peak(&[0.1, -0.7, 0.3]), 0.7);
        assert_eq!(super::peak(&[]), 0.0);
    }

    /// Records 1 s from the default mic (needs a microphone).
    #[test]
    #[ignore]
    fn records_from_the_default_mic() {
        let r = super::Recorder::start(None);
        std::thread::sleep(std::time::Duration::from_secs(1));
        let s = r.finish().unwrap();
        assert!(s.len() > 12_000, "{} samples", s.len());
        println!("devices: {:?}", super::devices());
    }
}
