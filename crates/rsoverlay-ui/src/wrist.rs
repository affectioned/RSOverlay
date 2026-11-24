//! Wrist watch-face — the always-there VR entry point.
//!
//! ```text
//! ╭──────────────────────────────────────────╮
//! │  10:42 PM                HMD 82  L 64    │
//! │  Tuesday · Nov 24        R 71            │
//! │                                          │
//! │   (▦)      (⌨)      (▶)      (🔔3)       │
//! │  Menu    Keyboard   Play   Alerts        │
//! ╰──────────────────────────────────────────╯
//! ```
//!
//! Drawn on a transparent clear inside a rounded black card, so it reads
//! as a floating watch, not a rectangle bolted to the controller.

use crate::battery::BatteryReading;
use crate::icons::{self, icon};
use crate::{locale, motion, shell, theme};
use chrono::Local;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WristAction {
    pub toggle_dashboard: bool,
    pub toggle_keyboard: bool,
    pub open_notifications: bool,
}

pub fn show(
    ctx: &egui::Context,
    batteries: &[BatteryReading],
    estimates: &[Option<String>],
    playing: bool,
    track: Option<&str>,
    unread: usize,
    accent: egui::Color32,
) -> WristAction {
    let mut act = WristAction::default();
    egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
        let card = ui.max_rect().shrink(4.0);
        ui.painter().rect_filled(card, 28.0, theme::BG_ROOT);
        ui.painter().rect_stroke(card, 28.0, egui::Stroke::new(1.5_f32, accent.gamma_multiply(0.35)));
        let inner = card.shrink2(egui::vec2(26.0, 20.0));
        let mut ui = ui.child_ui(inner, egui::Layout::top_down(egui::Align::Min));

        let now = Local::now();
        let time = if locale::uses_24h_clock() {
            now.format("%H:%M").to_string()
        } else {
            now.format("%-I:%M %p").to_string()
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(time).size(50.0).strong().color(theme::FG_PRIMARY));
                ui.label(theme::body_muted(&now.format("%A · %b %-d").to_string()));
                if let Some(t) = track {
                    ui.add(egui::Label::new(egui::RichText::new(t).size(theme::SIZE_CAPTION + 2.0).color(accent)).truncate(true));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.vertical(|ui| {
                    for (i, b) in batteries.iter().enumerate() {
                        let pct = (b.fraction * 100.0).round() as u32;
                        let color = if !b.charging && pct < 20 { theme::DANGER } else { accent };
                        let bolt = if b.charging { "⚡" } else { "" };
                        ui.label(
                            egui::RichText::new(format!("{} {bolt}{pct}{}", shell::short_device_label(&b.label), estimates.get(i).cloned().flatten().map(|t| format!(" · {t}")).unwrap_or_default()))
                                .monospace()
                                .size(theme::SIZE_CAPTION + 2.0)
                                .color(color),
                        );
                    }
                });
            });
        });

        ui.add_space(14.0);
        let n = 4.0;
        let slot = ui.available_width() / n;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let play_icon = if playing { icons::PAUSE } else { icons::PLAY };
            for (i, (src, label)) in [
                (icons::GRID, "Menu"),
                (icons::KEYBOARD, "Keyboard"),
                (play_icon, if playing { "Pause" } else { "Play" }),
                (icons::BELL, "Alerts"),
            ]
            .into_iter()
            .enumerate()
            {
                let badge = if i == 3 { unread } else { 0 };
                if round_button(ui, slot, src, label, badge, accent) {
                    match i {
                        0 => act.toggle_dashboard = true,
                        1 => act.toggle_keyboard = true,
                        2 => {
                            crate::media::send(crate::media::MediaCommand::PlayPause);
                        }
                        _ => act.open_notifications = true,
                    }
                }
            }
        });
    });
    act
}

/// 64 pt round icon button with a caption, centred in a `slot`-wide cell.
fn round_button(
    ui: &mut egui::Ui,
    slot: f32,
    src: egui::ImageSource<'static>,
    label: &str,
    badge: usize,
    accent: egui::Color32,
) -> bool {
    let (cell, _) = ui.allocate_exact_size(egui::vec2(slot, 96.0), egui::Sense::hover());
    let c = egui::pos2(cell.center().x, cell.top() + 34.0);
    let hit = egui::Rect::from_center_size(c, egui::vec2(68.0, 68.0));
    let resp = ui.interact(hit, ui.id().with(label), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    let fill = motion::mix(egui::Color32::from_rgb(0x16, 0x19, 0x1E), accent.gamma_multiply(0.22), hover);
    let p = ui.painter();
    p.circle_filled(c, 32.0, fill);
    p.circle_stroke(c, 32.0, egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.25 + 0.5 * hover)));
    icon(src, 28.0, motion::mix(theme::FG_PRIMARY, accent, hover))
        .paint_at(ui, egui::Rect::from_center_size(c, egui::vec2(28.0, 28.0)));
    let p = ui.painter();
    p.text(
        egui::pos2(c.x, cell.bottom() - 10.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(theme::SIZE_CAPTION + 2.0),
        theme::FG_MUTED,
    );
    if badge > 0 {
        let b = c + egui::vec2(22.0, -22.0);
        p.circle_filled(b, 10.0, accent);
        p.text(b, egui::Align2::CENTER_CENTER, badge.min(99).to_string(),
            egui::FontId::monospace(theme::SIZE_CAPTION), theme::BG_ROOT);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
