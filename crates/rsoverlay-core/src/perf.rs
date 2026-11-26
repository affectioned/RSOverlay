//! VR performance summary from SteamVR compositor frame timings: the
//! numbers VR players actually watch — is the game hitting the headset's
//! refresh rate, how much GPU headroom is left, how often SteamVR had to
//! reproject.

/// One compositor frame (fields as SteamVR reports them).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Time between the game's frames, ms.
    pub interval_ms: f32,
    /// GPU time (game + compositor), ms.
    pub gpu_ms: f32,
    /// Times the frame was shown; > 1 means SteamVR reprojected it.
    pub presents: u32,
    pub dropped: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VrPerf {
    /// Headset refresh rate, Hz.
    pub hz: f32,
    /// Game frames per second.
    pub fps: f32,
    /// Average GPU frame time, ms.
    pub gpu_ms: f32,
    /// Frame budget left at this refresh rate, ms (negative = over).
    pub headroom_ms: f32,
    /// Share of frames SteamVR had to show more than once, 0..=100.
    pub reprojected_pct: f32,
    /// Frames the compositor dropped in the window.
    pub dropped: u32,
}

/// Summarise recent frames. `None` without usable samples or refresh
/// rate. O(n).
pub fn summarize(frames: &[Frame], hz: f32) -> Option<VrPerf> {
    if hz <= 1.0 {
        return None;
    }
    let valid: Vec<&Frame> = frames.iter().filter(|f| f.interval_ms > 0.0 && f.interval_ms.is_finite()).collect();
    if valid.is_empty() {
        return None;
    }
    let n = valid.len() as f32;
    let interval = valid.iter().map(|f| f.interval_ms).sum::<f32>() / n;
    let gpu_ms = valid.iter().map(|f| f.gpu_ms.max(0.0)).sum::<f32>() / n;
    let reprojected = valid.iter().filter(|f| f.presents > 1).count() as f32;
    Some(VrPerf {
        hz,
        // A game can't usefully exceed the display: clamp tiny overshoot.
        fps: (1000.0 / interval).min(hz),
        gpu_ms,
        headroom_ms: 1000.0 / hz - gpu_ms,
        reprojected_pct: 100.0 * reprojected / n,
        dropped: valid.iter().map(|f| f.dropped).sum(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(interval_ms: f32, gpu_ms: f32, presents: u32) -> Frame {
        Frame { interval_ms, gpu_ms, presents, dropped: 0 }
    }

    #[test]
    fn steady_90hz_game() {
        let frames = vec![f(1000.0 / 90.0, 7.0, 1); 90];
        let p = summarize(&frames, 90.0).unwrap();
        assert!((p.fps - 90.0).abs() < 0.01);
        assert!((p.headroom_ms - (11.111 - 7.0)).abs() < 0.01);
        assert_eq!(p.reprojected_pct, 0.0);
    }

    #[test]
    fn half_rate_with_reprojection() {
        // Game at 45 fps on a 90 Hz headset: every frame shown twice.
        let frames = vec![f(1000.0 / 45.0, 13.0, 2); 45];
        let p = summarize(&frames, 90.0).unwrap();
        assert!((p.fps - 45.0).abs() < 0.01);
        assert_eq!(p.reprojected_pct, 100.0);
        assert!(p.headroom_ms < 0.0, "over budget");
    }

    #[test]
    fn needs_samples_and_refresh_rate() {
        assert!(summarize(&[], 90.0).is_none());
        assert!(summarize(&[f(11.1, 7.0, 1)], 0.0).is_none());
        assert!(summarize(&[f(0.0, 7.0, 1), f(f32::NAN, 1.0, 1)], 90.0).is_none());
    }
}
