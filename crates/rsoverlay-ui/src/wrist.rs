//! Wrist toolbar — the always-there VR control centre, laid out as the
//! stacked rows XSOverlay users know (our own look and icons). Every
//! button carries a visible label, so nothing has to be learned by
//! trial and error.
//!
//! Compact by default: the four everyday actions + More, the clock with
//! media, and device batteries. What compact shows is user-editable
//! (Settings → Wrist toolbar, [`compact`] bits). More reveals the rest
//! (all actions, layout slots, views); the choice persists. Expanded:
//!
//! ```text
//! ╭─────────────────────────────────────────────────╮
//! │  ＋   ↻   ⌨   ▦   🗀   ⚙   🗑                    │  actions
//! │ Add Recenter Keyboard Dashboard Layouts …       │
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

/// Bits of the compact-wrist mask (`VrCfg::wrist_compact`).
pub mod compact {
    /// Action names, bit i = action i (shared with the full-toolbar mask).
    pub const ACTION_NAMES: [&str; 7] = ["Add", "Recenter", "Keyboard", "Dashboard", "Layouts", "Settings", "Close all"];
    pub const CLOCK: u32 = 1 << 8;
    pub const MEDIA: u32 = 1 << 9;
    pub const BATTERY: u32 = 1 << 10;
    pub const LAYOUTS: u32 = 1 << 11;
    pub const VIEWS: u32 = 1 << 12;
    /// Session timer line under the clock.
    pub const TIMER: u32 = 1 << 13;
    /// Status chips (one row, each its own module).
    pub const PERF: u32 = 1 << 14;
    pub const RAM: u32 = 1 << 15;
    pub const ALERTS: u32 = 1 << 16;
    /// Non-action sections, in settings order.
    pub const SECTIONS: &[(u32, &str)] = &[
        (CLOCK, "Clock"),
        (TIMER, "Session timer"),
        (BATTERY, "Batteries"),
        (MEDIA, "Media"),
        (LAYOUTS, "Layout slots"),
        (VIEWS, "Views"),
    ];
    /// Status-chip modules, in row order.
    pub const CHIPS: &[(u32, &str)] = &[(PERF, "CPU / GPU"), (RAM, "RAM"), (ALERTS, "Alerts")];
    /// Compact action order: reach order, everyday four first.
    pub const ORDER: [usize; 7] = [3, 2, 0, 1, 4, 5, 6];
}

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
    /// Which action buttons to show (bit i = actions[i]).
    pub buttons: u8,
    /// Compact contents (see [`compact`]).
    pub compact: u32,
    /// Full set (layouts, views, devices) vs the compact default.
    pub expanded: bool,
    /// Edit mode: everything is shown; tapping an item shows / hides it
    /// on the compact wrist instead of running it.
    pub editing: bool,
    /// Latest machine load (only sampled when a chip needs it).
    pub system: Option<&'a rsoverlay_sysinfo::Snapshot>,
    /// Notifications not yet looked at.
    pub unread: usize,
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
    /// The More / Less button.
    pub toggle_more: bool,
    /// Edit / Done.
    pub toggle_edit: bool,
    /// Edit mode: flip this compact bit.
    pub toggle_compact: Option<u32>,
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
        let r1 = row(&mut y, 80.0, 0.0);
        slab(ui, r1, accent);
        let actions: [(egui::ImageSource<'static>, &str, &str, bool); 7] = [
            (icons::PLUS, "Add", "Add a window or monitor", false),
            (icons::RECENTER, "Recenter", "Recenter panels in front of you", false),
            (icons::KEYBOARD, "Keyboard", "Show or hide the keyboard", d.keyboard_open),
            (icons::GRID, "Dashboard", "Open or close the dashboard", false),
            (icons::FOLDER, "Layouts", "Saved panel layouts", false),
            (icons::SETTINGS, "Settings", "Settings", false),
            (icons::TRASH, "Close all", "Close all floating panels", false),
        ];
        // Compact: the user's compact picks, in reach order. Expanded:
        // every enabled action in toolbar order. Editing: all of them.
        let shown: Vec<_> = if d.editing {
            (0..7).map(|i| (i, actions[i].clone())).collect()
        } else if d.expanded {
            (0..7).filter(|&i| d.buttons & (1 << i) != 0).map(|i| (i, actions[i].clone())).collect()
        } else {
            compact::ORDER.iter().filter(|&&i| d.compact & (1 << i) != 0).map(|&i| (i, actions[i].clone())).collect()
        };
        // Section visibility: expanded / editing show everything; compact
        // follows the mask.
        let has = |bit: u32| d.editing || d.expanded || d.compact & bit != 0;
        // Edit-mode hit areas: (rect, compact bit).
        let mut edits: Vec<(egui::Rect, u32)> = Vec::new();
        let show_edit = d.expanded && !d.editing;
        let cell = r1.width() / (shown.len() + 1 + usize::from(show_edit)) as f32;
        let more = egui::Rect::from_min_size(egui::pos2(r1.right() - cell, r1.top()), egui::vec2(cell, r1.height()));
        let (more_src, more_label, more_tip) = if d.editing {
            (icons::EDIT, "Done", "Finish editing the compact wrist")
        } else if d.expanded {
            (icons::MORE, "Less", "Show fewer options")
        } else {
            (icons::MORE, "More", "Show more options")
        };
        if labelled_button(ui, more.shrink(6.0), more_src, 24.0, d.expanded || d.editing, more_label, more_tip, accent) {
            if d.editing {
                act.toggle_edit = true;
            } else {
                act.toggle_more = true;
            }
        }
        if show_edit {
            let e = more.translate(egui::vec2(-cell, 0.0));
            if labelled_button(ui, e.shrink(6.0), icons::EDIT, 24.0, false, "Edit", "Choose what the compact wrist shows", accent) {
                act.toggle_edit = true;
            }
        }
        for (slot, (i, (src, label, tip, on))) in shown.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(r1.left() + slot as f32 * cell, r1.top()), egui::vec2(cell, r1.height()));
            if d.editing {
                paint_labelled(ui, c.shrink(6.0), src, 24.0, label, theme::FG_PRIMARY, theme::FG_PRIMARY);
                edits.push((c.shrink(6.0), 1 << i));
                continue;
            }
            if labelled_button(ui, c.shrink(6.0), src, 24.0, on, label, tip, accent) {
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
        if has(compact::LAYOUTS) {
        let r2 = row(&mut y, 64.0, full.width() * 0.08);
        slab(ui, r2, accent);
        edits.push((r2, compact::LAYOUTS));
        let half = r2.width() * 0.40;
        let sel = egui::Rect::from_min_size(r2.min, egui::vec2(half, r2.height())).shrink(8.0);
        ui.painter().rect_filled(sel, theme::RADIUS_CONTROL, theme::BG_RAISED);
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
        let tool_defs: [(egui::ImageSource<'static>, &str, &str, bool, bool); 4] = [
            (star, "Startup", "Load this slot at startup", d.slot_favorite, d.slot_filled),
            (icons::SAVE, "Save", "Save the current arrangement here", false, true),
            (icons::LOAD, "Load", "Load this slot", false, d.slot_filled),
            (icons::TRASH, "Clear", "Clear this slot", false, d.slot_filled),
        ];
        for (i, (src, label, tip, on, enabled)) in tool_defs.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(tools.left() + i as f32 * tw, tools.top()), egui::vec2(tw, tools.height()));
            if !enabled {
                paint_labelled(ui, c.shrink(2.0), src, 18.0, label, theme::FG_FAINT, theme::FG_FAINT);
            } else if labelled_button(ui, c.shrink(2.0), src, 18.0, on, label, tip, accent) {
                act.layout = Some(match i {
                    0 => LayoutAction::ToggleFavorite(d.slot),
                    1 => LayoutAction::Save(d.slot),
                    2 => LayoutAction::Load(d.slot),
                    _ => LayoutAction::Clear(d.slot),
                });
            }
        }
        }

        // 3 · Clock + views (+ media inside the same slab).
        let show_clock = has(compact::CLOCK);
        let show_views = has(compact::VIEWS);
        // Editing with nothing playing: a stand-in so Media can be toggled.
        let title = d.title.filter(|_| has(compact::MEDIA)).or(d.editing.then_some("Media · shows while something plays"));
        let top_h = if show_clock || show_views { 108.0 } else { 8.0 };
        let media_h = if title.is_some() { 56.0 } else { 0.0 };
        if show_clock || show_views || title.is_some() {
        let r3 = row(&mut y, top_h + media_h, full.width() * 0.12);
        slab(ui, r3, accent);
        if show_clock {
        let clock_w = if show_views { r3.width() / 2.0 - 16.0 } else { r3.width() - 20.0 };
        edits.push((egui::Rect::from_min_size(r3.min + egui::vec2(8.0, 6.0), egui::vec2(clock_w, 76.0)), compact::CLOCK));
        let now = Local::now();
        let time = if locale::uses_24h_clock() { now.format("%H:%M").to_string() } else { now.format("%-I:%M %p").to_string() };
        let p = ui.painter();
        p.text(r3.left_top() + egui::vec2(18.0, 8.0), egui::Align2::LEFT_TOP, time, theme::display(40.0), theme::FG_PRIMARY);
        p.text(r3.left_top() + egui::vec2(18.0, 60.0), egui::Align2::LEFT_TOP, now.format("%a, %b %-d").to_string(),
            egui::FontId::proportional(theme::SIZE_BODY), theme::FG_MUTED);
        if has(compact::TIMER) {
            let s = d.session_secs;
            let at = r3.left_top() + egui::vec2(18.0, 82.0);
            let galley = p.layout_no_wrap(format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
                egui::FontId::monospace(theme::SIZE_CAPTION + 2.0), theme::FG_MUTED);
            if d.editing {
                edits.push((egui::Rect::from_min_size(at - egui::vec2(6.0, 0.0), galley.size() + egui::vec2(60.0, 4.0)), compact::TIMER));
            }
            p.galley(at, galley, theme::FG_MUTED);
        }
        }
        if show_views {
        let left = if show_clock { r3.center().x - 6.0 } else { r3.left() + 10.0 };
        let views = egui::Rect::from_min_max(egui::pos2(left, r3.top() + 10.0), egui::pos2(r3.right() - 10.0, r3.top() + 98.0));
        ui.painter().rect_filled(views, theme::RADIUS_CARD, theme::BG_CARD);
        edits.push((views, compact::VIEWS));
        let vw = views.width() / 3.0;
        let view_defs: [(egui::ImageSource<'static>, &str, &str); 3] = [
            (icons::LAYERS, "Windows", "Windows & monitors"),
            (icons::MUSIC, "Media", "Media & volume"),
            (icons::CHART, "Stats", "Pop out performance stats"),
        ];
        for (i, (src, label, tip)) in view_defs.into_iter().enumerate() {
            let c = egui::Rect::from_min_size(egui::pos2(views.left() + i as f32 * vw, views.top()), egui::vec2(vw, views.height()));
            if labelled_button(ui, c.shrink(6.0), src, 28.0, false, label, tip, accent) {
                match i {
                    0 => act.open_page = Some(Page::Desktop),
                    1 => act.open_page = Some(Page::Audio),
                    _ => act.popout_stats = true,
                }
            }
        }
        }
        if let Some(title) = title {
            let m = egui::Rect::from_min_max(egui::pos2(r3.left() + 10.0, r3.top() + top_h), egui::pos2(r3.right() - 10.0, r3.bottom() - 8.0));
            ui.painter().rect_filled(m, theme::RADIUS_CONTROL, theme::BG_CARD);
            edits.push((m, compact::MEDIA));
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

        }

        // 4 · Devices.
        if d.editing && d.batteries.is_empty() {
            let r5 = row(&mut y, 52.0, full.width() * 0.16);
            slab(ui, r5, accent);
            ui.painter().text(r5.center(), egui::Align2::CENTER_CENTER, "Batteries · show when devices report them",
                egui::FontId::proportional(theme::SIZE_BODY), theme::FG_MUTED);
            edits.push((r5, compact::BATTERY));
        } else if has(compact::BATTERY) && !d.batteries.is_empty() {
            let r5 = row(&mut y, 52.0, full.width() * 0.16);
            slab(ui, r5, accent);
            edits.push((r5, compact::BATTERY));
            let tw = r5.width() / d.batteries.len() as f32;
            for (i, b) in d.batteries.iter().enumerate() {
                let c = egui::Rect::from_min_size(egui::pos2(r5.left() + i as f32 * tw, r5.top()), egui::vec2(tw, r5.height())).shrink(6.0);
                let pct = (b.fraction * 100.0).round() as u32;
                let col = if !b.charging && pct < 20 { theme::DANGER } else { accent };
                let src = if b.label == "HMD" { icons::HEADSET } else { icons::CONTROLLER };
                icon(src, 22.0, col).paint_at(ui, egui::Rect::from_center_size(egui::pos2(c.left() + 18.0, c.center().y - 4.0), egui::vec2(22.0, 22.0)));
                let bar = egui::Rect::from_min_size(egui::pos2(c.left() + 6.0, c.bottom() - 7.0), egui::vec2(24.0, 4.0));
                ui.painter().rect_filled(bar, 2.0, theme::BG_HOVER);
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
        // 5 · Status chips.
        let chips: Vec<(u32, String, egui::ImageSource<'static>, egui::Color32)> = compact::CHIPS
            .iter()
            .filter(|&&(bit, _)| has(bit))
            .map(|&(bit, _)| chip_content(bit, d, accent))
            .collect();
        if !chips.is_empty() {
            let r6 = row(&mut y, 48.0, full.width() * 0.16);
            slab(ui, r6, accent);
            let cw = r6.width() / chips.len() as f32;
            for (i, (bit, text, src, tint)) in chips.into_iter().enumerate() {
                let c = egui::Rect::from_min_size(egui::pos2(r6.left() + i as f32 * cw, r6.top()), egui::vec2(cw, r6.height())).shrink(5.0);
                if d.editing {
                    edits.push((c, bit));
                } else if bit == compact::ALERTS {
                    let resp = ui.interact(c, ui.id().with("wrist-alerts"), egui::Sense::click()).on_hover_text("Open notifications");
                    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
                    if hover > 0.0 {
                        ui.painter().rect_filled(c, theme::RADIUS_CONTROL, theme::BG_RAISED.gamma_multiply(hover));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        act.open_page = Some(Page::Notifications);
                    }
                }
                icon(src, 18.0, tint).paint_at(ui, egui::Rect::from_center_size(egui::pos2(c.left() + 18.0, c.center().y), egui::vec2(18.0, 18.0)));
                ui.painter_at(c).text(egui::pos2(c.left() + 34.0, c.center().y), egui::Align2::LEFT_CENTER, text,
                    egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), theme::FG_PRIMARY);
            }
        }

        if d.editing {
            act.toggle_compact = edit_overlays(ctx, &edits, d.compact, accent);
        }
    });
    act
}

/// Edit mode: a foreground layer over every item (so the item's own
/// buttons can't fire). Shown items get an accent ring, hidden ones are
/// dimmed; a tap flips the item's compact bit.
fn edit_overlays(ctx: &egui::Context, items: &[(egui::Rect, u32)], compact: u32, accent: egui::Color32) -> Option<u32> {
    let mut flip = None;
    egui::Area::new(egui::Id::new("wrist-edit")).order(egui::Order::Foreground).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| {
        // Grow the layer to the whole screen so it owns hits everywhere.
        ui.expand_to_include_rect(ctx.screen_rect());
        for &(r, bit) in items {
            let on = compact & bit != 0;
            let resp = ui
                .interact(r, egui::Id::new(("wrist-edit", bit)), egui::Sense::click())
                .on_hover_text(if on { "Shown on the compact wrist — tap to hide" } else { "Hidden on the compact wrist — tap to show" });
            let p = ui.painter();
            if !on {
                p.rect_filled(r, theme::RADIUS_CONTROL, egui::Color32::from_black_alpha(190));
            }
            let stroke = if on { egui::Stroke::new(2.0_f32, accent) } else { egui::Stroke::new(1.0_f32, theme::FG_FAINT) };
            p.rect_stroke(r, theme::RADIUS_CONTROL, if resp.hovered() { egui::Stroke::new(2.5_f32, theme::FG_PRIMARY) } else { stroke });
            if r.width() < 200.0 {
                // Narrow action cells: a dot, so the label doesn't hit the icon.
                let c = r.right_top() + egui::vec2(-8.0, 8.0);
                if on {
                    p.circle_filled(c, 3.5, accent);
                } else {
                    p.circle_stroke(c, 3.0, egui::Stroke::new(1.5_f32, theme::FG_MUTED));
                }
            } else {
                p.text(r.right_top() + egui::vec2(-8.0, 5.0), egui::Align2::RIGHT_TOP, if on { "Shown" } else { "Hidden" },
                    egui::FontId::proportional(theme::SIZE_CAPTION - 1.0), if on { accent } else { theme::FG_MUTED });
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                flip = Some(bit);
            }
        }
    });
    flip
}

