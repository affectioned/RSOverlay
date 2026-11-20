//! Shared types and lifecycle primitives used across RSOverlay crates.

pub mod config;
pub mod panel;
pub mod runtime;

pub use config::Config;
pub use panel::{Anchor, Panel, PanelId, PanelKind};
pub use runtime::RuntimeMode;
