//! Input injection: move the cursor and fire mouse/keyboard events on the
//! host OS. Used by the overlay's laser-pointer path and by the preview
//! mirror's "forward click" affordance.
//!
//! Windows: `SendInput` in absolute mode against the virtual desktop.
//! Other platforms: stub returning `Err` — Linux (uinput) and macOS (CGEvent)
//! land in follow-up milestones.

use anyhow::Result;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Monitor geometry used to map overlay-local coordinates to host pixels.
#[derive(Copy, Clone, Debug)]
pub struct MonitorGeom {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
}

pub mod elevation;
pub mod physical;
pub mod shortcut;

pub use shortcut::Shortcut;

/// Opaque handle to the platform input backend.
pub struct InputInjector;

impl InputInjector {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Move the pointer to a pixel on the given monitor.
    pub fn move_to(&self, mon: &MonitorGeom, px: i32, py: i32) -> Result<()> {
        move_to_impl(mon, px, py)
    }

    /// Click (down + up) a button at the given pixel on the given monitor.
    pub fn click_at(&self, mon: &MonitorGeom, px: i32, py: i32, btn: MouseButton) -> Result<()> {
        click_at_impl(mon, px, py, btn)
    }

    /// Press or release one mouse button at a pixel (no implicit click),
    /// so lasers can drag, select text and hold buttons in real windows.
    pub fn button(&self, mon: &MonitorGeom, px: i32, py: i32, btn: MouseButton, down: bool) -> Result<()> {
        button_impl(mon, px, py, btn, down)
    }

    /// Vertical mouse wheel; `delta` in WHEEL_DELTA units (120 = one
    /// notch, positive = away from the user / scroll up).
    pub fn wheel(&self, delta: i32) -> Result<()> {
        wheel_impl(delta, false)
    }

    /// Horizontal wheel (tilt-wheel), WHEEL_DELTA units; positive
    /// scrolls right.
    pub fn hwheel(&self, delta: i32) -> Result<()> {
        wheel_impl(delta, true)
    }

    /// Send a Unicode character to whatever has keyboard focus on the
    /// host. Uses `KEYEVENTF_UNICODE` so the OS maps the character to
    /// the current keyboard layout — no key-code lookups, no layout
    /// assumptions.
    pub fn type_char(&self, ch: char) -> Result<()> {
        type_char_impl(ch)
    }

    /// Send a raw virtual-key code (synthetic key-down + key-up). Use
    /// for keys that aren't producible via a character — Enter,
    /// Backspace, arrow keys, modifiers held across other events.
    pub fn send_key(&self, vk: VirtualKey) -> Result<()> {
        send_key_impl(vk)
    }

    /// Send a modifier chord: holds all `modifiers` down, taps `ch`,
    /// then releases the modifiers in reverse order. The classic
    /// Ctrl+C / Ctrl+V / Alt+Tab / Win+D path. `ch` is interpreted as
    /// a virtual-key code when the chord includes Ctrl/Alt/Super so
    /// the OS fires the real shortcut (e.g. Ctrl+'c' → copy) instead
    /// of typing a literal character.
    pub fn send_chord(&self, modifiers: &[VirtualKey], ch: char) -> Result<()> {
        send_chord_impl(modifiers, ch)
    }

    /// Fire a parsed [`Shortcut`] as one atomic `SendInput` batch:
    /// modifiers down → key tap → modifiers up (reverse). Atomic so a
    /// physical key event can't interleave mid-chord.
    pub fn send_shortcut(&self, s: &Shortcut) -> Result<()> {
        send_shortcut_impl(s)
    }
}

/// A small subset of virtual-key codes the keyboard overlay actually
/// uses. These line up 1:1 with Win32's VK_* constants on Windows.
/// On other platforms the host-specific mapping will need extending
/// once we implement an injector backend there.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VirtualKey {
    Enter,
    Backspace,
    Tab,
    Escape,
    Space,
    Shift,
    Ctrl,
    Alt,
    Super,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    Delete,
}

impl Default for InputInjector {
    fn default() -> Self {
        Self::new().expect("InputInjector::new never fails in current impls")
    }
}

#[cfg(windows)]
fn move_to_impl(mon: &MonitorGeom, px: i32, py: i32) -> Result<()> {
    win::send_mouse(mon, px, py, None)
}

#[cfg(windows)]
fn click_at_impl(mon: &MonitorGeom, px: i32, py: i32, btn: MouseButton) -> Result<()> {
    win::send_mouse(mon, px, py, Some(btn))
}