/// One status chip: (bit, text, icon, icon tint).
fn chip_content(bit: u32, d: &WristData, accent: egui::Color32) -> (u32, String, egui::ImageSource<'static>, egui::Color32) {
    let pct = |v: Option<f32>| v.map_or("--".to_string(), |v| format!("{:.0}%", v));
    let hot = |v: Option<f32>| if v.is_some_and(|v| v >= 90.0) { theme::DANGER } else { accent };
    match bit {
        compact::PERF => {
            let (cpu, gpu) = (d.system.and_then(|s| s.cpu_pct), d.system.and_then(|s| s.gpu_pct));
            let worst = cpu.into_iter().chain(gpu).reduce(f32::max);
            (bit, format!("CPU {} · GPU {}", pct(cpu), pct(gpu)), icons::CHART, hot(worst))
        }
        compact::RAM => {
            let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
            let text = match d.system.filter(|s| s.ram_total > 0) {
                Some(s) => format!("RAM {:.1}/{:.0} GB", gb(s.ram_used), gb(s.ram_total)),
                None => "RAM --".into(),
            };
            let frac = d.system.filter(|s| s.ram_total > 0).map(|s| 100.0 * s.ram_used as f32 / s.ram_total as f32);
            (bit, text, icons::PULSE, hot(frac))
        }
        _ => {
            let text = match d.unread {
                0 => "No alerts".to_string(),
                1 => "1 alert".to_string(),
                n => format!("{n} alerts"),
            };
            (bit, text, icons::BELL, if d.unread > 0 { accent } else { theme::FG_MUTED })
        }
    }
}

