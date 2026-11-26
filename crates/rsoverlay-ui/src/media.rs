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
    /// Latest track + album-art accent (art only while `want_art`).
    latest: std::sync::Mutex<(Option<MediaInfo>, Option<egui::Color32>)>,
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
                    art = if art_for.is_empty() { None } else { art_color() };
                }
                if let Ok(mut l) = s.latest.lock() {
                    *l = (info, art);
                }
                drop(s);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        Self { shared }
    }

    /// Latest result; never blocks on the media service.
    pub fn latest(&self, want_art: bool) -> (Option<MediaInfo>, Option<egui::Color32>) {
        self.shared.want_art.store(want_art, std::sync::atomic::Ordering::Relaxed);
        self.shared.latest.lock().map(|l| l.clone()).unwrap_or_default()
    }
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

/// Most vivid colour of an image (BGRA or RGBA bytes), lifted so it
/// reads as an accent on true black. Pure: hue-binned, weighted by
/// saturation × value; greys and near-blacks are ignored.
pub fn vivid_color(px: &[u8], bgra: bool) -> Option<egui::Color32> {
    let mut bins = [(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); 12];
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
        let w = s * mx;
        let bin = &mut bins[((h / 30.0) as usize).min(11)];
        bin.0 += rf * w;
        bin.1 += gf * w;
        bin.2 += bf * w;
        bin.3 += w;
    }
    let best = bins.iter().max_by(|a, b| a.3.total_cmp(&b.3))?;
    if best.3 <= 0.0 {
        return None;
    }
    let (r, g, b) = (best.0 / best.3, best.1 / best.3, best.2 / best.3);
    // Legibility on black: lift value to ≥ 0.85, keep saturation moderate.
    let mx = r.max(g).max(b).max(1e-3);
    let lift = 0.88 / mx;
    let mean = (r + g + b) / 3.0;
    let soften = |c: f32| ((mean + (c - mean) * 0.85) * lift).clamp(0.0, 1.0);
    Some(egui::Color32::from_rgb((soften(r) * 255.0) as u8, (soften(g) * 255.0) as u8, (soften(b) * 255.0) as u8))
}

/// Accent colour from the current SMTC session's album art.
#[cfg(windows)]
pub fn art_color() -> Option<egui::Color32> {
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    let mgr = Mgr::RequestAsync().ok()?.get().ok()?;
    let props = mgr.GetCurrentSession().ok()?.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let fmt = dec.BitmapPixelFormat().ok()?;
    let data = dec.GetPixelDataAsync().ok()?.get().ok()?.DetachPixelData().ok()?;
    match fmt {
        BitmapPixelFormat::Bgra8 => vivid_color(&data, true),
        BitmapPixelFormat::Rgba8 => vivid_color(&data, false),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn art_color() -> Option<egui::Color32> {
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

    #[test]
    fn all_grey_gives_none() {
        let px = [100u8, 100, 100, 255].repeat(500);
        assert_eq!(vivid_color(&px, true), None);
    }
}
