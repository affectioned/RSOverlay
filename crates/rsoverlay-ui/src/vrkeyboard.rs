//! VR keyboard — behaves like a physical keyboard.
//!
//! - Keys are **scan codes** with real down/up: the OS applies the
//!   user's layout (dead keys, AltGr, IME) exactly as for hardware.
//! - Key caps are **labelled from the foreground window's layout**
//!   (`ToUnicodeEx`), refreshed as focus / modifiers change.
//! - **Both hands** type at once — each laser is its own finger; no
//!   single egui pointer.
//! - **Real holding**: one key-down on press, one key-up on release,
//!   nothing in between — a held key reads as *held* to games (raw input
//!   / GetAsyncKeyState), not as a burst of presses. Typing-style
//!   auto-repeat is an opt-in toggle on the bar.
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
/// Full layout: an F-key row (Esc · F1–F12 · PrtSc · Del) on top.
fn f_row() -> Vec<Key> {
    let mut r = vec![named(ScanKey::new(0x01), 1.0, "Esc")];
    let fk = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];
    let sc = [0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x57, 0x58];
    for (l, s) in fk.iter().zip(sc) {
        r.push(named(ScanKey::new(s), 1.0, l));
    }
    r.push(named(ScanKey::ext(0x37), 1.0, "PrtSc"));
    r.push(named(ScanKey::ext(0x53), 1.0, "Del"));
    r
}

fn rows_for(iso: bool, full: bool) -> Vec<Vec<Key>> {
    let mut v: Vec<Vec<Key>> = rows(iso).into_iter().collect();
    if full {
        v.insert(0, f_row());
    }
    v
}

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
            .chain(std::iter::once(named(ScanKey::new(0x1C), 2.25, "⏎")))
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
const BACKSPACE: ScanKey = ScanKey::new(0x0E);
const REPEAT_DELAY: f64 = 0.45;

/// Holding backspace, like an iPhone: a pause, then letters one by one,
/// then whole words, speeding up. One step per delete; step 0 is the
/// press itself. Pure: how many deletes `held` seconds of holding has
/// earned in total.
pub(crate) fn backspace_steps(held: f64) -> u32 {
    const DELAY: f64 = 0.5;
    const CHAR_EVERY: f64 = 0.1;
    const CHARS: u32 = 12;
    const WORDS_AT: f64 = DELAY + CHAR_EVERY * CHARS as f64;
    const WORD_EVERY: f64 = 0.22;
    const FAST_AT: f64 = WORDS_AT + 2.0;
    const FAST_EVERY: f64 = 0.12;
    if held < DELAY {
        return 1;
    }
    if held < WORDS_AT {
        return 1 + (((held - DELAY) / CHAR_EVERY) as u32 + 1).min(CHARS);
    }
    let slow = 1 + CHARS;
    if held < FAST_AT {
        return slow + ((held - WORDS_AT) / WORD_EVERY) as u32 + 1;
    }
    let slow_words = ((FAST_AT - WORDS_AT) / WORD_EVERY) as u32 + 1;
    slow + slow_words + ((held - FAST_AT) / FAST_EVERY) as u32
}

/// Whether delete step `i` (see [`backspace_steps`]) removes a whole
/// word rather than one character.
pub(crate) fn backspace_is_word(i: u32) -> bool {
    i > 12
}

/// Remove the last word (and the spaces after it) from `buf`.
fn pop_word(buf: &mut String) {
    while buf.ends_with(char::is_whitespace) {
        buf.pop();
    }
    while buf.chars().last().is_some_and(|c| !c.is_whitespace()) {
        buf.pop();
    }
}
const REPEAT_EVERY: f64 = 0.035;
const DOUBLE_TAP: f64 = 0.40;
const PRESS: f32 = 0.75;
const RELEASE: f32 = 0.35;

