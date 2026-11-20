//! Visual tokens. One source of truth so every card looks like it came from
//! the same product.
//!
//! Direction: "Terminal Brutalist × Spatial Glass-Lite" — monochrome chassis,
//! solid-filled rounded cards, one muted-teal accent. See
//! `docs/ARCHITECTURE.md` and the design research notes for the why.

use egui::Color32;

/// AMOLED-true black. Every unlit OLED pixel stays off at this value, which
/// is the look the owner wants everywhere. Cards sit one step up so they
/// still read distinct against the void.
pub const BG_ROOT: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
pub const BG_CARD: Color32 = Color32::from_rgb(0x0F, 0x11, 0x15);
/// Inner stroke on every card — replaces the shadow we'd otherwise burn into
/// the OpenVR compositor at low alpha (which bands).
pub const STROKE_CARD: Color32 = Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 0x0F); // ~6%
pub const FG_PRIMARY: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
pub const FG_MUTED: Color32 = Color32::from_rgb(0x8A, 0x8F, 0x99);
pub const ACCENT: Color32 = Color32::from_rgb(0x7D, 0xD3, 0xC0);

pub const RADIUS_CARD: f32 = 10.0;
pub const RADIUS_PILL: f32 = 14.0;
pub const RADIUS_CHIP: f32 = 6.0;

pub const PAD_CARD_H: f32 = 18.0;
pub const PAD_CARD_V: f32 = 16.0;

pub const GUTTER: f32 = 12.0;
pub const COL_LEFT_WIDTH: f32 = 300.0;

pub const SIZE_HEADLINE: f32 = 42.0;
pub const SIZE_CARD_TITLE: f32 = 18.0;
pub const SIZE_BODY: f32 = 15.0;
pub const SIZE_CAPTION: f32 = 11.0;

/// Install the base style on the egui context once at startup.
pub fn install(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.override_text_color = Some(FG_PRIMARY);
    style.visuals.panel_fill = BG_ROOT;
    style.visuals.window_fill = BG_ROOT;
    style.visuals.extreme_bg_color = BG_CARD;
    style.visuals.widgets.noninteractive.bg_fill = BG_CARD;
    style.spacing.item_spacing = egui::vec2(GUTTER, 8.0);
    ctx.set_style(style);

    // Tabular numerals look right on data; we fake it by using the monospace
    // family for numerics only (done at the call site via `ui.monospace`).
}

/// A card frame with the standard fill + stroke + padding.
pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(BG_CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_CARD))
        .rounding(RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(PAD_CARD_H, PAD_CARD_V))
}

/// A card frame with the pill radius — used for toasts.
pub fn pill() -> egui::Frame {
    egui::Frame::none()
        .fill(BG_CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_CARD))
        .rounding(RADIUS_PILL)
        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
}

/// Section label: 11px, tracked +40, muted, uppercase caller.
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .size(SIZE_CAPTION)
        .color(FG_MUTED)
        .extra_letter_spacing(1.4)
}

pub fn card_title(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_CARD_TITLE).color(FG_PRIMARY).strong()
}

pub fn body(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_BODY).color(FG_PRIMARY)
}

pub fn body_muted(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_BODY).color(FG_MUTED)
}

pub fn headline(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_HEADLINE).color(FG_PRIMARY).strong()
}

pub fn accent_dot() -> egui::RichText {
    egui::RichText::new("●").color(ACCENT)
}
