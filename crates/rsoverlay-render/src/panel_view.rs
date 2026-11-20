//! Per-panel rendering. P0 scope: solid color fills only. Future kinds
//! (desktop mirror, UI) plug in here and write into the same
//! [`OverlayTarget`].

use crate::{clear_target, ClearColor, GpuCtx, OverlayTarget};
use rsoverlay_core::{Panel, PanelKind};

/// Render a single panel into its target this frame.
pub fn render_panel(gpu: &GpuCtx, target: &OverlayTarget, panel: &Panel) {
    match &panel.kind {
        PanelKind::SolidColor { rgba } => {
            let color = ClearColor {
                r: rgba[0] as f64 / 255.0,
                g: rgba[1] as f64 / 255.0,
                b: rgba[2] as f64 / 255.0,
                a: rgba[3] as f64 / 255.0,
            };
            clear_target(gpu, target, color);
        }
        PanelKind::Desktop { .. } | PanelKind::Window { .. } | PanelKind::Ui { .. } => {
            // Stub for P0. These land in the capture / ui crates.
            clear_target(gpu, target, ClearColor::BLACK);
        }
    }
}
