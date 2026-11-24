//! VR keyboard — behaves like a physical keyboard.
//!
//! - Keys are **scan codes** with real down/up: the OS applies the
//!   user's layout (dead keys, AltGr, IME) exactly as for hardware.
//! - Key caps are **labelled from the foreground window's layout**
//!   (`ToUnicodeEx`), refreshed as focus / modifiers change.
//! - **Both hands** type at once — each laser is its own finger; no
//!   single egui pointer.
//! - **Hold to repeat** (450 ms delay, ~28 Hz), like hardware typematic.
//! - **Modifiers**: tap = sticky for the next key, double-tap = locked
//!   until tapped again.

use crate::laser::Hand;
use crate::theme;
use rsoverlay_input::physical::{self, LabelMods, ScanKey};
use std::collections::HashMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Mod {
    Shift,
    Ctrl,
    Alt,
    Win,
    AltGr,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Kind {
    /// Printable: label comes from the live layout.
    Char,
    /// Fixed label (Esc, Tab, arrows…).
    Named(&'static str),
    Mod(Mod, &'static str),
}

#[derive(Copy, Clone, Debug)]
struct Key {
    k: ScanKey,
    w: f32,
    kind: Kind,
}

const fn ch(scan: u16) -> Key {
    Key { k: ScanKey::new(scan), w: 1.0, kind: Kind::Char }
}
const fn named(k: ScanKey, w: f32, l: &'static str) -> Key {
    Key { k, w, kind: Kind::Named(l) }
}
const fn modk(k: ScanKey, w: f32, m: Mod, l: &'static str) -> Key {
    Key { k, w, kind: Kind::Mod(m, l) }
}

const ISO_KEY: ScanKey = ScanKey::new(0x56);

/// 60 % ANSI-ish matrix, 15 units wide. The ISO extra key (0x56) is
/// spliced in next to left Shift when the active layout defines it.
fn rows(iso: bool) -> [Vec<Key>; 5] {
    let mut r3 = vec![modk(ScanKey::new(0x2A), if iso { 1.25 } else { 2.25 }, Mod::Shift, "Shift")];
    if iso {
        r3.push(Key { k: ISO_KEY, w: 1.0, kind: Kind::Char });
    }
    r3.extend((0x2C..=0x35).map(ch));
    r3.push(modk(ScanKey::new(0x36), 2.75, Mod::Shift, "Shift"));
    [
        std::iter::once(named(ScanKey::new(0x01), 1.0, "Esc"))
            .chain((0x02..=0x0D).map(ch))
            .chain(std::iter::once(named(ScanKey::new(0x0E), 2.0, "⌫")))
            .collect(),
        std::iter::once(named(ScanKey::new(0x0F), 1.5, "Tab"))
            .chain((0x10..=0x1B).map(ch))
            .chain(std::iter::once(Key { k: ScanKey::new(0x2B), w: 1.5, kind: Kind::Char }))
            .collect(),
        std::iter::once(named(ScanKey::new(0x3A), 1.75, "Caps"))
            .chain((0x1E..=0x28).map(ch))
            .chain(std::iter::once(named(ScanKey::new(0x1C), 2.25, "Enter")))
            .collect(),
        r3,
        vec![
            modk(ScanKey::new(0x1D), 1.5, Mod::Ctrl, "Ctrl"),
            modk(ScanKey::ext(0x5B), 1.25, Mod::Win, "Win"),
            modk(ScanKey::new(0x38), 1.25, Mod::Alt, "Alt"),
            named(ScanKey::new(0x39), 5.75, ""),
            modk(ScanKey::ext(0x38), 1.25, Mod::AltGr, "AltGr"),
            named(ScanKey::ext(0x4B), 1.0, "←"),
            named(ScanKey::ext(0x50), 1.0, "↓"),
            named(ScanKey::ext(0x48), 1.0, "↑"),
            named(ScanKey::ext(0x4D), 1.0, "→"),
        ],
    ]
}

pub const TOP_BAR: f32 = 44.0;
const REPEAT_DELAY: f64 = 0.45;
const REPEAT_EVERY: f64 = 0.035;
const DOUBLE_TAP: f64 = 0.40;
const PRESS: f32 = 0.75;
const RELEASE: f32 = 0.35;

#[derive(Copy, Clone, Debug, Default)]
struct ModState {
    /// Key is down at the OS level.
    down: bool,
    locked: bool,
    last_tap: f64,
}

#[derive(Default)]
pub struct VrKeyboard {
    mods: HashMap<ScanKey, (Mod, ModState)>,
    /// Per hand: char/named key currently held, (key, down-at, last repeat).
    held: [Option<(ScanKey, f64, f64)>; 2],
    latched: [bool; 2],
    labels: HashMap<ScanKey, String>,
    label_key: Option<(isize, LabelMods)>,
    label_at: f64,
    iso: bool,
    /// Pinned: can't be grabbed/moved.
    pub locked_pos: bool,
}

#[derive(Debug, Default)]
pub struct KbFrame {
    pub haptic: Vec<Hand>,
    pub close: bool,
}

fn slot(h: Hand) -> usize {
    match h {
        Hand::Left => 0,
        Hand::Right => 1,
    }
}

impl VrKeyboard {
    fn mod_active(&self, m: Mod) -> bool {
        self.mods.values().any(|(mm, s)| *mm == m && (s.down || s.locked))
    }

    fn refresh_labels(&mut self, time: f64) {
        let hkl = physical::foreground_layout();
        let mods = LabelMods {
            shift: self.mod_active(Mod::Shift),
            altgr: self.mod_active(Mod::AltGr),
            caps: physical::caps_lock_on(),
        };
        if self.label_key == Some((hkl, mods)) && time - self.label_at < 0.5 {
            return;
        }
        self.label_key = Some((hkl, mods));
        self.label_at = time;
        self.iso = physical::key_label(ISO_KEY, LabelMods::default(), hkl).is_some();
        self.labels.clear();
        for row in rows(self.iso) {
            for k in row.into_iter().filter(|k| k.kind == Kind::Char) {
                if let Some(l) = physical::key_label(k.k, mods, hkl) {
                    self.labels.insert(k.k, l);
                }
            }
        }
    }

    /// Release every sticky (not locked) modifier — called when a normal
    /// key is released.
    fn release_sticky(&mut self) {
        for (k, (_, s)) in self.mods.iter_mut() {
            if s.down && !s.locked {
                let _ = physical::send_scan(*k, false);
                s.down = false;
            }
        }
    }

    fn press_mod(&mut self, k: ScanKey, m: Mod, time: f64) {
        let s = &mut self.mods.entry(k).or_insert((m, ModState::default())).1;
        if s.locked {
            s.locked = false;
            s.down = false;
            let _ = physical::send_scan(k, false);
        } else if s.down && time - s.last_tap < DOUBLE_TAP {
            s.locked = true;
        } else if s.down {
            s.down = false;
            let _ = physical::send_scan(k, false);
        } else {
            s.down = true;
            s.last_tap = time;
            let _ = physical::send_scan(k, true);
        }
    }

    /// Lay out, draw and drive the keyboard for one frame. `hands`:
    /// (hand, laser point on this panel, trigger pull).
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        hands: &[(Hand, Option<egui::Pos2>, f32)],
        time: f64,
        accent: egui::Color32,
    ) -> KbFrame {
        self.refresh_labels(time);
        let mut out = KbFrame::default();
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());

        // Card + top bar.
        let card = screen.shrink(4.0);
        painter.rect_filled(card, 24.0, theme::BG_ROOT);
        painter.rect_stroke(card, 24.0, egui::Stroke::new(1.5_f32, accent.gamma_multiply(0.3)));
        let bar = egui::Rect::from_min_size(card.min + egui::vec2(16.0, 6.0), egui::vec2(card.width() - 32.0, TOP_BAR));
        painter.text(bar.left_center(), egui::Align2::LEFT_CENTER, "KEYBOARD",
            egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), theme::FG_MUTED);
        let close_r = egui::Rect::from_center_size(bar.right_center() - egui::vec2(18.0, 0.0), egui::vec2(36.0, 32.0));
        let lock_r = close_r.translate(egui::vec2(-46.0, 0.0));

        let area = egui::Rect::from_min_max(egui::pos2(card.left() + 12.0, bar.bottom() + 4.0), card.max - egui::vec2(12.0, 12.0));
        let unit = area.width() / 15.0;
        let row_h = area.height() / 5.0;
        let gap = (unit * 0.08).max(3.0);

        // Geometry this frame.
        let mut keys: Vec<(egui::Rect, Key)> = Vec::with_capacity(70);
        for (r, row) in rows(self.iso).iter().enumerate() {
            let mut x = area.left();
            for k in row {
                let rect = egui::Rect::from_min_size(egui::pos2(x, area.top() + r as f32 * row_h), egui::vec2(k.w * unit, row_h)).shrink(gap / 2.0);
                keys.push((rect, *k));
                x += k.w * unit;
            }
        }
        let key_at = |p: egui::Pos2| keys.iter().find(|(r, _)| r.contains(p)).map(|(_, k)| *k);

        // Input, per hand.
        for &(hand, pos, trigger) in hands {
            let s = slot(hand);
            let was = self.latched[s];
            let now = if was { trigger > RELEASE } else { trigger >= PRESS };
            self.latched[s] = now;
            if now && !was {
                let Some(p) = pos else { continue };
                if close_r.contains(p) {
                    out.close = true;
                    out.haptic.push(hand);
                    continue;
                }
                if lock_r.contains(p) {
                    self.locked_pos = !self.locked_pos;
                    out.haptic.push(hand);
                    continue;
                }
                if let Some(k) = key_at(p) {
                    out.haptic.push(hand);
                    match k.kind {
                        Kind::Mod(m, _) => self.press_mod(k.k, m, time),
                        _ => {
                            let _ = physical::send_scan(k.k, true);
                            self.held[s] = Some((k.k, time, time));
                        }
                    }
                }
            } else if !now && was {
                if let Some((k, _, _)) = self.held[s].take() {
                    let _ = physical::send_scan(k, false);
                    self.release_sticky();
                    // Shift/AltGr may have dropped: relabel now.
                    self.label_at = f64::NEG_INFINITY;
                }
            } else if let Some((k, down_at, last)) = self.held[s] {
                // Typematic repeat: key-down again, no key-up, like hardware.
                if time - down_at > REPEAT_DELAY && time - last > REPEAT_EVERY {
                    let _ = physical::send_scan(k, true);
                    self.held[s] = Some((k, down_at, time));
                }
            }
        }

        // Draw keys.
        let hovered: Vec<egui::Pos2> = hands.iter().filter_map(|h| h.1).collect();
        for (rect, k) in &keys {
            let held = self.held.iter().flatten().any(|(hk, _, _)| hk == &k.k);
            let (mod_on, mod_locked) = match k.kind {
                Kind::Mod(..) => self.mods.get(&k.k).map_or((false, false), |(_, s)| (s.down, s.locked)),
                _ => (false, false),
            };
            let hover = hovered.iter().any(|p| rect.contains(*p));
            let fill = if held || mod_on || mod_locked {
                accent.gamma_multiply(if mod_locked { 0.55 } else { 0.32 })
            } else if hover {
                egui::Color32::from_rgb(0x2A, 0x2F, 0x37)
            } else {
                egui::Color32::from_rgb(0x16, 0x19, 0x1E)
            };
            painter.rect_filled(*rect, 10.0, fill);
            if mod_locked {
                painter.rect_stroke(*rect, 10.0, egui::Stroke::new(2.0_f32, accent));
            }
            let (text, size) = match k.kind {
                Kind::Char => (self.labels.get(&k.k).cloned().unwrap_or_default(), row_h * 0.38),
                Kind::Named(l) | Kind::Mod(_, l) => (l.to_string(), row_h * 0.26),
            };
            let caps_on = k.k == ScanKey::new(0x3A) && physical::caps_lock_on();
            let color = if caps_on { accent } else { theme::FG_PRIMARY };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), color);
        }

        // Bar buttons — vector-drawn so they never depend on font coverage.
        for (r, on) in [(lock_r, self.locked_pos), (close_r, false)] {
            let hover = hovered.iter().any(|p| r.contains(*p));
            let fill = if on { accent.gamma_multiply(0.3) } else if hover { egui::Color32::from_rgb(0x2A, 0x2F, 0x37) } else { egui::Color32::from_rgb(0x16, 0x19, 0x1E) };
            painter.rect_filled(r, 10.0, fill);
        }
        let stroke = egui::Stroke::new(2.0_f32, theme::FG_PRIMARY);
        draw_lock(&painter, lock_r.center(), self.locked_pos, if self.locked_pos { egui::Stroke::new(2.0_f32, accent) } else { stroke });
        draw_close(&painter, close_r.center(), stroke);

        // One cursor per hand.
        for &(_, pos, trigger) in hands {
            if let Some(p) = pos {
                crate::laser::paint_cursor(ctx, Some((p, trigger >= PRESS)), accent);
            }
        }
        out
    }

    /// Release everything held (keyboard closed / app exiting) so no key
    /// stays stuck down in the OS.
    pub fn release_all(&mut self) {
        for (k, _, _) in self.held.iter_mut().filter_map(|h| h.take()) {
            let _ = physical::send_scan(k, false);
        }
        for (k, (_, s)) in self.mods.iter_mut() {
            if s.down || s.locked {
                let _ = physical::send_scan(*k, false);
            }
            *s = ModState::default();
        }
        self.latched = [false; 2];
    }
}

