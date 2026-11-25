//! SteamVR device icons, read from the user's own SteamVR install at
//! runtime (never shipped with RSOverlay) and turned into alpha masks so
//! the UI can tint them to the theme.
//!
//! Each file is decoded once and the `Arc` reused, which also keeps the
//! UI's texture cache (keyed by the `Arc`) stable across battery polls.

use rsoverlay_core::steamvr::SteamVrDirs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Icon files are a few KB; anything far bigger isn't an icon.
const MAX_ICON_BYTES: u64 = 512 * 1024;
/// …and nothing needs more than this per side.
const MAX_ICON_SIDE: u32 = 512;

#[derive(Default)]
pub struct DeviceIcons {
    dirs: Option<SteamVrDirs>,
    located: bool,
    cache: HashMap<PathBuf, Option<Arc<egui::ColorImage>>>,
}

impl DeviceIcons {
    fn dirs(&mut self) -> Option<&SteamVrDirs> {
        if !self.located {
            self.located = true;
            self.dirs = SteamVrDirs::locate();
        }
        self.dirs.as_ref()
    }

    /// Icon for a live device from its OpenVR icon path + resource root.
    pub fn for_device(&mut self, icon: Option<&str>, resource_root: Option<&str>) -> Option<Arc<egui::ColorImage>> {
        let path = self.dirs()?.resolve_icon(icon?, resource_root.unwrap_or_default())?;
        self.load(&path)
    }

    /// Representative icon for a demo device label (desktop preview).
    pub fn for_label(&mut self, label: &str) -> Option<Arc<egui::ColorImage>> {
        let path = self.dirs()?.demo_icon(label)?;
        self.load(&path)
    }

    fn load(&mut self, path: &Path) -> Option<Arc<egui::ColorImage>> {
        if let Some(c) = self.cache.get(path) {
            return c.clone();
        }
        let img = decode_mask(path).map(Arc::new);
        if img.is_none() {
            tracing::debug!("device icon unusable: {}", path.display());
        }
        self.cache.insert(path.to_path_buf(), img.clone());
        img
    }
}

/// PNG → white image whose alpha is the icon's shape (SteamVR icons are
/// solid silhouettes in SteamVR green; we keep only the silhouette).
/// Size-capped and decoded with the `png` crate's own bounds checks.
fn decode_mask(path: &Path) -> Option<egui::ColorImage> {
    if std::fs::metadata(path).ok()?.len() > MAX_ICON_BYTES {
        return None;
    }
    let mut dec = png::Decoder::new(std::fs::File::open(path).ok()?);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_ICON_SIDE || h > MAX_ICON_SIDE {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let data = buf.get(..frame.buffer_size())?;
    let alpha: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().map(|p| p[1]).collect(),
        // No alpha channel: use brightness as the shape.
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().map(|p| p.iter().copied().max().unwrap_or(0)).collect(),
        png::ColorType::Grayscale => data.to_vec(),
        png::ColorType::Indexed => return None, // EXPAND turns these into RGB(A)
    };
    if alpha.len() != (w * h) as usize {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        &alpha.iter().flat_map(|a| [255, 255, 255, *a]).collect::<Vec<u8>>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
        let f = std::fs::File::create(path).unwrap();
        let mut e = png::Encoder::new(f, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(rgba).unwrap();
    }

    #[test]
    fn green_icon_becomes_a_white_alpha_mask() {
        let dir = std::env::temp_dir().join(format!("rso-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("i.png");
        // 2×1: opaque SteamVR green, fully transparent.
        write_png(&p, 2, 1, &[0x8b, 0xb3, 0x1c, 255, 0, 0, 0, 0]);
        let m = decode_mask(&p).unwrap();
        assert_eq!(m.size, [2, 1]);
        assert_eq!(m.pixels[0], egui::Color32::WHITE);
        assert_eq!(m.pixels[1].a(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_png_and_missing_files_are_ignored() {
        let dir = std::env::temp_dir().join(format!("rso-icons-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.png"), b"not a png").unwrap();
        assert!(decode_mask(&dir.join("x.png")).is_none());
        assert!(decode_mask(&dir.join("missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read-only: decodes the real icons from this machine's SteamVR.
    /// `cargo test -p rsoverlay-app device_icons -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_steamvr_icons() {
        let mut d = DeviceIcons::default();
        for l in ["HMD", "Left hand", "Right hand", "Tracker · VIVE Tracker 3.0"] {
            let i = d.for_label(l);
            println!("{l:<28} {:?}", i.as_ref().map(|i| i.size));
            assert!(i.is_some(), "{l}");
        }
    }
}
