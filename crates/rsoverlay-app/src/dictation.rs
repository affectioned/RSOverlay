//! Voice typing controller: the keyboard's mic button → record → Whisper
//! → text. Mutes the VRChat mic over OSC while recording (when it knows
//! the mic is live) and unmutes it right after.

use rsoverlay_audio::mic::Recorder;
use rsoverlay_core::config::VoiceModel;
use rsoverlay_osc::{MicState, MuteWatch, OscClient};
use rsoverlay_speech::{Event, Speech};
use rsoverlay_ui::vrkeyboard::Dictation as KbState;
use std::time::Instant;
use tracing::{info, warn};

pub struct Dictation {
    rec: Option<(Recorder, Instant)>,
    speech: Option<Speech>,
    status: KbState,
    /// We muted VRChat for this recording (and should unmute after).
    muted_vrc: bool,
}

/// The settings the mic button uses.
pub struct VoicePrefs<'a> {
    pub model: VoiceModel,
    pub mic: &'a str,
    pub mute_vrchat: bool,
}

impl Dictation {
    pub fn new() -> Self {
        Self { rec: None, speech: None, status: KbState::Idle, muted_vrc: false }
    }

    /// Mic button: start listening, or stop and transcribe.
    pub fn toggle(&mut self, prefs: &VoicePrefs, osc: Option<(&OscClient, &MuteWatch)>) {
        if let Some((rec, _)) = self.rec.take() {
            self.finish(rec, prefs.model, osc);
            return;
        }
        if self.speech.as_ref().is_some_and(|s| s.busy()) {
            return;
        }
        if let (true, Some((client, watch))) = (prefs.mute_vrchat, osc) {
            if watch.state() == MicState::Live {
                info!("voice typing: muting VRChat");
                self.muted_vrc = client.toggle_voice().is_ok();
            }
        }
        let mic = (!prefs.mic.is_empty()).then(|| prefs.mic.to_string());
        self.rec = Some((Recorder::start(mic), Instant::now()));
        self.status = KbState::Recording { level: 0.0, secs: 0.0 };
    }

    fn finish(&mut self, rec: Recorder, model: VoiceModel, osc: Option<(&OscClient, &MuteWatch)>) {
        if std::mem::take(&mut self.muted_vrc) {
            if let Some((client, watch)) = osc {
                // Only if it's still muted (you may have unmuted yourself).
                if watch.state() != MicState::Live {
                    info!("voice typing: unmuting VRChat");
                    let _ = client.toggle_voice();
                }
            }
        }
        match rec.finish() {
            Ok(audio) => {
                let s = self.speech.get_or_insert_with(|| Speech::spawn(rsoverlay_core::config::models_dir()));
                s.transcribe(audio, model);
                self.status = KbState::Working("Transcribing…".into());
            }
            Err(e) => {
                warn!("voice typing: {e}");
                self.status = KbState::Failed(e);
            }
        }
    }

    /// Per frame: progress, the auto-stop at the time limit, and the
    /// finished text (once).
    pub fn update(&mut self, model: VoiceModel, osc: Option<(&OscClient, &MuteWatch)>) -> Option<String> {
        if let Some((rec, t0)) = &self.rec {
            if rec.ended() {
                let (rec, _) = self.rec.take().expect("checked");
                self.finish(rec, model, osc);
            } else {
                self.status = KbState::Recording { level: rec.level(), secs: t0.elapsed().as_secs_f32() };
            }
        }
        let s = self.speech.as_mut()?;
        let mut text = None;
        while let Some(e) = s.poll() {
            match e {
                Event::Status(t) => self.status = KbState::Working(t),
                Event::Text(t) => {
                    self.status = KbState::Idle;
                    text = Some(t).filter(|t| !t.trim().is_empty());
                }
                Event::Error(e) => self.status = KbState::Failed(e),
            }
        }
        text
    }

    /// Listening right now.
    pub fn recording(&self) -> bool {
        self.rec.is_some()
    }

    pub fn state(&self) -> KbState {
        self.status.clone()
    }
}
