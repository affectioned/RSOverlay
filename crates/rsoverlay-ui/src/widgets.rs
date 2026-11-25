//! Modular VR widgets: any dashboard card can pop out into its own
//! floating panel; panels merge into tabbed groups and split again.
//!
//! The pop-out hook lives in each card's header (`popout_button`) and
//! only renders where it's enabled (the VR dashboard), so the desktop
//! preview is unchanged. Requests travel through egui context data, so
//! cards don't need a reference to DevView.

use crate::icons::{self, icon};
use crate::window_panel::{pill, Attach};
use crate::{motion, theme};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    #[default]
    NowPlaying,
    System,
    Volume,
    Battery,
    Macros,
    Chat,
    Notifications,
}

impl WidgetKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::NowPlaying => "Now playing",
            Self::System => "System",
            Self::Volume => "Volume",
            Self::Battery => "Battery",
            Self::Macros => "Macros",
            Self::Chat => "Chat",
            Self::Notifications => "Notifications",
        }
    }

    pub fn icon(self) -> egui::ImageSource<'static> {
        match self {
            Self::NowPlaying => icons::PLAY,
            Self::System => icons::PULSE,
            Self::Volume => icons::SPEAKER,
            Self::Battery => icons::BATTERY,
            Self::Macros => icons::GRID,
            Self::Chat => icons::CHAT,
            Self::Notifications => icons::BELL,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::NowPlaying => "now-playing",
            Self::System => "system",
            Self::Volume => "volume",
            Self::Battery => "battery",
            Self::Macros => "macros",
            Self::Chat => "chat",
            Self::Notifications => "notifications",
        }
    }

    /// `system`, `now-playing`, … (CLI / config).
    pub fn from_slug(s: &str) -> Option<Self> {
        Self::from_caption(match s {
            "now-playing" | "media" => "Now playing",
            "system" => "System",
            "volume" => "Volume",
            "battery" => "Battery",
            "macros" => "Macros",
            "chat" => "VRChat chatbox",
            "notifications" => "Notifications",
            _ => return None,
        })
    }

    /// Card caption → widget (captions are what card headers know).
    pub fn from_caption(caption: &str) -> Option<Self> {
        Some(match caption {
            "Now playing" => Self::NowPlaying,
            "System" => Self::System,
            "Volume" => Self::Volume,
            "Battery" => Self::Battery,
            "Macros" => Self::Macros,
            "VRChat chatbox" => Self::Chat,
            "Notifications" => Self::Notifications,
            _ => return None,
        })
    }
}

const POPOUT_ON: &str = "rso-popout-enabled";
const POPOUT_REQ: &str = "rso-popout-request";

pub fn set_popout_enabled(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(POPOUT_ON), on));
}

pub fn take_popout_request(ctx: &egui::Context) -> Option<WidgetKind> {
    ctx.data_mut(|d| d.remove_temp::<WidgetKind>(egui::Id::new(POPOUT_REQ)))
}

const HIGHLIGHT: &str = "rso-highlight";
const HIGHLIGHT_SECS: f64 = 2.4;

/// Ask the card with `caption` to scroll into view and flash (search).
pub fn highlight_card(ctx: &egui::Context, caption: &str) {
    let t = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(HIGHLIGHT), (caption.to_string(), t, false)));
}

/// Card-header hook: scroll to + pulse the card a search result picked.
fn highlight_hook(ui: &mut egui::Ui, caption: &str, accent: egui::Color32) {
    let id = egui::Id::new(HIGHLIGHT);
    let Some((cap, start, scrolled)) = ui.ctx().data(|d| d.get_temp::<(String, f64, bool)>(id)) else { return };
    if cap != caption {
        return;
    }
    let age = ui.input(|i| i.time) - start;
    if age > HIGHLIGHT_SECS {
        ui.ctx().data_mut(|d| d.remove::<(String, f64, bool)>(id));
        return;
    }
    if !scrolled {
        ui.scroll_to_cursor(Some(egui::Align::Center));
        ui.ctx().data_mut(|d| d.insert_temp(id, (cap, start, true)));
    }
    // Three soft pulses, fading out.
    let pulse = (0.5 + 0.5 * (age as f32 * std::f32::consts::TAU * 1.25).cos()) * (1.0 - (age / HIGHLIGHT_SECS) as f32);
    let r = ui.min_rect().expand2(egui::vec2(10.0, 6.0));
    ui.painter().rect_stroke(r, 10.0, egui::Stroke::new(2.5_f32, accent.gamma_multiply(pulse.max(0.15))));
    ui.ctx().request_repaint();
}

/// Ask for a widget pop-out from code (search results).
pub fn request_popout(ctx: &egui::Context, kind: WidgetKind) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(POPOUT_REQ), kind));
}

/// Small ↗ after a card caption: "pop this card out as a VR widget".
/// Also hosts the search highlight hook, since every card calls it.
pub fn popout_button(ui: &mut egui::Ui, caption: &str, accent: egui::Color32) {
    highlight_hook(ui, caption, accent);
    let on = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(POPOUT_ON))).unwrap_or(false);
    let Some(kind) = WidgetKind::from_caption(caption).filter(|_| on) else { return };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 22.0), egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    let p = ui.painter();
    p.rect_filled(rect, 6.0, motion::mix(crate::theme::BG_RAISED, accent.gamma_multiply(0.25), hover));
    // ↗ drawn as vectors: box corner + diagonal arrow.
    let c = rect.center();
    let s = egui::Stroke::new(1.6_f32, motion::mix(theme::FG_MUTED, accent, hover));
    p.line_segment([c + egui::vec2(-4.0, 4.0), c + egui::vec2(4.0, -4.0)], s);
    p.line_segment([c + egui::vec2(0.0, -4.0), c + egui::vec2(4.0, -4.0)], s);
    p.line_segment([c + egui::vec2(4.0, -4.0), c + egui::vec2(4.0, 0.0)], s);
    if resp.on_hover_text("Pop out as a floating widget").clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(POPOUT_REQ), kind));
    }
}

