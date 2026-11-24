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
    Settings,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Home,
        Page::Desktop,
        Page::Audio,
        Page::Chat,
        Page::Macros,
        Page::Notifications,
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
            Self::Settings => icons::SETTINGS,
        }
    }
}

/// Left navigation rail. `badge` puts a count pill on Notifications.
pub fn nav_rail(ui: &mut egui::Ui, page: &mut Page, badge: usize, accent: egui::Color32) {
    for p in Page::ALL {
        let count = if p == Page::Notifications { badge } else { 0 };
        if nav_item(ui, p, *page == p, count, accent) {
            *page = p;
        }
        ui.add_space(4.0);
    }
}

fn nav_item(ui: &mut egui::Ui, p: Page, selected: bool, badge: usize, accent: egui::Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ITEM_H), egui::Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS_CARD, accent.gamma_multiply(0.14));
        // Left accent bar — the "you are here" mark.
        let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 10.0), egui::vec2(3.0, rect.height() - 20.0));
        painter.rect_filled(bar, 1.5, accent);
    } else if resp.hovered() {
        painter.rect_filled(rect, theme::RADIUS_CARD, egui::Color32::from_white_alpha(0x0A));
    }
    let tint = if selected { accent } else { theme::FG_MUTED };
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), egui::vec2(20.0, 20.0));
    icon(p.icon(), 20.0, tint).paint_at(ui, icon_rect);
    painter.text(
        egui::pos2(rect.left() + 48.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        p.label(),
        egui::FontId::proportional(theme::SIZE_BODY),
        if selected { theme::FG_PRIMARY } else { theme::FG_MUTED },
    );
    if badge > 0 {
        let text = if badge > 99 { "99+".to_string() } else { badge.to_string() };
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(theme::SIZE_CAPTION), theme::BG_ROOT);
        let w = galley.size().x + 12.0;
        let pill = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0 - w / 2.0, rect.center().y), egui::vec2(w, 18.0));
        painter.rect_filled(pill, 9.0, accent);
        painter.galley(pill.center() - galley.size() / 2.0, galley, theme::BG_ROOT);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// What the header asked for this frame.
#[derive(Default)]
pub struct HeaderActions {
    pub toggle_keyboard: bool,
    pub open_notifications: bool,
}

pub fn header(
    ui: &mut egui::Ui,
    batteries: &[BatteryReading],
    keyboard_open: bool,
    unread: usize,
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
            for b in batteries.iter().rev() {
                battery_chip(ui, b, accent);
            }
        });
    });
    out
}

fn icon_button(ui: &mut egui::Ui, src: egui::ImageSource<'static>, active: bool, accent: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let fill = if active {
        accent.gamma_multiply(0.18)
    } else if resp.hovered() {
        egui::Color32::from_white_alpha(0x0E)
    } else {
        theme::BG_CARD
    };
    ui.painter().rect_filled(rect, theme::RADIUS_CARD, fill);
    let tint = if active { accent } else { theme::FG_MUTED };
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
