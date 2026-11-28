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
/// open; otherwise shown while the Windows button has them up (or it's
/// pinned), and it must be in view (or held). The dashboard plays no
/// part: opening or closing it never shows or hides windows.
pub(crate) fn window_shown(steamvr_menu: bool, windows_up: bool, pinned: bool, grabbed: bool, in_view: bool) -> bool {
    !steamvr_menu && (pinned || windows_up) && (grabbed || in_view)
}

/// Windows button state after this frame: a newly opened window brings
/// the windows back up (you just asked for one, so it must be seen).
pub(crate) fn windows_up_after(up: bool, prev_count: usize, count: usize) -> bool {
    up || count > prev_count
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

/// Window panel texture multiplier range (on the 1920-texel base):
/// up to 4800 texels, enough for a 4K monitor at 1:1.
pub(crate) const RENDER_K: (f32, f32) = (1.0, 2.5);

/// Angle a panel `width_m` wide spans at `dist_m`, radians.
#[cfg(test)]
pub(crate) fn angular_width(width_m: f32, dist_m: f32) -> f32 {
    2.0 * (0.5 * width_m / dist_m.max(0.05)).atan()
}

/// Native resolution: the texture multiplier that draws the capture
/// one source pixel per texel, so it is copied, not resampled, and
/// SteamVR's sampling is the only filtering (what XSOverlay's monitor
/// capture looks like). `content_texels` is how wide the capture was
/// drawn last frame at `current_k`; the capture's size in points
/// doesn't depend on k, so this lands exactly in one step. Smaller
/// sources stay at the base (the toolbar needs its texels).
pub(crate) fn native_render_k(current_k: f32, src_px: u32, content_texels: f32) -> f32 {
    if content_texels < 1.0 || src_px == 0 {
        return current_k;
    }
    // Unrounded: base x k must come out a whole texel count exactly
    // (2560 / 1920, not 1.333 → 2559). ResSettle ignores float noise.
    (current_k * src_px as f32 / content_texels).clamp(RENDER_K.0, RENDER_K.1)
}

/// Steady pointer (Accessibility): an adaptive low-pass on the laser
/// ray itself, before anything uses it, so the beam, cursor, keyboard
/// and clicks all follow the same steadied ray. Nearly still (tremor)
/// → heavy smoothing; moving on purpose → almost none (a "1€ filter":
/// the cut-off frequency rises with the filtered angular speed).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RayStabilizer {
    /// (origin, direction, low-passed angular velocity vector).
    state: Option<(Vec3A, Vec3A, Vec3A)>,
}

/// Cut-off when still, Hz: from barely-there (strength 0+) to strong.
const STEADY_FC: (f32, f32) = (4.0, 0.3);
/// Cut-off rise per rad/s of deliberate motion.
const STEADY_BETA: f32 = 3.0;
/// Cut-off for the speed estimate itself, Hz (averages tremor out).
const STEADY_DFC: f32 = 1.0;

fn lp_alpha(fc: f32, dt: f32) -> f32 {
    let tau = 1.0 / (std::f32::consts::TAU * fc);
    1.0 / (1.0 + tau / dt.max(1e-4))
}

impl RayStabilizer {
    /// Filter one frame's ray. `strength` 0 = off (raw ray), 1 = strongest.
    pub fn step(&mut self, origin: Vec3A, dir: Vec3A, strength: f32, dt: f32) -> (Vec3A, Vec3A) {
        let dir = dir.normalize_or_zero();
        if strength <= 0.0 || dir == Vec3A::ZERO {
            self.state = None;
            return (origin, dir);
        }
        let Some((o0, d0, vel0)) = self.state else {
            self.state = Some((origin, dir, Vec3A::ZERO));
            return (origin, dir);
        };
        // Angular velocity (rad/s) as a vector, low-passed, then its size:
        // a tremor's back-and-forth cancels out; a deliberate move doesn't.
        let raw_vel = (dir - d0) / dt.max(1e-4);
        let vel = vel0 + (raw_vel - vel0) * lp_alpha(STEADY_DFC, dt);
        let speed = vel.length();
        let s = strength.clamp(0.0, 1.0);
        let fc = STEADY_FC.0 + (STEADY_FC.1 - STEADY_FC.0) * s + STEADY_BETA * speed;
        let a = lp_alpha(fc, dt);
        let d = (d0 + (dir - d0) * a).normalize_or_zero();
        let o = o0 + (origin - o0) * a;
        self.state = Some((o, d, vel));
        (o, d)
    }
}

/// The real mouse is yours: after you move it, the laser leaves it
/// alone for this long (clicks and drags still move it).
pub(crate) const MOUSE_YIELD_S: f64 = 2.0;
/// Laser hover only moves the real mouse once its point has moved this
/// far (screen px), so hand tremor doesn't keep grabbing it.
pub(crate) const MOUSE_DEADBAND_PX: i32 = 4;