fn slab(ui: &egui::Ui, r: egui::Rect, _accent: egui::Color32) {
    theme::surface(ui.painter(), r, 18.0);
}

/// Icon over a short label. The label is always visible; `tip` adds
/// the longer explanation on hover.
#[allow(clippy::too_many_arguments)]
fn labelled_button(
    ui: &mut egui::Ui,
    r: egui::Rect,
    src: egui::ImageSource<'static>,
    size: f32,
    on: bool,
    label: &str,
    tip: &str,
    accent: egui::Color32,
) -> bool {
    let resp = ui.interact(r, ui.id().with(tip), egui::Sense::click()).on_hover_text(tip);
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    if hover > 0.0 || on {
        let fill = if on { theme::BG_RAISED } else { theme::BG_RAISED.gamma_multiply(hover) };
        ui.painter().rect_filled(r, theme::RADIUS_CONTROL, fill);
    }
    let icon_tint = if on { accent } else { motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover) };
    let text_tint = motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover.max(if on { 1.0 } else { 0.0 }));
    paint_labelled(ui, r, src, size, label, icon_tint, text_tint);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn paint_labelled(ui: &egui::Ui, r: egui::Rect, src: egui::ImageSource<'static>, size: f32, label: &str, icon_tint: egui::Color32, text_tint: egui::Color32) {
    let text = (theme::SIZE_CAPTION).min(r.height() * 0.24);
    let gap = 4.0;
    let block = size + gap + text;
    let top = r.center().y - block / 2.0;
    icon(src, size, icon_tint).paint_at(ui, egui::Rect::from_center_size(egui::pos2(r.center().x, top + size / 2.0), egui::vec2(size, size)));
    ui.painter().text(egui::pos2(r.center().x, top + size + gap), egui::Align2::CENTER_TOP, label, egui::FontId::proportional(text), text_tint);
}

