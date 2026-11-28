//! Alert sounds: notifications (default / warning / error) and low
//! battery. Each kind plays a built-in synthesised sound, nothing, or a
//! WAV the user dropped into the sounds folder (`sounds/*.wav`).
//!
//! **Security:** the XSOverlay-compatible notification API lets any local
//! client send `audioPath`, including absolute or UNC paths. Playing a
//! client-supplied path would let a client make Windows open
//! `\\host\share\x.wav` (leaking the user's NTLM hash) or probe local
//! files. So a client only ever picks a *category* via
//! [`AlertKind::from_audio_path`]; which file plays is the user's choice,
//! restricted to plain file names inside the sounds folder.

use crate::typing::{scale_pcm16, valid_pack_name, MAX_WAV_BYTES};
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Notification,
    Warning,
    Error,
    LowBattery,
}

impl AlertKind {
    pub const ALL: [AlertKind; 4] = [Self::Notification, Self::Warning, Self::Error, Self::LowBattery];

    pub fn label(self) -> &'static str {
        match self {
            Self::Notification => "Notification",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::LowBattery => "Low battery",
        }
    }

    /// Category for a notification's `audioPath`. `None` = silent.
    /// Anything that isn't a known keyword (absolute paths, URLs, UNC
    /// shares, garbage) plays the default notification sound — the path
    /// itself is never opened.
    pub fn from_audio_path(p: &str) -> Option<AlertKind> {
        match p.trim().to_ascii_lowercase().as_str() {
            "none" | "silent" | "off" => None,
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => Some(Self::Notification),
        }
    }
}

/// What a slot plays: `"builtin"`, `"none"`, or a WAV file name inside
/// the sounds folder.
pub const BUILTIN: &str = "builtin";
pub const NONE: &str = "none";

/// A user sound choice that names a file: plain name, `.wav`, no path
/// separators / drive / parent references.
pub fn valid_sound_file(name: &str) -> bool {
    valid_pack_name(name) && name.to_ascii_lowercase().ends_with(".wav")
}

/// WAV files directly inside `dir`, sorted (what the slot pickers offer).
pub fn list_sound_files(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_sound_file(n))
        .collect();
    files.sort_by_key(|n| n.to_lowercase());
    files
}

/// One soft tone with a bell-ish decay.
/// Built-in sound per kind: recordings from Kenney's "Interface Sounds"
/// (www.kenney.nl, CC0; `assets/kenney/License.txt`), converted to 16-bit
/// mono WAV. notification = confirmation_004 (~1.4 kHz), warning =
/// question_002 (~700 Hz), error = error_006 (low buzz), low battery =
/// minimize_006 (soft falling tone, ~380 Hz). Chosen to stay mellow:
/// no shrill high tones (see `no_builtin_is_shrill`).
fn builtin_wav(kind: AlertKind) -> &'static [u8] {
    match kind {
        AlertKind::Notification => include_bytes!("../assets/kenney/notification.wav"),
        AlertKind::Warning => include_bytes!("../assets/kenney/warning.wav"),
        AlertKind::Error => include_bytes!("../assets/kenney/error.wav"),
        AlertKind::LowBattery => include_bytes!("../assets/kenney/low_battery.wav"),
    }
}

/// The built-in sound for `kind` at `volume`.
pub fn builtin(kind: AlertKind, volume: f32) -> Vec<u8> {
    scale_pcm16(builtin_wav(kind), volume).unwrap_or_default()
}

/// The WAV a slot should play at `volume`, or `None` for silence.
/// A custom file that's missing, oversized, not a valid name or not
/// PCM16 falls back to the built-in sound (never to the raw file, so
/// volume always applies and unparsed data never reaches winmm).
pub fn render(kind: AlertKind, choice: &str, sounds_dir: &Path, volume: f32) -> Option<Vec<u8>> {
    let volume = volume.clamp(0.0, 1.0);
    if choice == NONE || volume <= 0.0 {
        return None;
    }
    if choice != BUILTIN && valid_sound_file(choice) {
        let p = sounds_dir.join(choice);
        let custom = std::fs::metadata(&p)
            .ok()
            .filter(|m| m.is_file() && m.len() <= MAX_WAV_BYTES)
            .and_then(|_| std::fs::read(&p).ok())
            .and_then(|w| scale_pcm16(&w, volume));
        match custom {
            Some(w) => return Some(w),
            None => tracing::warn!("alert sound {:?} unusable (missing, too big or not 16-bit PCM) — built-in used", p.display()),
        }
    }
    Some(builtin(kind, volume))
}

