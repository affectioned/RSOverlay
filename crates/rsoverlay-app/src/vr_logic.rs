//! The VR shell's decisions, free of SteamVR: which panel the laser
//! picks, where a grabbed panel goes, the stick lock, what's shown,
//! hidden or throttled, focus. `vr.rs` calls these every frame; the tests
//! below drive them with scripted controller sessions against
//! [`SimOverlay`], a stand-in for SteamVR's overlay hit-testing.

// The VR loop that calls these is Windows-only.
#![cfg_attr(not(windows), allow(dead_code))]

use glam::{Affine3A, Quat, Vec3A};

pub(crate) const STICK_DEAD: f32 = 0.2;

/// Grab smoothing time constant range (Settings › Pointer › Grab
/// smoothing 0..1); the default 0.3 ≈ 0.05 s kills hand tremor on big
/// panels without feeling laggy.
pub(crate) const GRAB_TAU: (f32, f32) = (0.005, 0.15);

/// Auto-hidden window toolbars stay up this long after the laser leaves.
pub(crate) const TOOLBAR_LINGER_S: f64 = 1.5;

/// Focus moves to another window only when it's this many degrees
/// closer to your gaze (no flicker between neighbours).
pub(crate) const FOCUS_HYSTERESIS_DEG: f32 = 6.0;

/// Stick (x, y) after the one-action-at-a-time lock: the axis that
/// leaves the dead zone first wins until the stick recentres.
pub(crate) fn lock_axis(axis: &mut Option<bool>, x: f32, y: f32) -> (f32, f32) {
    if x == 0.0 && y == 0.0 {
        *axis = None;
        return (0.0, 0.0);
    }
    match *axis.get_or_insert(x.abs() > y.abs()) {
        true => (x, 0.0),
        false => (0.0, y),
    }
}

/// Panels on a horizontal arc around `eye`, centred on the flat
/// forward direction `fwd`, left to right, each `radius` from the eye,
/// upright and facing it. `widths` in metres, `gap` along the arc.
pub(crate) fn arc_poses(eye: Vec3A, fwd: Vec3A, widths: &[f32], radius: f32, gap: f32, drop: f32) -> Vec<Affine3A> {
    let fwd = Vec3A::new(fwd.x, 0.0, fwd.z).try_normalize().unwrap_or(Vec3A::NEG_Z);
    let total: f32 = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    let mut along = -total / 2.0;
    widths
        .iter()
        .map(|w| {
            let a = (along + w / 2.0) / radius; // radians, + = to the right
            along += w + gap;
            let dir = glam::Quat::from_rotation_y(-a) * glam::Vec3::from(fwd);
            let pos = eye + Vec3A::from(dir) * radius + Vec3A::new(0.0, drop, 0.0);
            face(pos, Vec3A::new(eye.x, pos.y, eye.z))
        })
        .collect()
}

/// Degrees between the head's gaze and the direction to `p`.
pub(crate) fn gaze_angle(hmd: &Affine3A, p: Vec3A) -> f32 {
    let gaze = -hmd.matrix3.z_axis;
    let to = (p - hmd.translation).try_normalize().unwrap_or(gaze);
    gaze.dot(to).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Focus mode: which window (by seq) you're working in. The laser
/// wins; otherwise the window nearest your gaze, switching only when
/// another is clearly nearer than the current one.
pub(crate) fn pick_focus(current: Option<u32>, laser: Option<u32>, by_gaze: &[(u32, f32)]) -> Option<u32> {
    if laser.is_some() {
        return laser;
    }
    let best = by_gaze.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1))?;
    match current.and_then(|c| by_gaze.iter().find(|(s, _)| *s == c)) {
        Some(&(c, a)) if a <= best.1 + FOCUS_HYSTERESIS_DEG => Some(c),
        _ => Some(best.0),
    }
}

/// XSO-style auto-hide: shown while the angle between the head's
/// gaze and the panel's facing direction stays under `clip_deg`.
/// Panels face the viewer along +Z, so straight-on is gaze == −Z.
pub(crate) fn in_view(hmd: &Affine3A, panel: &Affine3A, clip_deg: f32) -> bool {
    let gaze = -hmd.matrix3.z_axis;
    let toward = -panel.matrix3.z_axis;
    gaze.dot(toward).clamp(-1.0, 1.0).acos().to_degrees() < clip_deg
}

