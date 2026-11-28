//! Typing sounds for the VR keyboard.
//!
//! The built-in sounds are recordings of a real keyboard (see
//! [`KEYCHRON_CREDIT`]). Users can swap in their own pack: a folder of WAVs
//! under `sounds/<pack>/` next to config.toml, one file per [`KeySound`];
//! any file missing falls back to the recorded one.
//!
//! Playback is `PlaySoundW(SND_MEMORY | SND_ASYNC)`: fire-and-forget, a new
//! click cuts the previous one off (fine for ~25 ms ticks), no audio
//! thread of our own. Volume is baked into the buffers, since PlaySound
//! has no gain control.

use std::path::Path;

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

/// The built-in sound for `kind` at `volume`: its first recorded take.
pub fn builtin(kind: KeySound, volume: f32) -> Vec<u8> {
    scale_pcm16(keychron_takes(kind)[0], volume).unwrap_or_default()
}

/// Minimal RIFF/WAVE container around mono PCM16 samples (tests build
/// sample files with it).
#[cfg(test)]
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

/// Most takes of one key a pack may hold: `key.wav`, `key2.wav` ..
/// `key8.wav` (played in a shuffled order).
pub const MAX_VARIANTS: usize = 8;

/// File name of take `n` (1-based) of a key: `key.wav`, `key2.wav`, ...
pub fn variant_file(kind: KeySound, n: usize) -> String {
    let base = kind.file_name();
    if n <= 1 {
        base.to_string()
    } else {
        format!("{}{n}.wav", base.trim_end_matches(".wav"))
    }
}

/// The built-in recorded pack: a Keychron V1 Ultra (red linear switches)
/// cut from "Typing on Keychron V1 Ultra (Red Linear Switch).wav" by
/// C40115, Wikimedia Commons, CC BY 4.0
/// (https://commons.wikimedia.org/wiki/File:Typing_on_Keychron_V1_Ultra_(Red_Linear_Switch).wav):
/// single strokes cut out, levelled and faded. Per [`KeySound`], its takes.
fn keychron_takes(kind: KeySound) -> &'static [&'static [u8]] {
    match kind {
        KeySound::Key => &[
            include_bytes!("../assets/keychron/key.wav"),
            include_bytes!("../assets/keychron/key2.wav"),
            include_bytes!("../assets/keychron/key3.wav"),
            include_bytes!("../assets/keychron/key4.wav"),
            include_bytes!("../assets/keychron/key5.wav"),
            include_bytes!("../assets/keychron/key6.wav"),
        ],
        KeySound::Space => &[
            include_bytes!("../assets/keychron/space.wav"),
            include_bytes!("../assets/keychron/space2.wav"),
            include_bytes!("../assets/keychron/space3.wav"),
        ],
        KeySound::Enter => &[include_bytes!("../assets/keychron/enter.wav")],
        KeySound::Backspace => &[include_bytes!("../assets/keychron/backspace.wav")],
        KeySound::Modifier => &[include_bytes!("../assets/keychron/modifier.wav")],
    }
}

/// Credit line for the recorded pack (CC BY 4.0 asks for it).
pub const KEYCHRON_CREDIT: &str = "Keychron V1 Ultra recording by C40115 (Wikimedia Commons), CC BY 4.0";

/// The VR keyboard's sound bank + player. Single-threaded: lives on the
/// VR loop. Buffers are only replaced after stopping playback, because
/// `SND_MEMORY | SND_ASYNC` reads them while the click plays.
pub struct TypingSounds {
    /// Raw takes (unscaled) per [`KeySound`]; empty = the recorded default.
    raw: [Vec<Vec<u8>>; 5],
    /// Ready-to-play WAVs at the current volume, at least one per key.
    bank: [Vec<Vec<u8>>; 5],
    /// Last take played per key, and a little PRNG to shuffle takes.
    last: [std::cell::Cell<usize>; 5],
    seed: std::cell::Cell<u32>,
    /// None until the first `configure`.
    pack: Option<Option<String>>,
    volume: f32,
}

impl Default for TypingSounds {
    fn default() -> Self {
        Self::new()
    }
}

impl TypingSounds {
    pub fn new() -> Self {
        let mut t = Self {
            raw: Default::default(),
            bank: Default::default(),
            last: Default::default(),
            seed: std::cell::Cell::new(0x9E37_79B9),
            pack: None,
            volume: 0.4,
        };
        t.configure(Path::new(""), None, 0.4);
        t
    }