/// Buffer currently handed to winmm. Replaced only *after* the next
/// `PlaySoundW` call (which stops the previous sound first), so the
/// memory winmm reads always outlives its playback.
static PLAYING: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Play an owned WAV asynchronously (cuts off whatever is playing).
pub fn play(wav: Vec<u8>) {
    let Ok(mut slot) = PLAYING.lock() else { return };
    crate::typing::play_bytes(&wav);
    // Moving the Vec doesn't move its heap buffer: the pointer winmm got
    // stays valid while the Vec lives in the slot.
    *slot = Some(wav);
}

/// Render + play in one go. Alerts are rare, so reading a custom file
/// each time keeps edits to the sounds folder live with no cache.
pub fn play_alert(kind: AlertKind, choice: &str, sounds_dir: &Path, volume: f32) {
    if let Some(w) = render(kind, choice, sounds_dir, volume) {
        play(w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_pick_a_category_never_a_path() {
        assert_eq!(AlertKind::from_audio_path("default"), Some(AlertKind::Notification));
        assert_eq!(AlertKind::from_audio_path("Warning"), Some(AlertKind::Warning));
        assert_eq!(AlertKind::from_audio_path("error"), Some(AlertKind::Error));
        assert_eq!(AlertKind::from_audio_path("none"), None);
        for hostile in [r"\\evil\share\x.wav", r"C:\Windows\Media\ding.wav", "../../x.wav", "http://x/y.wav"] {
            assert_eq!(AlertKind::from_audio_path(hostile), Some(AlertKind::Notification), "{hostile}");
        }
    }

    #[test]
    fn sound_file_names_stay_inside_the_folder() {
        assert!(valid_sound_file("ping.wav"));
        assert!(valid_sound_file("Ping.WAV"));
        for bad in ["ping.mp3", "../ping.wav", r"a\ping.wav", "C:ping.wav", ".wav/..", ""] {
            assert!(!valid_sound_file(bad), "{bad}");
        }
    }

    #[test]
    fn slots_render_builtin_none_and_fall_back() {
        let dir = std::env::temp_dir().join(format!("rso-alerts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Custom PCM16 file is used (and scaled).
        let custom = crate::typing::wav_pcm16_mono(&[1000, -1000], 8000);
        std::fs::write(dir.join("mine.wav"), &custom).unwrap();
        // Not a WAV at all.
        std::fs::write(dir.join("junk.wav"), b"hello").unwrap();
        for k in AlertKind::ALL {
            assert!(render(k, NONE, &dir, 1.0).is_none());
            assert!(render(k, BUILTIN, &dir, 0.0).is_none(), "volume 0 = silent");
            let b = render(k, BUILTIN, &dir, 0.5).unwrap();
            assert_eq!(&b[..4], b"RIFF");
            let m = render(k, "mine.wav", &dir, 0.5).unwrap();
            assert_eq!(m.len(), custom.len());
            assert_eq!(i16::from_le_bytes([m[44], m[45]]), 500);
            assert_eq!(render(k, "junk.wav", &dir, 0.5).unwrap(), b, "junk → built-in");
            assert_eq!(render(k, "missing.wav", &dir, 0.5).unwrap(), b, "missing → built-in");
            assert_eq!(render(k, r"..\..\x.wav", &dir, 0.5).unwrap(), b, "bad name → built-in");
        }
        assert_eq!(list_sound_files(&dir), vec!["junk.wav".to_string(), "mine.wav".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builtins_differ_per_kind() {
        let all: Vec<Vec<u8>> = AlertKind::ALL.iter().map(|k| builtin(*k, 0.6)).collect();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }

    /// Tester: the first low-battery sound (a 7 kHz glass ping) was "really
    /// squeaky". Guard every built-in against high, thin tones: the energy
    /// of the signal's first difference vs the signal itself grows with
    /// frequency, (2 sin(pi f / fs))^2 for a sine, about 0.18 at 3 kHz.
    #[test]
    fn no_builtin_is_shrill() {
        for k in AlertKind::ALL {
            let w = builtin_wav(k);
            let x: Vec<f64> = w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f64).collect();
            let e: f64 = x.iter().map(|v| v * v).sum::<f64>().max(1.0);
            let d: f64 = x.windows(2).map(|p| (p[1] - p[0]).powi(2)).sum();
            assert!(d / e < 0.18, "{k:?} is mostly above ~3 kHz ({:.2})", d / e);
            let secs = x.len() as f64 / 44_100.0;
            assert!((0.2..1.0).contains(&secs), "{k:?} lasts {secs:.2} s");
        }
    }
}
