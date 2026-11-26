//! Physical-keyboard emulation: keys are hardware **scan codes**, so the
//! OS applies the user's real layout (dead keys, AltGr, IME) exactly as
//! for a USB keyboard. Key-cap labels are asked of that same layout via
//! `ToUnicodeEx`, against the *foreground* window's HKL — the layout the
//! keystrokes will actually be interpreted with.

use anyhow::Result;

/// A physical key position: Set-1 scan code + E0 "extended" prefix.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScanKey {
    pub scan: u16,
    pub ext: bool,
}

impl ScanKey {
    pub const fn new(scan: u16) -> Self {
        Self { scan, ext: false }
    }
    pub const fn ext(scan: u16) -> Self {
        Self { scan, ext: true }
    }
}

/// Modifier state used when asking the layout for a key's label.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LabelMods {
    pub shift: bool,
    pub altgr: bool,
    pub caps: bool,
}

#[cfg(windows)]
mod imp {
    use super::{LabelMods, ScanKey};
    use anyhow::{anyhow, Result};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, GetKeyboardLayout, MapVirtualKeyExW, SendInput, ToUnicodeEx, INPUT, INPUT_0, INPUT_KEYBOARD,
        KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VSC_TO_VK_EX, VIRTUAL_KEY,
    };
    use windows::Win32::UI::TextServices::HKL;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    pub fn send_scan(k: ScanKey, down: bool) -> Result<()> {
        let mut flags = KEYEVENTF_SCANCODE;
        if k.ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: k.scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        let n = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if n != 1 {
            return Err(anyhow!("SendInput rejected scan code {:#x}", k.scan));
        }
        Ok(())
    }

    /// Installed keyboard layouts (input languages).
    pub fn layout_count() -> usize {
        // SAFETY: a null buffer only asks for the count.
        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList(None).max(0) as usize }
    }

    /// Ask the focused window to switch to the next installed layout
    /// (what Windows' own language hotkey does).
    pub fn next_layout() -> bool {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_INPUTLANGCHANGEREQUEST};
        const HKL_NEXT: isize = 1;
        // SAFETY: posting a standard message to the foreground window.
        unsafe { PostMessageW(GetForegroundWindow(), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(HKL_NEXT)).is_ok() }
    }

    /// Short name of a layout's language, e.g. "en-US", "de-DE".
    pub fn layout_name(hkl: isize) -> String {
        use windows::Win32::Globalization::LCIDToLocaleName;
        let lcid = (hkl as usize & 0xFFFF) as u32;
        let mut buf = [0u16; 85];
        // SAFETY: buffer sized to LOCALE_NAME_MAX_LENGTH.
        let n = unsafe { LCIDToLocaleName(lcid, Some(&mut buf), 0) };
        if n <= 1 {
            return format!("{lcid:04x}");
        }
        String::from_utf16_lossy(&buf[..n as usize - 1])
    }

    /// The layout of whatever window keystrokes will land in.
    pub fn foreground_layout() -> isize {
        unsafe {
            let tid = GetWindowThreadProcessId(GetForegroundWindow(), None);
            GetKeyboardLayout(tid).0
        }
    }

    pub fn caps_lock_on() -> bool {
        unsafe { GetKeyState(0x14) & 1 != 0 }
    }

    pub fn key_label(k: ScanKey, m: LabelMods, hkl: isize) -> Option<String> {
        unsafe {
            let hkl = HKL(hkl);
            let code = u32::from(k.scan) | if k.ext { 0xE000 } else { 0 };
            let vk = MapVirtualKeyExW(code, MAPVK_VSC_TO_VK_EX, hkl);
            if vk == 0 {
                return None;
            }
            let mut state = [0u8; 256];
            if m.shift {
                state[0x10] = 0x80; // VK_SHIFT
            }
            if m.caps {
                state[0x14] = 0x01; // VK_CAPITAL toggled
            }
            if m.altgr {
                state[0x11] = 0x80; // VK_CONTROL
                state[0x12] = 0x80; // VK_MENU
            }
            let mut buf = [0u16; 8];
            // Flag 0x4: don't mutate the kernel's dead-key state while
            // probing (Windows 10 1607+), so labelling never eats a
            // pending accent the user is typing.
            let n = ToUnicodeEx(vk, code, &state, &mut buf, 0x4, hkl);
            let len = match n {
                0 => return None,
                n if n < 0 => 1, // dead key: the accent char is in buf[0]
                n => n as usize,
            };
            let s = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
            // Control characters (Tab, Enter…) aren't printable labels.
            (!s.chars().any(char::is_control)).then_some(s)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{LabelMods, ScanKey};
    use anyhow::Result;
    pub fn send_scan(_k: ScanKey, _down: bool) -> Result<()> {
        anyhow::bail!("input injection only implemented on Windows")
    }
    pub fn foreground_layout() -> isize {
        0
    }
    pub fn layout_count() -> usize {
        1
    }
    pub fn next_layout() -> bool {
        false
    }
    pub fn layout_name(_hkl: isize) -> String {
        String::new()
    }
    pub fn caps_lock_on() -> bool {
        false
    }
    pub fn key_label(_k: ScanKey, _m: LabelMods, _hkl: isize) -> Option<String> {
        None
    }
}

/// Press or release a physical key.
pub fn send_scan(k: ScanKey, down: bool) -> Result<()> {
    imp::send_scan(k, down)
}

/// HKL (as an integer) of the foreground window's keyboard layout.
pub fn foreground_layout() -> isize {
    imp::foreground_layout()
}

/// How many keyboard layouts (input languages) are installed.
pub fn layout_count() -> usize {
    imp::layout_count()
}

/// Switch the focused window to the next installed layout.
pub fn next_layout() -> bool {
    imp::next_layout()
}

/// "en-US", "de-DE", … for a layout handle.
pub fn layout_name(hkl: isize) -> String {
    imp::layout_name(hkl)
}

pub fn caps_lock_on() -> bool {
    imp::caps_lock_on()
}

/// What this key produces under `mods` in layout `hkl`, if printable.
pub fn key_label(k: ScanKey, mods: LabelMods, hkl: isize) -> Option<String> {
    imp::key_label(k, mods, hkl)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Reads (never types) the live layout: Q's scan code should label
    /// as a letter in every Latin layout, and Shift should change it.
    #[test]
    #[ignore]
    fn live_layout_labels() {
        let hkl = foreground_layout();
        let lo = key_label(ScanKey::new(0x10), LabelMods::default(), hkl);
        let hi = key_label(ScanKey::new(0x10), LabelMods { shift: true, ..Default::default() }, hkl);
        println!("hkl={hkl:#x} Q-key: {lo:?} / {hi:?}; 2-key shifted: {:?}",
            key_label(ScanKey::new(0x03), LabelMods { shift: true, ..Default::default() }, hkl));
        assert!(lo.is_some() && lo != hi);
    }
}
