//! Typing sounds for the VR keyboard.
//!
//! Built-in clicks are synthesised at runtime (no shipped audio, nothing
//! borrowed from other overlays). Users can swap in their own pack: a
//! folder of WAVs under `sounds/<pack>/` next to config.toml, one file per
//! [`KeySound`]; any file missing falls back to the built-in click.
//!
//! Playback is `PlaySoundW(SND_MEMORY | SND_ASYNC)`: fire-and-forget, a new
//! click cuts the previous one off (fine for ~25 ms ticks), no audio
//! thread of our own. Volume is baked into the buffers, since PlaySound
//! has no gain control.

use std::path::{Path, PathBuf};

/// Which click to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeySound {
    Key,
    Space,
    Enter,
    Backspace,
    Modifier,
}

impl KeySound {
    pub const ALL: [KeySound; 5] = [Self::Key, Self::Space, Self::Enter, Self::Backspace, Self::Modifier];

    /// File name inside a pack folder.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Key => "key.wav",
            Self::Space => "space.wav",
            Self::Enter => "enter.wav",
            Self::Backspace => "backspace.wav",
            Self::Modifier => "modifier.wav",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Custom WAVs larger than this are ignored (a click is a few KB).
pub const MAX_WAV_BYTES: u64 = 2 << 20;
const RATE: u32 = 44_100;

/// Shape of one synthesised click.
struct Click {
    len_s: f32,
    /// Bright "tick" tone (Hz) and its weight.
    tone: f32,
    tone_w: f32,
    /// Low "body" thump (Hz) and its weight.
    body: f32,
    body_w: f32,
    noise_w: f32,
    /// Exponential decay time constant (s).
    tau: f32,
    /// Overall level relative to a normal key.
    gain: f32,
}

fn click_shape(kind: KeySound) -> Click {
    match kind {
        KeySound::Key => Click { len_s: 0.022, tone: 2300.0, tone_w: 0.45, body: 180.0, body_w: 0.30, noise_w: 0.55, tau: 0.004, gain: 1.0 },
        KeySound::Space => Click { len_s: 0.040, tone: 900.0, tone_w: 0.25, body: 140.0, body_w: 0.60, noise_w: 0.45, tau: 0.007, gain: 1.0 },
        KeySound::Enter => Click { len_s: 0.045, tone: 1600.0, tone_w: 0.40, body: 160.0, body_w: 0.55, noise_w: 0.40, tau: 0.009, gain: 1.0 },
        KeySound::Backspace => Click { len_s: 0.025, tone: 1900.0, tone_w: 0.50, body: 170.0, body_w: 0.30, noise_w: 0.45, tau: 0.005, gain: 0.95 },
        KeySound::Modifier => Click { len_s: 0.018, tone: 2600.0, tone_w: 0.40, body: 200.0, body_w: 0.20, noise_w: 0.50, tau: 0.003, gain: 0.7 },
    }
}

/// Built-in click as a 16-bit mono 44.1 kHz WAV, at `volume` (0..=1).
/// O(n) in the click length (≤ ~2 000 samples).
pub fn builtin(kind: KeySound, volume: f32) -> Vec<u8> {
    let c = click_shape(kind);
    let n = (c.len_s * RATE as f32) as usize;
    // Deterministic LCG noise: same click every run, no rand dependency.
    let mut seed: u32 = 0x1234_5678 ^ (kind.index() as u32).wrapping_mul(0x9E37_79B9);
    let mut raw: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let tick = (std::f32::consts::TAU * c.tone * t).sin();
            let body = (std::f32::consts::TAU * c.body * t).sin() * (-t / (c.tau * 2.0)).exp();
            (c.noise_w * noise + c.tone_w * tick) * (-t / c.tau).exp() + c.body_w * body
        })
        .collect();
    // Short fade-in so the click doesn't start on a DC step (pops).
    for (i, s) in raw.iter_mut().take(16).enumerate() {
        *s *= i as f32 / 16.0;
    }
    let peak = raw.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    let scale = 0.9 * c.gain * volume.clamp(0.0, 1.0) * i16::MAX as f32 / peak;
    let samples: Vec<i16> = raw.iter().map(|s| (s * scale).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).collect();
    wav_pcm16_mono(&samples, RATE)
}

/// Low-battery chime: two soft descending tones (≈ 0.4 s), synthesised.
pub fn alert_chime(volume: f32) -> Vec<u8> {
    let note = |f: f32, len_s: f32| -> Vec<f32> {
        let n = (len_s * RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                // Soft attack, bell-like decay, a touch of 2nd harmonic.
                let env = (t / 0.008).min(1.0) * (-t / 0.09).exp();
                env * ((std::f32::consts::TAU * f * t).sin() + 0.25 * (std::f32::consts::TAU * 2.0 * f * t).sin())
            })
            .collect()
    };
    let mut raw = note(880.0, 0.18);
    raw.extend(note(660.0, 0.24));
    let peak = raw.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    let scale = 0.85 * volume.clamp(0.0, 1.0) * i16::MAX as f32 / peak;
    let samples: Vec<i16> = raw.iter().map(|s| (s * scale).round() as i16).collect();
    wav_pcm16_mono(&samples, RATE)
}

