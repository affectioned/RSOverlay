//! Voice typing: speech → text with Whisper (whisper.cpp), on the CPU,
//! entirely on this PC. Multilingual: the spoken language is detected.
//!
//! The model is downloaded on first use into `%APPDATA%\RSOverlay\models`
//! (checked against its SHA-256), loaded once and kept for the session.
//! All the work happens on one background thread; the UI polls
//! [`Speech::poll`] for status lines and the finished text.

use rsoverlay_core::config::VoiceModel;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use tracing::{info, warn};

/// A Whisper model file from whisper.cpp's model repository.
#[derive(Copy, Clone, Debug)]
pub struct ModelFile {
    pub file: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

pub fn model_file(m: VoiceModel) -> ModelFile {
    match m {
        VoiceModel::Fast => ModelFile {
            file: "ggml-base-q5_1.bin",
            sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
            bytes: 59_707_625,
        },
        VoiceModel::Accurate => ModelFile {
            file: "ggml-small-q5_1.bin",
            sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
            bytes: 190_085_487,
        },
    }
}

/// Is `m` already downloaded into `dir`?
pub fn model_ready(dir: &Path, m: VoiceModel) -> bool {
    let f = model_file(m);
    std::fs::metadata(dir.join(f.file)).is_ok_and(|md| md.len() == f.bytes)
}

/// What the worker reports back.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Progress line for the keyboard bar ("Downloading speech model 42%").
    Status(String),
    /// Finished: what was said (may be empty: nothing recognised).
    Text(String),
    Error(String),
}

struct Job {
    audio: Vec<f32>,
    model: VoiceModel,
}

/// Handle to the transcription thread.
pub struct Speech {
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Event>,
    busy: bool,
}

impl Speech {
    pub fn spawn(models_dir: PathBuf) -> Self {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (events, rx) = mpsc::channel::<Event>();
        let _ = std::thread::Builder::new().name("speech".into()).spawn(move || worker(models_dir, jobs, events));
        Self { tx, rx, busy: false }
    }

    /// Transcribe 16 kHz mono audio. Results arrive through [`poll`].
    pub fn transcribe(&mut self, audio: Vec<f32>, model: VoiceModel) {
        if self.tx.send(Job { audio, model }).is_ok() {
            self.busy = true;
        }
    }

    /// A job is queued or running.
    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn poll(&mut self) -> Option<Event> {
        let e = self.rx.try_recv().ok()?;
        if matches!(e, Event::Text(_) | Event::Error(_)) {
            self.busy = false;
        }
        Some(e)
    }
}

fn worker(dir: PathBuf, jobs: mpsc::Receiver<Job>, events: mpsc::Sender<Event>) {
    whisper_rs::install_logging_hooks();
    let mut loaded: Option<(VoiceModel, whisper_rs::WhisperContext)> = None;
    while let Ok(job) = jobs.recv() {
        let r = run(&dir, &job, &mut loaded, &events);
        let _ = events.send(match r {
            Ok(t) => Event::Text(t),
            Err(e) => {
                warn!("voice typing: {e:#}");
                Event::Error(format!("{e:#}"))
            }
        });
    }
}