    /// Apply the current settings; cheap when nothing changed.
    /// `pack = None` → the recorded Keychron pack; otherwise a folder under
    /// `sounds_dir`. Anything else (like the retired synthesised packs'
    /// "builtin:…" values) is the recorded pack too.
    pub fn configure(&mut self, sounds_dir: &Path, pack: Option<&str>, volume: f32) {
        let pack = pack.filter(|p| valid_pack_name(p)).map(str::to_owned);
        let volume = volume.clamp(0.0, 1.0);
        let pack_changed = self.pack.as_ref() != Some(&pack);
        if !pack_changed && (volume - self.volume).abs() < 0.005 {
            return;
        }
        stop();
        if pack_changed {
            self.raw = KeySound::ALL.map(|k| match pack.as_deref() {
                None => keychron_takes(k).iter().map(|b| b.to_vec()).collect(),
                Some(folder) => {
                    let f = sounds_dir.join(folder);
                    (1..=MAX_VARIANTS).filter_map(|n| read_capped(&f.join(variant_file(k, n)))).collect()
                }
            });
            self.pack = Some(pack);
        }
        self.volume = volume;
        for k in KeySound::ALL {
            let i = k.index();
            let takes: Vec<Vec<u8>> = self.raw[i].iter().filter_map(|w| scale_pcm16(w, volume)).collect();
            // A pack without this key falls back to the recorded takes.
            self.bank[i] = if takes.is_empty() {
                keychron_takes(k).iter().filter_map(|w| scale_pcm16(w, volume)).collect()
            } else {
                takes
            };
        }
    }

    pub fn play(&self, kind: KeySound) {
        let i = kind.index();
        let takes = &self.bank[i];
        if takes.is_empty() {
            return;
        }
        // A random take, never the same one twice in a row.
        let mut x = self.seed.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed.set(x);
        let mut n = x as usize % takes.len();
        if takes.len() > 1 && n == self.last[i].get() {
            n = (n + 1) % takes.len();
        }
        self.last[i].set(n);
        play_bytes(&takes[n]);
    }

    /// How many takes each key has (tests / diagnostics).
    pub fn takes(&self, kind: KeySound) -> usize {
        self.bank[kind.index()].len()
    }
}

impl Drop for TypingSounds {
    fn drop(&mut self) {
        // The buffers are about to be freed: make sure nothing reads them.
        stop();
    }
}

/// A pack file, if present and small enough.
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

    fn peak(w: &[u8]) -> u16 {
        samples(w).iter().map(|v| v.abs() as u16).max().unwrap_or(0)
    }

    #[test]
    fn builtin_sounds_are_short_recordings_that_scale_with_volume() {
        for k in KeySound::ALL {
            let loud = builtin(k, 1.0);
            assert_eq!(&loud[0..4], b"RIFF");
            assert!(loud.len() > 44 && loud.len() < 48_000 * 2 / 8, "{k:?} should be < 125 ms");
            assert!(peak(&loud) > 3_000, "{k:?} is audible");
            assert!(peak(&builtin(k, 0.25)) < peak(&loud) / 3);
            assert_eq!(peak(&builtin(k, 0.0)), 0);
        }
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
        for (name, w) in clips {
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

    fn samples(w: &[u8]) -> Vec<f32> {
        w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32).collect()
    }

    #[test]
    fn retired_synthesised_packs_play_the_recording() {
        let dir = std::env::temp_dir();
        let mut t = TypingSounds::new();
        for old in ["builtin:thock", "builtin:clicks"] {
            assert!(!valid_pack_name(old), "never treated as a folder");
            t.configure(&dir, Some(old), 0.4);
            assert_eq!(t.takes(KeySound::Key), 6, "{old}");
        }
    }

    #[test]
    fn recorded_keychron_pack_is_the_default_with_several_takes() {
        let t = TypingSounds::new();
        assert_eq!(t.takes(KeySound::Key), 6);
        assert_eq!(t.takes(KeySound::Space), 3);
        for k in KeySound::ALL {
            for w in keychron_takes(k) {
                // Valid 16-bit PCM the player accepts, short.
                let scaled = scale_pcm16(w, 1.0).expect("16-bit PCM WAV");
                assert!(scaled.len() < 48_000 * 2 / 8, "{k:?} take under 125 ms");
            }
        }
    }

    #[test]
    fn pack_folders_can_hold_several_takes_per_key() {
        let dir = std::env::temp_dir().join(format!("rso-pack-{}", std::process::id()));
        let pack = dir.join("Mine");
        std::fs::create_dir_all(&pack).unwrap();
        for n in ["key.wav", "key2.wav", "key3.wav", "space.wav"] {
            std::fs::write(pack.join(n), builtin(KeySound::Key, 1.0)).unwrap();
        }
        let mut t = TypingSounds::new();
        t.configure(&dir, Some("Mine"), 0.5);
        assert_eq!(t.takes(KeySound::Key), 3);
        assert_eq!(t.takes(KeySound::Space), 1);
        // Missing files fall back to the recorded takes.
        assert_eq!(t.takes(KeySound::Enter), 1);
        assert_eq!(t.bank[KeySound::Enter.index()][0], builtin(KeySound::Enter, 0.5));
        assert_eq!(variant_file(KeySound::Backspace, 4), "backspace4.wav");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
