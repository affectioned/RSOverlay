//! UAC / integrity awareness.
//!
//! Windows' UIPI drops synthetic input sent from a normal-integrity
//! process into an elevated ("Run as administrator") one. Capture still
//! works — only input is blocked — so the overlay needs to know when a
//! target is elevated to tell the user instead of silently failing, and
//! optionally relaunch itself elevated.

#[cfg(windows)]
mod imp {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SW_SHOWNORMAL};

    fn token_elevated(process: HANDLE) -> Option<bool> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut e as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok.then_some(e.TokenIsElevated != 0)
        }
    }

    pub fn current_process_elevated() -> bool {
        token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
    }

    pub fn window_elevated(hwnd: isize) -> bool {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(HWND(hwnd), Some(&mut pid));
            if pid == 0 {
                return false;
            }
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                // Can't even query it: protected/system — treat as
                // unreachable rather than pretend input will land.
                return true;
            };
            let r = token_elevated(h);
            let _ = CloseHandle(h);
            // A normal-integrity caller is denied the token of an
            // elevated process; that denial is itself the answer.
            r.unwrap_or(true)
        }
    }

    pub fn foreground_elevated() -> bool {
        let hwnd = unsafe { GetForegroundWindow() };
        hwnd.0 != 0 && window_elevated(hwnd.0)
    }

    pub fn relaunch_elevated(args: &[String]) -> bool {
        let Ok(exe) = std::env::current_exe() else { return false };
        let params = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
        let (exe, params) = (HSTRING::from(exe.as_os_str()), HSTRING::from(params));
        // "runas" → standard UAC consent prompt. > 32 = success.
        let r = unsafe { ShellExecuteW(HWND(0), w!("runas"), &exe, &params, PCWSTR::null(), SW_SHOWNORMAL) };
        r.0 > 32
    }

    fn quote(a: &str) -> String {
        if a.is_empty() || a.contains([' ', '\t', '"']) {
            format!("\"{}\"", a.replace('"', "\\\""))
        } else {
            a.to_string()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn current_process_elevated() -> bool {
        false
    }
    pub fn window_elevated(_hwnd: isize) -> bool {
        false
    }
    pub fn foreground_elevated() -> bool {
        false
    }
    pub fn relaunch_elevated(_args: &[String]) -> bool {
        false
    }
}

/// Is RSOverlay itself running elevated?
pub fn current_process_elevated() -> bool {
    imp::current_process_elevated()
}

/// Does `hwnd` belong to an elevated process? (Conservatively `true`
/// when its token can't be read from here.)
pub fn window_elevated(hwnd: isize) -> bool {
    imp::window_elevated(hwnd)
}

/// Would injected input be blocked for this window? Only when it's
/// elevated *and* we aren't.
pub fn input_blocked(hwnd: isize) -> bool {
    !current_process_elevated() && window_elevated(hwnd)
}

/// Is the window keystrokes currently go to elevated (and we aren't)?
pub fn foreground_input_blocked() -> bool {
    !current_process_elevated() && imp::foreground_elevated()
}

/// Relaunch this exe with `args` through the UAC consent prompt.
/// `true` if the elevated copy started (the caller should exit).
pub fn relaunch_elevated(args: &[String]) -> bool {
    imp::relaunch_elevated(args)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_not_elevated_in_tests_and_own_windows_are_reachable() {
        // Dev/CI shells run unelevated; if someone runs tests as admin
        // this just documents that state instead of failing.
        let me = current_process_elevated();
        println!("tests elevated: {me}");
        if !me {
            assert!(!foreground_input_blocked() || imp::foreground_elevated());
        }
    }
}
