//! Motion primitives — easing + a global "reduce motion" switch.
//!
//! Cost model: every animation here is a scalar interpolated on the CPU
//! and applied as a position offset or vertex-colour alpha. No offscreen
//! layers, no extra render passes; egui only schedules extra frames while
//! a value is actually moving.

use std::sync::atomic::{AtomicBool, Ordering};

static REDUCE: AtomicBool = AtomicBool::new(false);

/// Standard durations (seconds). Short on purpose: in VR, long UI
/// motion reads as lag rather than polish.
pub const FAST: f32 = 0.12;
pub const MEDIUM: f32 = 0.18;
pub const SLOW: f32 = 0.40;

pub fn set_reduce_motion(on: bool) {
    REDUCE.store(on, Ordering::Relaxed);
}

pub fn reduced() -> bool {
    REDUCE.load(Ordering::Relaxed)
}

/// Duration honouring the reduce-motion switch.
pub fn dur(secs: f32) -> f32 {
    if reduced() {
        0.0
    } else {
        secs
    }
}

/// Cubic ease-out: fast start, gentle settle.
pub fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Eased 0→1 progress of an animation that started at `start` (egui
/// input time, seconds). Requests a repaint while still running.
pub fn progress(ctx: &egui::Context, start: f64, secs: f32) -> f32 {
    let secs = dur(secs);
    if secs <= 0.0 {
        return 1.0;
    }
    let t = ((ctx.input(|i| i.time) - start) as f32 / secs).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}

/// Animate a value towards `target` (linear in time, `secs` for a full
/// unit change), honouring reduce-motion.
pub fn value(ctx: &egui::Context, id: egui::Id, target: f32, secs: f32) -> f32 {
    ctx.animate_value_with_time(id, target, dur(secs))
}

/// 0→1 hover/active blend for an element.
pub fn toggle(ctx: &egui::Context, id: egui::Id, on: bool, secs: f32) -> f32 {
    ctx.animate_bool_with_time(id, on, dur(secs))
}

/// Linear colour blend in premultiplied space.
pub fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_endpoints_and_monotonic() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(2.0), 1.0);
        let mut prev = 0.0;
        for i in 1..=20 {
            let v = ease_out(i as f32 / 20.0);
            assert!(v >= prev);
            prev = v;
        }
        // Ease-out front-loads: halfway in time is well past halfway.
        assert!(ease_out(0.5) > 0.8);
    }

    #[test]
    fn mix_endpoints() {
        let a = egui::Color32::BLACK;
        let b = egui::Color32::WHITE;
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5).r(), 128);
    }

    #[test]
    fn reduce_motion_zeroes_durations() {
        set_reduce_motion(true);
        assert_eq!(dur(SLOW), 0.0);
        set_reduce_motion(false);
        assert_eq!(dur(SLOW), SLOW);
    }
}