/// Upright pose at `pos` whose +Z (visible face) points at `eye`.
/// No roll — panels stay level however you twist the controller.
pub(crate) fn face(pos: Vec3A, eye: Vec3A) -> Affine3A {
    let z = (eye - pos).try_normalize().unwrap_or(Vec3A::Z);
    let x = Vec3A::Y.cross(z).try_normalize().unwrap_or(Vec3A::X);
    let y = z.cross(x);
    Affine3A::from_cols(x, y, z, pos)
}

/// Frame-rate-independent exponential approach for poses.
pub(crate) fn smooth(from: Affine3A, to: Affine3A, dt: f32, smoothing: f32) -> Affine3A {
    let tau = GRAB_TAU.0 + (GRAB_TAU.1 - GRAB_TAU.0) * smoothing.clamp(0.0, 1.0);
    let k = 1.0 - (-dt.max(0.0) / tau).exp();
    let (_, r0, t0) = from.to_scale_rotation_translation();
    let (_, r1, t1) = to.to_scale_rotation_translation();
    Affine3A::from_rotation_translation(r0.slerp(r1, k), t0.lerp(t1, k))
}

/// SteamVR draws overlays by sort order, not depth. Each panel gets
/// an explicit layer (higher = drawn on top), and the laser picks the
/// top layer where panels overlap, so it hits what you see.
/// Windows 0, dashboard 10, keyboard 20, wrist 30 (toasts 60,
/// beams 100).
pub(crate) fn layer(panel: usize) -> u32 {
    match panel {
        0 => 30,
        1 => 10,
        2 => 20,
        _ => 0,
    }
}

/// Does hit `a` (panel, distance) beat `b`? Top layer first, then
/// nearest.
pub(crate) fn hit_beats(a: (usize, f32), b: (usize, f32)) -> bool {
    (layer(a.0), -a.1) > (layer(b.0), -b.1)
}

pub(crate) fn stick(v: f32) -> f32 {
    if v.abs() < STICK_DEAD {
        0.0
    } else {
        v.signum() * (v.abs() - STICK_DEAD) / (1.0 - STICK_DEAD)
    }
}

/// Where a grabbed panel goes this frame. `hit` is the laser point the
/// panel is held at, `offset` the grab point → centre in panel axes per
/// metre of width. With `tilt` (the panel's rotation relative to the
/// hand at grab) it turns with the hand; without, it stays upright and
/// faces `eye`.
pub(crate) fn grab_target(hit: Vec3A, offset: Vec3A, width: f32, tilt: Option<Quat>, hand_rot: Quat, eye: Option<Vec3A>) -> Affine3A {
    let mut target = match tilt {
        Some(rel) => Affine3A::from_rotation_translation(hand_rot * rel, hit.into()),
        None => face(hit, eye.unwrap_or(hit + Vec3A::Z)),
    };
    target.translation = hit + target.matrix3 * (offset * width);
    if let (Some(e), None) = (eye, tilt) {
        target = Affine3A { translation: target.translation, ..face(target.translation, e) };
    }
    target
}

/// Is a window panel visible this frame? Hidden while SteamVR's menu is
/// open; otherwise shown when pinned, when the Windows button has them
/// up, or (windows-follow-dashboard) with the dashboard; and it must be
/// in view (or held).
pub(crate) fn window_shown(steamvr_menu: bool, follow_dash: bool, windows_up: bool, pinned: bool, dash_visible: bool, grabbed: bool, in_view: bool) -> bool {
    !steamvr_menu && (pinned || windows_up || (follow_dash && dash_visible)) && (grabbed || in_view)
}

/// The Windows button's state when the follow-dashboard setting is
/// (re)read: following → windows start down (they come with the
/// dashboard); not following → up (always there until hidden).
pub(crate) fn sync_windows_up(prev_follow: Option<bool>, follow: bool, up: bool) -> bool {
    if prev_follow == Some(follow) {
        up
    } else {
        !follow
    }
}

/// Should a window re-render this frame? Always when looked at (and not
/// dimmed by Focus), when it has input, or with "Keep smooth"; otherwise
/// at the unwatched rate.
pub(crate) fn should_render(in_view: bool, dimmed: bool, smooth: bool, since_last_s: f64, background_s: f64, has_events: bool) -> bool {
    smooth || has_events || (in_view && !dimmed) || since_last_s >= background_s
}

