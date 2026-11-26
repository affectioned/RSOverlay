//! System load card — CPU / GPU / RAM / VRAM meters + overlay fps.

use crate::icons::{self, icon};
use crate::{motion, theme};
use rsoverlay_sysinfo::{format_bytes, Sampler};

/// Meters turn red above this load.
const HOT_PCT: f32 = 90.0;

#[derive(Default)]
pub struct SystemCard {
    sampler: Option<Sampler>,
}

impl SystemCard {
    /// Latest sample (starts the sampler on first use).
    pub fn snapshot(&mut self) -> Option<rsoverlay_sysinfo::Snapshot> {
        Some(self.sampler.get_or_insert_with(Sampler::spawn).snapshot())
    }

    /// The *PC's* performance (game + machine). RSOverlay's own numbers
    /// live separately in Settings › RSOverlay performance.
    pub fn show(&mut self, ui: &mut egui::Ui, vr: Option<&rsoverlay_core::perf::VrPerf>, accent: egui::Color32) {
        let s = self.sampler.get_or_insert_with(Sampler::spawn).snapshot();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(icon(icons::PULSE, 18.0, crate::theme::FG_MUTED));
                ui.add_space(6.0);
                ui.label(theme::card_title("PC performance"));
                crate::widgets::popout_button(ui, "PC performance", accent);
                // The game's frame rate (VR), never RSOverlay's own.
                if let Some(v) = vr {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let behind = v.fps < v.hz * 0.9;
                        ui.label(
                            egui::RichText::new(format!("{:.0} / {:.0} fps", v.fps, v.hz))
                                .monospace()
                                .size(theme::SIZE_CAPTION + 1.0)
                                .color(if behind { theme::DANGER } else { theme::FG_MUTED }),
                        )
                        .on_hover_text("Game frame rate / headset refresh rate (SteamVR)");
                    });
                }
            });
            ui.add_space(6.0);
            meter(ui, "CPU", s.cpu_pct, None, accent);
            // Busiest core: games like VRChat hit one main thread's limit
            // long before the average looks high.
            meter(ui, "Core", s.cpu_max_core_pct, None, accent);
            meter(ui, "GPU", s.gpu_pct, None, accent);
            let ram = (s.ram_total > 0).then(|| 100.0 * s.ram_used as f32 / s.ram_total as f32);
            meter(ui, "RAM", ram, Some(format_bytes(s.ram_used)), accent);
            if let (Some(u), Some(t)) = (s.vram_used, s.vram_total) {
                meter(ui, "VRAM", Some(100.0 * u as f32 / t as f32), Some(format_bytes(u)), accent);
            }
            if let Some(v) = vr {
                ui.add_space(8.0);
                vr_rows(ui, v, accent);
            }
        });
    }
}

/// SteamVR frame stats: frame rate vs refresh, GPU time + headroom,
/// reprojection. Red when the game is falling behind.
fn vr_rows(ui: &mut egui::Ui, v: &rsoverlay_core::perf::VrPerf, accent: egui::Color32) {
    let line = |ui: &mut egui::Ui, label: &str, value: String, bad: bool, tip: &str| {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(44.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(44.0);
                ui.label(theme::body_muted(label));
            });
            ui.label(egui::RichText::new(value).monospace().size(theme::SIZE_CAPTION + 1.0).color(if bad { theme::DANGER } else { theme::FG_PRIMARY }));
        })
        .response
        .on_hover_text(tip);
    };
    let behind = v.fps < v.hz * 0.9;
    let dot = if behind { theme::DANGER } else { accent };
    ui.horizontal(|ui| {
        ui.painter().circle_filled(ui.cursor().min + egui::vec2(4.0, 10.0), 3.5, dot);
        ui.add_space(12.0);
        ui.label(theme::body_muted("VR · SteamVR frame timing"));
    });
    line(ui, "FPS", format!("{:.0} / {:.0}", v.fps, v.hz), behind, "Game frames per second / your headset's refresh rate");
    line(ui, "Frame", format!("{:.1} ms GPU · {:+.1} ms", v.gpu_ms, v.headroom_ms), v.headroom_ms < 0.0,
        "GPU time per frame, and how much of the frame budget is left (negative = over budget)");
    line(ui, "Reproj", format!("{:.0}% · {} dropped", v.reprojected_pct, v.dropped), v.reprojected_pct > 10.0,
        "Frames SteamVR had to reuse because the game wasn't ready in time (lower is better)");
}

/// `label ▕████░░░░▏ value`. `detail` replaces the % readout (e.g. GB).
fn meter(ui: &mut egui::Ui, label: &str, pct: Option<f32>, detail: Option<String>, accent: egui::Color32) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(44.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(44.0);
            ui.label(theme::body_muted(label));
        });
        const VALUE_W: f32 = 64.0;
        let bar_w = (ui.available_width() - VALUE_W - ui.spacing().item_spacing.x).max(40.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, 6.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 3.0, crate::theme::veil(0x18));
        let target = pct.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
        // 1 Hz samples would make bars step; glide between them instead.
        let shown = motion::value(ui.ctx(), egui::Id::new(("sys-meter", label)), target, motion::SLOW);
        let mut fill = rect;
        fill.set_width(rect.width() * shown);
        let hot = pct.is_some_and(|v| v >= HOT_PCT);
        p.rect_filled(fill, 3.0, if hot { theme::DANGER } else { accent });
        let text = match (pct, detail) {
            (None, _) => "—".to_string(),
            (Some(_), Some(d)) => d,
            (Some(v), None) => format!("{v:.0}%"),
        };
        ui.add_sized(
            egui::vec2(VALUE_W, 18.0),
            egui::Label::new(egui::RichText::new(text).monospace().size(theme::SIZE_CAPTION + 1.0).color(theme::FG_PRIMARY)),
        );
    });
}
