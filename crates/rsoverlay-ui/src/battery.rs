//! Tracked-device battery widget + low-battery alerts.
//!
//! The UI crate stays OpenVR-agnostic: the app polls `IVRSystem` and
//! hands us plain [`BatteryReading`]s. Everything here is pure so the
//! alert hysteresis is unit-testable without a headset.

use crate::icons::{self, icon};
use crate::theme;
use rsoverlay_notify::Notification;

#[derive(Clone, Debug, PartialEq)]
pub struct BatteryReading {
    /// Stable per-session key (the OpenVR device index).
    pub id: u32,
    /// "HMD", "Left hand", "Tracker · VIVE Tracker 3.0", …
    pub label: String,
    /// 0.0..=1.0
    pub fraction: f32,
    pub charging: bool,
}

/// Re-arm margin above the threshold. Without it a device hovering at
/// the threshold (19 % ↔ 20 % jitter) would re-alert every poll.
pub const REARM_MARGIN_PCT: u8 = 5;

/// Fires one toast per device when it drops below `warn_pct` while
/// discharging. Re-arms once the device charges or climbs back above
/// `warn_pct + REARM_MARGIN_PCT`.
#[derive(Default)]
pub struct LowBatteryWatch {
    /// Bitset of device indices already alerted. OpenVR caps tracked
    /// devices at 64, so one u64 covers every index.
    alerted: u64,
}

impl LowBatteryWatch {
    pub fn update(&mut self, readings: &[BatteryReading], warn_pct: u8) -> Vec<Notification> {
        let warn = f32::from(warn_pct) / 100.0;
        let rearm = f32::from(warn_pct.saturating_add(REARM_MARGIN_PCT)) / 100.0;
        let mut out = Vec::new();
        for r in readings {
            let Some(bit) = 1u64.checked_shl(r.id) else { continue };
            let armed = self.alerted & bit == 0;
            if r.charging || r.fraction >= rearm {
                self.alerted &= !bit;
            } else if armed && r.fraction < warn {
                self.alerted |= bit;
                out.push(Notification {
                    title: format!("{} battery low", r.label),
                    content: format!("{}% remaining", pct(r.fraction)),
                    timeout: 6.0,
                    ..Default::default()
                });
            }
        }
        out
    }
}

fn pct(fraction: f32) -> u32 {
    (fraction * 100.0).round() as u32
}

/// Dashboard card. `warn_pct` is editable inline.
pub fn card(
    ui: &mut egui::Ui,
    readings: &[BatteryReading],
    warn_pct: &mut u8,
    accent: egui::Color32,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(icon(icons::BATTERY, 18.0, accent));
            ui.add_space(6.0);
            ui.label(theme::caption("Battery"));
        });
        ui.add_space(6.0);
        if readings.is_empty() {
            ui.label(theme::body_muted("no battery-reporting devices"));
        }
        for r in readings {
            row(ui, r, *warn_pct, accent);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("warn below"));
            ui.add(egui::Slider::new(warn_pct, 5..=50).suffix("%"));
        });
    });
}

fn row(ui: &mut egui::Ui, r: &BatteryReading, warn_pct: u8, accent: egui::Color32) {
    let low = !r.charging && pct(r.fraction) < u32::from(warn_pct);
    let fill = if low { theme::DANGER } else { accent };
    ui.horizontal(|ui| {
        ui.label(theme::body_muted(&r.label));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = if r.charging {
                format!("⚡{}%", pct(r.fraction))
            } else {
                format!("{}%", pct(r.fraction))
            };
            ui.monospace(egui::RichText::new(text).color(fill).size(theme::SIZE_BODY));
        });
    });
    // Thin level bar under the label; painter-drawn so it stays 3px on
    // every pixels-per-point.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 1.5, egui::Color32::from_white_alpha(0x18));
    let mut filled = rect;
    filled.set_width(rect.width() * r.fraction.clamp(0.0, 1.0));
    p.rect_filled(filled, 1.5, fill);
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: u32, fraction: f32, charging: bool) -> BatteryReading {
        BatteryReading { id, label: format!("dev{id}"), fraction, charging }
    }

    #[test]
    fn alerts_once_per_crossing() {
        let mut w = LowBatteryWatch::default();
        assert!(w.update(&[r(1, 0.50, false)], 20).is_empty());
        assert_eq!(w.update(&[r(1, 0.19, false)], 20).len(), 1);
        assert!(w.update(&[r(1, 0.15, false)], 20).is_empty());
    }

    #[test]
    fn hysteresis_blocks_jitter() {
        let mut w = LowBatteryWatch::default();
        w.update(&[r(1, 0.19, false)], 20);
        // Climbs to 21 % (below 20 + 5) and drops again: no re-alert.
        assert!(w.update(&[r(1, 0.21, false)], 20).is_empty());
        assert!(w.update(&[r(1, 0.19, false)], 20).is_empty());
        // Clears the re-arm margin, then drops: alerts again.
        w.update(&[r(1, 0.25, false)], 20);
        assert_eq!(w.update(&[r(1, 0.18, false)], 20).len(), 1);
    }

    #[test]
    fn charging_rearms_and_suppresses() {
        let mut w = LowBatteryWatch::default();
        w.update(&[r(2, 0.10, false)], 20);
        assert!(w.update(&[r(2, 0.10, true)], 20).is_empty());
        assert_eq!(w.update(&[r(2, 0.10, false)], 20).len(), 1);
    }

    #[test]
    fn devices_tracked_independently() {
        let mut w = LowBatteryWatch::default();
        let n = w.update(&[r(0, 0.1, false), r(63, 0.1, false), r(64, 0.1, false)], 20);
        // Index 64 is out of OpenVR's range and ignored rather than wrapping.
        assert_eq!(n.len(), 2);
    }
}
