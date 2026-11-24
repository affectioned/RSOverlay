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

/// Playtime estimate from each device's recent discharge (or charge)
/// rate: least-squares slope over the last [`EST_WINDOW_S`]. Battery %
/// moves in 1 % steps, so it waits for [`EST_MIN_SPAN_S`] of history and
/// a [`EST_MIN_DELTA`] change before claiming anything; plugging in or
/// unplugging restarts the history.
#[derive(Default)]
pub struct BatteryEstimator {
    hist: std::collections::HashMap<u32, (bool, std::collections::VecDeque<(f64, f32)>)>,
}

pub const EST_WINDOW_S: f64 = 15.0 * 60.0;
pub const EST_MIN_SPAN_S: f64 = 120.0;
pub const EST_MIN_DELTA: f32 = 0.02;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Estimate {
    /// Seconds until empty (discharging) or full (charging).
    pub secs: f64,
    pub charging: bool,
}

impl BatteryEstimator {
    pub fn update(&mut self, now_s: f64, readings: &[BatteryReading]) {
        for r in readings {
            let (charging, q) = self.hist.entry(r.id).or_insert_with(|| (r.charging, Default::default()));
            if *charging != r.charging {
                *charging = r.charging;
                q.clear();
            }
            q.push_back((now_s, r.fraction));
            while q.front().is_some_and(|(t, _)| now_s - t > EST_WINDOW_S) {
                q.pop_front();
            }
        }
        self.hist.retain(|id, _| readings.iter().any(|r| r.id == *id));
    }

    pub fn estimate(&self, r: &BatteryReading) -> Option<Estimate> {
        let (charging, q) = self.hist.get(&r.id)?;
        let (t0, f0) = *q.front()?;
        let (t1, _) = *q.back()?;
        let span = t1 - t0;
        let lo = q.iter().map(|(_, f)| *f).fold(f32::MAX, f32::min);
        let hi = q.iter().map(|(_, f)| *f).fold(f32::MIN, f32::max);
        if span < EST_MIN_SPAN_S || hi - lo < EST_MIN_DELTA {
            return None;
        }
        // Least-squares slope (fraction per second).
        let n = q.len() as f64;
        let mt = q.iter().map(|(t, _)| t - t0).sum::<f64>() / n;
        let mf = q.iter().map(|(_, f)| f64::from(*f - f0)).sum::<f64>() / n;
        let (mut num, mut den) = (0.0, 0.0);
        for (t, f) in q {
            let dt = t - t0 - mt;
            num += dt * (f64::from(*f - f0) - mf);
            den += dt * dt;
        }
        if den <= 0.0 {
            return None;
        }
        let slope = num / den;
        let secs = if *charging {
            if slope <= 0.0 {
                return None;
            }
            f64::from(1.0 - r.fraction) / slope
        } else {
            if slope >= 0.0 {
                return None;
            }
            f64::from(r.fraction) / -slope
        };
        (secs.is_finite() && secs < 48.0 * 3600.0).then_some(Estimate { secs, charging: *charging })
    }
}

/// "2h 05m" / "45m" / "<1m".
pub fn format_duration(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    match m {
        0 => "<1m".into(),
        m if m < 60 => format!("{m}m"),
        m => format!("{}h {:02}m", m / 60, m % 60),
    }
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
    estimates: &BatteryEstimator,
    warn_pct: &mut u8,
    osc: &mut BatteryOsc,
    accent: egui::Color32,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(icon(icons::BATTERY, 18.0, accent));
            ui.add_space(6.0);
            ui.label(theme::caption("Battery"));
            crate::widgets::popout_button(ui, "Battery", accent);
        });
        ui.add_space(6.0);
        if readings.is_empty() {
            ui.label(theme::body_muted("no battery-reporting devices"));
        }
        for r in readings {
            row(ui, r, estimates.estimate(r), *warn_pct, accent);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::body_muted("warn below"));
            ui.add(egui::Slider::new(warn_pct, 5..=50).suffix("%"));
        });
        ui.checkbox(&mut osc.enabled, "send to VRChat avatar").on_hover_text(format!(
            "OSC /avatar/parameters/{p}/HMD, {p}/LeftHand, {p}/Charging/HMD ... (float 0-1 / bool)",
            p = osc.prefix
        ));
    });
}