/// Who owns the real mouse: notices when it moved without us (you, on
/// the PC mouse) and yields hover moves for [`MOUSE_YIELD_S`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MouseYield {
    prev: Option<(i32, i32)>,
    expected: Option<(i32, i32)>,
    yield_until: f64,
}

impl MouseYield {
    /// Once per frame, before any laser moves, with the real cursor
    /// position (screen px; None if unknown / hidden).
    pub fn observe(&mut self, actual: Option<(i32, i32)>, now: f64) {
        let far = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs() > 2 || (a.1 - b.1).abs() > 2;
        if let (Some(a), Some(p)) = (actual, self.prev) {
            if far(a, p) && self.expected.is_none_or(|e| far(a, e)) {
                self.yield_until = now + MOUSE_YIELD_S;
            }
        }
        self.prev = actual.or(self.prev);
        self.expected = None;
    }

    /// May a laser *hover* move the real mouse now?
    pub fn may_hover(&self, now: f64) -> bool {
        now >= self.yield_until
    }

    /// We moved the real mouse to `p` (screen px) this frame.
    pub fn moved(&mut self, p: (i32, i32)) {
        self.expected = Some(p);
        self.prev = Some(p);
    }
}

/// Has the laser point moved far enough from where it last put the
/// mouse to move it again?
pub(crate) fn beyond_deadband(last: Option<(i32, i32)>, now: (i32, i32)) -> bool {
    last.is_none_or(|l| (l.0 - now.0).abs() >= MOUSE_DEADBAND_PX || (l.1 - now.1).abs() >= MOUSE_DEADBAND_PX)
}

/// SteamVR's laser stays on this long after it reports leaving our
/// panels (crossing the gap between two of them).
pub(crate) const STEAM_GRACE_S: f64 = 0.5;
/// If SteamVR sends nothing this long after engaging, use our laser.
pub(crate) const STEAM_FALLBACK_S: f64 = 1.0;

/// SteamVR laser-mouse event on panel `.0` from hand slot `.1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LaserEv {
    Enter,
    Leave,
    Move(f32, f32),
    /// Position and button (1 left, 2 right, 4 middle).
    Down(f32, f32, u32),
    Up,
    Scroll(f32, f32),
}

/// RSOverlay on SteamVR's own laser (tester: far more accurate than
/// ours; the compositor draws it and decides the hit with no lag).
/// Our ray only *starts* it: when it first reaches a panel we set
/// MakeOverlaysInteractiveIfVisible, SteamVR's laser appears and takes
/// the controllers (our own input goes quiet), and from then on only
/// SteamVR's focus events decide when you've left. The first build let
/// our (by then input-less) ray decide that too, and the two fought.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SteamLaser {
    engaged: bool,
    flag_panel: Option<usize>,
    since: f64,
    last_event: f64,
    last_hover: f64,
    hover: Option<usize>,
    pos: Option<(f32, f32)>,
    hand: usize,
    down: Option<u32>,
    release_next: bool,
    /// Left on SteamVR's word while our ray still saw a panel: don't
    /// re-engage until our ray has been off the panels once.
    need_fresh: bool,
    /// SteamVR has reported its laser on a panel this stint ("left"
    /// only counts after that).
    seen: bool,
}

impl Default for SteamLaser {
    fn default() -> Self {
        Self {
            engaged: false,
            flag_panel: None,
            since: 0.0,
            last_event: f64::NEG_INFINITY,
            last_hover: f64::NEG_INFINITY,
            hover: None,
            pos: None,
            hand: 1,
            down: None,
            release_next: false,
            need_fresh: false,
            seen: false,
        }
    }
}

/// This frame's pointing, when SteamVR's laser is engaged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SteamLaserFrame {
    /// SteamVR's laser is on: our beams / cursors hide, panels take input
    /// from the fields below.
    pub engaged: bool,
    /// Panel carrying the interactive flag.
    pub flag_panel: Option<usize>,
    /// Panel under SteamVR's laser, and where (panel points).
    pub panel: Option<usize>,
    pub pos: Option<(f32, f32)>,
    pub hand: usize,
    /// Button held (1 left, 2 right, 4 middle).
    pub down: Option<u32>,
    pub scroll: (f32, f32),
}