/// Minimal RIFF/WAVE container around mono PCM16 samples.
pub(crate) fn wav_pcm16_mono(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// A user WAV with its samples scaled by `volume`, if it's PCM16 (any
/// channel count / rate). Every chunk offset and length is bounds-checked
/// against the buffer, so a truncated or hostile file yields `None`, never
/// an out-of-range read. O(n) in the file size.
pub fn scale_pcm16(wav: &[u8], volume: f32) -> Option<Vec<u8>> {
    if wav.get(0..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut out = wav.to_vec();
    let mut at = 12usize;
    let mut pcm16 = false;
    let gain = volume.clamp(0.0, 1.0);
    while at.checked_add(8)? <= wav.len() {
        let id = wav.get(at..at + 4)?;
        let len = u32_at(wav, at + 4)? as usize;
        let body = at + 8;
        let end = body.checked_add(len)?;
        match id {
            b"fmt " => {
                let format = u16_at(wav, body)?;
                let bits = u16_at(wav, body + 14)?;
                pcm16 = format == 1 && bits == 16;
            }
            b"data" => {
                if !pcm16 {
                    return None;
                }
                // Some writers overstate the data length; clamp to the file.
                let end = end.min(wav.len());
                let mut i = body;
                while i + 2 <= end {
                    let s = i16::from_le_bytes([wav[i], wav[i + 1]]);
                    let v = (s as f32 * gain).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                    out[i..i + 2].copy_from_slice(&v.to_le_bytes());
                    i += 2;
                }
                return Some(out);
            }
            _ => {}
        }
        // Chunks are word-aligned.
        at = end.checked_add(len & 1)?;
    }
    None
}

/// Pack folder names are plain names: no separators, no `.` / `..`, so a
/// config value can't point outside the sounds folder.
pub fn valid_pack_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':'])
}

/// Sound packs available under `dir` (sub-folders with at least one of
/// the expected WAVs), sorted.
pub fn list_packs(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut packs: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_pack_name(n))
        .filter(|n| KeySound::ALL.iter().any(|k| dir.join(n).join(k.file_name()).is_file()))
        .collect();
    packs.sort();
    packs
}

/// The VR keyboard's sound bank + player. Single-threaded: lives on the
/// VR loop. Buffers are only replaced after stopping playback, because
/// `SND_MEMORY | SND_ASYNC` reads them while the click plays.
pub struct TypingSounds {
    /// Raw pack files (unscaled), per [`KeySound`]; `None` = built-in.
    raw: [Option<Vec<u8>>; 5],
    /// Ready-to-play WAVs at the current volume.
    bank: [Vec<u8>; 5],
    pack: Option<String>,
    volume: f32,
}

impl Default for TypingSounds {
    fn default() -> Self {
        Self::new()
    }
}

impl TypingSounds {
    pub fn new() -> Self {
        let volume = 0.4;
        Self { raw: Default::default(), bank: KeySound::ALL.map(|k| builtin(k, volume)), pack: None, volume }
    }

    /// Apply the current settings; cheap when nothing changed.
    /// `pack = None` → built-in clicks.
    pub fn configure(&mut self, sounds_dir: &Path, pack: Option<&str>, volume: f32) {
        let pack = pack.filter(|p| valid_pack_name(p)).map(str::to_owned);
        let volume = volume.clamp(0.0, 1.0);
        let pack_changed = pack != self.pack;
        if !pack_changed && (volume - self.volume).abs() < 0.005 {
            return;
        }
        stop();
        if pack_changed {
            let folder: Option<PathBuf> = pack.as_ref().map(|p| sounds_dir.join(p));
            self.raw = KeySound::ALL.map(|k| folder.as_ref().and_then(|f| read_capped(&f.join(k.file_name()))));
            self.pack = pack;
        }
        self.volume = volume;
        for k in KeySound::ALL {
            let i = k.index();
            self.bank[i] = self.raw[i].as_deref().and_then(|w| scale_pcm16(w, volume)).unwrap_or_else(|| builtin(k, volume));
        }
    }

    pub fn play(&self, kind: KeySound) {
        play_bytes(&self.bank[kind.index()]);
    }
}

impl Drop for TypingSounds {
    fn drop(&mut self) {
        // The buffers are about to be freed: make sure nothing reads them.
        stop();
    }
}

