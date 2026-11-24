//! Wrist toolbar — the always-there VR control centre, laid out as the
//! stacked rows XSOverlay users know (our own look and icons):
//!
//! ```text
//! ╭─────────────────────────────────────────────────╮
//! │  ＋   ↻   ⌨   ▦   🗀   ⚙   🗑                    │  actions
//! ╰─────────────────────────────────────────────────╯
//!    ╭ ‹ Slot 1 › ──────── ★  💾  ⇥  🗑 ╮              layouts
//!    ╭ 10:42 PM          ┃ ◈    ♫    ▮▮▮ ╮            clock + views
//!    │ Tue 24/11/2026 · 00:43:19           │
//!    │ ♪ Song — Artist          ‹ ⏯ ›     │            media
//!    ╰────────────────────────────────────╯
//!           [HMD 82% · 1h40]  [L 64%]  [R 71%]        devices
//! ```

use crate::battery::BatteryReading;
use crate::icons::{self, icon};
use crate::layouts_page::LayoutAction;
use crate::shell::Page;
use crate::{locale, motion, theme};
use chrono::Local;

/// Everything the wrist shows, gathered by DevView.
pub struct WristData<'a> {
    pub batteries: &'a [BatteryReading],
    pub estimates: &'a [Option<String>],
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub playing: bool,
    pub session_secs: u64,
    pub slot: usize,
    pub slot_filled: bool,
    pub slot_favorite: bool,
    pub keyboard_open: bool,
    pub accent: egui::Color32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WristAction {
    pub toggle_dashboard: bool,
    pub toggle_keyboard: bool,
    /// Show the dashboard on this page.
    pub open_page: Option<Page>,
    pub recenter: bool,
    pub close_all: bool,
    pub popout_stats: bool,
    /// Move the selected layout slot (−1 / +1).
    pub slot_delta: i32,
    pub layout: Option<LayoutAction>,
}

const ROW_GAP: f32 = 8.0;

