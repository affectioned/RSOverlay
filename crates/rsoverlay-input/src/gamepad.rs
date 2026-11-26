//! Gamepad shortcuts (XInput): button combinations that show the
//! dashboard, recenter or toggle the keyboard, for players who hold a
//! gamepad (sims) instead of VR controllers. Read-only: RSOverlay never
//! sends gamepad input.

/// XInput button bits (`XINPUT_GAMEPAD_*`).
pub mod button {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;

    /// (bit, short name) in display order.
    pub const NAMES: [(u16, &str); 14] = [
        (BACK, "Back"),
        (START, "Start"),
        (LB, "LB"),
        (RB, "RB"),
        (A, "A"),
        (B, "B"),
        (X, "X"),
        (Y, "Y"),
        (LEFT_THUMB, "L-stick"),
        (RIGHT_THUMB, "R-stick"),
        (DPAD_UP, "Up"),
        (DPAD_DOWN, "Down"),
        (DPAD_LEFT, "Left"),
        (DPAD_RIGHT, "Right"),
    ];
}

/// "Back + LB" for a chord; "not set" for 0.
pub fn chord_name(chord: u16) -> String {
    if chord == 0 {
        return "not set".into();
    }
    button::NAMES.iter().filter(|(b, _)| chord & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(" + ")
}

/// The chord just became fully held this poll (edge, not level), so
/// holding it fires once.
pub fn chord_pressed(prev: u16, now: u16, chord: u16) -> bool {
    chord != 0 && now & chord == chord && prev & chord != chord
}

/// Polls every connected XInput pad and ORs their buttons together.
/// Empty slots are re-checked only every 2 s: XInputGetState on a
/// disconnected slot is slow.
#[derive(Default)]
pub struct Gamepads {
    connected: [bool; 4],
    rescan: Option<std::time::Instant>,
}

impl Gamepads {
    pub fn poll(&mut self) -> u16 {
        let rescan = self.rescan.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(2));
        if rescan {
            self.rescan = Some(std::time::Instant::now());
        }
        let mut held = 0;
        for (slot, connected) in self.connected.iter_mut().enumerate() {
            if !*connected && !rescan {
                continue;
            }
            match read(slot as u32) {
                Some(b) => {
                    *connected = true;
                    held |= b;
                }
                None => *connected = false,
            }
        }
        held
    }
}

#[cfg(windows)]
fn read(slot: u32) -> Option<u16> {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    let mut st = XINPUT_STATE::default();
    // SAFETY: valid out-pointer; returns ERROR_SUCCESS (0) when a pad is there.
    let r = unsafe { XInputGetState(slot, &mut st) };
    (r == 0).then_some(st.Gamepad.wButtons.0)
}

#[cfg(not(windows))]
fn read(_slot: u32) -> Option<u16> {
    None
}

#[cfg(test)]
mod tests {
    use super::{button as b, *};

    #[test]
    fn chord_fires_once_when_completed() {
        let chord = b::BACK | b::LB;
        assert!(!chord_pressed(0, b::BACK, chord));
        assert!(chord_pressed(b::BACK, b::BACK | b::LB, chord));
        // Still held: no repeat. Extra buttons don't matter.
        assert!(!chord_pressed(b::BACK | b::LB, b::BACK | b::LB | b::A, chord));
        // Unset chord never fires.
        assert!(!chord_pressed(0, 0xFFFF, 0));
    }

    #[test]
    fn names() {
        assert_eq!(chord_name(b::BACK | b::LB), "Back + LB");
        assert_eq!(chord_name(0), "not set");
    }
}
