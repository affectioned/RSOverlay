//! Wrist background image: any PNG / JPG the user drops into the
//! `backgrounds` folder next to config.toml, shown behind the wrist's
//! clock and media, darkened so the text stays readable.
//!
//! Only plain file names inside that folder are accepted (no paths), and
//! files are size-capped and decoded off the UI thread, scaled down.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Largest file we'll try to decode.
const MAX_FILE: u64 = 25 << 20;
/// Longest edge after decoding (plenty for a ~0.3 m wide wrist panel).
const MAX_EDGE: u32 = 1024;

/// `%APPDATA%\RSOverlay\backgrounds`.
pub fn dir() -> PathBuf {
    rsoverlay_core::Config::default_path().with_file_name("backgrounds")
}

/// Show a folder in Explorer.
pub fn open_folder(dir: &Path) {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        // SAFETY: valid wide strings for the call's duration.
        unsafe { ShellExecuteW(HWND(0), w!("open"), &HSTRING::from(dir.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    }
    #[cfg(not(windows))]
    let _ = dir;
}

fn is_image(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg")
}

/// A bare file name, nothing that could leave the folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".." && is_image(name)
}

/// Image files in `dir`, sorted.
pub fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_name(n))
        .collect();
    v.sort_by_key(|n| n.to_lowercase());
    v
}

/// Background state on the DevView: the choice, the decoded image and
/// a loader thread's result slot.
pub struct WristBackground {
    /// File name in [`dir`]; empty = none.
    pub file: String,
    /// 0 = image as is, 1 = black.
    pub dim: f32,
    /// Files found (scanned when the card first shows, and on Refresh).
    pub files: Vec<String>,
    pub scanned: bool,
    pub image: Option<Arc<egui::ColorImage>>,
    loaded: String,
    pending: Option<Arc<Mutex<Option<Option<egui::ColorImage>>>>>,
}

impl Default for WristBackground {
    fn default() -> Self {
        Self { file: String::new(), dim: 0.55, files: Vec::new(), scanned: false, image: None, loaded: String::new(), pending: None }
    }
}

impl WristBackground {
    /// Keep the decoded image in step with `file` (decodes on a thread).
    pub fn tick(&mut self) {
        if let Some(slot) = &self.pending {
            let done = slot.lock().ok().and_then(|mut s| s.take());
            if let Some(img) = done {
                self.image = img.map(Arc::new);
                self.pending = None;
            }
        }
        if self.file == self.loaded {
            return;
        }
        self.loaded = self.file.clone();
        self.image = None;
        if !valid_name(&self.file) {
            return;
        }
        let path = dir().join(&self.file);
        let slot = Arc::new(Mutex::new(None));
        self.pending = Some(slot.clone());
        let _ = std::thread::Builder::new().name("rso-wrist-bg".into()).spawn(move || {
            let img = load(&path);
            if img.is_none() {
                tracing::warn!("wrist background: couldn't load {}", path.display());
            }
            if let Ok(mut s) = slot.lock() {
                *s = Some(img);
            }
        });
    }
}

fn load(path: &Path) -> Option<egui::ColorImage> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE {
        return None;
    }
    decode(&std::fs::read(path).ok()?)
}

/// PNG / JPG bytes → RGBA, longest edge ≤ [`MAX_EDGE`] (Windows' own
/// decoder, as for album art).
#[cfg(windows)]
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode};
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(bytes).ok()?;
    writer.StoreAsync().ok()?.get().ok()?;
    writer.DetachStream().ok()?;
    stream.Seek(0).ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let (w, h) = (dec.OrientedPixelWidth().ok()?, dec.OrientedPixelHeight().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let k = (MAX_EDGE as f32 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1));
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(sw).ok()?;
    t.SetScaledHeight(sh).ok()?;
    let px = dec
        .GetPixelDataTransformedAsync(BitmapPixelFormat::Rgba8, BitmapAlphaMode::Straight, &t, ExifOrientationMode::RespectExifOrientation, ColorManagementMode::DoNotColorManage)
        .ok()?
        .get()
        .ok()?
        .DetachPixelData()
        .ok()?;
    (px.len() == (sw * sh * 4) as usize).then(|| egui::ColorImage::from_rgba_unmultiplied([sw as usize, sh as usize], &px))
}

#[cfg(not(windows))]
fn decode(_bytes: &[u8]) -> Option<egui::ColorImage> {
    None
}

/// Paint `img` to cover `r` (cropped to fill, rounded), darkened by
/// `dim`. Texture cached per image.
pub fn paint(ui: &egui::Ui, img: &Arc<egui::ColorImage>, r: egui::Rect, rounding: f32, dim: f32) {
    let ctx = ui.ctx();
    // One cached texture, replaced when the image changes (album covers
    // change every song: a texture per image would pile up).
    let key = egui::Id::new("wrist-bg");
    let ptr = Arc::as_ptr(img) as usize;
    let tex = match ctx.data(|d| d.get_temp::<(usize, egui::TextureHandle)>(key)) {
        Some((p, t)) if p == ptr => t,
        _ => {
            // Created outside `data_mut`: load_texture locks the context too.
            let t = ctx.load_texture("wrist-bg", (**img).clone(), egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(key, (ptr, t.clone())));
            t
        }
    };
    let uv = cover_uv(img.size[0] as f32 / img.size[1].max(1) as f32, r.width() / r.height().max(1.0));
    egui::Image::new((tex.id(), r.size())).uv(uv).rounding(rounding).paint_at(ui, r);
    ui.painter().rect_filled(r, rounding, egui::Color32::from_black_alpha((dim.clamp(0.0, 1.0) * 255.0) as u8));
}

/// UV rect that crops an image of aspect `img` to fill a box of aspect
/// `target` (centred).
fn cover_uv(img: f32, target: f32) -> egui::Rect {
    if img > target {
        let w = target / img;
        egui::Rect::from_min_max(egui::pos2((1.0 - w) / 2.0, 0.0), egui::pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = img / target;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - h) / 2.0), egui::pos2(1.0, (1.0 + h) / 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bare_image_names() {
        assert!(valid_name("sunset.JPG"));
        assert!(valid_name("my bg.png"));
        assert!(!valid_name("../secret.png"));
        assert!(!valid_name("C:\\x.png"));
        assert!(!valid_name("\\\\server\\share\\x.png"));
        assert!(!valid_name("notes.txt"));
        assert!(!valid_name(""));
    }

    /// Real decode through Windows' codecs, scaled to MAX_EDGE.
    #[cfg(windows)]
    #[test]
    fn decodes_and_scales_a_png() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/store/vr-dashboard.png")).unwrap();
        let img = decode(&bytes).expect("decoded");
        assert_eq!(img.size[0].max(img.size[1]), MAX_EDGE as usize);
        assert!(img.pixels.iter().any(|p| p.r() > 0 || p.g() > 0 || p.b() > 0), "not all black");
        assert!(decode(b"not an image").is_none());
    }

    #[test]
    fn cover_crops_the_long_side() {
        // Wide image in a square: crop left/right.
        let u = cover_uv(2.0, 1.0);
        assert!((u.width() - 0.5).abs() < 1e-6 && u.height() == 1.0);
        // Tall image in a wide box: crop top/bottom.
        let u = cover_uv(0.5, 2.0);
        assert!((u.height() - 0.25).abs() < 1e-6 && u.width() == 1.0);
    }
}