pub fn show(ctx: &egui::Context, d: &WristData) -> WristAction {
    let mut act = WristAction::default();
    let accent = d.accent;
    egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
        let full = ui.max_rect().shrink(4.0);
        let mut y = full.top();
        let row = |y: &mut f32, h: f32, inset: f32| {
            let r = egui::Rect::from_min_size(egui::pos2(full.left() + inset, *y), egui::vec2(full.width() - 2.0 * inset, h));
            *y += h + ROW_GAP;
            r
        };

        // 1 · Actions.
        let r1 = row(&mut y, 64.0, 0.0);
        slab(ui, r1, accent);
        let actions: [(egui::ImageSource<'static>, &str, bool); 7] = [
            (icons::PLUS, "Add a window", false),
            (icons::RECENTER, "Recenter panels in front of you", false),
            (icons::KEYBOARD, "Keyboard", d.keyboard_open),
            (icons::GRID, "Dashboard", false),
            (icons::FOLDER, "Layouts", false),
            (icons::SETTINGS, "Settings", false),
            (icons::TRASH, "Close all floating panels", false),
        ];
        let cell = r1.width() / actions.len() as f32;
        for (i, (src, tip, on)) in actions.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(r1.left() + i as f32 * cell, r1.top()), egui::vec2(cell, r1.height()));
            if i > 0 {
                ui.painter().line_segment(
                    [egui::pos2(c.left(), c.top() + 16.0), egui::pos2(c.left(), c.bottom() - 16.0)],
                    egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(0x14)),
                );
            }
            if icon_button(ui, c.shrink(6.0), src, 26.0, on, tip, accent) {
                match i {
                    0 => act.open_page = Some(Page::Desktop),
                    1 => act.recenter = true,
                    2 => act.toggle_keyboard = true,
                    3 => act.toggle_dashboard = true,
                    4 => act.open_page = Some(Page::Layouts),
                    5 => act.open_page = Some(Page::Settings),
                    _ => act.close_all = true,
                }
            }
        }

        // 2 · Layouts.
        let r2 = row(&mut y, 48.0, full.width() * 0.12);
        slab(ui, r2, accent);
        let half = r2.width() * 0.42;
        let sel = egui::Rect::from_min_size(r2.min, egui::vec2(half, r2.height())).shrink(6.0);
        ui.painter().rect_filled(sel, 10.0, egui::Color32::from_rgb(0x16, 0x19, 0x1E));
        let arrow_w = 34.0;
        let prev_r = egui::Rect::from_min_size(sel.min, egui::vec2(arrow_w, sel.height()));
        let next_r = egui::Rect::from_min_size(egui::pos2(sel.right() - arrow_w, sel.top()), egui::vec2(arrow_w, sel.height()));
        if glyph_button(ui, prev_r, crate::glyphs::Glyph::Left, "Previous slot", accent) {
            act.slot_delta = -1;
        }
        if glyph_button(ui, next_r, crate::glyphs::Glyph::Right, "Next slot", accent) {
            act.slot_delta = 1;
        }
        let label = if d.slot_filled { format!("Slot {}", d.slot + 1) } else { format!("Slot {} · empty", d.slot + 1) };
        ui.painter().text(sel.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(theme::SIZE_BODY), theme::FG_PRIMARY);
        let tools = egui::Rect::from_min_max(egui::pos2(r2.left() + half, r2.top()), r2.max).shrink(4.0);
        let tw = tools.width() / 4.0;
        let star = if d.slot_favorite { icons::STAR } else { icons::STAR_OUTLINE };
        let tool_defs: [(egui::ImageSource<'static>, &str, bool, bool); 4] = [
            (star, "Load this slot at startup", d.slot_favorite, d.slot_filled),
            (icons::SAVE, "Save the current arrangement here", false, true),
            (icons::LOAD, "Load this slot", false, d.slot_filled),
            (icons::TRASH, "Clear this slot", false, d.slot_filled),
        ];
        for (i, (src, tip, on, enabled)) in tool_defs.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(tools.left() + i as f32 * tw, tools.top()), egui::vec2(tw, tools.height()));
            if !enabled {
                icon(src, 20.0, egui::Color32::from_white_alpha(0x30)).paint_at(ui, egui::Rect::from_center_size(c.center(), egui::vec2(20.0, 20.0)));
            } else if icon_button(ui, c.shrink(2.0), src, 20.0, on, tip, accent) {
                act.layout = Some(match i {
                    0 => LayoutAction::ToggleFavorite(d.slot),
                    1 => LayoutAction::Save(d.slot),
                    2 => LayoutAction::Load(d.slot),
                    _ => LayoutAction::Clear(d.slot),
                });
            }
        }

        // 3 · Clock + views (+ media inside the same slab).
        let media_h = if d.title.is_some() { 56.0 } else { 0.0 };
        let r3 = row(&mut y, 108.0 + media_h, full.width() * 0.12);
        slab(ui, r3, accent);
        let now = Local::now();
        let time = if locale::uses_24h_clock() { now.format("%H:%M").to_string() } else { now.format("%-I:%M %p").to_string() };
        let p = ui.painter();
        p.text(r3.left_top() + egui::vec2(18.0, 12.0), egui::Align2::LEFT_TOP, time, egui::FontId::proportional(40.0), theme::FG_PRIMARY);
        p.text(r3.left_top() + egui::vec2(18.0, 60.0), egui::Align2::LEFT_TOP, now.format("%a %d/%m/%Y").to_string(),
            egui::FontId::proportional(theme::SIZE_BODY + 1.0), theme::FG_PRIMARY);
        let s = d.session_secs;
        p.text(r3.left_top() + egui::vec2(18.0, 82.0), egui::Align2::LEFT_TOP, format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
            egui::FontId::monospace(theme::SIZE_CAPTION + 2.0), theme::FG_MUTED);
        let views = egui::Rect::from_min_max(egui::pos2(r3.center().x - 6.0, r3.top() + 10.0), egui::pos2(r3.right() - 10.0, r3.top() + 98.0));
        ui.painter().rect_filled(views, 14.0, egui::Color32::from_white_alpha(0x0A));
        let vw = views.width() / 3.0;
        let view_defs: [(egui::ImageSource<'static>, &str); 3] =
            [(icons::LAYERS, "Windows & monitors"), (icons::MUSIC, "Media & volume"), (icons::CHART, "Performance")];
        for (i, (src, tip)) in view_defs.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(views.left() + i as f32 * vw, views.top()), egui::vec2(vw, views.height()));
            if icon_button(ui, c.shrink(6.0), src, 30.0, false, tip, accent) {
                match i {
                    0 => act.open_page = Some(Page::Desktop),
                    1 => act.open_page = Some(Page::Audio),
                    _ => act.popout_stats = true,
                }
            }
        }
        if let Some(title) = d.title {
            let m = egui::Rect::from_min_max(egui::pos2(r3.left() + 10.0, r3.top() + 108.0), egui::pos2(r3.right() - 10.0, r3.bottom() - 8.0));
            ui.painter().rect_filled(m, 12.0, egui::Color32::from_white_alpha(0x0A));
            let text_r = egui::Rect::from_min_max(m.min + egui::vec2(14.0, 6.0), egui::pos2(m.right() - 150.0, m.bottom() - 6.0));
            let tp = ui.painter_at(text_r);
            tp.text(text_r.left_top(), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(theme::SIZE_BODY), theme::FG_PRIMARY);
            if let Some(a) = d.artist {
                tp.text(text_r.left_top() + egui::vec2(0.0, 22.0), egui::Align2::LEFT_TOP, a, egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), theme::FG_MUTED);
            }
            let ctrl = egui::Rect::from_min_max(egui::pos2(m.right() - 146.0, m.top() + 6.0), m.max - egui::vec2(6.0, 6.0));
            let cw = ctrl.width() / 3.0;
            let play = if d.playing { icons::PAUSE } else { icons::PLAY };
            let cmds = [crate::media::MediaCommand::Previous, crate::media::MediaCommand::PlayPause, crate::media::MediaCommand::Next];
            for (i, cmd) in cmds.into_iter().enumerate() {
                let c = egui::Rect::from_min_size(egui::pos2(ctrl.left() + i as f32 * cw, ctrl.top()), egui::vec2(cw, ctrl.height()));
                let clicked = match i {
                    1 => icon_button(ui, c.shrink(2.0), play.clone(), 20.0, false, "Play / pause", accent),
                    0 => glyph_button(ui, c.shrink(2.0), crate::glyphs::Glyph::Left, "Previous", accent),
                    _ => glyph_button(ui, c.shrink(2.0), crate::glyphs::Glyph::Right, "Next", accent),
                };
                if clicked {
                    crate::media::send(cmd);
                }
            }
        }

        // 4 · Devices.
        if !d.batteries.is_empty() {
            let r5 = row(&mut y, 52.0, full.width() * 0.16);
            slab(ui, r5, accent);
            let tw = r5.width() / d.batteries.len() as f32;
            for (i, b) in d.batteries.iter().enumerate() {
                let c = egui::Rect::from_min_size(egui::pos2(r5.left() + i as f32 * tw, r5.top()), egui::vec2(tw, r5.height())).shrink(6.0);
                let pct = (b.fraction * 100.0).round() as u32;
                let col = if !b.charging && pct < 20 { theme::DANGER } else { accent };
                let src = if b.label == "HMD" { icons::HEADSET } else { icons::CONTROLLER };
                icon(src, 22.0, col).paint_at(ui, egui::Rect::from_center_size(egui::pos2(c.left() + 18.0, c.center().y - 4.0), egui::vec2(22.0, 22.0)));
                let bar = egui::Rect::from_min_size(egui::pos2(c.left() + 6.0, c.bottom() - 7.0), egui::vec2(24.0, 4.0));
                ui.painter().rect_filled(bar, 2.0, egui::Color32::from_white_alpha(0x20));
                ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(24.0 * b.fraction.clamp(0.0, 1.0), 4.0)), 2.0, col);
                let txt = match (b.charging, d.estimates.get(i).cloned().flatten()) {
                    (true, _) => format!("⚡{pct}%"),
                    (false, Some(e)) => format!("{pct}% · {e}"),
                    (false, None) => format!("{pct}%"),
                };
                ui.painter().text(egui::pos2(c.left() + 38.0, c.center().y), egui::Align2::LEFT_CENTER, txt,
                    egui::FontId::monospace(theme::SIZE_CAPTION + 1.0), theme::FG_PRIMARY);
            }
        }
    });
    act
}