fn row(ui: &mut egui::Ui, r: &BatteryReading, est: Option<Estimate>, warn_pct: u8, accent: egui::Color32) {
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
            if let Some(e) = est {
                let t = if e.charging {
                    format!("full in ~{}", format_duration(e.secs))
                } else {
                    format!("~{} left", format_duration(e.secs))
                };
                ui.label(egui::RichText::new(t).size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
            }
        });
    });
    // Thin level bar under the label; painter-drawn so it stays 3px on
    // every pixels-per-point.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 1.5, egui::Color32::from_white_alpha(0x18));
    // Bar glides to new readings instead of jumping every 5 s poll.
    let shown = crate::motion::value(ui.ctx(), egui::Id::new(("battery-bar", r.id)), r.fraction.clamp(0.0, 1.0), crate::motion::SLOW);
    let mut filled = rect;
    filled.set_width(rect.width() * shown);
    p.rect_filled(filled, 1.5, fill);
    ui.add_space(4.0);
}

/// Publishes battery levels as VRChat avatar parameters:
/// `<prefix>/<Device>` float 0..1 and `<prefix>/Charging/<Device>` bool.
/// Sends a device only when its level moved >= 1 % or charging flipped,
/// plus a full resend every [`OSC_HEARTBEAT`] so an avatar loaded
/// mid-session picks the values up.
pub struct BatteryOsc {
    pub enabled: bool,
    pub prefix: String,
    last: std::collections::HashMap<u32, (f32, bool)>,
    last_full: Option<std::time::Instant>,
}

pub const OSC_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);

impl Default for BatteryOsc {
    fn default() -> Self {
        Self { enabled: false, prefix: "RSO/Battery".into(), last: Default::default(), last_full: None }
    }
}

impl BatteryOsc {
    pub fn messages(
        &mut self,
        readings: &[BatteryReading],
        now: std::time::Instant,
    ) -> Vec<(String, rsoverlay_osc::OscValue)> {
        use rsoverlay_osc::OscValue;
        if !self.enabled {
            return Vec::new();
        }
        let full = self.last_full.is_none_or(|t| now.duration_since(t) >= OSC_HEARTBEAT);
        if full {
            self.last_full = Some(now);
        }
        let mut out = Vec::new();
        for r in readings {
            let changed = match self.last.get(&r.id) {
                None => true,
                Some(&(f, c)) => (f - r.fraction).abs() >= 0.01 || c != r.charging,
            };
            if !(full || changed) {
                continue;
            }
            self.last.insert(r.id, (r.fraction, r.charging));
            let dev = osc_device_name(r);
            out.push((format!("{}/{dev}", self.prefix), OscValue::Float32(r.fraction.clamp(0.0, 1.0))));
            out.push((format!("{}/Charging/{dev}", self.prefix), OscValue::Bool(r.charging)));
        }
        out
    }
}