#[cfg(windows)]
fn type_char_impl(ch: char) -> Result<()> {
    win::send_unicode_char(ch)
}

#[cfg(windows)]
fn send_key_impl(vk: VirtualKey) -> Result<()> {
    win::send_vk(vk)
}

#[cfg(windows)]
fn send_chord_impl(modifiers: &[VirtualKey], ch: char) -> Result<()> {
    win::send_chord(modifiers, ch)
}

#[cfg(windows)]
fn send_shortcut_impl(s: &Shortcut) -> Result<()> {
    win::send_shortcut(s)
}

#[cfg(not(windows))]
fn send_shortcut_impl(_s: &Shortcut) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(windows)]
fn button_impl(mon: &MonitorGeom, px: i32, py: i32, btn: MouseButton, down: bool) -> Result<()> {
    win::send_button(mon, px, py, btn, down)
}

#[cfg(windows)]
fn wheel_impl(delta: i32, horizontal: bool) -> Result<()> {
    win::send_wheel(delta, horizontal)
}

#[cfg(not(windows))]
fn button_impl(_mon: &MonitorGeom, _px: i32, _py: i32, _btn: MouseButton, _down: bool) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn wheel_impl(_delta: i32, _horizontal: bool) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn move_to_impl(_mon: &MonitorGeom, _px: i32, _py: i32) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn click_at_impl(_mon: &MonitorGeom, _px: i32, _py: i32, _btn: MouseButton) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn type_char_impl(_ch: char) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn send_key_impl(_vk: VirtualKey) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn send_chord_impl(_mods: &[VirtualKey], _ch: char) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(windows)]
mod win {
    use super::{MonitorGeom, MouseButton, Shortcut, VirtualKey};
    use anyhow::{anyhow, Result};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
        KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_HWHEEL, MOUSEINPUT, VIRTUAL_KEY, VK_BACK,
        VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LCONTROL, VK_LEFT,
        VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    pub fn send_unicode_char(ch: char) -> Result<()> {
        // UTF-16 can split a char into a surrogate pair. KEYEVENTF_UNICODE
        // sends one wchar at a time; we pair down+up for each unit.
        let mut utf16 = [0u16; 2];
        let encoded = ch.encode_utf16(&mut utf16);
        let mut events: Vec<INPUT> = Vec::with_capacity(encoded.len() * 2);
        for &unit in encoded.iter() {
            events.push(kb_input(unit, 0, KEYEVENTF_UNICODE));
            events.push(kb_input(unit, 0, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        send_many(&events)
    }

    pub fn send_vk(vk: VirtualKey) -> Result<()> {
        let code = virtual_key_code(vk);
        let events = [
            kb_input(0, code.0, KEYBD_EVENT_FLAGS(0)),
            kb_input(0, code.0, KEYEVENTF_KEYUP),
        ];
        send_many(&events)
    }

    pub fn send_chord(modifiers: &[VirtualKey], ch: char) -> Result<()> {
        // Translate the character to a virtual-key code so Ctrl+C
        // actually fires the Copy shortcut instead of typing 'c'. VK
        // codes for printable ASCII map to uppercase of the same char
        // (per Microsoft's virtual-key table).
        let main_vk: u16 = match ch {
            'a'..='z' => (ch as u16).wrapping_sub(b'a' as u16) + 0x41,
            'A'..='Z' => (ch as u16).wrapping_sub(b'A' as u16) + 0x41,
            '0'..='9' => (ch as u16).wrapping_sub(b'0' as u16) + 0x30,
            _ => return Err(anyhow!("send_chord: no VK mapping for {ch:?}")),
        };
        let mut events: Vec<INPUT> = Vec::with_capacity(modifiers.len() * 2 + 2);
        // Press all modifiers in order.
        for m in modifiers {
            events.push(kb_input(0, virtual_key_code(*m).0, KEYBD_EVENT_FLAGS(0)));
        }
        // Tap the main key.
        events.push(kb_input(0, main_vk, KEYBD_EVENT_FLAGS(0)));
        events.push(kb_input(0, main_vk, KEYEVENTF_KEYUP));
        // Release modifiers in reverse order (so Ctrl-release comes
        // last if there was also Shift, etc).
        for m in modifiers.iter().rev() {
            events.push(kb_input(0, virtual_key_code(*m).0, KEYEVENTF_KEYUP));
        }
        send_many(&events)
    }

    pub fn send_shortcut(s: &Shortcut) -> Result<()> {
        let mods: Vec<u16> = s.mods.vks().collect();
        let key = s.key.vk();
        let ext = if s.key.is_extended() {
            KEYEVENTF_EXTENDEDKEY
        } else {
            KEYBD_EVENT_FLAGS(0)
        };
        // LWIN is itself an extended key.
        let mod_flags = |vk: u16| {
            if vk == 0x5B { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) }
        };
        let mut events: Vec<INPUT> = Vec::with_capacity(mods.len() * 2 + 2);
        for &m in &mods {
            events.push(kb_input(0, m, mod_flags(m)));
        }
        events.push(kb_input(0, key, ext));
        events.push(kb_input(0, key, ext | KEYEVENTF_KEYUP));
        for &m in mods.iter().rev() {
            events.push(kb_input(0, m, mod_flags(m) | KEYEVENTF_KEYUP));
        }
        send_many(&events)
    }

    fn virtual_key_code(vk: VirtualKey) -> VIRTUAL_KEY {
        match vk {
            VirtualKey::Enter => VK_RETURN,
            VirtualKey::Backspace => VK_BACK,
            VirtualKey::Tab => VK_TAB,
            VirtualKey::Escape => VK_ESCAPE,
            VirtualKey::Space => VK_SPACE,
            VirtualKey::Shift => VK_LSHIFT,
            VirtualKey::Ctrl => VK_LCONTROL,
            VirtualKey::Alt => VK_LMENU,
            VirtualKey::Super => VK_LWIN,
            VirtualKey::ArrowUp => VK_UP,
            VirtualKey::ArrowDown => VK_DOWN,
            VirtualKey::ArrowLeft => VK_LEFT,
            VirtualKey::ArrowRight => VK_RIGHT,
            VirtualKey::Home => VK_HOME,
            VirtualKey::End => VK_END,
            VirtualKey::Delete => VK_DELETE,
        }
    }

    fn kb_input(scan: u16, vk: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_many(events: &[INPUT]) -> Result<()> {
        let n = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
        if n as usize != events.len() {
            return Err(anyhow!(
                "SendInput accepted {}/{} keyboard events",
                n,
                events.len()
            ));
        }
        Ok(())
    }

    pub fn send_mouse(
        mon: &MonitorGeom,
        px: i32,
        py: i32,
        click: Option<MouseButton>,
    ) -> Result<()> {
        // Absolute SendInput uses normalized 0..=65535 coords across the
        // virtual desktop, not a single monitor. Translate.
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if vw <= 0 || vh <= 0 {
            return Err(anyhow!("virtual desktop has zero area"));
        }
        let abs_x = mon.origin_x.saturating_add(px.clamp(0, mon.width as i32 - 1));
        let abs_y = mon.origin_y.saturating_add(py.clamp(0, mon.height as i32 - 1));
        let norm_x = (((abs_x - vx) as i64) * 65535 / (vw as i64 - 1).max(1)) as i32;
        let norm_y = (((abs_y - vy) as i64) * 65535 / (vh as i64 - 1).max(1)) as i32;

        let mut events: Vec<INPUT> = Vec::with_capacity(3);
        events.push(mouse_input(
            norm_x,
            norm_y,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ));
        if let Some(btn) = click {
            let (down, up) = match btn {
                MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            };
            events.push(mouse_input(
                norm_x,
                norm_y,
                MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | down,
            ));
            events.push(mouse_input(
                norm_x,
                norm_y,
                MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK | up,
            ));
        }
        let n = unsafe {
            SendInput(
                &events,
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if (n as usize) != events.len() {
            return Err(anyhow!("SendInput accepted {}/{} events", n, events.len()));
        }
        Ok(())
    }

    pub fn send_button(mon: &MonitorGeom, px: i32, py: i32, btn: MouseButton, down: bool) -> Result<()> {
        // Move first (absolute), then the single down-or-up transition.
        send_mouse(mon, px, py, None)?;
        let flag = match (btn, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        let ev = [mouse_input(0, 0, flag)];
        if unsafe { SendInput(&ev, std::mem::size_of::<INPUT>() as i32) } != 1 {
            return Err(anyhow!("SendInput rejected mouse button event"));
        }
        Ok(())
    }

    pub fn send_wheel(delta: i32, horizontal: bool) -> Result<()> {
        let mut ev = mouse_input(0, 0, if horizontal { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL });
        // mouseData carries the signed wheel delta.
        ev.Anonymous.mi.mouseData = delta as u32;
        if unsafe { SendInput(&[ev], std::mem::size_of::<INPUT>() as i32) } != 1 {
            return Err(anyhow!("SendInput rejected wheel event"));
        }
        Ok(())
    }

    fn mouse_input(
        dx: i32,
        dy: i32,
        flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
    ) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}