/// Padlock: body + shackle; an open shackle swings up and to the right.
pub fn draw_lock(p: &egui::Painter, c: egui::Pos2, closed: bool, s: egui::Stroke) {
    let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 3.0), egui::vec2(14.0, 10.0));
    p.rect_stroke(body, 2.0, s);
    let lift = if closed { 0.0 } else { 4.0 };
    let (l, r, top) = (c.x - 4.5, c.x + 4.5, body.top() - 6.0 - lift);
    let pts: Vec<egui::Pos2> = (0..=12)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
            egui::pos2(c.x + 4.5 * a.cos(), top + 4.5 + 4.5 * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(pts, s));
    p.line_segment([egui::pos2(l, top + 4.5), egui::pos2(l, body.top() - if closed { 0.0 } else { lift })], s);
    p.line_segment([egui::pos2(r, top + 4.5), egui::pos2(r, body.top())], s);
}

pub fn draw_close(p: &egui::Painter, c: egui::Pos2, s: egui::Stroke) {
    let d = 6.0;
    p.line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], s);
    p.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_fifteen_units() {
        for iso in [false, true] {
            for (i, r) in rows(iso).iter().enumerate() {
                let w: f32 = r.iter().map(|k| k.w).sum();
                assert!((w - 15.0).abs() < 1e-4, "row {i} iso={iso} is {w} units");
            }
        }
    }

    #[test]
    fn scan_codes_unique_per_layout() {
        let all: Vec<ScanKey> = rows(true).iter().flatten().map(|k| k.k).collect();
        let mut dedup = all.clone();
        dedup.sort_by_key(|k| (k.scan, k.ext));
        dedup.dedup();
        // Two Shifts share no code (0x2A / 0x36); Alt vs AltGr differ by E0.
        assert_eq!(all.len(), dedup.len());
    }
}
