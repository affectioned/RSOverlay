//! Dashboard shell: page model, sidebar and header strip.
//!
//! The same layout on the desktop and in VR, all on AMOLED black. A
//! labelled sidebar (every destination says what it is; no guessing
//! from icons), a quiet header, page titles over an accent rule:
//!
//! ```text
//! ┌────────────┬─────────────────────────────────────────┐
//! │ ◈ RSOverlay│ 10:42 PM · Tue, Nov 24  [Search…]  82 🔔 │  header
//! │ ⌨ Kbd ＋Add├─────────────────────────────────────────┤  quick actions
//! │            │  Home                                   │  (desktop only)
//! │            │  ──                                     │
//! │ ⌂ Home     │                                         │
//! │ ▭ Desktop  │        page content (scrolls)           │
//! │ ♪ Audio    │                                         │
//! │ …          │                                         │
//! └────────────┴─────────────────────────────────────────┘
//! ```
//!
//! Every interactive element is ≥ 40 px tall: laser pointers jitter, so
//! VR hit targets need to be roughly twice a desktop button.

use crate::battery::BatteryReading;
use crate::icons::{self, icon};
use crate::locale;
use crate::motion;
use crate::theme;
use chrono::Local;

pub const RAIL_WIDTH: f32 = 200.0;
pub const NAV_ITEM_H: f32 = 40.0;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Home,
    Desktop,
    Audio,
    Chat,
    Macros,
    Notifications,
    Layouts,
    Settings,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Home,
        Page::Desktop,
        Page::Audio,
        Page::Chat,
        Page::Macros,
        Page::Notifications,
        Page::Layouts,
        Page::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Desktop => "Desktop",
            Self::Audio => "Audio",
            Self::Chat => "Chat",
            Self::Macros => "Macros",
            Self::Notifications => "Notifications",
            Self::Layouts => "Layouts",
            Self::Settings => "Settings",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Desktop => "desktop",
            Self::Audio => "audio",
            Self::Chat => "chat",
            Self::Macros => "macros",
            Self::Notifications => "notifications",
            Self::Layouts => "layouts",
            Self::Settings => "settings",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.slug() == s)
    }

    fn icon(self) -> egui::ImageSource<'static> {
        match self {
            Self::Home => icons::HOME,
            Self::Desktop => icons::MONITOR,
            Self::Audio => icons::SPEAKER,
            Self::Chat => icons::CHAT,
            Self::Macros => icons::GRID,
            Self::Notifications => icons::BELL,
            Self::Layouts => icons::BRAND,
            Self::Settings => icons::SETTINGS,
        }
    }
}

/// What the sidebar's quick actions asked for this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QuickActions {
    pub toggle_keyboard: bool,
    pub add_window: bool,
}

/// Left sidebar: wordmark, quick-action tiles (desktop only: in VR the
/// wrist toolbar already carries Keyboard / Add / Recenter / Stats), then
/// the labelled destinations. `badge` puts a count on Notifications.
///
/// The selection highlight is one shape that *slides* to the active
/// row: a placeholder is reserved in the paint list before the rows are
/// drawn, then filled once every row rect is known, so it sits beneath
/// icons and labels without a second layer.
pub fn nav_rail(ui: &mut egui::Ui, page: &mut Page, badge: usize, keyboard_open: bool, vr: bool, accent: egui::Color32) -> QuickActions {
    // Hairline between sidebar and content.
    let edge = ui.max_rect().right() + 6.0;
    ui.painter().line_segment(
        [egui::pos2(edge, ui.max_rect().top() - 4.0), egui::pos2(edge, ui.max_rect().bottom())],
        egui::Stroke::new(1.0_f32, theme::STROKE_CARD),
    );
    ui.horizontal(|ui| {
        ui.set_height(36.0);
        ui.add_space(8.0);
        ui.add(icon(icons::BRAND, 22.0, accent));
        ui.add_space(2.0);
        ui.label(egui::RichText::new("RSOverlay").font(theme::display(19.0)).color(theme::FG_PRIMARY));
    });
    let quick = if vr {
        ui.add_space(6.0);
        QuickActions::default()
    } else {
        ui.add_space(8.0);
        let q = quick_actions(ui, keyboard_open, accent);
        ui.add_space(10.0);
        q
    };
    let slot = ui.painter().add(egui::Shape::Noop);
    let mut selected_rect = None;
    for p in Page::ALL {
        let count = if p == Page::Notifications { badge } else { 0 };
        let (rect, clicked) = nav_item(ui, p.icon(), p.label(), *page == p, count, accent);
        if clicked {
            *page = p;
        }
        if *page == p {
            selected_rect = Some(rect);
        }
    }
    if let Some(target) = selected_rect {
        let ctx = ui.ctx().clone();
        let y = motion::value(&ctx, egui::Id::new("nav-highlight-y"), target.top(), motion::MEDIUM);
        let rect = egui::Rect::from_min_size(egui::pos2(target.left(), y), target.size());
        ui.painter().set(slot, egui::Shape::rect_filled(rect, theme::RADIUS_CONTROL, accent.gamma_multiply(0.14)));
    }
    quick
}

