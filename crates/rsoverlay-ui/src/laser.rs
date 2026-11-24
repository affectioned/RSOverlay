//! Laser-pointer → egui input. Pure state machine, no OpenVR types, so
//! the interaction model is unit-tested without a headset.
//!
//! Per frame the app feeds one [`HandSample`] per tracked controller
//! (hit UV on our overlay, if any; trigger pull; stick). Out come egui
//! events, an optional haptic tick and the cursor to draw.
//!
//! Rules:
//! - **Active hand**: the hand that last pulled the trigger while on the
//!   overlay owns the pointer; if it drifts off, the other hand takes
//!   over as soon as it points at the panel.
//! - **Trigger hysteresis**: press ≥ 0.75, release ≤ 0.35 — analog
//!   triggers rest at 0.0x and sag mid-pull; a single threshold
//!   chatters.
//! - **Drags survive leaving the panel**: release is always delivered
//!   (at the last on-panel position), so a slider let go off-edge
//!   doesn't stay grabbed.
//! - **Scroll**: stick Y with a 0.2 dead zone, speed in points/second.

pub const PRESS: f32 = 0.75;
pub const RELEASE: f32 = 0.35;
pub const DEAD_ZONE: f32 = 0.2;
pub const SCROLL_SPEED: f32 = 900.0;
/// Click stabilisation radius, points. egui turns a press into a drag
/// after 6 pt of movement (hard-coded `MAX_CLICK_DIST`) — about a
/// millimetre on a wrist panel, i.e. any hand tremor during a trigger
/// pull. While held, the pointer stays pinned at the press point until
/// the laser leaves this radius; then it's a genuine drag.
pub const DRAG_SLOP: f32 = 24.0;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Copy, Clone, Debug)]
pub struct HandSample {
    pub hand: Hand,
    /// Hit position on the panel, already in egui points.
    pub hit: Option<egui::Pos2>,
    pub trigger: f32,
    pub stick_y: f32,
    /// Which button a trigger press means this frame — Primary, or
    /// Secondary / Middle while an XSO-style click modifier is held.
    pub button: egui::PointerButton,
}

#[derive(Debug, Default, PartialEq)]
pub struct LaserFrame {
    pub events: Vec<egui::Event>,
    /// Hand that should get a click tick this frame.
    pub haptic: Option<Hand>,
    /// Cursor to draw: position + "pressed" for the filled state.
    pub cursor: Option<(egui::Pos2, bool)>,
}

#[derive(Default)]
pub struct LaserInput {
    active: Option<Hand>,
    pressed: bool,
    /// Per-hand trigger latch (hysteresis), indexed by `slot()`.
    latched: [bool; 2],
    last_pos: Option<egui::Pos2>,
    /// Where the current press started (stabilisation anchor).
    press_anchor: Option<egui::Pos2>,
    /// The press has escaped DRAG_SLOP — report real positions.
    dragging: bool,
    /// Button of the current press (release must match it).
    press_button: Option<egui::PointerButton>,
    /// 0 = raw, 1 = heavy. Velocity-adaptive: slow moves are steadied,
    /// fast moves pass through (XSO-style "responsive when fast").
    pub smoothing: f32,
    /// Stick-scroll multiplier (Settings › Pointer).
    pub scroll_mult: f32,
    smoothed: Option<egui::Pos2>,
}

fn slot(h: Hand) -> usize {
    match h {
        Hand::Left => 0,
        Hand::Right => 1,
    }
}

impl LaserInput {
    /// `uv_to_points` converts compositor UV (v up, OpenVR convention)
    /// to panel points.
    pub fn uv_to_points(uv: [f32; 2], panel: egui::Vec2) -> egui::Pos2 {
        egui::pos2(uv[0].clamp(0.0, 1.0) * panel.x, (1.0 - uv[1].clamp(0.0, 1.0)) * panel.y)
    }

