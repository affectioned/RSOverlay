//! Visual tokens. One source of truth so every card looks like it came from
//! the same product.
//!
//! Direction: Anthropic's design language, on AMOLED black. Warm
//! neutrals instead of cool blue-greys, a serif display face over a
//! quiet sans, flat surfaces with hairline borders (no glows, sheens or
//! shadows), sentence-case labels, and one restrained accent (the
//! owner's rose by default). It's a work tool first: calm, legible, nothing shouting.

use egui::Color32;

/// AMOLED-true black. Every unlit OLED pixel stays off at this value, which
/// is the look the owner wants everywhere. Surfaces step up from it in
/// warm greys.
pub const BG_ROOT: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
/// Cards, sidebar, panels.
pub const BG_CARD: Color32 = Color32::from_rgb(0x14, 0x14, 0x13);
/// Controls on a card (buttons, inputs, slider rails).
pub const BG_RAISED: Color32 = Color32::from_rgb(0x1F, 0x1E, 0x1D);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x2B, 0x2A, 0x27);
pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x36, 0x35, 0x31);
/// Hairline border on every surface.
pub const STROKE_CARD: Color32 = Color32::from_rgb(0x2A, 0x29, 0x26);
pub const FG_PRIMARY: Color32 = Color32::from_rgb(0xF5, 0xF4, 0xEF);
pub const FG_MUTED: Color32 = Color32::from_rgb(0xA3, 0xA0, 0x97);
/// Quieter than muted: disabled glyphs, placeholder text.
pub const FG_FAINT: Color32 = Color32::from_rgb(0x5E, 0x5C, 0x56);
/// Default accent (rose), overridden at runtime by the Appearance card.
pub const ACCENT: Color32 = Color32::from_rgb(0xE8, 0x8F, 0xA2);
/// Warning state (low battery). A true red, apart from every accent.
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x53, 0x4B);

/// Accent choices offered in the Appearance picker: muted, warm tones.
pub const ACCENT_PRESETS: &[(&str, Color32)] = &[
    ("rose",    Color32::from_rgb(0xE8, 0x8F, 0xA2)),
    ("clay",    Color32::from_rgb(0xD9, 0x77, 0x57)),
    ("kraft",   Color32::from_rgb(0xD4, 0xA2, 0x7F)),
    ("olive",   Color32::from_rgb(0x9A, 0xAE, 0x7A)),
    ("sky",     Color32::from_rgb(0x6A, 0x9B, 0xCC)),
    ("heather", Color32::from_rgb(0xA8, 0x95, 0xD0)),
    ("ivory",   Color32::from_rgb(0xF5, 0xF4, 0xEF)),
];

pub const RADIUS_CARD: f32 = 14.0;
pub const RADIUS_CONTROL: f32 = 10.0;
pub const RADIUS_PILL: f32 = 14.0;
pub const RADIUS_CHIP: f32 = 6.0;

pub const PAD_CARD_H: f32 = 20.0;
pub const PAD_CARD_V: f32 = 18.0;

pub const GUTTER: f32 = 14.0;

pub const SIZE_HEADLINE: f32 = 40.0;
pub const SIZE_CARD_TITLE: f32 = 16.0;
pub const SIZE_BODY: f32 = 15.0;
pub const SIZE_CAPTION: f32 = 12.0;

/// Display serif (Source Serif 4): page titles, greetings, the clock.
pub fn serif(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("serif".into()))
}

/// Medium-weight sans (Inter): card titles, selected nav items.
pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("medium".into()))
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter", &include_bytes!("fonts/Inter-Regular.ttf")[..]),
        ("inter-medium", &include_bytes!("fonts/Inter-Medium.ttf")[..]),
        ("source-serif", &include_bytes!("fonts/SourceSerif4-Regular.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), egui::FontData::from_static(bytes));
    }
    // egui's bundled fonts stay on as fallbacks (symbols, emoji).
    let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
    fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "inter".into());
    for (family, first) in [("medium", "inter-medium"), ("serif", "source-serif")] {
        let mut chain = vec![first.to_string()];
        chain.extend(fallback.iter().cloned());
        fonts.families.insert(egui::FontFamily::Name(family.into()), chain);
    }
    ctx.set_fonts(fonts);
}

