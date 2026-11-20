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

#[cfg(not(windows))]
fn move_to_impl(_mon: &MonitorGeom, _px: i32, _py: i32) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(not(windows))]
fn click_at_impl(_mon: &MonitorGeom, _px: i32, _py: i32, _btn: MouseButton) -> Result<()> {
    anyhow::bail!("input injection only implemented on Windows")
}

#[cfg(windows)]
mod win {
    use super::{MonitorGeom, MouseButton};
    use anyhow::{anyhow, Result};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

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