/// Unlabelled icon button, for controls whose meaning is universal
/// (play / pause). Still names itself on hover.
fn icon_button(ui: &mut egui::Ui, r: egui::Rect, src: egui::ImageSource<'static>, size: f32, on: bool, tip: &str, accent: egui::Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(tip), egui::Sense::click()).on_hover_text(tip);
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    if hover > 0.0 || on {
        ui.painter().rect_filled(r, theme::RADIUS_CONTROL, theme::BG_RAISED.gamma_multiply(if on { 1.0 } else { hover }));
    }
    let tint = if on { accent } else { motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover) };
    icon(src, size, tint).paint_at(ui, egui::Rect::from_center_size(r.center(), egui::vec2(size, size)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn glyph_button(ui: &mut egui::Ui, r: egui::Rect, g: crate::glyphs::Glyph, tip: &str, _accent: egui::Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(tip), egui::Sense::click()).on_hover_text(tip);
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(r, theme::RADIUS_CONTROL, theme::BG_HOVER.gamma_multiply(hover));
    }
    crate::glyphs::draw(ui.painter(), g, r.center(), 14.0, motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data<'a>(system: Option<&'a rsoverlay_sysinfo::Snapshot>, unread: usize) -> WristData<'a> {
        WristData {
            batteries: &[],
            estimates: &[],
            title: None,
            artist: None,
            playing: false,
            session_secs: 0,
            slot: 0,
            slot_filled: false,
            slot_favorite: false,
            keyboard_open: false,
            accent: egui::Color32::WHITE,
            buttons: 0x7F,
            compact: rsoverlay_core::config::DEFAULT_WRIST_COMPACT,
            expanded: false,
            editing: false,
            system,
            unread,
        }
    }

    #[test]
    fn alert_chip_counts_unread() {
        assert_eq!(chip_content(compact::ALERTS, &data(None, 0), egui::Color32::WHITE).1, "No alerts");
        assert_eq!(chip_content(compact::ALERTS, &data(None, 1), egui::Color32::WHITE).1, "1 alert");
        assert_eq!(chip_content(compact::ALERTS, &data(None, 4), egui::Color32::WHITE).1, "4 alerts");
    }

    #[test]
    fn perf_chip_goes_red_when_either_load_is_pegged() {
        let s = rsoverlay_sysinfo::Snapshot { cpu_pct: Some(20.0), gpu_pct: Some(97.0), ..Default::default() };
        let (_, text, _, tint) = chip_content(compact::PERF, &data(Some(&s), 0), egui::Color32::WHITE);
        assert_eq!(text, "CPU 20% · GPU 97%");
        assert_eq!(tint, theme::DANGER);
        let (_, text, _, _) = chip_content(compact::PERF, &data(None, 0), egui::Color32::WHITE);
        assert_eq!(text, "CPU -- · GPU --");
    }

    #[test]
    fn module_bits_are_distinct_and_clear_of_actions() {
        let all: Vec<u32> = compact::SECTIONS.iter().chain(compact::CHIPS).map(|&(b, _)| b).collect();
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a.count_ones(), 1);
            assert!(*a > 0x7F, "module bits must not overlap the 7 action bits");
            assert!(all[i + 1..].iter().all(|b| b != a));
        }
    }
}