/// Install the base style on the egui context once at startup.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    style.visuals.override_text_color = Some(FG_PRIMARY);
    style.visuals.panel_fill = BG_ROOT;
    style.visuals.window_fill = BG_RAISED;
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, STROKE_CARD);
    style.visuals.window_rounding = egui::Rounding::same(RADIUS_CONTROL);
    // Shadows band in the SteamVR compositor; hairlines carry depth.
    style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    // Text fields + slider rails: one step above the card so inputs read
    // as inputs.
    style.visuals.extreme_bg_color = BG_RAISED;
    style.visuals.widgets.noninteractive.bg_fill = BG_CARD;
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, STROKE_CARD);
    style.spacing.item_spacing = egui::vec2(GUTTER, 8.0);
    // Scrollbar: wide enough and bright enough to actually read against
    // AMOLED-true-black. Default egui scrollbars disappear on #000.
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 4.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.scroll.handle_min_length = 32.0;
    // Controls: warm raised fills with a hairline edge, like Claude's
    // secondary buttons.
    for (w, fill) in [
        (&mut style.visuals.widgets.inactive, BG_RAISED),
        (&mut style.visuals.widgets.hovered, BG_HOVER),
        (&mut style.visuals.widgets.active, BG_ACTIVE),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::new(1.0_f32, STROKE_CARD);
        w.rounding = egui::Rounding::same(RADIUS_CONTROL);
        w.expansion = 0.0;
    }
    style.visuals.widgets.inactive.fg_stroke.color = FG_PRIMARY;
    // Tooltips name every icon. A laser never holds perfectly still, so
    // "only when still" would mean "never" in VR.
    style.interaction.show_tooltips_only_when_still = false;
    style.interaction.tooltip_delay = 0.25;
    // VR hit targets: lasers jitter, so every control gets real height.
    style.spacing.interact_size.y = 30.0;
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style(style);
}

/// A card frame: flat warm surface, hairline edge.
pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(BG_CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_CARD))
        .rounding(RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(PAD_CARD_H, PAD_CARD_V))
}

/// A faint white veil, as a premultiplied colour built directly.
/// `Color32::from_white_alpha` builds in *linear* light and gamma-encodes
/// it, so a "3 %" veil lands near 20 % grey, far too bright on AMOLED
/// black. `veil(8)` really is ~3 %.
pub fn veil(level: u8) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(level, level, level, level)
}

/// A free-standing panel surface (VR dashboard, wrist rows, widgets,
/// keyboard): true black body, one warm hairline. Nothing else.
pub fn surface(p: &egui::Painter, r: egui::Rect, radius: f32) {
    p.rect_filled(r, radius, BG_ROOT);
    p.rect_stroke(r, radius, egui::Stroke::new(1.0_f32, STROKE_CARD));
}

/// A card frame with the pill radius — used for toasts.
pub fn pill() -> egui::Frame {
    egui::Frame::none()
        .fill(BG_CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE_CARD))
        .rounding(RADIUS_PILL)
        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
}

/// Section label: small, muted, sentence case (no shouting caps).
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_CAPTION + 1.0).color(FG_MUTED)
}

/// Card title: medium-weight sans.
pub fn card_title(text: &str) -> egui::RichText {
    egui::RichText::new(text).font(medium(SIZE_CARD_TITLE)).color(FG_PRIMARY)
}

pub fn body(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_BODY).color(FG_PRIMARY)
}

pub fn body_muted(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(SIZE_BODY).color(FG_MUTED)
}

pub fn headline(text: &str) -> egui::RichText {
    egui::RichText::new(text).font(serif(SIZE_HEADLINE)).color(FG_PRIMARY)
}

pub fn accent_dot_for(c: Color32) -> egui::RichText {
    egui::RichText::new("●").color(c)
}

/// Size the next slider in a `label · slider · value` row to the room
/// left after the label, so rows never push a card past its column.
pub fn fit_slider(ui: &mut egui::Ui) {
    let room = ui.available_width() - 72.0 - ui.spacing().item_spacing.x;
    ui.spacing_mut().slider_width = room.clamp(40.0, 220.0);
}