impl SteamLaser {
    /// `ray_panel`: a panel our own ray is on (starts SteamVR's laser).
    pub fn step(&mut self, ray_panel: Option<usize>, events: &[(usize, usize, LaserEv)], now: f64) -> SteamLaserFrame {
        if std::mem::take(&mut self.release_next) {
            self.down = None;
        }
        let mut pressed_now = false;
        let mut scroll = (0.0, 0.0);
        for &(panel, hand, e) in events {
            self.last_event = now;
            self.hand = hand;
            match e {
                LaserEv::Enter => self.hover = Some(panel),
                LaserEv::Leave => {
                    if self.hover == Some(panel) {
                        self.hover = None;
                        self.pos = None;
                        // Grace counts from the moment it left.
                        self.last_hover = now;
                    }
                }
                LaserEv::Move(x, y) => {
                    self.hover = Some(panel);
                    self.pos = Some((x, y));
                }
                LaserEv::Down(x, y, b) => {
                    self.hover = Some(panel);
                    self.pos = Some((x, y));
                    self.down = Some(b);
                    pressed_now = true;
                }
                // A click inside one frame still shows as held for one.
                LaserEv::Up if pressed_now => self.release_next = true,
                LaserEv::Up => self.down = None,
                LaserEv::Scroll(x, y) => {
                    scroll.0 += x;
                    scroll.1 += y;
                }
            }
        }
        if self.hover.is_some() {
            self.last_hover = now;
            self.seen = true;
        }
        if !self.engaged {
            if ray_panel.is_none() {
                self.need_fresh = false;
            }
            if let (Some(p), false) = (ray_panel, self.need_fresh) {
                self.engaged = true;
                self.flag_panel = Some(p);
                self.since = now;
                self.last_hover = now;
            }
        } else {
            let left = self.seen && self.hover.is_none() && self.down.is_none() && now - self.last_hover >= STEAM_GRACE_S;
            let silent = self.last_event < self.since && now - self.since >= STEAM_FALLBACK_S;
            if left || silent {
                *self = Self { need_fresh: ray_panel.is_some(), hand: self.hand, last_event: self.last_event, ..Self::default() };
            }
        }
        if !self.engaged {
            return SteamLaserFrame { hand: self.hand, ..Default::default() };
        }
        SteamLaserFrame {
            engaged: true,
            flag_panel: self.flag_panel,
            panel: self.hover,
            pos: self.pos.filter(|_| self.hover.is_some()),
            hand: self.hand,
            down: self.down,
            scroll,
        }
    }
}

/// SteamVR's intersection-mask limit per overlay.
pub(crate) const MASK_MAX: usize = 32;

/// The laser-catching areas of a panel (x, y, w, h; points, y down) from
/// what it drew this frame: clipped to the panel, tiny bits dropped,
/// rounded outward. None = no mask needed (the drawn areas cover the
/// whole panel, or nothing was drawn). More than [`MASK_MAX`] → one box
/// around them all.
pub(crate) fn mask_rects(areas: &[[f32; 4]], size: (f32, f32)) -> Option<Vec<[f32; 4]>> {
    let mut out: Vec<[f32; 4]> = Vec::new();
    for &[x, y, w, h] in areas {
        let (x0, y0) = (x.max(0.0).floor(), y.max(0.0).floor());
        let (x1, y1) = ((x + w).min(size.0).ceil(), (y + h).min(size.1).ceil());
        if x1 - x0 < 4.0 || y1 - y0 < 4.0 {
            continue;
        }
        let r = [x0, y0, x1 - x0, y1 - y0];
        if !out.contains(&r) {
            out.push(r);
        }
    }
    if out.is_empty() {
        return None;
    }
    if out.iter().any(|r| r[2] >= size.0 - 1.0 && r[3] >= size.1 - 1.0) {
        return None;
    }
    if out.len() > MASK_MAX {
        let x0 = out.iter().map(|r| r[0]).fold(f32::MAX, f32::min);
        let y0 = out.iter().map(|r| r[1]).fold(f32::MAX, f32::min);
        let x1 = out.iter().map(|r| r[0] + r[2]).fold(0.0, f32::max);
        let y1 = out.iter().map(|r| r[1] + r[3]).fold(0.0, f32::max);
        out = vec![[x0, y0, x1 - x0, y1 - y0]];
    }
    Some(out)
}

fn in_rect(r: &[f32; 4], p: (f32, f32), margin: f32) -> bool {
    p.0 >= r[0] - margin && p.0 <= r[0] + r[2] + margin && p.1 >= r[1] - margin && p.1 <= r[1] + r[3] + margin
}

/// Which way y runs in SteamVR's intersection mask: its header names the
/// fields "TopLeft" but documents nothing, and OpenVR uses both origins
/// elsewhere. Checked once at runtime against SteamVR itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MaskOrigin {
    #[default]
    Unknown,
    TopDown,
    BottomUp,
    /// Neither checked out: no masks (the whole panel catches the laser,
    /// as before).
    Off,
}

/// A probe pair for the check: a point inside a mask rect whose vertical
/// mirror is in none (so the wrong convention misses it), and that mirror.
pub(crate) fn mask_probe(rects: &[[f32; 4]], size: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    let n = 24;
    for gy in 1..n {
        for gx in 1..n {
            let p = (size.0 * gx as f32 / n as f32, size.1 * gy as f32 / n as f32);
            let m = (p.0, size.1 - p.1);
            let inside = rects.iter().any(|r| in_rect(r, p, -6.0));
            let mirror_out = !rects.iter().any(|r| in_rect(r, m, 6.0));
            if inside && mirror_out {
                return Some((p, m));
            }
        }
    }
    None
}