fn slab(ui: &egui::Ui, r: egui::Rect, accent: egui::Color32) {
    ui.painter().rect_filled(r, 22.0, egui::Color32::from_black_alpha(242));
    ui.painter().rect_stroke(r, 22.0, egui::Stroke::new(1.2_f32, accent.gamma_multiply(0.22)));
}

fn icon_button(ui: &mut egui::Ui, r: egui::Rect, src: egui::ImageSource<'static>, size: f32, on: bool, tip: &str, accent: egui::Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(tip), egui::Sense::click()).on_hover_text(tip);
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    if hover > 0.0 || on {
        let fill = if on { accent.gamma_multiply(0.22) } else { egui::Color32::from_white_alpha((hover * 18.0) as u8) };
        ui.painter().rect_filled(r, 12.0, fill);
    }
    let tint = if on { accent } else { motion::mix(theme::FG_PRIMARY, accent, hover) };
    icon(src, size, tint).paint_at(ui, egui::Rect::from_center_size(r.center(), egui::vec2(size, size)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn glyph_button(ui: &mut egui::Ui, r: egui::Rect, g: crate::glyphs::Glyph, tip: &str, accent: egui::Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(tip), egui::Sense::click()).on_hover_text(tip);
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(r, 10.0, egui::Color32::from_white_alpha((hover * 18.0) as u8));
    }
    crate::glyphs::draw(ui.painter(), g, r.center(), 14.0, motion::mix(theme::FG_PRIMARY, accent, hover));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
