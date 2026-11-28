//! Bundled SVG icons: Tabler Icons 3.24.0 (MIT, `icons/LICENSE-tabler-icons.txt`,
//! https://tabler.io/icons), drawn white so the UI tints them at runtime.
//! The RSOverlay mark (`brand.svg`) is our own.
//!
//! Icons are embedded at compile time and loaded through `egui_extras`'
//! SVG loader so they stay crisp at any overlay texture size.

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
svg_icon!(SPEAKER,  "speaker.svg");
svg_icon!(HOME,     "home.svg");
svg_icon!(GRID,     "grid.svg");
svg_icon!(KEYBOARD, "keyboard.svg");
svg_icon!(MOON,     "moon.svg");
svg_icon!(PLUS,        "plus.svg");
svg_icon!(MORE,        "more.svg");
svg_icon!(RECENTER,    "recenter.svg");
svg_icon!(FOLDER,      "folder.svg");
svg_icon!(TRASH,       "trash.svg");
svg_icon!(LAYERS,      "layers.svg");
svg_icon!(MUSIC,       "music.svg");
svg_icon!(CHART,       "chart.svg");
svg_icon!(HEADSET,     "headset.svg");
svg_icon!(CONTROLLER,  "controller.svg");
svg_icon!(STAR,        "star.svg");
svg_icon!(STAR_OUTLINE, "star_outline.svg");
svg_icon!(SAVE,        "save.svg");
svg_icon!(LOAD,        "load.svg");
svg_icon!(EDIT,        "edit.svg");
svg_icon!(LAYOUTS,     "layouts.svg");

/// Install the SVG loader on the egui context. Call once at startup.
pub fn install_loaders(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// Raster density for the SVGs, in pixels per point: the highest UI
/// scale we render at (VR, 2.25 ppp) × 1.5 supersampling, so a downscale
/// never blurs. egui 0.26 rasterises `fit_to_exact_size` images at
/// *point* size, ignoring pixels-per-point (upstream TODO) — an 18 pt
/// icon came out 18 px and was stretched 2.25× on VR panels, visibly
/// soft on high-PPD headsets.
const RASTER_PX_PER_PT: f32 = 2.25 * 1.5;
/// Tabler's viewBox (the brand mark's 48 units just raster sharper).
const VIEWBOX: f32 = 24.0;

/// Build a right-sized image widget for one of the bundled icons, tinted
/// by the given color. Icons ship as white-fill SVGs so any tint lands cleanly.
///
/// `fit_to_original_size(scale)` sets the raster size (VIEWBOX × scale
/// px); `max_size` then shrinks only the *displayed* size to `size` pt.
pub fn icon(source: ImageSource<'static>, size: f32, tint: egui::Color32) -> egui::Image<'static> {
    let scale = (size * RASTER_PX_PER_PT / VIEWBOX).max(1.0);
    egui::Image::new(source)
        .fit_to_original_size(scale)
        .max_size(egui::vec2(size, size))
        .tint(tint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_rasterise_at_pixel_density_but_display_at_point_size() {
        let img = icon(SETTINGS, 18.0, egui::Color32::WHITE);
        // The raster comes back at 24 × scale ≈ 61 px; displayed at 18 pt.
        let raster = 18.0 * RASTER_PX_PER_PT;
        let shown = img.calc_size(egui::vec2(1000.0, 1000.0), Some(egui::vec2(raster, raster)));
        assert!((shown.x - 18.0).abs() < 0.01 && (shown.y - 18.0).abs() < 0.01, "{shown:?}");
    }
}
