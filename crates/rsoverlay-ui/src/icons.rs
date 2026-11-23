//! Bundled SVG icons — AMOLED-ready, teal-accented, hand-authored.
//!
//! Icons are embedded at compile time and loaded through `egui_extras`'
//! SVG loader so they stay crisp at any overlay texture size. The fill is
//! baked into the SVGs at the theme accent — if we ever add runtime accent
//! swapping, we switch to a token-substitution pass here.

use egui::ImageSource;

macro_rules! svg_icon {
    ($name:ident, $file:expr) => {
        pub const $name: ImageSource<'static> = ImageSource::Bytes {
            uri: ::std::borrow::Cow::Borrowed(concat!("bytes://rsoverlay/", $file)),
            bytes: ::egui::load::Bytes::Static(include_bytes!(concat!("icons/", $file))),
        };
    };
}

svg_icon!(BRAND,    "brand.svg");
svg_icon!(PLAY,     "play.svg");
svg_icon!(PAUSE,    "pause.svg");
svg_icon!(BELL,     "bell.svg");
svg_icon!(MONITOR,  "monitor.svg");
svg_icon!(CHAT,     "chat.svg");
svg_icon!(POINTER,  "pointer.svg");
svg_icon!(PULSE,    "pulse.svg");
svg_icon!(CLOCK,    "clock.svg");
svg_icon!(SETTINGS, "settings.svg");
svg_icon!(BATTERY,  "battery.svg");

/// Install the SVG loader on the egui context. Call once at startup.
pub fn install_loaders(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// Build a right-sized image widget for one of the bundled icons, tinted
/// by the given color. Icons ship as white-fill SVGs so any tint lands cleanly.
pub fn icon(source: ImageSource<'static>, size: f32, tint: egui::Color32) -> egui::Image<'static> {
    egui::Image::new(source)
        .fit_to_exact_size(egui::vec2(size, size))
        .tint(tint)
}
