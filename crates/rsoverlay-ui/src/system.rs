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

    pub fn show(&mut self, ui: &mut egui::Ui, overlay_fps: f32, accent: egui::Color32) {
        let s = self.sampler.get_or_insert_with(Sampler::spawn).snapshot();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(icon(icons::PULSE, 18.0, crate::theme::FG_MUTED));
                ui.add_space(6.0);
                ui.label(theme::card_title("System"));
                crate::widgets::popout_button(ui, "System", accent);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("{overlay_fps:.0} fps"))
                            .monospace()
                            .size(theme::SIZE_CAPTION + 1.0)
                            .color(theme::FG_MUTED),
                    );
                });
            });
            ui.add_space(6.0);
            meter(ui, "CPU", s.cpu_pct, None, accent);
            meter(ui, "GPU", s.gpu_pct, None, accent);
            let ram = (s.ram_total > 0).then(|| 100.0 * s.ram_used as f32 / s.ram_total as f32);
            meter(ui, "RAM", ram, Some(format_bytes(s.ram_used)), accent);
            if let (Some(u), Some(t)) = (s.vram_used, s.vram_total) {
                meter(ui, "VRAM", Some(100.0 * u as f32 / t as f32), Some(format_bytes(u)), accent);
            }
        });
    }
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