fn read_capped(p: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(p).ok()?.len();
    if len > MAX_WAV_BYTES {
        tracing::warn!("typing sound {} is over {} bytes — ignored", p.display(), MAX_WAV_BYTES);
        return None;
    }
    std::fs::read(p).ok()
}

#[cfg(windows)]
pub(crate) fn play_bytes(wav: &[u8]) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    // SAFETY: with SND_MEMORY the "name" is a pointer to an in-memory WAV,
    // read by winmm as bytes (never dereferenced as u16 by us, so the
    // alignment of the cast doesn't matter). The buffer is owned by
    // `TypingSounds` and only freed/replaced after `stop()`, so it
    // outlives the asynchronous playback.
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr().cast()), HMODULE::default(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(windows)]
fn stop() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_FLAGS};
    // SAFETY: a null sound name stops any sound this process is playing.
    unsafe {
        let _ = PlaySoundW(PCWSTR::null(), HMODULE::default(), SND_FLAGS(0));
    }
}

#[cfg(not(windows))]
pub(crate) fn play_bytes(_wav: &[u8]) {}

#[cfg(not(windows))]
fn stop() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_clicks_are_valid_short_pcm16_and_scale_with_volume() {
        for k in KeySound::ALL {
            let loud = builtin(k, 1.0);
            assert_eq!(&loud[0..4], b"RIFF");
            assert!(loud.len() > 44 && loud.len() < 44 + 2 * 44_100 / 10, "{k:?} should be < 100 ms");
            let peak = |w: &[u8]| w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c).unsigned_abs()).max().unwrap();
            assert!(peak(&loud) > 20_000);
            assert!(peak(&builtin(k, 0.25)) < peak(&loud) / 3);
            assert_eq!(peak(&builtin(k, 0.0)), 0);
        }
    }

    #[test]
    fn alert_chime_is_a_short_valid_wav() {
        let w = alert_chime(0.6);
        assert_eq!(&w[0..4], b"RIFF");
        let secs = (w.len() - 44) as f32 / 2.0 / 44_100.0;
        assert!((0.35..0.5).contains(&secs), "{secs}");
        assert!(scale_pcm16(&w, 0.5).is_some(), "parses as PCM16");
    }

    #[test]
    fn scaling_a_pcm16_wav_halves_samples_and_keeps_the_header() {
        let w = wav_pcm16_mono(&[1000, -2000, 32767], 8000);
        let s = scale_pcm16(&w, 0.5).unwrap();
        assert_eq!(&s[..44], &w[..44]);
        let v: Vec<i16> = s[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
        assert_eq!(v, vec![500, -1000, 16384]);
    }

    #[test]
    fn malformed_wavs_are_rejected_without_panicking() {
        let good = wav_pcm16_mono(&[1, 2, 3], 8000);
        assert!(scale_pcm16(b"", 1.0).is_none());
        assert!(scale_pcm16(b"RIFF\0\0\0\0WAVE", 1.0).is_none());
        // Every truncation of a valid file: either rejected or in-bounds.
        for n in 0..good.len() {
            let _ = scale_pcm16(&good[..n], 1.0);
        }
        // Chunk length pointing far past the end.
        let mut huge = good.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(scale_pcm16(&huge, 1.0).is_none());
        // 8-bit PCM isn't scaled (falls back to built-in).
        let mut eight = good;
        eight[34..36].copy_from_slice(&8u16.to_le_bytes());
        assert!(scale_pcm16(&eight, 1.0).is_none());
    }

    /// Plays every built-in click synchronously and checks winmm accepted
    /// the generated WAV. Ignored by default: it makes noise.
    #[cfg(windows)]
    #[test]
    #[ignore = "plays audio"]
    fn winmm_accepts_builtin_clicks() {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
        let clips = KeySound::ALL.map(|k| (format!("{k:?}"), builtin(k, 0.3)));
        for (name, w) in clips.into_iter().chain([("chime".to_string(), alert_chime(0.4))]) {
            // SAFETY: synchronous play; `w` outlives the call.
            let ok = unsafe { PlaySoundW(PCWSTR(w.as_ptr().cast()), HMODULE::default(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
            assert!(ok.as_bool(), "{name} rejected by PlaySoundW");
        }
    }

    #[test]
    fn pack_names_cannot_escape_the_sounds_folder() {
        assert!(valid_pack_name("Thocky"));
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:"] {
            assert!(!valid_pack_name(bad), "{bad}");
        }
    }

    #[test]
    fn list_packs_finds_folders_with_wavs_only() {
        let dir = std::env::temp_dir().join(format!("rso-sounds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Clacky")).unwrap();
        std::fs::create_dir_all(dir.join("Empty")).unwrap();
        std::fs::write(dir.join("Clacky").join("key.wav"), builtin(KeySound::Key, 0.5)).unwrap();
        assert_eq!(list_packs(&dir), vec!["Clacky".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