/// Avatar-parameter-safe device name: `HMD`, `LeftHand`, `RightHand`,
/// `Tracker<index>`, else `Device<index>`.
pub fn osc_device_name(r: &BatteryReading) -> String {
    match r.label.as_str() {
        "HMD" => "HMD".into(),
        "Left hand" => "LeftHand".into(),
        "Right hand" => "RightHand".into(),
        l if l.starts_with("Tracker") => format!("Tracker{}", r.id),
        _ => format!("Device{}", r.id),
    }
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
    fn osc_sends_on_change_and_heartbeat() {
        use std::time::{Duration, Instant};
        let mut o = BatteryOsc { enabled: true, ..Default::default() };
        let t0 = Instant::now();
        let hmd = |f, c| BatteryReading { label: "HMD".into(), ..r(0, f, c) };
        let first = o.messages(&[hmd(0.80, false)], t0);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].0, "RSO/Battery/HMD");
        assert_eq!(first[1].0, "RSO/Battery/Charging/HMD");
        // < 1 % drift, before heartbeat: silent.
        assert!(o.messages(&[hmd(0.805, false)], t0 + Duration::from_secs(5)).is_empty());
        // Charging flip: resend.
        assert_eq!(o.messages(&[hmd(0.805, true)], t0 + Duration::from_secs(10)).len(), 2);
        // Heartbeat: resend even unchanged.
        assert_eq!(o.messages(&[hmd(0.805, true)], t0 + OSC_HEARTBEAT).len(), 2);
    }

    #[test]
    fn osc_names_and_disabled() {
        let mut o = BatteryOsc::default();
        assert!(o.messages(&[r(0, 0.5, false)], std::time::Instant::now()).is_empty());
        let mut t = r(5, 0.5, false);
        t.label = "Tracker x".into();
        assert_eq!(osc_device_name(&t), "Tracker5");
        t.label = "Left hand".into();
        assert_eq!(osc_device_name(&t), "LeftHand");
    }

    fn rd(id: u32, f: f32, charging: bool) -> BatteryReading {
        BatteryReading { id, label: "HMD".into(), fraction: f, charging }
    }

    #[test]
    fn estimate_needs_history_then_tracks_drain() {
        let mut e = BatteryEstimator::default();
        // 1 % per minute, sampled every 5 s.
        let mut last = rd(0, 1.0, false);
        for i in 0..=120 {
            let t = i as f64 * 5.0;
            last = rd(0, 1.0 - (t / 60.0) as f32 * 0.01, false);
            e.update(t, std::slice::from_ref(&last));
            if t < EST_MIN_SPAN_S {
                assert!(e.estimate(&last).is_none(), "too early at {t}s");
            }
        }
        // At 90 %, 1 %/min -> ~90 minutes left.
        let est = e.estimate(&last).expect("estimate after 10 min");
        assert!(!est.charging);
        assert!((est.secs / 60.0 - 90.0).abs() < 2.0, "{}", est.secs / 60.0);
    }

    #[test]
    fn plugging_in_resets_and_reports_time_to_full() {
        let mut e = BatteryEstimator::default();
        for i in 0..60 {
            e.update(i as f64 * 5.0, &[rd(0, 0.5 - i as f32 * 0.001, false)]);
        }
        e.update(300.0, &[rd(0, 0.44, true)]);
        assert!(e.estimate(&rd(0, 0.44, true)).is_none(), "history restarted");
        let mut r = rd(0, 0.44, true);
        for i in 1..=60 {
            r = rd(0, 0.44 + i as f32 * 0.002, true); // +2.4 %/min
            e.update(300.0 + i as f64 * 5.0, std::slice::from_ref(&r));
        }
        let est = e.estimate(&r).unwrap();
        assert!(est.charging);
        assert!((est.secs / 60.0 - (1.0 - 0.56) / 0.024).abs() < 1.0);
    }

    #[test]
    fn duration_format() {
        assert_eq!(format_duration(20.0), "<1m");
        assert_eq!(format_duration(45.0 * 60.0), "45m");
        assert_eq!(format_duration(125.0 * 60.0), "2h 05m");
    }

    #[test]
    fn devices_tracked_independently() {
        let mut w = LowBatteryWatch::default();
        let n = w.update(&[r(0, 0.1, false), r(63, 0.1, false), r(64, 0.1, false)], 20);
        // Index 64 is out of OpenVR's range and ignored rather than wrapping.
        assert_eq!(n.len(), 2);
    }
}