/// Run the check: set the mask one way, probe both points through
/// SteamVR; the convention where the inside point hits and its mirror
/// misses is the real one.
pub(crate) fn calibrate_mask(
    mut set_mask: impl FnMut(bool),
    mut hits: impl FnMut((f32, f32)) -> bool,
    probe: ((f32, f32), (f32, f32)),
) -> MaskOrigin {
    for top_down in [true, false] {
        set_mask(top_down);
        if hits(probe.0) && !hits(probe.1) {
            return if top_down { MaskOrigin::TopDown } else { MaskOrigin::BottomUp };
        }
    }
    MaskOrigin::Off
}

/// Frame pacing: finish this long before SteamVR's next vsync, on top
/// of our measured work time (the compositor starts composing a few ms
/// ahead of vsync).
pub(crate) const PACE_MARGIN_S: f64 = 0.003;

/// How long to sleep after a frame so the next one starts just in time
/// to be drawn and submitted before SteamVR composes: one update per
/// headset frame, at a steady phase. Tester: a flat 11 ms sleep gave
/// ~60 updates/s, out of step with a 90 Hz+ headset, so in-panel cursors
/// trailed the controller-parented beam.
pub(crate) fn pace_sleep(since_vsync: f64, period: f64, work: f64) -> f64 {
    let period = period.clamp(1.0 / 240.0, 1.0 / 30.0);
    // Phase (since vsync) at which the next frame should start.
    let start = (period - work - PACE_MARGIN_S).rem_euclid(period);
    let mut wait = (start - since_vsync.rem_euclid(period)).rem_euclid(period);
    // Just missed it: don't spin a frame right away twice.
    if wait < 0.0005 {
        wait += period;
    }
    wait
}

/// Laser cursor: how far it sits back from the surface along the beam
/// (keeps it in front of the panel), metres.
pub(crate) const CURSOR_LIFT_M: f32 = 0.002;

/// Where the laser cursor goes: on the beam, `dist` along it (just short
/// of the surface), facing the eye. Its own overlay, parented to the
/// controller like the beam, so it rides the beam's tip every frame
/// instead of being drawn into the panel (a frame or two late).
pub(crate) fn cursor_pose(origin: Vec3A, dir: Vec3A, dist: f32, eye: Vec3A) -> Affine3A {
    let d = dir.normalize_or_zero();
    let p = origin + d * (dist - CURSOR_LIFT_M).max(0.0);
    let z = (eye - p).try_normalize().unwrap_or(-d);
    let x = Vec3A::Y.cross(z).try_normalize().unwrap_or(Vec3A::X);
    Affine3A::from_cols(x, z.cross(x), z, p)
}

/// Debounce for texture rebuilds: a new multiplier is applied once it
/// has been wanted for [`RES_SETTLE_S`] and the panel isn't held.
pub(crate) const RES_SETTLE_S: f64 = 0.75;

#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct ResSettle {
    pending: Option<(f32, f64)>,
}

impl ResSettle {
    /// `Some(k)` when the texture should be rebuilt at `k` now.
    pub fn step(&mut self, current: f32, wanted: f32, now: f64, held: bool) -> Option<f32> {
        if (wanted - current).abs() < 1e-3 {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((k, since)) if (k - wanted).abs() < 1e-3 => {
                if !held && now - since >= RES_SETTLE_S {
                    self.pending = None;
                    Some(wanted)
                } else {
                    None
                }
            }
            _ => {
                self.pending = Some((wanted, now));
                None
            }
        }
    }
}

/// Readable size: source pixels per headset pixel. Around 1.3 small
/// UI text (9–12 px) still lands on 7–9 headset pixels: legible.
pub(crate) const READ_RATIO: f32 = 1.3;
/// Widest the content may get, radians (90°): past this you'd be
/// turning your head to read a line.
pub(crate) const READ_MAX_ANGLE: f32 = std::f32::consts::FRAC_PI_2;

/// Panel width that makes `src_px` source pixels across readable at
/// `dist_m`: the content (`content_frac` of the panel's width) spans
/// src_px / ([`READ_RATIO`] x headset px per radian), at most
/// [`READ_MAX_ANGLE`]. Returns (width m, capped).
pub(crate) fn readable_width(src_px: u32, content_frac: f32, dist_m: f32, px_per_rad: f32) -> (f32, bool) {
    let want = src_px as f32 / (READ_RATIO * px_per_rad.max(1.0));
    let angle = want.min(READ_MAX_ANGLE);
    let content_w = 2.0 * dist_m.max(0.05) * (0.5 * angle).tan();
    (content_w / content_frac.clamp(0.2, 1.0), want > READ_MAX_ANGLE)
}