/// Two-column grid of labelled action tiles.
fn quick_actions(ui: &mut egui::Ui, keyboard_open: bool, accent: egui::Color32) -> QuickActions {
    let mut out = QuickActions::default();
    let tiles: [(egui::ImageSource<'static>, &str, &str, bool, u8); 2] = [
        (icons::KEYBOARD, "Keyboard", if keyboard_open { "Hide the keyboard" } else { "Show the keyboard" }, keyboard_open, 0),
        (icons::PLUS, "Add window", "Capture a window or monitor", false, 1),
    ];
    const GAP: f32 = 6.0;
    let w = (ui.available_width() - GAP) / 2.0;
    for pair in tiles.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (src, label, tip, on, id) in pair.iter().cloned() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 56.0), egui::Sense::click());
                let resp = resp.on_hover_text(tip);
                let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
                let fill = motion::mix(theme::BG_CARD, theme::BG_RAISED, hover);
                ui.painter().rect_filled(rect, theme::RADIUS_CONTROL, fill);
                let stroke = if on { accent } else { theme::STROKE_CARD };
                ui.painter().rect_stroke(rect, theme::RADIUS_CONTROL, egui::Stroke::new(1.0_f32, stroke));
                let tint = if on { accent } else { motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover) };
                icon(src, 20.0, tint).paint_at(ui, egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 10.0), egui::vec2(20.0, 20.0)));
                ui.painter().text(rect.center() + egui::vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, label,
                    egui::FontId::proportional(theme::SIZE_CAPTION), motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover.max(on as u8 as f32)));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    match id {
                        0 => out.toggle_keyboard = true,
                        _ => out.add_window = true,
                    }
                }
            }
        });
        ui.add_space(GAP - ui.spacing().item_spacing.y);
    }
    out
}

fn nav_item(
    ui: &mut egui::Ui,
    src: egui::ImageSource<'static>,
    label: &str,
    selected: bool,
    badge: usize,
    accent: egui::Color32,
) -> (egui::Rect, bool) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ITEM_H), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered() && !selected, motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, theme::RADIUS_CONTROL, theme::BG_CARD.gamma_multiply(hover));
    }
    // Icon takes the accent and the label brightens as the row activates.
    let on = motion::toggle(ui.ctx(), resp.id.with("sel"), selected, motion::MEDIUM);
    let tint = motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, on.max(hover));
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 22.0, rect.center().y), egui::vec2(18.0, 18.0));
    icon(src, 18.0, motion::mix(tint, accent, on)).paint_at(ui, icon_rect);
    let painter = ui.painter();
    let font = if selected { theme::medium(theme::SIZE_BODY) } else { egui::FontId::proportional(theme::SIZE_BODY) };
    painter.text(egui::pos2(rect.left() + 42.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font, tint);
    if badge > 0 {
        let text = if badge > 99 { "99+".to_string() } else { badge.to_string() };
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(theme::SIZE_CAPTION), theme::BG_ROOT);
        let w = (galley.size().x + 12.0).max(20.0);
        let pill = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0 - w / 2.0, rect.center().y), egui::vec2(w, 20.0));
        painter.rect_filled(pill, 10.0, accent);
        painter.galley(pill.center() - galley.size() / 2.0, galley, theme::BG_ROOT);
    }
    (rect, resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked())
}

/// What the header asked for this frame.
#[derive(Default)]
pub struct HeaderActions {
    pub open_notifications: bool,
    /// A global-search result was chosen.
    pub search_pick: Option<crate::search::Target>,
}

/// Global search box state (query + highlighted result).
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub selected: usize,
}

