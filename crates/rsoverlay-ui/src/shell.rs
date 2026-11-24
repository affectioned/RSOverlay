//! Dashboard shell: page model, navigation rail and header strip.
//!
//! Layout, all on AMOLED black:
//!
//! ```text
//! ┌──────────────────────────────────────────────────────┐
//! │ (R) 10:42 PM  Tuesday · Nov 24     HMD 82  L 14  ⌨ 🔔3│  header
//! ├────────────┬─────────────────────────────────────────┤
//! │ ▌Home      │                                         │
//! │  Desktop   │        page content (scrolls)           │
//! │  Audio     │                                         │
//! │  …         │                                         │
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

pub const RAIL_WIDTH: f32 = 184.0;
pub const NAV_ITEM_H: f32 = 44.0;

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

/// Left navigation rail. `badge` puts a count pill on Notifications.
///
/// The selection highlight is one shape that *slides* to the active
/// row: a placeholder is reserved in the paint list before the rows are
/// drawn, then filled once every row rect is known, so it sits beneath
/// icons and labels without a second layer.
pub fn nav_rail(ui: &mut egui::Ui, page: &mut Page, badge: usize, accent: egui::Color32) {
    let slot = ui.painter().add(egui::Shape::Noop);
    let mut selected_rect = None;
    for p in Page::ALL {
        let count = if p == Page::Notifications { badge } else { 0 };
        let (rect, clicked) = nav_item(ui, p, *page == p, count, accent);
        if clicked {
            *page = p;
        }
        if *page == p {
            selected_rect = Some(rect);
        }
        ui.add_space(4.0);
    }
    let Some(target) = selected_rect else { return };
    let ctx = ui.ctx().clone();
    let y = motion::value(&ctx, egui::Id::new("nav-highlight-y"), target.top(), motion::MEDIUM);
    let rect = egui::Rect::from_min_size(egui::pos2(target.left(), y), target.size());
    let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 10.0), egui::vec2(3.0, rect.height() - 20.0));
    ui.painter().set(
        slot,
        egui::Shape::Vec(vec![
            egui::Shape::rect_filled(rect, theme::RADIUS_CARD, accent.gamma_multiply(0.14)),
            egui::Shape::rect_filled(bar, 1.5, accent),
        ]),
    );
}

fn nav_item(ui: &mut egui::Ui, p: Page, selected: bool, badge: usize, accent: egui::Color32) -> (egui::Rect, bool) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ITEM_H), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered() && !selected, motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, theme::RADIUS_CARD, crate::theme::veil((hover * 10.0) as u8));
    }
    // Icon + label tint cross-fade into the accent as the row activates.
    let on = motion::toggle(ui.ctx(), resp.id.with("sel"), selected, motion::MEDIUM);
    let tint = motion::mix(theme::FG_MUTED, accent, on);
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(20.0, 20.0));
    icon(p.icon(), 20.0, tint).paint_at(ui, icon_rect);
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.left() + 48.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        p.label(),
        egui::FontId::proportional(theme::SIZE_BODY),
        motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, on.max(hover)),
    );
    if badge > 0 {
        let text = if badge > 99 { "99+".to_string() } else { badge.to_string() };
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(theme::SIZE_CAPTION), theme::BG_ROOT);
        let w = galley.size().x + 12.0;
        let pill = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0 - w / 2.0, rect.center().y), egui::vec2(w, 18.0));
        painter.rect_filled(pill, 9.0, accent);
        painter.galley(pill.center() - galley.size() / 2.0, galley, theme::BG_ROOT);
    }
    (rect, resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked())
}

/// SteamVR-style dock: a centred row of large icon tiles (VR dashboard).
pub fn dock(ui: &mut egui::Ui, page: &mut Page, badge: usize, keyboard: &mut bool, accent: egui::Color32) {
    const TILE: egui::Vec2 = egui::vec2(76.0, 70.0);
    const GAP: f32 = 6.0;
    // Pages + a Keyboard toggle tile at the end.
    let n = Page::ALL.len() as f32 + 1.0;
    let total = n * TILE.x + (n - 1.0) * GAP;
    let left = ui.max_rect().left() + ((ui.available_width() - total) / 2.0).max(0.0);
    let capsule = egui::Rect::from_min_size(egui::pos2(left - 10.0, ui.cursor().top() - 8.0), egui::vec2(total + 20.0, TILE.y + 16.0));
    theme::glass(ui.painter(), capsule, 30.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = GAP;
        for p in Page::ALL {
            let (rect, resp) = ui.allocate_exact_size(TILE, egui::Sense::click());
            let selected = *page == p;
            let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
            let on = motion::toggle(ui.ctx(), resp.id.with("sel"), selected, motion::MEDIUM);
            let base = crate::theme::veil((hover * 16.0) as u8);
            ui.painter().rect_filled(rect, 22.0, motion::mix(base, accent.gamma_multiply(0.2), on));
            let tint = motion::mix(theme::FG_MUTED, accent, on);
            let icon_rect = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 10.0), egui::vec2(28.0, 28.0));
            icon(p.icon(), 28.0, tint).paint_at(ui, icon_rect);
            ui.painter().text(
                rect.center_bottom() - egui::vec2(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                p.label(),
                egui::FontId::proportional(theme::SIZE_CAPTION + 1.0),
                motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, on.max(hover)),
            );
            if p == Page::Notifications && badge > 0 {
                let c = rect.right_top() + egui::vec2(-16.0, 14.0);
                ui.painter().circle_filled(c, 9.0, accent);
                ui.painter().text(c, egui::Align2::CENTER_CENTER, badge.min(99).to_string(),
                    egui::FontId::monospace(theme::SIZE_CAPTION), theme::BG_ROOT);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *page = p;
            }
        }
        // Keyboard: a toggle, lit while the keyboard is open.
        let (rect, resp) = ui.allocate_exact_size(TILE, egui::Sense::click());
        let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
        let on = motion::toggle(ui.ctx(), resp.id.with("on"), *keyboard, motion::MEDIUM);
        let base = crate::theme::veil((hover * 16.0) as u8);
        ui.painter().rect_filled(rect, 22.0, motion::mix(base, accent.gamma_multiply(0.2), on));
        let tint = motion::mix(theme::FG_MUTED, accent, on);
        icon(icons::KEYBOARD, 28.0, tint).paint_at(ui, egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 10.0), egui::vec2(28.0, 28.0)));
        ui.painter().text(rect.center_bottom() - egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "Keyboard",
            egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), motion::mix(theme::FG_MUTED, theme::FG_PRIMARY, on.max(hover)));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *keyboard = !*keyboard;
        }
    });
}

