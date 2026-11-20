//! XSOverlay-compatible websocket notification API.
//!
//! Target: byte-level compat with XSOverlay's `/api/v1/send` message shape, so
//! clients like VRCX keep working without changes. Full schema and compat
//! matrix live in `docs/NOTIFICATION_API.md`.
//!
//! This milestone only stands up the schema types and a bind stub. Server
//! wiring lands next milestone.

use serde::{Deserialize, Serialize};

/// XSOverlay-compatible notification payload. Field names must match exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(rename = "messageType")]
    pub message_type: u32,
    pub index: u32,
    pub timeout: f32,
    pub height: f32,
    pub opacity: f32,
    pub volume: f32,
    #[serde(rename = "audioPath")]
    pub audio_path: String,
    pub title: String,
    pub content: String,
    #[serde(rename = "useBase64Icon")]
    pub use_base64_icon: bool,
    pub icon: String,
    #[serde(rename = "sourceApp")]
    pub source_app: String,
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            message_type: 1,
            index: 0,
            timeout: 3.0,
            height: 110.0,
            opacity: 1.0,
            volume: 0.7,
            audio_path: "default".into(),
            title: String::new(),
            content: String::new(),
            use_base64_icon: false,
            icon: "default".into(),
            source_app: "RSOverlay".into(),
        }
    }
}