/// A stand-in for a SteamVR overlay (tests): a flat quad `width` metres
/// wide (`aspect` = height / width) at `pose`, its face along +Z.
/// Also used live as our own hit test for the wrist (see vr.rs).
#[derive(Copy, Clone, Debug)]
pub(crate) struct SimOverlay {
    pub pose: Affine3A,
    pub width: f32,
    pub aspect: f32,
}

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

    /// The SteamVR menu, pins and the Windows button, over a session.
    #[test]
    fn what_windows_show_over_a_session() {
        // (steamvr menu, windows up, pinned, held, in view)
        assert!(window_shown(false, true, false, false, true), "up and in view");
        assert!(!window_shown(false, false, false, false, true), "Windows button hid them");
        assert!(window_shown(false, false, true, false, true), "pinned stays");
        assert!(!window_shown(true, true, true, false, true), "SteamVR menu hides everything");
        assert!(!window_shown(false, true, false, false, false), "out of view");
        assert!(window_shown(false, true, false, true, false), "held panels never fade");
    }

    /// Tester reports: closing the dashboard took the windows with it.
    /// Dashboard and Windows are independent: only the Windows button
    /// (or opening a new window) changes whether windows show.
    #[test]
    fn dashboard_and_windows_button_are_independent() {
        // window_shown has no dashboard input at all: a session of
        // dashboard open / close can't change the answer.
        let mut up = true;
        let shown = |up: bool| window_shown(false, up, false, false, true);
        assert!(shown(up), "reading Discord");
        // Dashboard opened and closed: nothing to do with windows.
        assert!(shown(up));
        // Windows button → hidden ("minimised"); dashboard still free.
        up = !up;
        assert!(!shown(up));
        // Windows button again → back, without the dashboard.
        up = !up;
        assert!(shown(up));
        // Hidden, then a new window is opened: everything comes back.
        up = false;
        up = windows_up_after(up, 1, 2);
        assert!(shown(up));
        // Closing one doesn't bring them back.
        assert!(!windows_up_after(false, 2, 1));
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

    /// Steam Frame-ish: ~16 px/deg at the centre (917 px/rad).
    const PPR: f32 = 917.0;

    /// The tester's 2560 x 1440 monitor in the 1920-texel panel was
    /// drawn ~1920 texels wide: resampled twice (here, then SteamVR),
    /// blurrier than XSOverlay. Native = exactly 2560 texels.
    #[test]
    fn monitor_capture_is_drawn_one_pixel_per_texel() {
        let k = native_render_k(1.0, 2560, 1920.0);
        assert!((k - 1.333).abs() < 1e-3);
        // Next frame the capture is 2560 texels wide: stays put.
        assert!((native_render_k(k, 2560, 1920.0 * k) - k).abs() < 1e-6);
        assert_eq!((1920.0 * k).round(), 2560.0, "a whole 2560 texels");
        // 4K fits; 8K is capped.
        assert!((native_render_k(1.0, 3840, 1920.0) - 2.0).abs() < 1e-3);
        assert_eq!(native_render_k(1.0, 7680, 1920.0), RENDER_K.1);
        // Tall window, height-limited: drawn 1000 texels wide for a
        // 1200-px window → 1.2x.
        assert!((native_render_k(1.0, 1200, 1000.0) - 1.2).abs() < 1e-3);
        // Small windows keep the base; nothing drawn yet → unchanged.
        assert_eq!(native_render_k(1.0, 800, 1500.0), 1.0);
        assert_eq!(native_render_k(1.3, 2560, 0.0), 1.3);
    }

    #[test]
    fn resolution_changes_wait_until_settled_and_released() {
        let mut s = ResSettle::default();
        assert_eq!(s.step(1.0, 1.0, 0.0, false), None, "nothing to do");
        assert_eq!(s.step(1.0, 1.41, 0.0, false), None, "just changed");
        assert_eq!(s.step(1.0, 1.41, 0.5, false), None, "not settled");
        assert_eq!(s.step(1.0, 1.41, 1.0, true), None, "held: wait");
        assert_eq!(s.step(1.0, 1.41, 1.1, false), Some(1.41));
        // Wobbling between two values restarts the clock.
        let mut s = ResSettle::default();
        s.step(1.0, 1.19, 0.0, false);
        s.step(1.0, 1.41, 0.5, false);
        assert_eq!(s.step(1.0, 1.41, 1.0, false), None);
        assert_eq!(s.step(1.0, 1.41, 1.3, false), Some(1.41));
        // Back to the current value cancels.
        let mut s = ResSettle::default();
        s.step(1.0, 1.41, 0.0, false);
        assert_eq!(s.step(1.0, 1.0, 0.5, false), None);
        assert_eq!(s.step(1.0, 1.41, 1.0, false), None, "clock restarted");
    }

    #[test]
    fn readable_size_for_a_window_and_a_big_monitor() {
        // A 1200-px Discord window at 1 m: ~1° per 16 px → ~58°.
        let (w, capped) = readable_width(1200, 1.0, 1.0, PPR);
        assert!(!capped);
        let angle = angular_width(w, 1.0).to_degrees();
        assert!((50.0..65.0).contains(&angle), "{angle}°");
        // Content is 80% of the panel width → panel 1/0.8 wider.
        let (w2, _) = readable_width(1200, 0.8, 1.0, PPR);
        assert!((w2 - w / 0.8).abs() < 1e-4);
        // The tester's 2560-px monitor would need ~123°: capped at 90°.
        let (w3, capped) = readable_width(2560, 1.0, 1.6, PPR);
        assert!(capped);
        assert!((angular_width(w3, 1.6).to_degrees() - 90.0).abs() < 0.5);
        // Readable means ≤ READ_RATIO source px per headset px.
        let px_on_screen = angular_width(w, 1.0) * PPR;
        assert!((1200.0 / px_on_screen - READ_RATIO).abs() < 0.01);
    }

    /// Tester: the keyboard cursor sat off from where the controller
    /// pointed. The cursor is on the beam itself now, at the hit.
    #[test]
    fn cursor_sits_on_the_beam_tip_facing_you() {
        let (o, d, eye) = (Vec3A::new(0.1, 1.0, -0.2), Vec3A::new(0.0, -0.6, -0.8), Vec3A::new(0.0, 1.6, 0.0));
        let c = cursor_pose(o, d, 0.45, eye);
        let p = c.translation;
        // On the ray, just short of the hit.
        let along = (p - o).dot(d.normalize());
        assert!((along - (0.45 - CURSOR_LIFT_M)).abs() < 1e-5);
        assert!(((p - o) - d.normalize() * along).length() < 1e-5, "off the beam");
        // Faces the eye; upright.
        assert!((c.matrix3.z_axis - (eye - p).normalize()).length() < 1e-5);
        assert!(c.matrix3.x_axis.y.abs() < 1e-5);
        // Same hit through a simulated keyboard (45 cm, tilted 40°):
        // the cursor is where SimOverlay says the ray lands.
        let kb = SimOverlay { pose: Affine3A::from_translation(glam::Vec3::new(0.1, 0.64, -0.68)) * Affine3A::from_rotation_x(-40f32.to_radians()), width: 0.75, aspect: 0.39 };
        let (_, dist) = kb.intersect(o, d.normalize()).expect("ray hits the keyboard");
        let hit = o + d.normalize() * dist;
        assert!((cursor_pose(o, d, dist, eye).translation - hit).length() <= CURSOR_LIFT_M + 1e-5);
    }

    /// Angle (degrees) between two directions.
    fn deg(a: Vec3A, b: Vec3A) -> f32 {
        a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Aim straight ahead plus a 10 Hz, ±0.6° tremor (a shaky hand).
    fn tremor(t: f32) -> Vec3A {
        let a = 0.6f32.to_radians() * (std::f32::consts::TAU * 10.0 * t).sin();
        Vec3A::new(a.sin(), 0.0, -a.cos())
    }

    #[test]
    fn steady_pointer_off_is_the_raw_ray() {
        let mut f = RayStabilizer::default();
        for i in 0..50 {
            let d = tremor(i as f32 / 90.0);
            let (_, out) = f.step(Vec3A::ZERO, d, 0.0, 1.0 / 90.0);
            assert!(deg(out, d) < 1e-3);
        }
    }

    /// Tester: shaky hands. Strong steadying cuts a 10 Hz tremor to a
    /// fraction; the default still takes most of it out.
    #[test]
    fn steady_pointer_takes_out_tremor() {
        let wobble = |strength: f32| {
            let mut f = RayStabilizer::default();
            let mut worst = 0f32;
            for i in 0..(90 * 3) {
                let t = i as f32 / 90.0;
                let (_, out) = f.step(Vec3A::ZERO, tremor(t), strength, 1.0 / 90.0);
                if t > 1.0 {
                    worst = worst.max(deg(out, -Vec3A::Z));
                }
            }
            worst
        };
        let raw = 0.6;
        assert!(wobble(1.0) < raw * 0.15, "strong: {}°", wobble(1.0));
        assert!(wobble(0.5) < raw * 0.5, "default: {}°", wobble(0.5));
    }

    /// …without making deliberate moves feel laggy: a 60°/s sweep stays
    /// within a few degrees, and it settles on the target once you stop.
    #[test]
    fn steady_pointer_keeps_up_with_deliberate_moves() {
        let mut f = RayStabilizer::default();
        let aim = |deg: f32| Vec3A::new(deg.to_radians().sin(), 0.0, -deg.to_radians().cos());
        let mut out = -Vec3A::Z;
        for i in 0..90 {
            let target = aim(60.0 * i as f32 / 90.0);
            out = f.step(Vec3A::ZERO, target, 1.0, 1.0 / 90.0).1;
            if i > 30 {
                assert!(deg(out, target) < 4.0, "lag {}° at frame {i}", deg(out, target));
            }
        }
        for _ in 0..90 {
            out = f.step(Vec3A::ZERO, aim(60.0), 1.0, 1.0 / 90.0).1;
        }
        assert!(deg(out, aim(60.0)) < 0.3, "settled {}°", deg(out, aim(60.0)));
    }

    /// Tester: with the laser resting on a window, the PC mouse couldn't
    /// be moved — the laser kept pulling it back.
    #[test]
    fn the_real_mouse_wins_over_a_resting_laser() {
        let mut m = MouseYield::default();
        // Laser drives the mouse; its own moves don't count as yours.
        m.observe(Some((100, 100)), 0.0);
        assert!(m.may_hover(0.0));
        m.moved((120, 100));
        m.observe(Some((120, 100)), 0.016);
        assert!(m.may_hover(0.016), "our own move isn't the user's");
        // You move the PC mouse: the laser yields.
        m.observe(Some((400, 300)), 0.032);
        assert!(!m.may_hover(0.5));
        assert!(!m.may_hover(0.032 + MOUSE_YIELD_S - 0.01));
        // Still moving it: the yield extends.
        m.observe(Some((420, 310)), 1.5);
        assert!(!m.may_hover(1.5 + MOUSE_YIELD_S - 0.01));
        // Hands off the mouse long enough: the laser may hover again.
        assert!(m.may_hover(1.5 + MOUSE_YIELD_S));
        // Hidden cursor (unknown): no false alarm.
        m.observe(None, 5.0);
        assert!(m.may_hover(5.0));
    }

    #[test]
    fn tremor_inside_the_deadband_does_not_move_the_mouse() {
        assert!(beyond_deadband(None, (10, 10)));
        assert!(!beyond_deadband(Some((10, 10)), (12, 8)), "a couple of px of shake");
        assert!(beyond_deadband(Some((10, 10)), (10, 10 + MOUSE_DEADBAND_PX)));
    }

    /// Full swap: our ray starts SteamVR's laser; SteamVR's events then
    /// drive the panels until it reports leaving.
    #[test]
    fn steamvr_laser_takes_over_and_hands_back() {
        let mut l = SteamLaser::default();
        assert!(!l.step(None, &[], 0.0).engaged);
        // Our ray reaches the keyboard (2): engage, flag on it.
        let f = l.step(Some(2), &[], 1.0);
        assert!(f.engaged);
        assert_eq!(f.flag_panel, Some(2));
        // SteamVR: enter, move, a click inside one frame.
        let f = l.step(Some(2), &[(2, 1, LaserEv::Enter), (2, 1, LaserEv::Move(500.0, 200.0))], 1.02);
        assert_eq!((f.panel, f.pos, f.down), (Some(2), Some((500.0, 200.0)), None));
        let f = l.step(None, &[(2, 1, LaserEv::Down(510.0, 205.0, 1)), (2, 1, LaserEv::Up)], 1.04);
        assert_eq!(f.down, Some(1), "a fast click still presses");
        assert_eq!(l.step(None, &[], 1.06).down, None, "...and releases next frame");
        // Right click held, then released.
        l.step(None, &[(2, 1, LaserEv::Down(10.0, 10.0, 2))], 1.1);
        assert_eq!(l.step(None, &[], 1.2).down, Some(2));
        assert_eq!(l.step(None, &[(2, 1, LaserEv::Up)], 1.3).down, None);
        // Laser crosses to the dashboard (1): leave + enter, either order.
        let f = l.step(None, &[(1, 1, LaserEv::Enter), (2, 1, LaserEv::Leave)], 1.4);
        assert_eq!(f.panel, Some(1));
        // Scroll deltas come through.
        assert_eq!(l.step(None, &[(1, 1, LaserEv::Scroll(0.0, -0.5))], 1.5).scroll, (0.0, -0.5));
        // Leaves every panel: grace, then handed back.
        let f = l.step(None, &[(1, 1, LaserEv::Leave)], 2.0);
        assert!(f.engaged && f.panel.is_none(), "grace");
        assert!(!l.step(None, &[], 2.0 + STEAM_GRACE_S + 0.01).engaged);
    }

    /// Tester: "glitching between our cursor & SteamVR's". Engaged,
    /// SteamVR takes the controllers, our ray goes blind (None) while
    /// SteamVR's laser is still on the keyboard: that must not end it.
    #[test]
    fn our_ray_going_blind_does_not_end_steamvrs_laser() {
        let mut l = SteamLaser::default();
        l.step(Some(2), &[], 0.0);
        l.step(None, &[(2, 1, LaserEv::Enter)], 0.05);
        for i in 1..200 {
            let f = l.step(None, &[], 0.05 + i as f64 * 0.016);
            assert!(f.engaged && f.panel == Some(2), "dropped at frame {i}");
        }
    }

    /// SteamVR said "left" while our ray still sees a panel (edges where
    /// the two rays disagree): don't bounce straight back in.
    #[test]
    fn no_bouncing_at_panel_edges() {
        let mut l = SteamLaser::default();
        l.step(Some(3), &[], 0.0);
        l.step(Some(3), &[(3, 1, LaserEv::Enter)], 0.1);
        l.step(Some(3), &[(3, 1, LaserEv::Leave)], 0.2);
        assert!(!l.step(Some(3), &[], 0.2 + STEAM_GRACE_S + 0.01).engaged);
        for i in 0..20 {
            assert!(!l.step(Some(3), &[], 1.0 + i as f64 * 0.1).engaged, "bounced back in");
        }
        // Our ray leaves, then comes back: a fresh start is fine.
        l.step(None, &[], 4.0);
        assert!(l.step(Some(3), &[], 4.1).engaged);
    }

    /// If SteamVR's laser never answers, our laser takes over.
    #[test]
    fn falls_back_to_our_laser_if_steamvr_is_silent() {
        let mut l = SteamLaser::default();
        assert!(l.step(Some(2), &[], 0.0).engaged);
        assert!(l.step(Some(2), &[], STEAM_FALLBACK_S - 0.01).engaged);
        assert!(!l.step(Some(2), &[], STEAM_FALLBACK_S + 0.01).engaged);
        assert!(!l.step(Some(2), &[], 3.0).engaged, "stays on ours while still pointing");
    }

    /// A compact-wrist-like panel: three rows with gaps between them.
    const WRIST_PT: (f32, f32) = (600.0, 520.0);
    fn wrist_rows() -> Vec<[f32; 4]> {
        vec![[4.0, 4.0, 592.0, 80.0], [40.0, 92.0, 520.0, 220.0], [60.0, 320.0, 480.0, 56.4]]
    }

    #[test]
    fn mask_comes_from_what_the_panel_drew() {
        let m = mask_rects(&wrist_rows(), WRIST_PT).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[2], [60.0, 320.0, 480.0, 57.0], "rounded outward");
        // A card covering the whole panel: no mask needed.
        assert_eq!(mask_rects(&[[0.0, 0.0, 600.0, 520.0]], WRIST_PT), None);
        // Nothing drawn: no mask (never make a panel unclickable).
        assert_eq!(mask_rects(&[], WRIST_PT), None);
        // Slivers and duplicates dropped; off-panel parts clipped.
        let m = mask_rects(&[[0.0, 0.0, 2.0, 50.0], [10.0, 10.0, 50.0, 50.0], [10.0, 10.0, 50.0, 50.0], [580.0, 500.0, 100.0, 100.0]], WRIST_PT).unwrap();
        assert_eq!(m, vec![[10.0, 10.0, 50.0, 50.0], [580.0, 500.0, 20.0, 20.0]]);
        // Too many for SteamVR: one box around them all.
        let many: Vec<[f32; 4]> = (0..40).map(|i| [i as f32 * 10.0, 100.0, 8.0, 8.0]).collect();
        assert_eq!(mask_rects(&many, WRIST_PT).unwrap(), vec![[0.0, 100.0, 398.0, 8.0]]);
    }

    /// A stand-in for SteamVR's masked intersection with a given (hidden)
    /// y convention: the check must find it either way, and switch masks
    /// off if SteamVR ignores them.
    #[test]
    fn mask_convention_is_checked_against_steamvr() {
        let rects = mask_rects(&wrist_rows(), WRIST_PT).unwrap();
        let probe = mask_probe(&rects, WRIST_PT).expect("the wrist has an asymmetric gap");
        for real_top_down in [true, false] {
            let current = std::cell::Cell::new(true);
            let origin = calibrate_mask(
                |top| current.set(top),
                |p| {
                    // SteamVR hits where the mask (as it reads it) covers p.
                    let y = if current.get() == real_top_down { p.1 } else { WRIST_PT.1 - p.1 };
                    rects.iter().any(|r| in_rect(r, (p.0, y), 0.0))
                },
                probe,
            );
            assert_eq!(origin, if real_top_down { MaskOrigin::TopDown } else { MaskOrigin::BottomUp });
        }
        // SteamVR ignoring masks (everything hits): off, safely.
        assert_eq!(calibrate_mask(|_| {}, |_| true, probe), MaskOrigin::Off);
        // A symmetric layout gives no usable probe: wait for another panel.
        assert_eq!(mask_probe(&[[0.0, 100.0, 600.0, 320.0]], WRIST_PT), None);
    }

    #[test]
    fn frames_are_paced_to_the_headset() {
        let p = 1.0 / 90.0;
        // 3 ms of work: start at period - 6 ms after vsync.
        let start = p - 0.003 - PACE_MARGIN_S;
        let w = pace_sleep(0.001, p, 0.003);
        assert!((0.001 + w - start).abs() < 1e-9, "wake at the start phase");
        // Already past the start phase: wait for the next one.
        let w = pace_sleep(start + 0.001, p, 0.003);
        assert!((w - (p - 0.001)).abs() < 1e-9);
        // Over a run of frames the update rate matches the headset.
        let (mut t, mut frames) = (0.0f64, 0);
        while t < 1.0 {
            t += 0.003; // work
            t += pace_sleep(t % p, p, 0.003);
            frames += 1;
        }
        assert!((88..=91).contains(&frames), "{frames} updates in 1 s at 90 Hz");
        // Work longer than a frame: never a zero / negative sleep.
        assert!(pace_sleep(0.0, p, 0.02) > 0.0);
    }
}