/// Window toolbar visible? Always without auto-hide; otherwise while the
/// laser was on it within the last [`TOOLBAR_LINGER_S`].
pub(crate) fn toolbar_visible(autohide: bool, now: f64, last_seen: f64) -> bool {
    !autohide || now - last_seen < TOOLBAR_LINGER_S
}

/// A stand-in for a SteamVR overlay (tests): a flat quad `width` metres
/// wide (`aspect` = height / width) at `pose`, its face along +Z.
#[cfg(test)]
#[derive(Copy, Clone, Debug)]
pub(crate) struct SimOverlay {
    pub pose: Affine3A,
    pub width: f32,
    pub aspect: f32,
}

#[cfg(test)]
impl SimOverlay {
    /// Ray hit like `ComputeOverlayIntersection`: (uv with v up from the
    /// bottom, distance), or None if the ray misses the quad.
    pub fn intersect(&self, origin: Vec3A, dir: Vec3A) -> Option<([f32; 2], f32)> {
        let n = self.pose.matrix3.z_axis;
        let denom = n.dot(dir);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = n.dot(self.pose.translation - origin) / denom;
        if t <= 0.0 {
            return None;
        }
        let local = self.pose.inverse().transform_point3a(origin + dir * t);
        let (w, h) = (self.width, self.width * self.aspect);
        let uv = [local.x / w + 0.5, local.y / h + 0.5];
        ((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1])).then_some((uv, t))
    }
}

/// The panel a ray picks among visible overlays: top layer first, then
/// nearest (the same rule the VR loop uses with SteamVR's intersections).
#[cfg(test)]
pub(crate) fn pick(panels: &[(usize, SimOverlay, bool)], origin: Vec3A, dir: Vec3A) -> Option<(usize, [f32; 2], f32)> {
    let mut best: Option<(usize, [f32; 2], f32)> = None;
    for &(i, o, visible) in panels {
        if !visible {
            continue;
        }
        if let Some((uv, d)) = o.intersect(origin, dir) {
            if best.is_none_or(|b| hit_beats((i, d), (b.0, b.2))) {
                best = Some((i, uv, d));
            }
        }
    }
    best
}

#[cfg(test)]
mod sim {
    //! Scripted VR sessions: an eye, panels as [`SimOverlay`]s and a
    //! controller ray, stepped frame by frame through the same decisions
    //! the VR loop makes.
    use super::*;

    const EYE: Vec3A = Vec3A::new(0.0, 1.6, 0.0);
    /// Panel indices, as in the VR loop.
    const WRIST: usize = 0;
    const KEYBOARD: usize = 2;
    const WINDOW: usize = 3;

    fn ray_at(from: Vec3A, target: Vec3A) -> (Vec3A, Vec3A) {
        (from, (target - from).normalize())
    }

    /// The beta.1 bug report: a keyboard tilted in front of a window's
    /// lower edge. Where they overlap the laser must hit the keyboard
    /// (drawn on top), even though the window is nearer along the ray.
    #[test]
    fn keyboard_over_a_window_gets_the_laser_where_it_is_drawn() {
        let window = SimOverlay { pose: face(Vec3A::new(0.0, 1.3, -1.0), EYE), width: 1.2, aspect: 0.6 };
        // Keyboard behind the window's bottom edge, tilted up 40°.
        let kb_pos = Vec3A::new(0.0, 1.05, -1.15);
        let kb_pose = Affine3A::from_rotation_translation(Quat::from_rotation_x(-40f32.to_radians()), kb_pos.into());
        let keyboard = SimOverlay { pose: kb_pose, width: 0.75, aspect: 0.4 };
        let panels = [(WINDOW, window, true), (KEYBOARD, keyboard, true)];
        let hand = Vec3A::new(0.1, 1.2, -0.3);

        // Overlap: the ray crosses the window first, but the keyboard wins.
        let (o, d) = ray_at(hand, kb_pos);
        assert!(window.intersect(o, d).is_some_and(|(_, dw)| keyboard.intersect(o, d).is_some_and(|(_, dk)| dw < dk)), "scene sanity: window nearer");
        assert_eq!(pick(&panels, o, d).map(|p| p.0), Some(KEYBOARD));
        // Above the keyboard: plain window.
        let (o, d) = ray_at(hand, Vec3A::new(0.0, 1.5, -1.0));
        assert_eq!(pick(&panels, o, d).map(|p| p.0), Some(WINDOW));
        // Hidden panels never catch the laser.
        let hidden = [(WINDOW, window, true), (KEYBOARD, keyboard, false)];
        let (o, d) = ray_at(hand, kb_pos);
        assert_eq!(pick(&hidden, o, d).map(|p| p.0), Some(WINDOW));
    }

