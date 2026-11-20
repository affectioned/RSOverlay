//! XSOverlay-compatible websocket notification API.
//!
//! Target: byte-level compat with XSOverlay's `/api/v1/send` message shape so
//! clients like VRCX keep working without changes. Full schema and compat
//! matrix live in `docs/NOTIFICATION_API.md`.

pub mod server;

pub use server::{spawn, NotifyChannel, NotifyConfig};

use serde::{Deserialize, Serialize};

/// XSOverlay-compatible notification payload. Field names must match exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(rename = "messageType", default = "default_message_type")]
    pub message_type: u32,
    #[serde(default)]
    pub index: u32,
    #[serde(default = "default_timeout")]
    pub timeout: f32,
    #[serde(default = "default_height")]
    pub height: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(rename = "audioPath", default = "default_audio_path")]
    pub audio_path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(rename = "useBase64Icon", default)]
    pub use_base64_icon: bool,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(rename = "sourceApp", default)]
    pub source_app: String,
}

fn default_message_type() -> u32 { 1 }
fn default_timeout() -> f32 { 3.0 }
fn default_height() -> f32 { 110.0 }
fn default_opacity() -> f32 { 1.0 }
fn default_volume() -> f32 { 0.7 }
fn default_audio_path() -> String { "default".into() }
fn default_icon() -> String { "default".into() }

impl Default for Notification {
    fn default() -> Self {
        Self {
            message_type: default_message_type(),
            index: 0,
            timeout: default_timeout(),
            height: default_height(),
            opacity: default_opacity(),
            volume: default_volume(),
            audio_path: default_audio_path(),
            title: String::new(),
            content: String::new(),
            use_base64_icon: false,
            icon: default_icon(),
            source_app: "RSOverlay".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_payload_fills_defaults() {
        let n: Notification = serde_json::from_str(r#"{"title":"hi","content":"world"}"#).unwrap();
        assert_eq!(n.title, "hi");
        assert_eq!(n.content, "world");
        assert_eq!(n.timeout, 3.0);
        assert_eq!(n.icon, "default");
    }

    #[test]
    fn full_payload_roundtrip() {
        let n = Notification {
            title: "T".into(),
            content: "C".into(),
            timeout: 5.0,
            ..Default::default()
        };
        let j = serde_json::to_string(&n).unwrap();
        let back: Notification = serde_json::from_str(&j).unwrap();
        assert_eq!(back.title, "T");
        assert_eq!(back.timeout, 5.0);
    }
}
