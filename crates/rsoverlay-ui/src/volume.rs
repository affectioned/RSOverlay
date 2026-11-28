//! Volume mixer card — master + per-app sliders with mute toggles.

use crate::icons::{self, icon};
use crate::theme;
use rsoverlay_audio::{Level, Mixer};

/// Lazily owns the mixer worker: no COM thread exists until the card is
/// first shown.
#[derive(Default)]
pub struct VolumeCard {
    mixer: Option<Mixer>,
}

impl VolumeCard {
    /// The mixer (started on first use).
    pub fn mixer(&mut self) -> &Mixer {
        self.mixer.get_or_insert_with(Mixer::spawn)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, accent: egui::Color32) {
        let mixer = self.mixer.get_or_insert_with(Mixer::spawn);
        let snap = mixer.snapshot();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(icon(icons::SPEAKER, 18.0, crate::theme::FG_MUTED));
                ui.add_space(6.0);
                ui.label(theme::card_title("Volume"));
                crate::widgets::popout_button(ui, "Volume", accent);
            });
            ui.add_space(6.0);
            let Some(master) = snap.master else {
                ui.label(theme::body_muted("no audio output device"));
                return;
            };
            if let Some(l) = row(ui, "Master", master, true) {
                apply(l, master, |v| mixer.set_master_volume(v), |m| mixer.set_master_mute(m));
            }
            if snap.apps.is_empty() {
                return;
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_source("volume-apps")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for app in &snap.apps {
                        ui.push_id(app.pid, |ui| {
                            if let Some(l) = row(ui, &app.name, app.level, false) {
                                let pid = app.pid;
                                apply(l, app.level, |v| mixer.set_app_volume(pid, v), |m| mixer.set_app_mute(pid, m));
                            }
                        });
                    }
                });
        });
    }
}

/// Send only what changed — a mute click must not also re-send volume.
fn apply(new: Level, old: Level, vol: impl FnOnce(f32), mute: impl FnOnce(bool)) {
    if new.muted != old.muted {
        mute(new.muted);
    }
    if (new.volume - old.volume).abs() > f32::EPSILON {
        vol(new.volume);
    }
}

/// One mixer row. Returns the edited level when the user touched it.
fn row(ui: &mut egui::Ui, name: &str, level: Level, strong: bool) -> Option<Level> {
    let mut l = level;
    let mut changed = false;
    ui.horizontal(|ui| {
        let glyph = if l.muted { "🔇" } else { "🔊" };
        let color = if l.muted { theme::DANGER } else { theme::FG_MUTED };
        if ui
            .add(egui::Button::new(egui::RichText::new(glyph).color(color)).min_size(egui::vec2(26.0, 22.0)))
            .on_hover_text(if l.muted { "Unmute" } else { "Mute" })
            .clicked()
        {
            l.muted = !l.muted;
            changed = true;
        }
        let text = egui::RichText::new(name).color(if strong { theme::FG_PRIMARY } else { theme::FG_MUTED });
        // Fixed-width, left-aligned name column so sliders line up.
        ui.allocate_ui_with_layout(
            egui::vec2(92.0, 20.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(92.0);
                ui.add(egui::Label::new(text).truncate(true));
            },
        );
        // Right-aligned % readout; the slider takes whatever is left.
        const VALUE_W: f32 = 34.0;
        ui.spacing_mut().slider_width = (ui.available_width() - VALUE_W - ui.spacing().item_spacing.x).max(40.0);
        let mut pct = l.volume * 100.0;
        let resp = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).show_value(false).trailing_fill(true));
        ui.add_sized(
            egui::vec2(VALUE_W, 18.0),
            egui::Label::new(
                egui::RichText::new(format!("{:.0}", pct)).monospace().size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED),
            ),
        );
        if resp.changed() {
            l.volume = pct / 100.0;
            changed = true;
        }
    });
    changed.then_some(l)
}