#[derive(Copy, Clone, Debug, Default)]
struct ModState {
    /// Latched (sticky) or held.
    down: bool,
    /// The OS has seen a key-down for it (internal typing never does).
    os: bool,
    locked: bool,
    last_tap: f64,
}

#[derive(Default)]
pub struct VrKeyboard {
    mods: HashMap<ScanKey, (Mod, ModState)>,
    /// Per hand: char/named key currently held, (key, down-at, last repeat).
    held: [Option<(ScanKey, f64, f64)>; 2],
    /// Per hand: backspace held since, and deletes done so far (iPhone
    /// style repeat, every mode).
    bs_hold: [Option<(f64, u32)>; 2],
    /// Per hand: when it last pressed a key; and the hand that pressed
    /// last. Only typing hands light keys up (see [`hand_highlights`]).
    pressed_at: [Option<f64>; 2],
    last_hand: Option<usize>,
    latched: [bool; 2],
    labels: HashMap<ScanKey, String>,
    label_key: Option<(isize, LabelMods)>,
    label_at: f64,
    iso: bool,
    /// Pinned: can't be grabbed/moved.
    pub locked_pos: bool,
    /// Opt-in typematic repeat for typing. Off = real hold (games).
    pub auto_repeat: bool,
    /// Full layout (F-key row) vs compact.
    pub full: bool,
    /// Labels under the *opposite* Shift state — drawn small in the
    /// key's corner as the secondary legend.
    alt_labels: HashMap<ScanKey, String>,
    /// VRChat mode: keys build a message here; Enter sends it to the
    /// chatbox (OSC) instead of typing into the desktop.
    pub vrchat: bool,
    vrc_buf: String,
    /// Several layouts installed: show the language pill.
    multi_layout: bool,
    /// Current layout's language ("en-US").
    lang: String,
    /// Focused window is elevated and we aren't (cached, 0.5 s).
    blocked: bool,
    blocked_at: f64,
}

#[derive(Debug, Default)]
pub struct KbFrame {
    pub haptic: Vec<Hand>,
    pub close: bool,
    /// Typing into an RSOverlay text field: events for that panel.
    pub text: Vec<egui::Event>,
    /// VRChat mode: message to send to the chatbox (Enter pressed).
    pub chat: Option<String>,
    /// Key presses this frame, as typing-sound kinds.
    pub sounds: Vec<rsoverlay_audio::typing::KeySound>,
    /// Diagnostics: per hand on the keyboard, (hand, key under it, point).
    pub hovered: Vec<(Hand, String, egui::Pos2)>,
}

/// Which typing click a key makes.
fn key_sound(kind: Kind, scan: u16) -> rsoverlay_audio::typing::KeySound {
    use rsoverlay_audio::typing::KeySound as S;
    match (kind, scan) {
        (Kind::Mod(..), _) => S::Modifier,
        (_, 0x39) => S::Space,
        (_, 0x1C) => S::Enter,
        (_, 0x0E) => S::Backspace,
        _ => S::Key,
    }
}

/// Seconds a hand keeps lighting keys after its last press while the
/// other hand is the one typing.
const OTHER_HAND_S: f64 = 3.0;

