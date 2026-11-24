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
// Premultiplied: RGB must not exceed alpha. The old (0xFF,0xFF,0xFF,0x0F)
// literal broke that and rendered as a near-solid white border.
pub const STROKE_CARD: Color32 = Color32::from_rgba_premultiplied(0x10, 0x10, 0x10, 0x10); // ~6%
pub const FG_PRIMARY: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
pub const FG_MUTED: Color32 = Color32::from_rgb(0x8A, 0x8F, 0x99);
/// Default accent — overridden at runtime by the user via the Appearance card.
pub const ACCENT: Color32 = Color32::from_rgb(0xE8, 0x8F, 0xA2); // rose
/// Warning state (low battery). Muted like the accents — reads as
/// "attention" without neon bleed in the compositor.
pub const DANGER: Color32 = Color32::from_rgb(0xE8, 0x70, 0x5F);

/// A handful of brand-neutral accent choices offered in the Appearance picker.
/// These are intentionally muted — no neon, so nothing reads as "gamer-tech"
/// in SteamVR's compositor.
pub const ACCENT_PRESETS: &[(&str, Color32)] = &[
    ("rose",   Color32::from_rgb(0xE8, 0x8F, 0xA2)),
    ("teal",   Color32::from_rgb(0x7D, 0xD3, 0xC0)),
    ("amber",  Color32::from_rgb(0xE6, 0xB3, 0x5C)),
    ("mauve",  Color32::from_rgb(0xC6, 0x9C, 0xE6)),
    ("sky",    Color32::from_rgb(0x82, 0xB1, 0xFF)),
    ("lime",   Color32::from_rgb(0xB2, 0xD9, 0x7C)),
    ("white",  Color32::from_rgb(0xF2, 0xF3, 0xF5)),
];

pub const RADIUS_CARD: f32 = 20.0;
pub const RADIUS_PILL: f32 = 14.0;
pub const RADIUS_CHIP: f32 = 6.0;

pub const PAD_CARD_H: f32 = 18.0;
pub const PAD_CARD_V: f32 = 16.0;

pub const GUTTER: f32 = 12.0;

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
    // Text fields + slider rails: one step above the card so inputs read
    // as inputs (equal to BG_CARD they vanished into the card).
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(0x16, 0x19, 0x1E);
    style.visuals.widgets.noninteractive.bg_fill = BG_CARD;
    style.spacing.item_spacing = egui::vec2(GUTTER, 8.0);
    // Scrollbar — wide enough and bright enough to actually read against
    // AMOLED-true-black. Default egui scrollbars disappear on #000.
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 4.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.scroll.handle_min_length = 32.0;
    // Controls sit two steps above the card fill — visible on OLED
    // without the old flat grey slabs. No borders; fill carries state.
    for (w, fill) in [
        (&mut style.visuals.widgets.inactive, egui::Color32::from_rgb(0x1C, 0x20, 0x26)),
        (&mut style.visuals.widgets.hovered, egui::Color32::from_rgb(0x27, 0x2C, 0x33)),
        (&mut style.visuals.widgets.active, egui::Color32::from_rgb(0x33, 0x39, 0x41)),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::NONE;
        w.rounding = egui::Rounding::same(14.0);
    }
    style.visuals.widgets.inactive.fg_stroke.color = FG_PRIMARY;
    // VR hit targets: lasers jitter, so every control gets real height.
    style.spacing.interact_size.y = 30.0;
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style(style);

    // Tabular numerals look right on data; we fake it by using the monospace
    // family for numerics only (done at the call site via `ui.monospace`).
}

/// A card frame: frosted glass tile — a faint white veil over whatever
/// is behind (black on AMOLED, the glass slab in VR), hairline edge.
pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(veil(0x08))
        .stroke(egui::Stroke::new(1.0_f32, veil(0x16)))
        .rounding(RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(PAD_CARD_H, PAD_CARD_V))
}

/// A faint white veil, as a premultiplied colour built directly.
/// `Color32::from_white_alpha` builds in *linear* light and gamma-encodes
/// it, so a "3 %" veil lands near 20 % grey — far too bright on AMOLED
/// black. `veil(8)` really is ~3 %.
pub fn veil(level: u8) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(level, level, level, level)
}

/// Liquid-glass material, painted directly: a dark translucent body
/// (keeps true-black AMOLED depth), a soft vertical sheen, a specular
/// highlight along the top edge, a hairline rim and a faint drop
/// shadow. Overlays can't blur what's behind them (SteamVR never gives
/// us those pixels), so depth comes from light, not blur.
pub fn glass(p: &egui::Painter, r: egui::Rect, radius: f32) {
    // Shadow: a few widening, fading rings below the slab.
    for (grow, a) in [(10.0_f32, 0x0C_u8), (5.0, 0x18), (2.0, 0x26)] {
        p.rect_filled(r.translate(egui::vec2(0.0, 4.0)).expand(grow), radius + grow, egui::Color32::from_black_alpha(a));
    }
    // Near-opaque black body: true black on OLED, the world only just
    // breathing through in VR.
    p.rect_filled(r, radius, egui::Color32::from_black_alpha(236));
    // Sheen: brighter at the top, fading down (stacked bands).
    let bands = 8;
    for i in 0..bands {
        let t0 = i as f32 / bands as f32;
        let band = egui::Rect::from_min_max(
            egui::pos2(r.left(), r.top() + r.height() * t0 * 0.6),
            egui::pos2(r.right(), r.top() + r.height() * (t0 + 1.0 / bands as f32) * 0.6),
        );
        let a = ((1.0 - t0) * 5.0) as u8;
        let rr = if i == 0 { egui::Rounding { nw: radius, ne: radius, sw: 0.0, se: 0.0 } } else { egui::Rounding::ZERO };
        p.rect_filled(band, rr, veil(a));
    }
    // Specular rim light along the top edge.
    let inset = radius * 0.9;
    p.line_segment(
        [egui::pos2(r.left() + inset, r.top() + 1.0), egui::pos2(r.right() - inset, r.top() + 1.0)],
        egui::Stroke::new(1.2_f32, veil(0x50)),
    );
    p.rect_stroke(r, radius, egui::Stroke::new(1.0_f32, veil(0x1E)));
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

pub fn accent_dot_for(c: Color32) -> egui::RichText {
    egui::RichText::new("●").color(c)
}