    pub fn update(&mut self, samples: &[HandSample], dt: f32) -> LaserFrame {
        let mut out = LaserFrame::default();

        // Edge-detect each hand's trigger with hysteresis.
        let mut pressed_now: Option<Hand> = None;
        for s in samples {
            let l = &mut self.latched[slot(s.hand)];
            let was = *l;
            *l = if was { s.trigger > RELEASE } else { s.trigger >= PRESS };
            if *l && !was && s.hit.is_some() {
                pressed_now = Some(s.hand);
            }
        }

        // Choose the pointer owner. A press on the panel always claims
        // it; otherwise keep the current owner while it's on the panel,
        // or hand over to any hand that is.
        let on_panel = |h: Hand| samples.iter().any(|s| s.hand == h && s.hit.is_some());
        if let Some(h) = pressed_now {
            if self.active != Some(h) && self.pressed {
                self.release(&mut out);
            }
            self.active = Some(h);
        } else if !self.pressed && !self.active.is_some_and(on_panel) {
            self.active = samples.iter().find(|s| s.hit.is_some()).map(|s| s.hand);
        }

        let sample = self.active.and_then(|h| samples.iter().find(|s| s.hand == h));
        let hit = sample.and_then(|s| s.hit).map(|p| {
            let s = self.smoothing.clamp(0.0, 1.0);
            let prev = self.smoothed.unwrap_or(p);
            // Follow factor: 1 at speed (≥ 30 pt/frame), down to 1 − 0.9·s
            // when nearly still.
            let speed = (p - prev).length();
            let k = (1.0 - 0.9 * s) + 0.9 * s * (speed / 30.0).min(1.0);
            let q = prev + (p - prev) * k;
            self.smoothed = Some(q);
            q
        });
        if hit.is_none() {
            self.smoothed = None;
        }

        // Stabilise: while held and within the slop, report the anchor.
        let hit = hit.map(|pos| match self.press_anchor {
            Some(a) if self.pressed && !self.dragging => {
                if pos.distance(a) > DRAG_SLOP {
                    self.dragging = true;
                    pos
                } else {
                    a
                }
            }
            _ => pos,
        });

        match hit {
            Some(pos) => {
                out.events.push(egui::Event::PointerMoved(pos));
                self.last_pos = Some(pos);
            }
            None if !self.pressed && self.last_pos.take().is_some() => {
                out.events.push(egui::Event::PointerGone);
            }
            None => {}
        }

        if let Some(s) = sample {
            let latched = self.latched[slot(s.hand)];
            // Press only on a fresh edge *on the panel*: a trigger already
            // held elsewhere and swept onto the panel must not click.
            if pressed_now == Some(s.hand) && !self.pressed && hit.is_some() {
                self.pressed = true;
                self.press_anchor = self.last_pos;
                self.dragging = false;
                self.press_button = Some(s.button);
                out.haptic = Some(s.hand);
                out.events.push(button(self.last_pos.unwrap_or_default(), s.button, true));
            } else if !latched && self.pressed {
                self.release(&mut out);
            }
            if hit.is_some() && s.stick_y.abs() > DEAD_ZONE {
                // Re-scale past the dead zone so scrolling starts at 0.
                let mag = (s.stick_y.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE);
                let mult = if self.scroll_mult > 0.0 { self.scroll_mult } else { 1.0 };
                out.events.push(egui::Event::Scroll(egui::vec2(0.0, s.stick_y.signum() * mag * SCROLL_SPEED * mult * dt)));
            }
        } else if self.pressed {
            // Owner lost tracking mid-drag: end the drag cleanly.
            self.release(&mut out);
        }

        out.cursor = hit.map(|p| (p, self.pressed));
        out
    }

    fn release(&mut self, out: &mut LaserFrame) {
        self.pressed = false;
        self.press_anchor = None;
        self.dragging = false;
        let b = self.press_button.take().unwrap_or(egui::PointerButton::Primary);
        out.events.push(button(self.last_pos.unwrap_or_default(), b, false));
    }
}

fn button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
}

