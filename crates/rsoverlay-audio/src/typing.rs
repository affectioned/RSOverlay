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

/// The `sound_pack` value that selects the classic built-in clicks
/// (not a folder name: `:` is never valid in one).
pub const CLASSIC_PACK: &str = "builtin:clicks";

/// The `sound_pack` value that selects the synthesised thock (the empty
/// default is the recorded Keychron pack).
pub const THOCK_PACK: &str = "builtin:thock";

/// Default built-in key sound: a "thock" in the style of a gasket-mounted
/// mechanical board (tester asked for that sound). Synthesised here, not
/// recorded: a short muffled top click (low-passed noise, ~1.5 ms) over a
/// damped low body (~250-320 Hz, falling slightly as the case rings
/// down) with a quieter second mode. Space / Enter are deeper and longer
/// (stabilised bars, with a faint stabiliser rattle); modifiers softer.
/// `variant` (0..3) nudges pitch and timing so fast typing isn't one
/// repeated sample. 16-bit mono 44.1 kHz WAV, < 100 ms.
pub fn builtin(kind: KeySound, volume: f32) -> Vec<u8> {
    thock(kind, volume, 0)
}

/// [`builtin`] with a per-press variation (0..3).
pub fn thock(kind: KeySound, volume: f32, variant: u32) -> Vec<u8> {
    // (length s, body Hz, body tau s, 2nd mode Hz, click level, gain)
    let (len_s, body, tau, mode2, click_w, gain) = match kind {
        KeySound::Key => (0.060, 300.0, 0.011, 780.0, 0.55, 1.0),
        KeySound::Backspace => (0.060, 285.0, 0.012, 740.0, 0.55, 0.95),
        KeySound::Modifier => (0.050, 320.0, 0.009, 820.0, 0.45, 0.75),
        KeySound::Space => (0.085, 175.0, 0.020, 430.0, 0.40, 1.0),
        KeySound::Enter => (0.075, 230.0, 0.016, 560.0, 0.45, 1.0),
    };
    let v = variant % 3;
    let detune = [1.0, 1.035, 0.97][v as usize];
    let n = (len_s * RATE as f32) as usize;
    let mut seed: u32 = 0x7EA5_1D3B ^ (kind.index() as u32).wrapping_mul(0x9E37_79B9) ^ v.wrapping_mul(0x85EB_CA6B);
    let mut lp = 0.0f32;
    // One-pole low-pass for the click (~3.5 kHz): soft, not a tick.
    let a = 1.0 - (-std::f32::consts::TAU * 3500.0 / RATE as f32).exp();
    let rattle_at = if matches!(kind, KeySound::Space | KeySound::Enter) { 0.012 } else { f32::MAX };
    let mut raw: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            lp += a * (noise - lp);
            let click = click_w * lp * (-t / 0.0015).exp();
            // Body pitch falls ~8 % over the ring-down (case resonance).
            let f = body * detune * (1.0 - 0.08 * (1.0 - (-t / 0.02).exp()));
            let phase = std::f32::consts::TAU * f * t;
            let body_s = phase.sin() * (-t / tau).exp();
            let mode2_s = 0.25 * (std::f32::consts::TAU * mode2 * detune * t).sin() * (-t / (tau * 0.45)).exp();
            // Stabiliser rattle: a faint second, smaller click.
            let r = t - rattle_at;
            let rattle = if r > 0.0 { 0.18 * lp * (-r / 0.002).exp() } else { 0.0 };
            click + body_s + mode2_s + rattle
        })
        .collect();
    for (i, s) in raw.iter_mut().take(12).enumerate() {
        *s *= i as f32 / 12.0;
    }
    // Tail fade so it never ends on a step.
    let tail = 64.min(raw.len());
    let len = raw.len();
    for (j, s) in raw[len - tail..].iter_mut().enumerate() {
        *s *= 1.0 - j as f32 / tail as f32;
    }
    let peak = raw.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    let scale = 0.9 * gain * volume.clamp(0.0, 1.0) * i16::MAX as f32 / peak;
    let samples: Vec<i16> = raw.iter().map(|s| (s * scale).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).collect();
    wav_pcm16_mono(&samples, RATE)
}