/// What the header asked for this frame.
#[derive(Default)]
pub struct HeaderActions {
    pub toggle_keyboard: bool,
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
            .desired_width(160.0),
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
                .stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.35)))
                .rounding(12.0)
                .inner_margin(egui::Margin::same(6.0))
                .show(ui, |ui| {
                    ui.set_width(380.0);
                    for (i, e) in results.iter().enumerate() {
                        let sel = i == st.selected;
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(380.0, 40.0), egui::Sense::click());
                        if sel || resp.hovered() {
                            ui.painter().rect_filled(r, 8.0, accent.gamma_multiply(if sel { 0.22 } else { 0.12 }));
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
    keyboard_open: bool,
    unread: usize,
    search: &mut SearchState,
    entries: &[crate::search::Entry],
    accent: egui::Color32,
) -> HeaderActions {
    let mut out = HeaderActions::default();
    let now = Local::now();
    let time = if locale::uses_24h_clock() {
        now.format("%H:%M").to_string()
    } else {
        now.format("%-I:%M %p").to_string()
    };
    let date = now.format("%A · %b %-d").to_string();
    ui.horizontal(|ui| {
        ui.set_height(56.0);
        ui.add(icon(icons::BRAND, 40.0, accent));
        ui.add_space(10.0);
        ui.label(egui::RichText::new(time).size(34.0).strong().color(theme::FG_PRIMARY));
        ui.add_space(6.0);
        ui.label(theme::body_muted(&date));
        ui.add_space(16.0);
        out.search_pick = search_box(ui, search, entries, accent);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, icons::BELL, unread > 0, accent).on_hover_text("Notifications").clicked() {
                out.open_notifications = true;
            }
            if icon_button(ui, icons::KEYBOARD, keyboard_open, accent)
                .on_hover_text(if keyboard_open { "Hide keyboard" } else { "Show keyboard" })
                .clicked()
            {
                out.toggle_keyboard = true;
            }
            ui.add_space(8.0);
            // Battery chips, right-to-left so the HMD ends up leftmost.
            // At most three chips so the header never collides with
            // search (the full list lives in the Battery card).
            for b in batteries.iter().take(3).rev() {
                battery_chip(ui, b, accent);
            }
        });
    });
    out
}

fn icon_button(ui: &mut egui::Ui, src: egui::ImageSource<'static>, active: bool, accent: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    let on = motion::toggle(ui.ctx(), resp.id.with("on"), active, motion::MEDIUM);
    let idle = motion::mix(theme::BG_CARD, egui::Color32::from_rgb(0x1C, 0x20, 0x26), hover);
    ui.painter().rect_filled(rect, theme::RADIUS_CARD, motion::mix(idle, accent.gamma_multiply(0.18), on));
    let tint = motion::mix(theme::FG_MUTED, accent, on);
    icon(src, 20.0, tint).paint_at(ui, egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn battery_chip(ui: &mut egui::Ui, b: &BatteryReading, accent: egui::Color32) {
    let short = short_device_label(&b.label);
    let pct = (b.fraction * 100.0).round() as u32;
    let color = if !b.charging && pct < 20 { theme::DANGER } else { accent };
    let text = if b.charging { format!("{short} ⚡{pct}") } else { format!("{short} {pct}") };
    // Painted rather than a Frame: a Frame in a right-to-left row
    // stretches to the full header height.
    let font = egui::FontId::monospace(theme::SIZE_CAPTION + 1.0);
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 16.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, theme::RADIUS_CHIP, theme::BG_CARD);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
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
    // Glow: a few widening, fading strokes.
    for (w, a) in [(10.0_f32, 0.10_f32), (6.0, 0.22), (3.0, 0.9)] {
        p.rect_stroke(r, 26.0, egui::Stroke::new(w, accent.gamma_multiply(a)));
    }
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
pub fn page_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(22.0).strong().color(theme::FG_PRIMARY));
    ui.add_space(theme::GUTTER);
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