    #[test]
    fn uv_matches_steamvr_orientation() {
        let o = SimOverlay { pose: face(Vec3A::new(0.0, 1.6, -1.0), EYE), width: 1.0, aspect: 0.5 };
        // Centre.
        let (uv, d) = o.intersect(EYE, -Vec3A::Z).unwrap();
        assert!((uv[0] - 0.5).abs() < 1e-4 && (uv[1] - 0.5).abs() < 1e-4 && (d - 1.0).abs() < 1e-4);
        // Upper-left area: u small, v large (v counts up from the bottom).
        let (o2, d2) = ray_at(EYE, Vec3A::new(-0.4, 1.8, -1.0));
        let (uv, _) = o.intersect(o2, d2).unwrap();
        assert!(uv[0] < 0.2 && uv[1] > 0.8, "{uv:?}");
        // Off the edge: miss.
        let (o3, d3) = ray_at(EYE, Vec3A::new(0.8, 1.6, -1.0));
        assert!(o.intersect(o3, d3).is_none());
    }

    #[test]
    fn wrist_is_drawn_over_everything() {
        assert!(hit_beats((WRIST, 2.0), (KEYBOARD, 0.5)));
        assert!(hit_beats((KEYBOARD, 2.0), (WINDOW, 0.5)));
    }

    /// Carrying a panel with the controller: upright + facing you by
    /// default; with Tilt, it follows the hand's roll.
    #[test]
    fn grabbed_panel_follows_the_hand_only_with_tilt() {
        let hit = Vec3A::new(0.0, 1.5, -1.0);
        let rolled = Quat::from_rotation_z(30f32.to_radians());

        // Default: the hand rolls 30°, the panel stays level and faces you.
        let p = grab_target(hit, Vec3A::ZERO, 1.0, None, rolled, Some(EYE));
        assert!(p.matrix3.x_axis.y.abs() < 1e-4, "no roll: {:?}", p.matrix3.x_axis);
        let to_eye = (EYE - p.translation).normalize();
        assert!(p.matrix3.z_axis.dot(to_eye) > 0.999);

        // Tilt on: grabbed level (rel = identity), the hand rolls 30° → the
        // panel rolls 30° with it.
        let p = grab_target(hit, Vec3A::ZERO, 1.0, Some(Quat::IDENTITY), rolled, Some(EYE));
        let roll = p.matrix3.x_axis.y.atan2(p.matrix3.x_axis.x).to_degrees();
        assert!((roll - 30.0).abs() < 0.1, "roll {roll}");

        // The grab point stays under the laser: a panel held by its left
        // edge hangs to the right of the hit.
        let p = grab_target(hit, Vec3A::new(0.5, 0.0, 0.0), 1.0, None, Quat::IDENTITY, Some(EYE));
        assert!(p.translation.x > hit.x + 0.45);
    }

    /// Stick while grabbing, frame by frame: with the lock, a sloppy
    /// diagonal push only resizes; after recentring it can push / pull.
    #[test]
    fn scripted_stick_session_with_the_axis_lock() {
        let frames = [(0.0, 0.0), (0.7, 0.3), (0.6, 0.8), (0.1, 0.9), (0.0, 0.0), (0.2, -0.9), (0.9, -0.5)];
        let mut axis = None;
        let out: Vec<(f32, f32)> = frames.iter().map(|&(x, y)| lock_axis(&mut axis, stick(x), stick(y))).collect();
        // Resizing frames: only x moves.
        assert!(out[1].1 == 0.0 && out[1].0 > 0.0);
        assert!(out[2].1 == 0.0 && out[2].0 > 0.0);
        // Back in the dead zone on x, still locked to resize: nothing.
        assert_eq!(out[3], (0.0, 0.0));
        // Recentred, then a pull: push/pull only, even when x creeps in.
        assert!(out[5].0 == 0.0 && out[5].1 < 0.0);
        assert!(out[6].0 == 0.0 && out[6].1 < 0.0);
    }

