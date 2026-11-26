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
        self.poll_with(std::time::Instant::now(), read)
    }

    /// [`Self::poll`] with the clock and the pad reader passed in (tests
    /// feed a scripted fake pad).
    pub fn poll_with(&mut self, now: std::time::Instant, mut read: impl FnMut(u32) -> Option<u16>) -> u16 {
        let rescan = self.rescan.is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_secs(2));
        if rescan {
            self.rescan = Some(now);
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

    /// A scripted pad set: which slots are plugged in and what they hold;
    /// counts how often each slot was asked.
    struct FakePads {
        held: [Option<u16>; 4],
        asked: [u32; 4],
    }

    impl FakePads {
        fn reader(&mut self) -> impl FnMut(u32) -> Option<u16> + '_ {
            |slot| {
                self.asked[slot as usize] += 1;
                self.held[slot as usize]
            }
        }
    }

    #[test]
    fn pads_combine_and_empty_slots_are_rescanned_only_every_two_seconds() {
        let t0 = std::time::Instant::now();
        let at = |ms| t0 + std::time::Duration::from_millis(ms);
        let mut fake = FakePads { held: [Some(b::BACK), None, Some(b::LB), None], asked: [0; 4] };
        let mut pads = Gamepads::default();
        // First poll scans everything; two pads' buttons combine.
        assert_eq!(pads.poll_with(at(0), fake.reader()), b::BACK | b::LB);
        assert_eq!(fake.asked, [1, 1, 1, 1]);
        // Next frames: only the connected slots are read.
        for ms in [11, 22, 33] {
            pads.poll_with(at(ms), fake.reader());
        }
        assert_eq!(fake.asked, [4, 1, 4, 1]);
        // A pad plugged into slot 1 shows up at the 2 s rescan.
        fake.held[1] = Some(b::A);
        assert_eq!(pads.poll_with(at(1000), fake.reader()), b::BACK | b::LB);
        assert_eq!(pads.poll_with(at(2100), fake.reader()), b::BACK | b::LB | b::A);
        // Unplugged: dropped immediately.
        fake.held[0] = None;
        assert_eq!(pads.poll_with(at(2111), fake.reader()), b::LB | b::A);
    }

    #[test]
    fn scripted_session_fires_each_shortcut_once() {
        // Back+LB (dashboard) held for several frames, released, pressed again.
        let chord = b::BACK | b::LB;
        let frames = [0, b::BACK, chord, chord, chord, b::BACK, 0, chord, 0];
        let mut prev = 0;
        let fired: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter_map(|(i, &now)| {
                let f = chord_pressed(prev, now, chord);
                prev = now;
                f.then_some(i)
            })
            .collect();
        assert_eq!(fired, [2, 7]);
    }

    #[test]
    fn names() {
        assert_eq!(chord_name(b::BACK | b::LB), "Back + LB");
        assert_eq!(chord_name(0), "not set");
    }
}
