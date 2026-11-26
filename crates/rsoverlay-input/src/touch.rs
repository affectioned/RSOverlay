//! Touch injection (`InjectTouchInput`): the laser as a finger on the
//! real window, for apps that behave better with touch than a mouse.
//! One contact; down / move / up. Coordinates are screen pixels.

use crate::MonitorGeom;
use anyhow::Result;

/// Phase of the single touch contact.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// Screen pixel for a point inside `geom` (window / monitor pixels).
pub fn to_screen(geom: &MonitorGeom, px: i32, py: i32) -> (i32, i32) {
    (geom.origin_x + px, geom.origin_y + py)
}

/// Inject one touch event at window / monitor pixel (px, py).
pub fn touch(geom: &MonitorGeom, px: i32, py: i32, phase: Phase) -> Result<()> {
    let (x, y) = to_screen(geom, px, py);
    imp::touch(x, y, phase)
}

#[cfg(windows)]
mod imp {
    use super::Phase;
    use anyhow::{anyhow, Result};
    use std::sync::OnceLock;
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::Input::Pointer::{
        InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE,
        POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_INFO, POINTER_TOUCH_INFO, TOUCH_FEEDBACK_NONE,
    };

    // WinUser.h TOUCH_MASK_* (touchMask is a plain u32 here).
    const TOUCH_MASK_CONTACTAREA: u32 = 0x1;
    const TOUCH_MASK_ORIENTATION: u32 = 0x2;
    const TOUCH_MASK_PRESSURE: u32 = 0x4;
    use windows::Win32::UI::WindowsAndMessaging::PT_TOUCH;

    /// Contact patch half-size, px.
    const R: i32 = 4;

    pub fn touch(x: i32, y: i32, phase: Phase) -> Result<()> {
        static READY: OnceLock<bool> = OnceLock::new();
        // SAFETY: plain Win32 call, once per process.
        let ready = *READY.get_or_init(|| unsafe { InitializeTouchInjection(1, TOUCH_FEEDBACK_NONE).is_ok() });
        if !ready {
            return Err(anyhow!("touch injection unavailable"));
        }
        let flags = match phase {
            Phase::Down => POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
            Phase::Move => POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT,
            Phase::Up => POINTER_FLAG_UP,
        };
        let info = POINTER_TOUCH_INFO {
            pointerInfo: POINTER_INFO {
                pointerType: PT_TOUCH,
                pointerId: 0,
                ptPixelLocation: POINT { x, y },
                pointerFlags: flags,
                ..Default::default()
            },
            touchMask: TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE,
            rcContact: RECT { left: x - R, top: y - R, right: x + R, bottom: y + R },
            orientation: 90,
            pressure: 32000,
            ..Default::default()
        };
        // SAFETY: one fully initialised contact.
        unsafe { InjectTouchInput(&[info]) }.map_err(|e| anyhow!("InjectTouchInput: {e}"))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Phase;
    pub fn touch(_x: i32, _y: i32, _phase: Phase) -> anyhow::Result<()> {
        anyhow::bail!("touch injection is Windows-only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_pixels_become_screen_pixels() {
        let g = MonitorGeom { origin_x: -1920, origin_y: 100, width: 800, height: 600 };
        assert_eq!(to_screen(&g, 10, 20), (-1910, 120));
    }
}
