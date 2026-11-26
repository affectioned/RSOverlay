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


/// Real touch injection into a window of our own (run with --ignored on
/// an interactive desktop). The window is small, topmost and checked to
/// be under the touch point first, so nothing else is ever touched.
#[cfg(all(test, windows))]
mod harness {
    use super::*;
    use std::sync::Mutex;
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// (message, x, y) the test window received.
    static SEEN: Mutex<Vec<(u32, i32, i32)>> = Mutex::new(Vec::new());
    const WM_POINTERUPDATE: u32 = 0x0245;
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if matches!(msg, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
            let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
            SEEN.lock().unwrap().push((msg, x, y));
            return LRESULT(0);
        }
        DefWindowProcW(h, msg, wp, lp)
    }

    fn pump(ms: u64) {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        let mut m = MSG::default();
        while std::time::Instant::now() < until {
            // SAFETY: standard message loop on this thread's window.
            unsafe {
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore]
    fn touch_reaches_the_window_under_it() {
        // SAFETY: plain window creation / destruction on this thread.
        unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let class = w!("RSOverlayTouchHarness");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            // Bottom-right corner of the primary screen, out of the way.
            let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            let (x, y, size) = (sw - 260, sh - 320, 200);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("RSOverlay touch test"),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                size,
                size,
                None,
                None,
                inst,
                None,
            );
            assert_ne!(hwnd.0, 0, "test window");
            pump(150);
            let (cx, cy) = (x + size / 2, y + size / 2);
            // Never inject unless it's our window under the point.
            if WindowFromPoint(POINT { x: cx, y: cy }) != hwnd {
                let _ = DestroyWindow(hwnd);
                eprintln!("SKIPPED: something else is on top of the test spot");
                return;
            }
            SEEN.lock().unwrap().clear();
            let screen = MonitorGeom { origin_x: 0, origin_y: 0, width: sw as u32, height: sh as u32 };
            touch(&screen, cx, cy, Phase::Down).expect("down");
            pump(60);
            touch(&screen, cx + 20, cy + 10, Phase::Move).expect("move");
            pump(60);
            touch(&screen, cx + 20, cy + 10, Phase::Up).expect("up");
            pump(200);
            let _ = DestroyWindow(hwnd);
            let seen = SEEN.lock().unwrap().clone();
            println!("{seen:?}");
            let has = |m: u32| seen.iter().any(|(msg, _, _)| *msg == m);
            assert!(has(WM_POINTERDOWN), "touch down reached the window: {seen:?}");
            assert!(has(WM_POINTERUP), "touch up reached the window: {seen:?}");
            // Down landed where we touched (screen coordinates).
            let &(_, dx, dy) = seen.iter().find(|(m, _, _)| *m == WM_POINTERDOWN).unwrap();
            assert!((dx - cx).abs() <= 2 && (dy - cy).abs() <= 2, "down at ({dx},{dy}), touched ({cx},{cy})");
            // The slide arrived as an update at the new spot.
            assert!(seen.iter().any(|(m, ux, uy)| *m == WM_POINTERUPDATE && (ux - (cx + 20)).abs() <= 2 && (uy - (cy + 10)).abs() <= 2), "{seen:?}");
        }
    }
}
