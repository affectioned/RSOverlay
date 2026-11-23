//! Virtual on-screen keyboard — the XSO-style "type into whatever has
//! focus on the host OS" affordance.
//!
//! Each key press turns into either a `type_char` (Unicode input, which
//! the OS maps to the current keyboard layout for free) or a
//! `send_key` for non-printable keys (Enter, Backspace, arrows,
//! modifiers). In preview mode the keys are driven by the mouse; in VR
//! the laser-pointer will drive the same `egui::Button` widgets once
//! that lands.

use crate::theme;
use rsoverlay_input::{InputInjector, VirtualKey};

/// Modifier state. Modifiers latch when toggled and are consumed by
/// the next character key — matching XSO's keyboard behavior.
#[derive(Default)]
pub struct ModifierState {
    pub shift: bool,
    pub caps: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
}

impl ModifierState {
    fn uppercase(&self) -> bool {
        self.shift ^ self.caps
    }

    /// After typing a character / firing a chord, release any transient
    /// modifiers. Shift + Ctrl + Alt + Win all release. Caps stays
    /// sticky (that's what Caps Lock is for).
    fn consume_transient(&mut self) {
        self.shift = false;
        self.ctrl = false;
        self.alt = false;
        self.win = false;
    }

    fn chord_mods(&self) -> Vec<VirtualKey> {
        let mut v = Vec::new();
        if self.ctrl { v.push(VirtualKey::Ctrl); }
        if self.alt { v.push(VirtualKey::Alt); }
        if self.win { v.push(VirtualKey::Super); }
        if self.shift { v.push(VirtualKey::Shift); }
        v
    }

    fn any_chord_mod(&self) -> bool {
        self.ctrl || self.alt || self.win
    }
}

/// Virtual keyboard state the dashboard owns and the keyboard_card()
/// mutates.
#[derive(Default)]
pub struct Keyboard {
    pub modifiers: ModifierState,
}

const ROW_NUMBERS: &[(char, char)] = &[
    ('1', '!'), ('2', '@'), ('3', '#'), ('4', '$'), ('5', '%'),
    ('6', '^'), ('7', '&'), ('8', '*'), ('9', '('), ('0', ')'),
];
const ROW_TOP: &[(char, char)] = &[
    ('q', 'Q'), ('w', 'W'), ('e', 'E'), ('r', 'R'), ('t', 'T'),
    ('y', 'Y'), ('u', 'U'), ('i', 'I'), ('o', 'O'), ('p', 'P'),
];
const ROW_HOME: &[(char, char)] = &[
    ('a', 'A'), ('s', 'S'), ('d', 'D'), ('f', 'F'), ('g', 'G'),
    ('h', 'H'), ('j', 'J'), ('k', 'K'), ('l', 'L'),
];
const ROW_BOTTOM: &[(char, char)] = &[
    ('z', 'Z'), ('x', 'X'), ('c', 'C'), ('v', 'V'), ('b', 'B'),
    ('n', 'N'), ('m', 'M'), (',', '<'), ('.', '>'), ('/', '?'),
];

/// Render the keyboard. On any key press, fire the appropriate input
/// event via the injector. The host-focused window receives it.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut Keyboard,
    injector: &InputInjector,
    accent: egui::Color32,
) {
    ui.vertical(|ui| {
        key_row(ui, state, injector, accent, ROW_NUMBERS);
        key_row(ui, state, injector, accent, ROW_TOP);
        key_row(ui, state, injector, accent, ROW_HOME);
        key_row(ui, state, injector, accent, ROW_BOTTOM);
        // Bottom bar: modifiers + space + enter + backspace.
        ui.horizontal(|ui| {
            modifier_toggle(ui, &mut state.modifiers.shift, "Shift", accent);
            modifier_toggle(ui, &mut state.modifiers.caps, "Caps", accent);
            modifier_toggle(ui, &mut state.modifiers.ctrl, "Ctrl", accent);
            modifier_toggle(ui, &mut state.modifiers.alt, "Alt", accent);
            modifier_toggle(ui, &mut state.modifiers.win, "Win", accent);
            if ui
                .add(egui::Button::new("space").min_size(egui::vec2(140.0, 24.0)))
                .clicked()
            {
                let _ = injector.type_char(' ');
                state.modifiers.consume_transient();
            }
            if ui.button("⏎").clicked() {
                let _ = injector.send_key(VirtualKey::Enter);
            }
            if ui.button("⌫").clicked() {
                let _ = injector.send_key(VirtualKey::Backspace);
            }
        });
    });
}

fn key_row(
    ui: &mut egui::Ui,
    state: &mut Keyboard,
    injector: &InputInjector,
    accent: egui::Color32,
    row: &[(char, char)],
) {
    ui.horizontal(|ui| {
        for (lo, hi) in row {
            let ch = if state.modifiers.uppercase() { *hi } else { *lo };
            let btn = egui::Button::new(
                egui::RichText::new(ch.to_string())
                    .color(theme::FG_PRIMARY)
                    .size(14.0),
            )
            .min_size(egui::vec2(28.0, 28.0));
            if ui.add(btn).clicked() {
                // Ctrl / Alt / Win latched → send a real chord (Ctrl+C
                // fires Copy, Alt+Tab switches window, etc). Otherwise
                // the OS maps the character through the host layout.
                if state.modifiers.any_chord_mod() {
                    let mods = state.modifiers.chord_mods();
                    let _ = injector.send_chord(&mods, ch);
                } else {
                    let _ = injector.type_char(ch);
                }
                state.modifiers.consume_transient();
            }
            let _ = accent;
        }
    });
}

fn modifier_toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, accent: egui::Color32) {
    let text = egui::RichText::new(label).color(if *on { accent } else { theme::FG_MUTED });
    if ui.add(egui::Button::new(text).min_size(egui::vec2(48.0, 24.0))).clicked() {
        *on = !*on;
    }
}
