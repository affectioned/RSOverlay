//! System media control — titles of whatever is playing + play/pause
//! / next / prev writes — via Windows
//! `GlobalSystemMediaTransportControlsSession` (SMTC). Non-Windows
//! stubs out.

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub source: String,
    pub playing: bool,
}

/// Polls SMTC on its own thread. The WinRT calls block until the
/// system media service answers, and that service can stall: polling
/// on the UI thread would freeze the whole overlay with it.
pub struct MediaWorker {
    shared: std::sync::Arc<WorkerShared>,
}

#[derive(Default)]
struct WorkerShared {
    /// Latest track + album-art look (art only while `want_art`).
    latest: std::sync::Mutex<(Option<MediaInfo>, Option<ArtLook>)>,
    want_art: std::sync::atomic::AtomicBool,
}

impl MediaWorker {
    pub fn spawn() -> Self {
        let shared = std::sync::Arc::new(WorkerShared::default());
        let weak = std::sync::Arc::downgrade(&shared);
        let _ = std::thread::Builder::new().name("rso-media".into()).spawn(move || {
            let mut art_for = String::new();
            let mut art = None;
            // Exits once the worker (the only strong owner) is dropped.
            while let Some(s) = weak.upgrade() {
                let info = poll();
                let want = s.want_art.load(std::sync::atomic::Ordering::Relaxed);
                let title = info.as_ref().map(|m| m.title.clone()).unwrap_or_default();
                if !want {
                    art_for.clear();
                    art = None;
                } else if title != art_for {
                    art_for = title;
                    art = if art_for.is_empty() { None } else { art_look() };
                }
                if let Ok(mut l) = s.latest.lock() {
                    *l = (info, art.clone());
                }
                drop(s);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        Self { shared }
    }

    /// Latest result; never blocks on the media service.
    pub fn latest(&self, want_art: bool) -> (Option<MediaInfo>, Option<ArtLook>) {
        self.shared.want_art.store(want_art, std::sync::atomic::Ordering::Relaxed);
        self.shared.latest.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

/// What the Media theme takes from the album art: an accent colour and
/// the cover itself (shown behind the clock + media on the wrist).
#[derive(Clone, Debug)]
pub struct ArtLook {
    pub accent: Option<egui::Color32>,
    pub cover: std::sync::Arc<egui::ColorImage>,
}

/// Longest edge of the cover kept for the wrist (it's shown ~300 pt
/// wide; full-size art is 800+ px).
pub const COVER_EDGE: usize = 320;

/// Cover pixels (`w` x `h`, BGRA / RGBA) -> RGBA image, area-averaged
/// down so the longest edge is at most `max`. Pure.
pub fn cover_image(px: &[u8], w: usize, h: usize, bgra: bool, max: usize) -> Option<egui::ColorImage> {
    if w == 0 || h == 0 || px.len() < w * h * 4 || max == 0 {
        return None;
    }
    let k = (w.max(h)).div_ceil(max).max(1);
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::with_capacity(ow * oh * 4);
    for oy in 0..oh {
        for ox in 0..ow {
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            for y in oy * k..((oy + 1) * k).min(h) {
                for x in ox * k..((ox + 1) * k).min(w) {
                    let p = &px[(y * w + x) * 4..][..4];
                    let (r, g, b) = if bgra { (p[2], p[1], p[0]) } else { (p[0], p[1], p[2]) };
                    acc[0] += u32::from(r);
                    acc[1] += u32::from(g);
                    acc[2] += u32::from(b);
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend_from_slice(&[(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, 255]);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([ow, oh], &out))
}

/// A made-up "Northern Lights" cover (aurora bands over a night sky)
/// for previews and screenshots: nobody's real album art.
pub fn demo_cover() -> egui::ColorImage {
    const N: usize = 256;
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            // Night sky: deep blue at the top to near-black at the bottom.
            let sky = [0.04 + 0.05 * (1.0 - v), 0.06 + 0.08 * (1.0 - v), 0.16 + 0.18 * (1.0 - v)];
            // Two wavy aurora curtains, green fading to violet upward.
            let mut glow = [0.0_f32; 3];
            for (k, (amp, base, col)) in [(0.08, 0.42, [0.20, 0.95, 0.62]), (0.06, 0.30, [0.55, 0.40, 0.95])].iter().enumerate() {
                let line = base + amp * (u * 6.3 + k as f32 * 1.7).sin() + 0.03 * (u * 17.0).sin();
                let d = v - line;
                let a = if d > 0.0 { (-(d / 0.035).powi(2)).exp() } else { (-(d / 0.16).powi(2)).exp() * 0.8 };
                for c in 0..3 {
                    glow[c] += col[c] * a;
                }
            }
            // Dark hills along the bottom.
            let hill = 0.80 + 0.05 * (u * 4.0).sin() + 0.03 * (u * 11.0 + 1.0).sin();
            let land = if v > hill { 0.15 } else { 1.0 };
            for c in 0..3 {
                px.push(((sky[c] + glow[c]).min(1.0) * land * 255.0) as u8);
            }
            px.push(255);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([N, N], &px)
}

#[derive(Copy, Clone, Debug)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
}

#[cfg(windows)]
pub fn poll() -> Option<MediaInfo> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    let mgr = Mgr::RequestAsync().ok()?.get().ok()?;
    let session = mgr.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let playback = session.GetPlaybackInfo().ok()?;
    let status = playback.PlaybackStatus().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().ok()?.to_string();
    let source = session.SourceAppUserModelId().ok()?.to_string();
    Some(MediaInfo {
        title,
        artist,
        source,
        playing: status == Status::Playing,
    })
}

/// Fire a transport command at whatever media app currently owns the
/// SMTC session (Spotify, browser tab, local player, whatever). The
/// toggle semantics for play/pause match what the media keys on your
/// keyboard already do — if nothing is playing and nothing is paused,
/// the command silently fails rather than guessing.
#[cfg(windows)]
pub fn send(cmd: MediaCommand) -> bool {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    let Some(mgr) = Mgr::RequestAsync().ok().and_then(|op| op.get().ok()) else {
        return false;
    };
    let Ok(session) = mgr.GetCurrentSession() else {
        return false;
    };
    let result = match cmd {
        MediaCommand::PlayPause => {
            let status = session
                .GetPlaybackInfo()
                .and_then(|p| p.PlaybackStatus())
                .ok();
            match status {
                Some(Status::Playing) => {
                    session.TryPauseAsync().ok().and_then(|op| op.get().ok())
                }
                _ => session.TryPlayAsync().ok().and_then(|op| op.get().ok()),
            }
        }
        MediaCommand::Next => session
            .TrySkipNextAsync()
            .ok()
            .and_then(|op| op.get().ok()),
        MediaCommand::Previous => session
            .TrySkipPreviousAsync()
            .ok()
            .and_then(|op| op.get().ok()),
    };
    result.unwrap_or(false)
}

#[cfg(not(windows))]
pub fn poll() -> Option<MediaInfo> {
    None
}

#[cfg(not(windows))]
pub fn send(_cmd: MediaCommand) -> bool {
    false
}

/// Most vivid colour of an image (BGRA or RGBA bytes), as an accent on
/// true black. Pure. Hues are binned and scored by how much of the image
/// is strongly coloured (saturation² × value, so a big vivid patch beats
/// a large dull one); the colour is then taken from that hue's most
/// saturated pixels, not their average (averaging washed it out; tester:
/// "lame"), and set to a bright, saturated tone that reads on black.
/// Greys and near-blacks are ignored.
pub fn vivid_color(px: &[u8], bgra: bool) -> Option<egui::Color32> {
    struct P {
        h: f32,
        s: f32,
        v: f32,
    }
    let mut pts = Vec::new();
    let mut score = [0.0_f32; 12];
    for p in px.as_chunks::<4>().0.iter().step_by(3) {
        let (r, g, b) = if bgra { (p[2], p[1], p[0]) } else { (p[0], p[1], p[2]) };
        let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let mx = rf.max(gf).max(bf);
        let mn = rf.min(gf).min(bf);
        let s = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        if s < 0.25 || mx < 0.2 || p[3] < 128 {
            continue;
        }
        let h = if mx == rf {
            ((gf - bf) / (mx - mn)).rem_euclid(6.0)
        } else if mx == gf {
            (bf - rf) / (mx - mn) + 2.0
        } else {
            (rf - gf) / (mx - mn) + 4.0
        } * 60.0;
        score[((h / 30.0) as usize).min(11)] += s * s * mx;
        pts.push(P { h, s, v: mx });
    }
    let (best, &sc) = score.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if sc <= 0.0 {
        return None;
    }
    // That hue's most saturated quarter: circular-mean hue, its saturation.
    let mut mine: Vec<&P> = pts.iter().filter(|p| ((p.h / 30.0) as usize).min(11) == best).collect();
    mine.sort_by(|a, b| (b.s * b.v).total_cmp(&(a.s * a.v)));
    let top = &mine[..mine.len().div_ceil(4)];
    let (sx, sy) = top.iter().fold((0.0_f32, 0.0_f32), |(x, y), p| (x + p.h.to_radians().cos(), y + p.h.to_radians().sin()));
    let hue = sy.atan2(sx).to_degrees().rem_euclid(360.0);
    let sat = (top.iter().map(|p| p.s).sum::<f32>() / top.len() as f32).clamp(0.55, 0.85);
    // Yellows / greens look brighter than blues at the same value: keep
    // every hue about as legible on black, never neon-white.
    let val = if (40.0..170.0).contains(&hue) { 0.88 } else { 0.97 };
    Some(hsv(hue, sat, val))
}

fn hsv(h: f32, s: f32, v: f32) -> egui::Color32 {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let u = |f: f32| ((f + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(u(r), u(g), u(b))
}

/// The current SMTC session's album art as pixels (bytes, w, h, is-BGRA).
#[cfg(windows)]
pub fn art_pixels() -> Option<(Vec<u8>, usize, usize, bool)> {
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    let mgr = Mgr::RequestAsync().ok()?.get().ok()?;
    let props = mgr.GetCurrentSession().ok()?.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let fmt = dec.BitmapPixelFormat().ok()?;
    let (w, h) = (dec.PixelWidth().ok()? as usize, dec.PixelHeight().ok()? as usize);
    let data = dec.GetPixelDataAsync().ok()?.get().ok()?.DetachPixelData().ok()?.to_vec();
    match fmt {
        BitmapPixelFormat::Bgra8 => Some((data, w, h, true)),
        BitmapPixelFormat::Rgba8 => Some((data, w, h, false)),
        _ => None,
    }
}

/// Accent colour from the current SMTC session's album art.
#[cfg(windows)]
pub fn art_look() -> Option<ArtLook> {
    let (px, w, h, bgra) = art_pixels()?;
    Some(ArtLook { accent: vivid_color(&px, bgra), cover: std::sync::Arc::new(cover_image(&px, w, h, bgra, COVER_EDGE)?) })
}

#[cfg(not(windows))]
pub fn art_look() -> Option<ArtLook> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_vivid_hue_not_the_grey_background() {
        // 80 % grey background, 20 % saturated blue.
        let mut px = Vec::new();
        for i in 0..1000 {
            if i % 5 == 0 {
                px.extend_from_slice(&[30, 60, 200, 255]); // RGBA blue
            } else {
                px.extend_from_slice(&[120, 120, 120, 255]);
            }
        }
        let c = vivid_color(&px, false).unwrap();
        assert!(c.b() > c.r() && c.b() > c.g(), "{c:?}");
        // Lifted for legibility on black.
        assert!(c.b() > 200);
    }

    /// Writes what's playing's art and the old / new accent swatches to
    /// RSO_ART_DUMP (raw RGBA) to compare by eye. Needs media playing.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn dump_current_art() {
        let (px, w, h, bgra) = art_pixels().expect("nothing with art playing");
        let c = vivid_color(&px, bgra);
        println!("accent {c:?}, art {w}x{h}");
        if let Ok(d) = std::env::var("RSO_ART_DUMP") {
            std::fs::write(format!("{d}/art_{w}x{h}_{}.raw", if bgra { "bgra" } else { "rgba" }), &px).unwrap();
        }
    }

    #[test]
    fn a_vivid_patch_is_taken_at_full_strength() {
        // Mostly dull brownish-red, a smaller saturated magenta patch.
        let mut px = Vec::new();
        for i in 0..1200 {
            if i % 4 == 0 {
                px.extend_from_slice(&[230, 30, 200, 255]);
            } else {
                px.extend_from_slice(&[120, 80, 70, 255]);
            }
        }
        let c = vivid_color(&px, false).unwrap();
        let (mx, mn) = (c.r().max(c.g()).max(c.b()), c.r().min(c.g()).min(c.b()));
        assert!(c.r() > c.g() && c.b() > c.g(), "magenta, not the dull red: {c:?}");
        assert!((mx - mn) as f32 / mx as f32 > 0.55, "saturated, not washed out: {c:?}");
    }

    #[test]
    fn cover_is_scaled_down_keeping_its_shape_and_colours() {
        // 800 x 400, left half blue, right half orange (BGRA).
        let (w, h) = (800usize, 400usize);
        let mut px = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                px.extend_from_slice(if x < w / 2 { &[200, 60, 30, 255] } else { &[40, 120, 230, 255] });
            }
        }
        let c = cover_image(&px, w, h, true, 320).unwrap();
        assert_eq!(c.size, [267, 134]);
        let at = |x: usize| c.pixels[(c.size[1] / 2) * c.size[0] + x];
        assert_eq!((at(0).r(), at(0).g(), at(0).b()), (30, 60, 200), "blue stays blue");
        assert_eq!((at(266).r(), at(266).b()), (230, 40), "orange stays orange");
        // Small art is kept as is.
        assert_eq!(cover_image(&px[..64 * 64 * 4], 64, 64, true, 320).unwrap().size, [64, 64]);
    }

    #[test]
    fn all_grey_gives_none() {
        let px = [100u8, 100, 100, 255].repeat(500);
        assert_eq!(vivid_color(&px, true), None);
    }
}
