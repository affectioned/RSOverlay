//! Night light — warm the whole overlay by cutting blue (and some
//! green), on demand or on a schedule.
//!
//! The result is an RGB *multiplier*, applied where it costs nothing:
//! `IVROverlay::SetOverlayColor` in SteamVR, a uniform in the preview
//! shaders. Multiplying (rather than painting an orange veil on top)
//! keeps blacks black and whites readable — same idea as Windows' own
//! night light.

use crate::theme;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum NightMode {
    #[default]
    Off,
    On,
    Scheduled,
}

impl NightMode {
    pub const ALL: [NightMode; 3] = [NightMode::Off, NightMode::On, NightMode::Scheduled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
            Self::Scheduled => "Scheduled",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Scheduled => "scheduled",
        }
    }

    pub fn from_slug(s: &str) -> Self {
        Self::ALL.into_iter().find(|m| m.slug() == s).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NightLight {
    pub mode: NightMode,
    /// 0.0 (neutral) ..= 1.0 (≈ 2700 K candle-warm).
    pub strength: f32,
    pub start_hour: u8,
    pub end_hour: u8,
    /// Smoothed multiplier actually applied — eases over ~1 s when the
    /// schedule flips so the room doesn't snap orange.
    current: [f32; 3],
}

impl Default for NightLight {
    fn default() -> Self {
        Self { mode: NightMode::Off, strength: 0.5, start_hour: 21, end_hour: 7, current: [1.0; 3] }
    }
}

/// Seconds for the eased schedule flip (time constant).
const FADE_TAU: f32 = 0.35;

impl NightLight {
    pub fn active_at(&self, minute_of_day: u16) -> bool {
        match self.mode {
            NightMode::Off => false,
            NightMode::On => true,
            NightMode::Scheduled => in_window(
                u16::from(self.start_hour) * 60,
                u16::from(self.end_hour) * 60,
                minute_of_day,
            ),
        }
    }

    /// Advance the smoothed tint by `dt` seconds toward the target for
    /// `minute_of_day`; returns the multiplier to apply this frame.
    pub fn tick(&mut self, minute_of_day: u16, dt: f32) -> [f32; 3] {
        let target = if self.active_at(minute_of_day) { multiplier(self.strength) } else { [1.0; 3] };
        // Frame-rate independent exponential approach.
        let k = if crate::motion::reduced() { 1.0 } else { 1.0 - (-dt.max(0.0) / FADE_TAU).exp() };
        for (c, t) in self.current.iter_mut().zip(target) {
            *c += (t - *c) * k;
        }
        self.current
    }

    /// Settings card. Returns nothing; edits `self` in place.
    pub fn card(&mut self, ui: &mut egui::Ui, minute_of_day: u16, accent: egui::Color32) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(crate::icons::icon(crate::icons::MOON, 18.0, accent));
                ui.add_space(6.0);
                ui.label(theme::caption("Night light"));
                crate::widgets::popout_button(ui, "Night light", accent);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (txt, col) = if self.active_at(minute_of_day) { ("active", accent) } else { ("idle", theme::FG_MUTED) };
                    ui.label(egui::RichText::new(txt).size(theme::SIZE_CAPTION).color(col));
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for m in NightMode::ALL {
                    let sel = self.mode == m;
                    let text = egui::RichText::new(m.label()).color(if sel { theme::BG_ROOT } else { theme::FG_MUTED });
                    if ui.selectable_label(sel, text).clicked() {
                        self.mode = m;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Warmth"));
                ui.add(egui::Slider::new(&mut self.strength, 0.0..=1.0).show_value(false));
            });
            if self.mode == NightMode::Scheduled {
                ui.horizontal(|ui| {
                    ui.label(theme::body_muted("From"));
                    ui.add(egui::Slider::new(&mut self.start_hour, 0..=23).suffix(":00"));
                });
                ui.horizontal(|ui| {
                    ui.label(theme::body_muted("Until"));
                    ui.add(egui::Slider::new(&mut self.end_hour, 0..=23).suffix(":00"));
                });
            }
        });
    }
}

/// Is `m` inside `[start, end)` on a 24 h clock, wrapping past midnight
/// (21:00 → 07:00)? `start == end` means "never" rather than "always",
/// so a half-configured schedule can't silently tint all day.
pub fn in_window(start: u16, end: u16, m: u16) -> bool {
    match start.cmp(&end) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => (start..end).contains(&m),
        std::cmp::Ordering::Greater => m >= start || m < end,
    }
}

/// Neutral → warm RGB multiplier. Blue drops fastest, green partly, red
/// untouched; at 1.0 this lands near a 2700 K white point.
pub fn multiplier(strength: f32) -> [f32; 3] {
    let s = strength.clamp(0.0, 1.0);
    [1.0, 1.0 - 0.22 * s, 1.0 - 0.58 * s]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_wrap_midnight() {
        let (s, e) = (21 * 60, 7 * 60);
        assert!(in_window(s, e, 23 * 60));
        assert!(in_window(s, e, 0));
        assert!(in_window(s, e, 6 * 60 + 59));
        assert!(!in_window(s, e, 7 * 60));
        assert!(!in_window(s, e, 12 * 60));
    }

    #[test]
    fn same_day_window_and_degenerate() {
        assert!(in_window(9 * 60, 17 * 60, 12 * 60));
        assert!(!in_window(9 * 60, 17 * 60, 17 * 60));
        assert!(!in_window(600, 600, 600));
    }

    #[test]
    fn multiplier_keeps_red_cuts_blue_most() {
        let [r, g, b] = multiplier(1.0);
        assert_eq!(r, 1.0);
        assert!(b < g && g < 1.0);
        assert_eq!(multiplier(0.0), [1.0; 3]);
    }

    #[test]
    fn tick_converges_without_overshoot() {
        let mut n = NightLight { mode: NightMode::On, strength: 1.0, ..Default::default() };
        let target = multiplier(1.0);
        let mut last = 1.0;
        for _ in 0..200 {
            let [_, _, b] = n.tick(0, 1.0 / 60.0);
            assert!(b <= last && b >= target[2] - 1e-6);
            last = b;
        }
        assert!((last - target[2]).abs() < 1e-3);
    }
}