/// Header search field + floating results list. Enter picks the
/// highlighted result; ↑/↓ move the highlight; Esc clears.
fn search_box(ui: &mut egui::Ui, st: &mut SearchState, entries: &[crate::search::Entry], accent: egui::Color32) -> Option<crate::search::Target> {
    let edit = ui.add(
        egui::TextEdit::singleline(&mut st.query)
            .hint_text("Search settings, pages, windows…")
            .margin(egui::vec2(12.0, 8.0))
            .desired_width(260.0),
    );
    if edit.changed() {
        st.selected = 0;
    }
    let results = crate::search::search(entries, &st.query, 8);
    if results.is_empty() {
        return None;
    }
    st.selected = st.selected.min(results.len() - 1);
    let (up, down, enter, esc) = ui.input(|i| {
        (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
    });
    if down {
        st.selected = (st.selected + 1).min(results.len() - 1);
    }
    if up {
        st.selected = st.selected.saturating_sub(1);
    }
    if esc {
        st.query.clear();
        return None;
    }
    let mut pick = (enter && (edit.has_focus() || edit.lost_focus())).then(|| results[st.selected].target.clone());
    egui::Area::new(egui::Id::new("search-results"))
        .order(egui::Order::Foreground)
        .fixed_pos(edit.rect.left_bottom() + egui::vec2(0.0, 6.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::none()
                .fill(theme::BG_CARD)
                .stroke(egui::Stroke::new(1.0_f32, theme::STROKE_CARD))
                .rounding(theme::RADIUS_CARD)
                .inner_margin(egui::Margin::same(6.0))
                .show(ui, |ui| {
                    ui.set_width(380.0);
                    for (i, e) in results.iter().enumerate() {
                        let sel = i == st.selected;
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(380.0, 40.0), egui::Sense::click());
                        if sel || resp.hovered() {
                            ui.painter().rect_filled(r, theme::RADIUS_CONTROL, if sel { theme::BG_HOVER } else { theme::BG_RAISED });
                        }
                        if sel {
                            let bar = egui::Rect::from_min_size(r.left_top() + egui::vec2(0.0, 10.0), egui::vec2(2.0, r.height() - 20.0));
                            ui.painter().rect_filled(bar, 1.0, accent);
                        }
                        ui.painter().text(r.left_center() + egui::vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, &e.title,
                            egui::FontId::proportional(theme::SIZE_BODY), theme::FG_PRIMARY);
                        ui.painter().text(r.right_center() - egui::vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, e.hint,
                            egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), theme::FG_MUTED);
                        if resp.clicked() {
                            pick = Some(e.target.clone());
                        }
                    }
                });
        });
    if pick.is_some() {
        st.query.clear();
        st.selected = 0;
    }
    pick
}

pub fn header(
    ui: &mut egui::Ui,
    batteries: &[BatteryReading],
    unread: usize,
    search: &mut SearchState,
    entries: &[crate::search::Entry],
    accent: egui::Color32,
) -> HeaderActions {
    let mut out = HeaderActions::default();
    let now = Local::now();
    let time = locale::format_time(&now);
    let date = locale::format_date(&now);
    ui.horizontal(|ui| {
        ui.set_height(44.0);
        ui.label(egui::RichText::new(time).font(theme::medium(theme::SIZE_BODY + 1.0)).color(theme::FG_PRIMARY));
        ui.label(theme::body_muted(&date));
        ui.add_space(12.0);
        // Chips only where they fit beside search (VR is narrower); the
        // full list lives in the Battery card and on the wrist.
        let chips = (((ui.available_width() - 290.0 - 56.0) / 124.0).floor().max(0.0) as usize).min(3);
        out.search_pick = search_box(ui, search, entries, accent);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = if unread > 0 { format!("Notifications · {unread} unread") } else { "Notifications".to_string() };
            if icon_button(ui, icons::BELL, unread > 0, accent).on_hover_text(tip).clicked() {
                out.open_notifications = true;
            }
            ui.add_space(4.0);
            // Battery chips, right-to-left so the HMD ends up leftmost.
            for b in batteries.iter().take(chips).rev() {
                battery_chip(ui, b, accent);
            }
        });
    });
    out
}

fn icon_button(ui: &mut egui::Ui, src: egui::ImageSource<'static>, active: bool, accent: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    ui.painter().rect_filled(rect, theme::RADIUS_CONTROL, motion::mix(theme::BG_ROOT, theme::BG_RAISED, hover));
    let tint = motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, hover);
    icon(src, 20.0, tint).paint_at(ui, egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)));
    if active {
        ui.painter().circle_filled(rect.center() + egui::vec2(7.0, -8.0), 4.0, accent);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn battery_chip(ui: &mut egui::Ui, b: &BatteryReading, _accent: egui::Color32) {
    let pct = (b.fraction * 100.0).round() as u32;
    let low = !b.charging && pct < 20;
    // With a SteamVR icon the icon names the device; otherwise "L", "HMD"…
    let has_icon = b.icon.is_some();
    let short = if has_icon { String::new() } else { format!("{} ", short_device_label(&b.label)) };
    let text = if b.charging { format!("{short}⚡{pct}%") } else { format!("{short}{pct}%") };
    // Painted rather than a Frame: a Frame in a right-to-left row
    // stretches to the full header height.
    let font = egui::FontId::proportional(theme::SIZE_CAPTION + 1.0);
    let color = if low { theme::DANGER } else { theme::FG_MUTED };
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let icon_w = if has_icon { 22.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 18.0 + icon_w, 28.0), egui::Sense::hover());
    ui.painter().rect_stroke(rect, rect.height() / 2.0, egui::Stroke::new(1.0_f32, if low { theme::DANGER } else { theme::STROKE_CARD }));
    if has_icon {
        let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 9.0 + 9.0, rect.center().y), egui::vec2(20.0, 14.0));
        crate::battery::paint_device_icon(ui, b, ir, color);
    }
    ui.painter().galley(egui::pos2(rect.left() + 9.0 + icon_w, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_text(format!("{} battery", b.label));
}

/// "Left hand" → "L", "Tracker · VIVE Tracker 3.0" → "TRK". Pure, tested.
pub fn short_device_label(label: &str) -> &'static str {
    match label {
        "HMD" => "HMD",
        "Left hand" => "L",
        "Right hand" => "R",
        l if l.starts_with("Tracker") => "TRK",
        _ => "DEV",
    }
}

