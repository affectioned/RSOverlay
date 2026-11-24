//! Shared types and lifecycle primitives used across RSOverlay crates.

pub mod config;
pub mod layouts;
pub mod panel;
pub mod runtime;

pub use config::{Appearance, Config, Macro, Macros, Notify, Placement, PlacementAnchor, Preview, Render};
pub use panel::{Anchor, Panel, PanelId, PanelKind};
pub use runtime::RuntimeMode;