/// Does hand `s` light up the key under its laser? Before anything is
/// typed, every hand does; after that the hand that typed last always
/// does, and the other only within [`OTHER_HAND_S`] of its own last
/// press (two-handed typing keeps both lit; a resting hand goes dark).
/// Pure.
pub(crate) fn hand_highlights(s: usize, last_hand: Option<usize>, pressed_at: Option<f64>, now: f64) -> bool {
    match last_hand {
        None => true,
        Some(l) => l == s || pressed_at.is_some_and(|t| now - t < OTHER_HAND_S),
    }
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
        self.multi_layout = physical::layout_count() > 1;
        self.lang = physical::layout_name(hkl);
        // ISO layouts give the extra key its own character (DE "<" vs "#",
        // UK "\\" vs "#"); ANSI layouts like US merely alias it to the
        // backslash key, so only show it when it differs.
        let iso_label = physical::key_label(ISO_KEY, LabelMods::default(), hkl);
        self.iso = iso_label.is_some() && iso_label != physical::key_label(ScanKey::new(0x2B), LabelMods::default(), hkl);
        self.labels.clear();
        self.alt_labels.clear();
        let alt = LabelMods { shift: !mods.shift, ..mods };
        for row in rows(self.iso) {
            for k in row.into_iter().filter(|k| k.kind == Kind::Char) {
                if let Some(l) = physical::key_label(k.k, mods, hkl) {
                    self.labels.insert(k.k, l);
                }
                if let Some(l) = physical::key_label(k.k, alt, hkl) {
                    self.alt_labels.insert(k.k, l);
                }
            }
        }
    }

    /// Release every sticky (not locked) modifier — called when a normal
    /// key is released.
    fn release_sticky(&mut self) {
        for (k, (_, s)) in self.mods.iter_mut() {
            if s.down && !s.locked {
                if s.os {
                    let _ = physical::send_scan(*k, false);
                    s.os = false;
                }
                s.down = false;
            }
        }
    }

    fn press_mod(&mut self, k: ScanKey, m: Mod, time: f64, internal: bool) {
        let s = &mut self.mods.entry(k).or_insert((m, ModState::default())).1;
        let up = |s: &mut ModState| {
            if s.os {
                let _ = physical::send_scan(k, false);
                s.os = false;
            }
        };
        if s.locked {
            s.locked = false;
            s.down = false;
            up(s);
        } else if s.down && time - s.last_tap < DOUBLE_TAP {
            s.locked = true;
        } else if s.down {
            s.down = false;
            up(s);
        } else {
            s.down = true;
            s.last_tap = time;
            if !internal {
                let _ = physical::send_scan(k, true);
                s.os = true;
            }
        }
    }

    fn egui_mods(&self) -> egui::Modifiers {
        egui::Modifiers {
            shift: self.mod_active(Mod::Shift),
            ctrl: self.mod_active(Mod::Ctrl),
            command: self.mod_active(Mod::Ctrl),
            alt: self.mod_active(Mod::Alt),
            mac_cmd: false,
        }
    }

    /// The egui events a key produces when typing into RSOverlay itself.
    fn internal_events(&self, k: &Key) -> Vec<egui::Event> {
        let key = |key: egui::Key| {
            let m = self.egui_mods();
            vec![
                egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m },
                egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m },
            ]
        };
        match (k.kind, k.k.scan, k.k.ext) {
            (Kind::Char, ..) if !self.mod_active(Mod::Ctrl) => {
                self.labels.get(&k.k).map(|t| vec![egui::Event::Text(t.clone())]).unwrap_or_default()
            }
            (Kind::Char, ..) => {
                // Ctrl+letter shortcuts (copy/paste/select-all).
                let l = self.labels.get(&k.k).and_then(|t| egui::Key::from_name(&t.to_uppercase()));
                l.map(key).unwrap_or_default()
            }
            (_, 0x39, false) => vec![egui::Event::Text(" ".into())],
            (_, 0x0E, false) => key(egui::Key::Backspace),
            (_, 0x1C, false) => key(egui::Key::Enter),
            (_, 0x0F, false) => key(egui::Key::Tab),
            (_, 0x01, false) => key(egui::Key::Escape),
            (_, 0x4B, true) => key(egui::Key::ArrowLeft),
            (_, 0x4D, true) => key(egui::Key::ArrowRight),
            (_, 0x48, true) => key(egui::Key::ArrowUp),
            (_, 0x50, true) => key(egui::Key::ArrowDown),
            _ => Vec::new(),
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
        internal: bool,
    ) -> KbFrame {
        self.refresh_labels(time);
        if time - self.blocked_at > 0.5 {
            self.blocked_at = time;
            self.blocked = rsoverlay_input::elevation::foreground_input_blocked();
        }
        let mut out = KbFrame::default();
        // The space left after any panels (in VR: the whole texture; in
        // the desktop dev harness: below the dev bar).
        let screen = ctx.available_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());

        // Card + top bar.
        let card = screen.shrink(4.0);
        theme::surface(&painter, card.shrink(6.0), 22.0);
        let bar = egui::Rect::from_min_size(card.min + egui::vec2(16.0, 6.0), egui::vec2(card.width() - 32.0, TOP_BAR));
        if self.vrchat {
            let shown = if self.vrc_buf.is_empty() { "VRChat message… (Enter sends)".to_string() } else { format!("{}|", self.vrc_buf) };
            painter.text(bar.left_center(), egui::Align2::LEFT_CENTER, shown,
                egui::FontId::proportional(theme::SIZE_BODY), if self.vrc_buf.is_empty() { theme::FG_MUTED } else { theme::FG_PRIMARY });
        } else if internal {
            painter.text(bar.left_center(), egui::Align2::LEFT_CENTER, "TYPING INTO RSOVERLAY",
                egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), accent);
        } else if self.blocked {
            painter.text(bar.left_center(), egui::Align2::LEFT_CENTER,
                "Focused window runs as administrator — Windows (UAC) blocks VR typing; use your PC keyboard",
                egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), theme::DANGER);
        } else {
            painter.text(bar.left_center(), egui::Align2::LEFT_CENTER, "KEYBOARD",
                egui::FontId::proportional(theme::SIZE_CAPTION + 1.0), theme::FG_MUTED);
        }
        let close_r = egui::Rect::from_center_size(bar.right_center() - egui::vec2(18.0, 0.0), egui::vec2(36.0, 32.0));
        let lock_r = close_r.translate(egui::vec2(-46.0, 0.0));
        let repeat_r = egui::Rect::from_center_size(lock_r.center() - egui::vec2(86.0, 0.0), egui::vec2(116.0, 32.0));
        let vrc_r = egui::Rect::from_center_size(repeat_r.center() - egui::vec2(112.0, 0.0), egui::vec2(96.0, 32.0));
        let paste_r = egui::Rect::from_center_size(vrc_r.center() - egui::vec2(88.0, 0.0), egui::vec2(72.0, 32.0));
        let copy_r = egui::Rect::from_center_size(paste_r.center() - egui::vec2(80.0, 0.0), egui::vec2(72.0, 32.0));
        let size_r = egui::Rect::from_center_size(copy_r.center() - egui::vec2(88.0, 0.0), egui::vec2(88.0, 32.0));
        let lang_r = egui::Rect::from_center_size(size_r.center() - egui::vec2(96.0, 0.0), egui::vec2(88.0, 32.0));

        let area = egui::Rect::from_min_max(egui::pos2(card.left() + 12.0, bar.bottom() + 4.0), card.max - egui::vec2(12.0, 12.0));
        let all_rows = rows_for(self.iso, self.full);
        let unit = area.width() / 15.0;
        let row_h = area.height() / all_rows.len() as f32;
        let gap = (unit * 0.08).max(3.0);

        // Geometry this frame.
        let mut keys: Vec<(egui::Rect, Key)> = Vec::with_capacity(70);
        for (r, row) in all_rows.iter().enumerate() {
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
            if !now {
                self.bs_hold[s] = None;
            }
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
                if repeat_r.contains(p) {
                    self.auto_repeat = !self.auto_repeat;
                    out.haptic.push(hand);
                    continue;
                }
                if self.multi_layout && lang_r.contains(p) {
                    // Next input language for the focused window; labels follow.
                    physical::next_layout();
                    self.label_at = f64::NEG_INFINITY;
                    out.haptic.push(hand);
                    continue;
                }
                if size_r.contains(p) {
                    self.full = !self.full;
                    out.haptic.push(hand);
                    continue;
                }
                if vrc_r.contains(p) {
                    self.vrchat = !self.vrchat;
                    out.haptic.push(hand);
                    continue;
                }
                if copy_r.contains(p) || paste_r.contains(p) {
                    // Ctrl+C / Ctrl+V as real chords (or egui copy when
                    // typing into RSOverlay itself).
                    out.haptic.push(hand);
                    let v = paste_r.contains(p);
                    if internal {
                        if !v {
                            out.text.push(egui::Event::Copy);
                        }
                    } else {
                        let key = ScanKey::new(if v { 0x2F } else { 0x2E });
                        let ctrl = ScanKey::new(0x1D);
                        let _ = physical::send_scan(ctrl, true);
                        let _ = physical::send_scan(key, true);
                        let _ = physical::send_scan(key, false);
                        let _ = physical::send_scan(ctrl, false);
                    }
                    continue;
                }
                if let Some(k) = key_at(p) {
                    self.pressed_at[s] = Some(time);
                    self.last_hand = Some(s);
                    out.haptic.push(hand);
                    out.sounds.push(key_sound(k.kind, k.k.scan));
                    if k.k == BACKSPACE {
                        self.bs_hold[s] = Some((time, 1));
                    }
                    match k.kind {
                        Kind::Mod(m, _) => self.press_mod(k.k, m, time, internal || self.vrchat),
                        _ if self.vrchat => {
                            // Compose a chatbox message locally.
                            match (k.kind, k.k.scan) {
                                (Kind::Char, _) => {
                                    if let Some(t) = self.labels.get(&k.k) {
                                        self.vrc_buf.push_str(t);
                                    }
                                }
                                (_, 0x39) => self.vrc_buf.push(' '),
                                (_, 0x0E) => {
                                    self.vrc_buf.pop();
                                }
                                (_, 0x1C) => out.chat = Some(std::mem::take(&mut self.vrc_buf)),
                                (_, 0x01) => self.vrc_buf.clear(),
                                _ => {}
                            }
                            self.release_sticky();
                            self.label_at = f64::NEG_INFINITY;
                        }
                        _ if internal => {
                            // RSOverlay text field focused: type into it,
                            // never into the desktop.
                            out.text.extend(self.internal_events(&k));
                            self.release_sticky();
                            self.label_at = f64::NEG_INFINITY;
                        }
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
            } else if let Some((t0, done)) = self.bs_hold[s] {
                // Backspace held: letters, then words (iPhone style).
                let total = backspace_steps(time - t0);
                for i in done..total {
                    let word = backspace_is_word(i);
                    if self.vrchat {
                        if word {
                            pop_word(&mut self.vrc_buf);
                        } else {
                            self.vrc_buf.pop();
                        }
                    } else if internal {
                        let mut m = self.egui_mods();
                        if word {
                            m.ctrl = true;
                            m.command = true;
                        }
                        for pressed in [true, false] {
                            out.text.push(egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: m });
                        }
                    } else if word {
                        // Ctrl+Backspace: delete the previous word.
                        let ctrl = ScanKey::new(0x1D);
                        let _ = physical::send_scan(ctrl, true);
                        let _ = physical::send_scan(BACKSPACE, true);
                        let _ = physical::send_scan(ctrl, false);
                    } else {
                        // Typematic: key-down again (it's still held).
                        let _ = physical::send_scan(BACKSPACE, true);
                    }
                    if word || i % 2 == 0 {
                        out.sounds.push(key_sound(Kind::Named("⌫"), BACKSPACE.scan));
                    }
                }
                if total > done {
                    self.bs_hold[s] = Some((t0, total));
                }
            } else if let Some((k, down_at, last)) = self.held[s].filter(|_| self.auto_repeat) {
                // Opt-in typematic repeat: key-down again, no key-up.
                if time - down_at > REPEAT_DELAY && time - last > REPEAT_EVERY {
                    let _ = physical::send_scan(k, true);
                    self.held[s] = Some((k, down_at, time));
                }
            }
        }

        // Draw keys.
        // A resting hand's laser doesn't light keys (tester): only the
        // hand that typed last, or one that typed in the last few seconds.
        let hovered: Vec<egui::Pos2> = hands
            .iter()
            .filter(|h| hand_highlights(slot(h.0), self.last_hand, self.pressed_at[slot(h.0)], time))
            .filter_map(|h| h.1)
            .collect();
        for (rect, k) in &keys {
            let held = self.held.iter().flatten().any(|(hk, _, _)| hk == &k.k);
            let (mod_on, mod_locked) = match k.kind {
                Kind::Mod(..) => self.mods.get(&k.k).map_or((false, false), |(_, s)| (s.down, s.locked)),
                _ => (false, false),
            };
            let hover = hovered.iter().any(|p| rect.contains(*p));
            for &(hand, pos, _) in hands {
                if let Some(p) = pos.filter(|p| rect.contains(*p)) {
                    let name = self.labels.get(&k.k).cloned().unwrap_or_else(|| format!("{:?}", k.kind));
                    out.hovered.push((hand, name, p));
                }
            }
            let fill = if held || mod_on || mod_locked {
                accent.gamma_multiply(if mod_locked { 0.55 } else { 0.32 })
            } else if hover {
                crate::theme::veil(0x24)
            } else {
                crate::theme::veil(0x10)
            };
            painter.rect_filled(*rect, rect.height().min(rect.width()) * 0.28, fill);
            if mod_locked {
                painter.rect_stroke(*rect, 10.0, egui::Stroke::new(2.0_f32, accent));
            }
            // Size from the smaller key dimension: correct at any panel aspect.
            let em = row_h.min(unit);
            let (text, size) = match k.kind {
                Kind::Char => (self.labels.get(&k.k).cloned().unwrap_or_default(), em * 0.42),
                Kind::Named(l) | Kind::Mod(_, l) => (l.to_string(), em * 0.27),
            };
            let caps_on = k.k == ScanKey::new(0x3A) && physical::caps_lock_on();
            let color = if caps_on { accent } else { theme::FG_PRIMARY };
            // Secondary legend (the other Shift state), small, top-right —
            // skipped for letters, whose shifted form is just the capital.
            if k.kind == Kind::Char {
                if let (Some(alt), Some(main)) = (self.alt_labels.get(&k.k), self.labels.get(&k.k)) {
                    if alt != main && alt.to_lowercase() != main.to_lowercase() {
                        painter.text(rect.right_top() + egui::vec2(-8.0, 6.0), egui::Align2::RIGHT_TOP, alt,
                            egui::FontId::proportional(em * 0.22), theme::FG_MUTED);
                    }
                }
            }
            match crate::glyphs::Glyph::from_label(&text) {
                Some(g) => crate::glyphs::draw(&painter, g, rect.center(), row_h.min(unit) * 0.34, color),
                None => {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), color);
                }
            }
        }

        // Bar buttons — vector-drawn so they never depend on font coverage.
        for (r, on) in [(lock_r, self.locked_pos), (close_r, false), (repeat_r, self.auto_repeat), (vrc_r, self.vrchat), (copy_r, false), (paste_r, false), (size_r, self.full)] {
            let hover = hovered.iter().any(|p| r.contains(*p));
            let fill = if on { accent.gamma_multiply(0.3) } else if hover { crate::theme::BG_HOVER } else { crate::theme::BG_RAISED };
            painter.rect_filled(r, 10.0, fill);
        }
        let stroke = egui::Stroke::new(2.0_f32, theme::FG_PRIMARY);
        draw_lock(&painter, lock_r.center(), self.locked_pos, if self.locked_pos { egui::Stroke::new(2.0_f32, accent) } else { stroke });
        draw_close(&painter, close_r.center(), stroke);
        for (r, t, on) in [(vrc_r, "VRChat", self.vrchat), (copy_r, "Copy", false), (paste_r, "Paste", false), (size_r, if self.full { "Full" } else { "Compact" }, self.full)] {
            painter.text(r.center(), egui::Align2::CENTER_CENTER, t,
                egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), if on { accent } else { theme::FG_PRIMARY });
        }
        if self.multi_layout {
            let hover = hovered.iter().any(|p| lang_r.contains(*p));
            painter.rect_filled(lang_r, 10.0, if hover { crate::theme::BG_HOVER } else { crate::theme::BG_RAISED });
            painter.text(lang_r.center(), egui::Align2::CENTER_CENTER, &self.lang,
                egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), theme::FG_PRIMARY);
        }
        painter.text(repeat_r.center(), egui::Align2::CENTER_CENTER,
            if self.auto_repeat { "repeat: on" } else { "hold: real" },
            egui::FontId::proportional(theme::SIZE_CAPTION + 2.0), if self.auto_repeat { accent } else { theme::FG_PRIMARY });

        // No cursor of its own: in VR the laser's ring is the cursor
        // (tester: two cursors on the keyboard).
        out
    }

    /// Release everything held (keyboard closed / app exiting) so no key
    /// stays stuck down in the OS.
    pub fn release_all(&mut self) {
        for (k, _, _) in self.held.iter_mut().filter_map(|h| h.take()) {
            let _ = physical::send_scan(k, false);
        }
        for (k, (_, s)) in self.mods.iter_mut() {
            if s.os {
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
mod highlight_tests {
    use super::*;

    #[test]
    fn a_resting_hand_does_not_light_keys() {
        // Nothing typed yet: both hands aim freely.
        assert!(hand_highlights(0, None, None, 5.0));
        // Right typed last: right lights keys, the never-used left doesn't.
        assert!(hand_highlights(1, Some(1), Some(10.0), 20.0));
        assert!(!hand_highlights(0, Some(1), None, 20.0));
        // Two-handed typing: left typed a moment ago, still lit.
        assert!(hand_highlights(0, Some(1), Some(18.5), 20.0));
        // Left has rested for a while: dark.
        assert!(!hand_highlights(0, Some(1), Some(12.0), 20.0));
    }
}

#[cfg(test)]
mod backspace_tests {
    use super::*;

    #[test]
    fn holding_backspace_goes_letters_then_words_then_faster() {
        // Just the press until the pause is over.
        assert_eq!(backspace_steps(0.0), 1);
        assert_eq!(backspace_steps(0.49), 1);
        // Then letters, ten a second.
        assert_eq!(backspace_steps(0.5), 2);
        assert_eq!(backspace_steps(1.0), 7);
        assert!(!backspace_is_word(backspace_steps(1.5) - 1));
        // Then words.
        let w = backspace_steps(2.0);
        assert!(backspace_is_word(w - 1));
        // Words come slower than letters at first, then speed up.
        let slow = backspace_steps(3.0) - backspace_steps(2.5);
        let fast = backspace_steps(6.0) - backspace_steps(5.5);
        assert!(fast > slow, "{slow} then {fast} per half second");
        // Never goes backwards.
        let mut last = 0;
        for i in 0..800 {
            let n = backspace_steps(i as f64 * 0.01);
            assert!(n >= last);
            last = n;
        }
    }

    #[test]
    fn word_delete_takes_the_word_and_its_spaces() {
        let mut b = String::from("hello there  world  ");
        pop_word(&mut b);
        assert_eq!(b, "hello there  ");
        pop_word(&mut b);
        assert_eq!(b, "hello ");
        pop_word(&mut b);
        pop_word(&mut b);
        assert_eq!(b, "");
    }
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
    fn full_layout_adds_a_fifteen_unit_f_row() {
        let r = rows_for(false, true);
        assert_eq!(r.len(), 6);
        assert!((r[0].iter().map(|k| k.w).sum::<f32>() - 15.0).abs() < 1e-4);
        assert_eq!(rows_for(false, false).len(), 5);
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