fn run(
    dir: &Path,
    job: &Job,
    loaded: &mut Option<(VoiceModel, whisper_rs::WhisperContext)>,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<String> {
    use anyhow::Context;
    // whisper.cpp is built for AVX2 (see .cargo/config.toml).
    #[cfg(target_arch = "x86_64")]
    anyhow::ensure!(
        std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma") && std::is_x86_feature_detected!("f16c"),
        "this CPU has no AVX2, which voice typing needs"
    );
    // Under half a second of audio: nothing to transcribe.
    if job.audio.len() < 8_000 {
        return Ok(String::new());
    }
    let f = model_file(job.model);
    let path = dir.join(f.file);
    if !model_ready(dir, job.model) {
        let mut last = u64::MAX;
        let body = rsoverlay_core::update::download_model(f.file, f.sha256, f.bytes as usize + 1024, &mut |got, total| {
            let pct = got * 100 / total.unwrap_or(f.bytes).max(1);
            if pct != last {
                last = pct;
                let _ = events.send(Event::Status(format!("Downloading speech model {pct}%")));
            }
        })
        .context("downloading the speech model")?;
        std::fs::create_dir_all(dir).context("creating the models folder")?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &body).context("saving the speech model")?;
        std::fs::rename(&tmp, &path).context("saving the speech model")?;
        info!("voice typing: downloaded {}", f.file);
    }
    if loaded.as_ref().map(|l| l.0) != Some(job.model) {
        let _ = events.send(Event::Status("Loading speech model…".into()));
        *loaded = None;
        let ctx = whisper_rs::WhisperContext::new_with_params(&path, whisper_rs::WhisperContextParameters::default())
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", f.file))?;
        *loaded = Some((job.model, ctx));
    }
    let ctx = &loaded.as_ref().expect("loaded above").1;
    let _ = events.send(Event::Status("Transcribing…".into()));
    let t0 = std::time::Instant::now();
    let mut state = ctx.create_state().map_err(|e| anyhow::anyhow!("whisper state: {e}"))?;
    let mut p = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    // Leave cores for the game.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(2, 8);
    p.set_n_threads(threads as _);
    p.set_language(Some("auto"));
    p.set_no_context(true);
    p.set_suppress_blank(true);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    state.full(p, &job.audio).map_err(|e| anyhow::anyhow!("transcribing: {e}"))?;
    let segments: Vec<String> = (0..state.full_n_segments())
        .filter_map(|i| state.get_segment(i))
        .filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned()))
        .collect();
    let text = clean(&segments);
    info!(
        "voice typing: {:.1} s of audio → {} chars in {:.1} s ({threads} threads)",
        job.audio.len() as f32 / 16_000.0,
        text.chars().count(),
        t0.elapsed().as_secs_f32()
    );
    Ok(text)
}

/// Join Whisper's segments, dropping its non-speech tags ("[BLANK_AUDIO]",
/// "(music)", "[Applause]").
pub fn clean(segments: &[String]) -> String {
    let mut out = String::new();
    for s in segments {
        let mut kept = String::new();
        let mut depth = 0u32;
        for c in s.chars() {
            match c {
                '[' | '(' if depth == 0 && kept.trim().is_empty() => depth = 1,
                ']' | ')' if depth == 1 => depth = 0,
                _ if depth == 1 => {}
                _ => kept.push(c),
            }
        }
        let t = kept.trim();
        if !t.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_speech_tags_are_dropped_and_segments_joined() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(clean(&s(&[" Hello there.", " How are you?"])), "Hello there. How are you?");
        assert_eq!(clean(&s(&[" [BLANK_AUDIO]"])), "");
        assert_eq!(clean(&s(&[" (music)", " Hi!"])), "Hi!");
        // Brackets inside a sentence are speech, keep them.
        assert_eq!(clean(&s(&[" I said (quietly) no"])), "I said (quietly) no");
    }

    #[test]
    fn model_files_are_whisper_ggml() {
        for m in [VoiceModel::Fast, VoiceModel::Accurate] {
            let f = model_file(m);
            assert!(f.file.starts_with("ggml-") && f.file.ends_with(".bin"));
            assert_eq!(f.sha256.len(), 64);
        }
    }

    /// RSO_TEST_WAV=<16 kHz mono 16-bit wav>: transcribe it with the
    /// Fast model and print the text.
    #[test]
    #[ignore]
    fn transcribes_a_wav() {
        let Ok(path) = std::env::var("RSO_TEST_WAV") else { return };
        let b = std::fs::read(path).unwrap();
        let data = b.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio: Vec<f32> = b[data..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect();
        let mut s = Speech::spawn(rsoverlay_core::config::models_dir());
        s.transcribe(audio, VoiceModel::Fast);
        loop {
            match s.rx.recv().unwrap() {
                Event::Text(t) => {
                    println!("text: {t:?}");
                    assert!(!t.is_empty());
                    break;
                }
                Event::Error(e) => panic!("{e}"),
                Event::Status(_) => {}
            }
        }
    }

    /// Downloads the Fast model (60 MB) and transcribes silence.
    #[test]
    #[ignore]
    fn downloads_and_runs_the_fast_model() {
        let dir = rsoverlay_core::config::models_dir();
        let mut s = Speech::spawn(dir);
        s.transcribe(vec![0.0; 16_000 * 2], VoiceModel::Fast);
        loop {
            match s.rx.recv().unwrap() {
                Event::Status(l) => println!("{l}"),
                Event::Text(t) => {
                    println!("text: {t:?}");
                    break;
                }
                Event::Error(e) => panic!("{e}"),
            }
        }
    }
}