/// The classic built-in click (bright tick), as before "Thock".
/// O(n) in the click length (≤ ~2 000 samples).
pub fn classic(kind: KeySound, volume: f32) -> Vec<u8> {
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
    /// Raw takes (unscaled) per [`KeySound`]; empty = synthesised fallback.
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
    /// `pack = None` → the recorded Keychron pack; [`THOCK_PACK`] →
    /// synthesised thock; [`CLASSIC_PACK`] → classic clicks; otherwise a
    /// folder under `sounds_dir`.
    pub fn configure(&mut self, sounds_dir: &Path, pack: Option<&str>, volume: f32) {
        let pack = pack.filter(|p| *p == CLASSIC_PACK || *p == THOCK_PACK || valid_pack_name(p)).map(str::to_owned);
        let volume = volume.clamp(0.0, 1.0);
        let pack_changed = self.pack.as_ref() != Some(&pack);
        if !pack_changed && (volume - self.volume).abs() < 0.005 {
            return;
        }
        stop();
        if pack_changed {
            self.raw = KeySound::ALL.map(|k| match pack.as_deref() {
                None => keychron_takes(k).iter().map(|b| b.to_vec()).collect(),
                Some(CLASSIC_PACK) | Some(THOCK_PACK) => Vec::new(),
                Some(folder) => {
                    let f = sounds_dir.join(folder);
                    (1..=MAX_VARIANTS).filter_map(|n| read_capped(&f.join(variant_file(k, n)))).collect()
                }
            });
            self.pack = Some(pack);
        }
        self.volume = volume;
        let style = self.pack.clone().flatten();
        for k in KeySound::ALL {
            let i = k.index();
            let takes: Vec<Vec<u8>> = self.raw[i].iter().filter_map(|w| scale_pcm16(w, volume)).collect();
            self.bank[i] = if !takes.is_empty() {
                takes
            } else if style.as_deref() == Some(CLASSIC_PACK) {
                vec![classic(k, volume)]
            } else if k == KeySound::Key {
                // Synthesised thock: three variations of an ordinary key.
                (0..3).map(|v| thock(k, volume, v)).collect()
            } else {
                vec![builtin(k, volume)]
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

    fn samples(w: &[u8]) -> Vec<f32> {
        w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32).collect()
    }

    /// Share of the signal's energy above `hz` (a crude spectral split:
    /// energy of the first difference vs the signal, which grows with
    /// frequency). Lower = darker / thockier.
    fn brightness(x: &[f32]) -> f32 {
        let e: f32 = x.iter().map(|v| v * v).sum::<f32>().max(1.0);
        let d: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        d / e
    }

    /// Tester asked for a Keychron-like (mechanical "thock") default:
    /// the default must be clearly darker than the classic click, short,
    /// clean at both ends, and fast keys must not repeat one sample.
    #[test]
    fn default_is_a_dark_thock_with_variations() {
        for k in KeySound::ALL {
            let w = builtin(k, 1.0);
            assert_eq!(&w[0..4], b"RIFF");
            assert!(w.len() < 44 + 2 * 44_100 / 10, "{k:?} under 100 ms");
            let t = samples(&w);
            assert!(t.first().unwrap().abs() < 2000.0 && t.last().unwrap().abs() < 500.0, "{k:?} starts / ends near zero");
            let c = samples(&classic(k, 1.0));
            assert!(brightness(&t) < brightness(&c) * 0.6, "{k:?}: thock {} vs click {}", brightness(&t), brightness(&c));
        }
        assert!(brightness(&samples(&builtin(KeySound::Space, 1.0))) < brightness(&samples(&builtin(KeySound::Key, 1.0))), "space is deeper");
        let v: Vec<Vec<u8>> = (0..3).map(|i| thock(KeySound::Key, 0.5, i)).collect();
        assert!(v[0] != v[1] && v[1] != v[2] && v[0] != v[2], "three different presses");
        assert_eq!(thock(KeySound::Key, 0.5, 3), v[0], "variants wrap");
    }

    #[test]
    fn classic_clicks_stay_selectable() {
        let dir = std::env::temp_dir();
        let mut t = TypingSounds::new();
        t.configure(&dir, Some(CLASSIC_PACK), 0.4);
        assert_eq!(t.bank[KeySound::Key.index()], vec![classic(KeySound::Key, 0.4)]);
        t.configure(&dir, Some(THOCK_PACK), 0.4);
        assert_eq!(t.bank[KeySound::Key.index()][0], builtin(KeySound::Key, 0.4));
        assert_eq!(t.takes(KeySound::Key), 3);
        assert!(!valid_pack_name(THOCK_PACK));
        // Never treated as a folder.
        assert!(!valid_pack_name(CLASSIC_PACK));
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
        // Missing files fall back to the synthesised sound.
        assert_eq!(t.takes(KeySound::Enter), 1);
        t.configure(&dir, Some(THOCK_PACK), 0.5);
        assert_eq!(t.takes(KeySound::Key), 3, "thock's own variations");
        assert_eq!(variant_file(KeySound::Backspace, 4), "backspace4.wav");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