/// Paint the laser cursor: accent ring, filled while pressed.
pub fn paint_cursor(ctx: &egui::Context, cursor: Option<(egui::Pos2, bool)>, accent: egui::Color32) {
    let Some((pos, pressed)) = cursor else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("laser-cursor")));
    let r = if pressed { 7.0 } else { 9.0 };
    painter.circle_filled(pos, r + 2.0, egui::Color32::from_black_alpha(160));
    if pressed {
        painter.circle_filled(pos, r, accent);
    } else {
        painter.circle_stroke(pos, r, egui::Stroke::new(2.0_f32, accent));
        painter.circle_filled(pos, 2.0, accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(hand: Hand, hit: Option<(f32, f32)>, trigger: f32) -> HandSample {
        HandSample { hand, hit: hit.map(|(x, y)| egui::pos2(x, y)), trigger, stick_y: 0.0, button: egui::PointerButton::Primary }
    }

    fn buttons(f: &LaserFrame) -> Vec<bool> {
        f.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton { pressed, .. } => Some(*pressed),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn uv_flips_v() {
        let p = LaserInput::uv_to_points([0.25, 1.0], egui::vec2(800.0, 400.0));
        assert_eq!(p, egui::pos2(200.0, 0.0));
    }

    #[test]
    fn click_with_hysteresis_and_haptic() {
        let mut l = LaserInput::default();
        let f = l.update(&[s(Hand::Right, Some((10.0, 10.0)), 0.0)], 0.011);
        assert_eq!(f.events, vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))]);
        // 0.6: between thresholds, not yet pressed.
        assert!(buttons(&l.update(&[s(Hand::Right, Some((10.0, 10.0)), 0.6)], 0.011)).is_empty());
        let f = l.update(&[s(Hand::Right, Some((10.0, 10.0)), 0.8)], 0.011);
        assert_eq!(buttons(&f), vec![true]);
        assert_eq!(f.haptic, Some(Hand::Right));
        // Sags to 0.5 mid-pull: still held.
        assert!(buttons(&l.update(&[s(Hand::Right, Some((12.0, 10.0)), 0.5)], 0.011)).is_empty());
        assert_eq!(buttons(&l.update(&[s(Hand::Right, Some((12.0, 10.0)), 0.2)], 0.011)), vec![false]);
    }

    #[test]
    fn drag_released_off_panel() {
        let mut l = LaserInput::default();
        l.update(&[s(Hand::Left, Some((5.0, 5.0)), 0.9)], 0.011);
        // Leaves the panel while held: no PointerGone, still pressed.
        let f = l.update(&[s(Hand::Left, None, 0.9)], 0.011);
        assert!(!f.events.contains(&egui::Event::PointerGone));
        // Let go off-panel: release delivered at the last on-panel point.
        let f = l.update(&[s(Hand::Left, None, 0.0)], 0.011);
        assert_eq!(
            f.events,
            vec![egui::Event::PointerButton {
                pos: egui::pos2(5.0, 5.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]
        );
        // Next idle frame off-panel: pointer gone.
        assert_eq!(l.update(&[s(Hand::Left, None, 0.0)], 0.011).events, vec![egui::Event::PointerGone]);
    }

    #[test]
    fn press_elsewhere_doesnt_click() {
        let mut l = LaserInput::default();
        assert!(buttons(&l.update(&[s(Hand::Right, None, 1.0)], 0.011)).is_empty());
        // Already-held trigger swept onto the panel must not click.
        assert!(buttons(&l.update(&[s(Hand::Right, Some((1.0, 1.0)), 1.0)], 0.011)).is_empty());
    }

    #[test]
    fn other_hand_takes_over_by_pressing() {
        let mut l = LaserInput::default();
        let both = |lt, rt| [s(Hand::Left, Some((1.0, 1.0)), lt), s(Hand::Right, Some((50.0, 50.0)), rt)];
        l.update(&both(0.0, 0.0), 0.011);
        let f = l.update(&both(0.0, 0.9), 0.011);
        assert_eq!(f.cursor, Some((egui::pos2(50.0, 50.0), true)));
        assert_eq!(f.haptic, Some(Hand::Right));
    }

    #[test]
    fn jitter_during_click_is_pinned() {
        let mut l = LaserInput::default();
        l.update(&[s(Hand::Right, Some((100.0, 100.0)), 0.0)], 0.011);
        l.update(&[s(Hand::Right, Some((100.0, 100.0)), 0.9)], 0.011);
        // Trigger-pull tremor: 10 pt away — still reported at the anchor.
        let f = l.update(&[s(Hand::Right, Some((108.0, 106.0)), 0.9)], 0.011);
        assert!(f.events.contains(&egui::Event::PointerMoved(egui::pos2(100.0, 100.0))));
        let f = l.update(&[s(Hand::Right, Some((108.0, 106.0)), 0.0)], 0.011);
        assert!(f.events.iter().any(|e| matches!(e,
            egui::Event::PointerButton { pos, pressed: false, .. } if *pos == egui::pos2(100.0, 100.0))));
    }

    #[test]
    fn real_drag_escapes_slop() {
        let mut l = LaserInput::default();
        l.update(&[s(Hand::Right, Some((100.0, 100.0)), 0.0)], 0.011);
        l.update(&[s(Hand::Right, Some((100.0, 100.0)), 0.9)], 0.011);
        let f = l.update(&[s(Hand::Right, Some((160.0, 100.0)), 0.9)], 0.011);
        assert!(f.events.contains(&egui::Event::PointerMoved(egui::pos2(160.0, 100.0))));
        // Once dragging, small moves pass straight through.
        let f = l.update(&[s(Hand::Right, Some((162.0, 101.0)), 0.9)], 0.011);
        assert!(f.events.contains(&egui::Event::PointerMoved(egui::pos2(162.0, 101.0))));
    }

    #[test]
    fn modifier_makes_a_right_click_and_release_matches() {
        let mut l = LaserInput::default();
        let mut smp = s(Hand::Right, Some((5.0, 5.0)), 0.0);
        l.update(&[smp], 0.011);
        smp.trigger = 0.9;
        smp.button = egui::PointerButton::Secondary;
        let f = l.update(&[smp], 0.011);
        assert!(f.events.iter().any(|e| matches!(e, egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, .. })));
        // Modifier let go before the trigger: release is still Secondary.
        smp.trigger = 0.0;
        smp.button = egui::PointerButton::Primary;
        let f = l.update(&[smp], 0.011);
        assert!(f.events.iter().any(|e| matches!(e, egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: false, .. })));
    }

    #[test]
    fn smoothing_steadies_slow_moves_but_follows_fast_ones() {
        let mut l = LaserInput { smoothing: 1.0, ..Default::default() };
        l.update(&[s(Hand::Right, Some((100.0, 100.0)), 0.0)], 0.011);
        // Tiny tremor: mostly absorbed.
        let f = l.update(&[s(Hand::Right, Some((102.0, 100.0)), 0.0)], 0.011);
        let egui::Event::PointerMoved(p) = f.events[0] else { panic!() };
        assert!(p.x < 101.0, "{p:?}");
        // Big sweep: follows (almost) fully.
        let f = l.update(&[s(Hand::Right, Some((300.0, 100.0)), 0.0)], 0.011);
        let egui::Event::PointerMoved(p) = f.events[0] else { panic!() };
        assert!(p.x > 295.0, "{p:?}");
    }

    #[test]
    fn stick_scrolls_past_dead_zone() {
        let mut l = LaserInput::default();
        let mut smp = s(Hand::Right, Some((1.0, 1.0)), 0.0);
        smp.stick_y = 0.1;
        assert!(!l.update(&[smp], 0.1).events.iter().any(|e| matches!(e, egui::Event::Scroll(_))));
        smp.stick_y = 1.0;
        let f = l.update(&[smp], 0.1);
        assert!(f.events.contains(&egui::Event::Scroll(egui::vec2(0.0, SCROLL_SPEED * 0.1))));
    }
}