/// Live feedback while a panel is grabbed (accessibility: you always
/// see *that* you're moving/resizing and *by how much*).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct GrabHud {
    pub width_m: f32,
    pub dist_m: f32,
    /// Size is changing this frame (stick X or two-hand).
    pub resizing: bool,
}

pub fn paint_grab_hud(ctx: &egui::Context, hud: GrabHud, accent: egui::Color32) {
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("grab-hud")));
    let r = ctx.screen_rect().shrink(5.0);
    p.rect_stroke(r, theme::RADIUS_CARD + 4.0, egui::Stroke::new(3.0_f32, accent));
    let text = if hud.resizing {
        format!("Resizing · {:.0} cm wide", hud.width_m * 100.0)
    } else {
        format!("Moving · {:.0} cm wide · {:.1} m away", hud.width_m * 100.0, hud.dist_m)
    };
    let galley = p.layout_no_wrap(text, egui::FontId::proportional(theme::SIZE_BODY + 1.0), theme::BG_ROOT);
    let pill = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 26.0), galley.size() + egui::vec2(28.0, 14.0));
    p.rect_filled(pill, pill.height() / 2.0, accent);
    p.galley(pill.center() - galley.size() / 2.0, galley, theme::BG_ROOT);
}

/// Laser-beam strip texture: white, fading in from the controller end
/// (left) so the beam reads as emitted, not a stick. Tinted at runtime.
/// The laser cursor's own texture (VR): white ring + dot with a dark
/// rim, tinted to the accent by the overlay colour.
pub fn paint_cursor_dot(ctx: &egui::Context) {
    let p = ctx.layer_painter(egui::LayerId::background());
    let r = ctx.screen_rect();
    let (c, s) = (r.center(), r.width().min(r.height()) / 2.0);
    p.circle_filled(c, s * 0.95, egui::Color32::from_black_alpha(150));
    p.circle_stroke(c, s * 0.72, egui::Stroke::new(s * 0.16, egui::Color32::WHITE));
    p.circle_filled(c, s * 0.2, egui::Color32::WHITE);
}

pub fn paint_beam(ctx: &egui::Context) {
    let p = ctx.layer_painter(egui::LayerId::background());
    let r = ctx.screen_rect();
    let n = 48;
    for i in 0..n {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        let a = ((t0 / 0.35).min(1.0) * 255.0) as u8;
        let seg = egui::Rect::from_min_max(egui::pos2(r.left() + t0 * r.width(), r.top()), egui::pos2(r.left() + t1 * r.width(), r.bottom()));
        p.rect_filled(seg, 0.0, crate::theme::veil(a));
    }
}

/// Page title row used at the top of every page body.
/// Page title: display type over a short accent rule, the product's
/// signature mark.
pub fn page_title(ui: &mut egui::Ui, title: &str, accent: egui::Color32) {
    ui.add_space(4.0);
    let r = ui.label(egui::RichText::new(title).font(theme::display(26.0)).color(theme::FG_PRIMARY)).rect;
    let rule = egui::Rect::from_min_size(r.left_bottom() + egui::vec2(1.0, 6.0), egui::vec2(28.0, 3.0));
    ui.painter().rect_filled(rule, 1.5, accent);
    ui.add_space(theme::GUTTER + 6.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_roundtrip() {
        for p in Page::ALL {
            assert_eq!(Page::from_slug(p.slug()), Some(p));
        }
        assert_eq!(Page::from_slug("nope"), None);
    }

    #[test]
    fn chip_labels() {
        assert_eq!(short_device_label("Left hand"), "L");
        assert_eq!(short_device_label("Tracker · VIVE Tracker 3.0"), "TRK");
        assert_eq!(short_device_label("Gamepad"), "DEV");
    }
}
