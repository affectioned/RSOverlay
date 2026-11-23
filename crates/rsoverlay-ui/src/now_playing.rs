//! SMTC now-playing → VRChat chatbox bridge.
//!
//! Pure state machine: the caller feeds it the latest [`MediaInfo`] and
//! the current instant, it answers with the chatbox line to send (if
//! any). No I/O here, so the pacing rules are unit-testable.
//!
//! Pacing:
//! - Track change → send immediately (subject to the min gap).
//! - Same track still playing → re-send every [`REFRESH`] so the bubble
//!   stays up (VRChat fades the chatbox after a while).
//! - Never faster than [`MIN_GAP`]; VRChat's chatbox spam filter starts
//!   dropping/timeouting messages sent in tight bursts.
//! - Paused / stopped / empty title → stay quiet, and forget the last
//!   track so resuming re-announces it.

use crate::media::MediaInfo;
use std::time::{Duration, Instant};

pub const MIN_GAP: Duration = Duration::from_secs(2);
pub const REFRESH: Duration = Duration::from_secs(20);
/// VRChat's `/chatbox/input` hard cap is 144 characters.
pub const CHATBOX_MAX_CHARS: usize = 144;

#[derive(Default)]
pub struct NowPlayingBridge {
    pub enabled: bool,
    last_line: Option<String>,
    last_sent: Option<Instant>,
}

impl NowPlayingBridge {
    /// Returns the chatbox line to send this tick, or `None`.
    pub fn tick(&mut self, media: Option<&MediaInfo>, now: Instant) -> Option<String> {
        if !self.enabled {
            self.last_line = None;
            return None;
        }
        let Some(line) = media.filter(|m| m.playing).and_then(format_line) else {
            self.last_line = None;
            return None;
        };
        let since = self.last_sent.map(|t| now.saturating_duration_since(t));
        if since.is_some_and(|d| d < MIN_GAP) {
            return None;
        }
        let changed = self.last_line.as_deref() != Some(line.as_str());
        let stale = since.is_none_or(|d| d >= REFRESH);
        if !changed && !stale {
            return None;
        }
        self.last_line = Some(line.clone());
        self.last_sent = Some(now);
        Some(line)
    }
}

/// `♪ Title — Artist`, truncated on a char boundary to the chatbox cap.
pub fn format_line(m: &MediaInfo) -> Option<String> {
    let title = m.title.trim();
    if title.is_empty() {
        return None;
    }
    let artist = m.artist.trim();
    let full = if artist.is_empty() {
        format!("♪ {title}")
    } else {
        format!("♪ {title} — {artist}")
    };
    Some(truncate_chars(&full, CHATBOX_MAX_CHARS))
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        // Leave room for the ellipsis so the result is still ≤ max chars.
        Some(_) => {
            let cut = s.char_indices().nth(max - 1).map_or(s.len(), |(i, _)| i);
            format!("{}…", &s[..cut])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, playing: bool) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: artist.into(),
            source: String::new(),
            playing,
        }
    }

    fn on() -> NowPlayingBridge {
        NowPlayingBridge { enabled: true, ..Default::default() }
    }

    #[test]
    fn disabled_is_silent() {
        let mut b = NowPlayingBridge::default();
        assert_eq!(b.tick(Some(&track("A", "B", true)), Instant::now()), None);
    }

    #[test]
    fn announces_then_refreshes() {
        let mut b = on();
        let t0 = Instant::now();
        let m = track("Song", "Band", true);
        assert_eq!(b.tick(Some(&m), t0).as_deref(), Some("♪ Song — Band"));
        assert_eq!(b.tick(Some(&m), t0 + Duration::from_secs(5)), None);
        assert!(b.tick(Some(&m), t0 + REFRESH).is_some());
    }

    #[test]
    fn track_change_respects_min_gap() {
        let mut b = on();
        let t0 = Instant::now();
        b.tick(Some(&track("One", "", true)), t0);
        let two = track("Two", "", true);
        assert_eq!(b.tick(Some(&two), t0 + Duration::from_millis(500)), None);
        assert_eq!(b.tick(Some(&two), t0 + MIN_GAP).as_deref(), Some("♪ Two"));
    }

    #[test]
    fn pause_then_resume_reannounces() {
        let mut b = on();
        let t0 = Instant::now();
        b.tick(Some(&track("S", "", true)), t0);
        assert_eq!(b.tick(Some(&track("S", "", false)), t0 + MIN_GAP), None);
        assert!(b.tick(Some(&track("S", "", true)), t0 + MIN_GAP * 2).is_some());
    }

    #[test]
    fn truncates_to_chatbox_cap() {
        let long = "あ".repeat(300);
        let line = format_line(&track(&long, "", true)).unwrap();
        assert_eq!(line.chars().count(), CHATBOX_MAX_CHARS);
        assert!(line.ends_with('…'));
    }
}