    /// XSO layout mode + the SteamVR menu, over a session.
    #[test]
    fn what_windows_show_over_a_session() {
        // (steamvr menu, dashboard open, pinned) → shown? (in view, not held)
        let follow = true;
        let up = sync_windows_up(None, follow, true);
        assert!(!up, "following: windows start down");
        assert!(!window_shown(false, follow, up, false, false, false, true), "unpinned hides with the dashboard closed");
        assert!(window_shown(false, follow, up, false, true, false, true), "…and shows with it open");
        assert!(window_shown(false, follow, up, true, false, false, true), "pinned stays");
        assert!(!window_shown(true, follow, up, true, true, false, true), "SteamVR menu hides everything");
        let up = sync_windows_up(Some(true), false, up);
        assert!(up, "follow off: windows up");
        assert!(window_shown(false, false, up, false, false, false, true), "follow off: always");
        assert!(!window_shown(false, false, up, true, true, false, false), "out of view");
        assert!(window_shown(false, false, up, true, true, true, false), "held panels never fade");
    }

    /// Tester report: reading Discord, close the dashboard, and the
    /// windows went with it. The Windows button brings back just the
    /// windows, without the dashboard, and closing the dashboard no
    /// longer takes them away.
    #[test]
    fn windows_button_shows_windows_without_the_dashboard() {
        let follow = true;
        let shown = |up: bool, dash: bool| window_shown(false, follow, up, false, dash, false, true);
        let mut up = sync_windows_up(None, follow, false);
        // Dashboard open → Discord is up; close the dashboard → gone.
        assert!(shown(up, true));
        assert!(!shown(up, false));
        // Windows button, dashboard still closed → just the windows.
        up = !up;
        assert!(shown(up, false));
        // Open and close the dashboard: the windows stay.
        assert!(shown(up, true));
        assert!(shown(up, false));
        // Settings re-read with the same value doesn't reset the button.
        up = sync_windows_up(Some(follow), follow, up);
        assert!(shown(up, false));
        // Windows button again → hidden ("minimised").
        up = !up;
        assert!(!shown(up, false));
        // Follow off: the button hides / shows the always-up windows.
        let mut up = sync_windows_up(Some(true), false, up);
        assert!(window_shown(false, false, up, false, false, false, true));
        up = !up;
        assert!(!window_shown(false, false, up, false, false, false, true), "hidden by the button");
        assert!(!window_shown(false, false, up, false, true, false, true), "even with the dashboard open");
    }

    #[test]
    fn render_throttling_and_toolbar_auto_hide() {
        let bg = 0.2;
        assert!(should_render(true, false, false, 0.0, bg, false), "looked at: every frame");
        assert!(!should_render(false, false, false, 0.05, bg, false), "away: waits");
        assert!(should_render(false, false, false, 0.25, bg, false), "…until the unwatched interval");
        assert!(!should_render(true, true, false, 0.05, bg, false), "dimmed by Focus: throttled even in view");
        assert!(should_render(false, true, true, 0.0, bg, false), "Keep smooth: always");
        assert!(should_render(false, false, false, 0.0, bg, true), "input: always");

        assert!(toolbar_visible(false, 100.0, f64::NEG_INFINITY), "auto-hide off: always");
        assert!(!toolbar_visible(true, 100.0, f64::NEG_INFINITY), "never pointed at: hidden");
        assert!(toolbar_visible(true, 100.0, 99.0), "lingers after the laser leaves");
        assert!(!toolbar_visible(true, 100.0, 98.0));
    }

    /// Focus over a session: gaze moves between two windows, the laser
    /// lands on one, then leaves.
    #[test]
    fn scripted_focus_session() {
        let mut focus = None;
        // Looking at window 1 (angles: w1 5°, w2 30°).
        focus = pick_focus(focus, None, &[(1, 5.0), (2, 30.0)]);
        assert_eq!(focus, Some(1));
        // Glance slightly towards 2: not clearly nearer, focus holds.
        focus = pick_focus(focus, None, &[(1, 12.0), (2, 9.0)]);
        assert_eq!(focus, Some(1));
        // Point the laser at 2: it takes focus immediately.
        focus = pick_focus(focus, Some(2), &[(1, 12.0), (2, 9.0)]);
        assert_eq!(focus, Some(2));
        // Laser off, gaze back on 1 and clearly nearer: focus moves.
        focus = pick_focus(focus, None, &[(1, 3.0), (2, 25.0)]);
        assert_eq!(focus, Some(1));
    }
}