#[derive(Debug, Default)]
pub struct WidgetAction {
    pub select: Option<usize>,
    /// Split the active tab out into its own panel.
    pub split: bool,
    pub attach: Option<Attach>,
    pub close: bool,
    /// Leave the edge-docked group.
    pub unlink: bool,
}

pub const TAB_H: f32 = 40.0;
pub const BAR_H: f32 = 46.0;

/// Frame around a widget body: tab strip on top, dock bar at the bottom.
/// `body` draws the active card into the middle.
pub fn frame(
    ctx: &egui::Context,
    tabs: &[WidgetKind],
    active: usize,
    attach: Attach,
    linked: bool,
    accent: egui::Color32,
    body: impl FnOnce(&mut egui::Ui, WidgetKind),
) -> WidgetAction {
    let mut out = WidgetAction::default();
    egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
        let full = ui.max_rect().shrink(4.0);
        theme::surface(ui.painter(), full.shrink(8.0), 22.0);
        let inner = full.shrink2(egui::vec2(14.0, 10.0));
        let tab_r = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), TAB_H));
        // How to merge, for whoever wonders where the hint went.
        ui.interact(tab_r, ui.id().with("tabs-hint"), egui::Sense::hover())
            .on_hover_text("Grab this panel and drop it onto another widget to merge them into tabs");
        let bar_r = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - BAR_H), inner.max);
        let body_r = egui::Rect::from_min_max(egui::pos2(inner.left(), tab_r.bottom() + 6.0), egui::pos2(inner.right(), bar_r.top() - 6.0));

        // Tabs.
        let mut tui = ui.child_ui(tab_r, egui::Layout::left_to_right(egui::Align::Center));
        for (i, k) in tabs.iter().enumerate() {
            let sel = i == active;
            let (r, resp) = tui.allocate_exact_size(egui::vec2(tab_width(&tui, k.label()), TAB_H - 6.0), egui::Sense::click());
            let hover = motion::toggle(tui.ctx(), resp.id, resp.hovered(), motion::FAST);
            let fill = if sel { accent.gamma_multiply(0.22) } else { motion::mix(crate::theme::BG_RAISED, crate::theme::BG_HOVER, hover) };
            tui.painter().rect_filled(r, 12.0, fill);
            let tint = if sel { accent } else { theme::FG_MUTED };
            icon(k.icon(), 16.0, tint).paint_at(&tui, egui::Rect::from_center_size(egui::pos2(r.left() + 18.0, r.center().y), egui::vec2(16.0, 16.0)));
            tui.painter().text(egui::pos2(r.left() + 32.0, r.center().y), egui::Align2::LEFT_CENTER, k.label(),
                egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), if sel { theme::FG_PRIMARY } else { theme::FG_MUTED });
            if resp.clicked() && !sel {
                out.select = Some(i);
            }
        }
        if tabs.len() > 1 {
            tui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill(ui, "Split", false, accent).on_hover_text("Pull this tab out into its own panel").clicked() {
                    out.split = true;
                }
            });
        }

        // Body: the live card.
        let mut bui = ui.child_ui(body_r, egui::Layout::top_down(egui::Align::Min));
        egui::ScrollArea::vertical().id_source("widget-body").auto_shrink([false, false]).show(&mut bui, |ui| {
            if let Some(k) = tabs.get(active) {
                body(ui, *k);
            }
        });

        // Dock bar.
        let mut dui = ui.child_ui(bar_r, egui::Layout::right_to_left(egui::Align::Center));
        (out.attach, out.close) = crate::window_panel::dock_controls(&mut dui, attach, accent);
        if linked {
            dui.add_space(8.0);
            out.unlink = crate::window_panel::unlink_pill(&mut dui, accent);
        }
    });
    out
}

fn tab_width(ui: &egui::Ui, label: &str) -> f32 {
    let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), theme::FG_PRIMARY);
    g.size().x + 46.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captions_map_both_ways() {
        for k in [
            WidgetKind::NowPlaying,
            WidgetKind::System,
            WidgetKind::Volume,
            WidgetKind::Battery,
            WidgetKind::Macros,
            WidgetKind::Notifications,
        ] {
            assert_eq!(WidgetKind::from_caption(k.label()), Some(k));
        }
        assert_eq!(WidgetKind::from_caption("VRChat chatbox"), Some(WidgetKind::Chat));
        assert_eq!(WidgetKind::from_caption("Appearance"), None);
    }

    #[test]
    fn popout_request_roundtrip() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        assert_eq!(take_popout_request(&ctx), None);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(POPOUT_REQ), WidgetKind::Volume));
        assert_eq!(take_popout_request(&ctx), Some(WidgetKind::Volume));
        assert_eq!(take_popout_request(&ctx), None);
    }
}
