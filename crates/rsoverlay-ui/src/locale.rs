//! System-locale affordances. Keeps the UI honest with the host's
//! conventions (12h vs 24h clock, eventually first-day-of-week, number
//! formatting, etc.).
//!
//! Windows: queried from the user's locale via `GetLocaleInfoEx`.
//! Linux/macOS: inferred from `LC_TIME` / `LC_ALL` / `LANG` as fallback.

/// Does the current user's locale use a 24-hour clock?
pub fn uses_24h_clock() -> bool {
    detect_24h().unwrap_or_else(default_from_env)
}

#[cfg(target_os = "windows")]
fn detect_24h() -> Option<bool> {
    use windows::core::PCWSTR;
    use windows::Win32::Globalization::{
        GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_ITIME,
    };

    // LOCALE_NAME_MAX_LENGTH is 85 (including null) per the Win32 docs.
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    unsafe {
        let mut name = [0u16; LOCALE_NAME_MAX_LENGTH];
        let n = GetUserDefaultLocaleName(&mut name);
        if n == 0 {
            return None;
        }
        let name_ptr = PCWSTR(name.as_ptr());
        let mut buf = [0u16; 8];
        let written = GetLocaleInfoEx(name_ptr, LOCALE_ITIME, Some(&mut buf));
        if written <= 0 {
            return None;
        }
        // LOCALE_ITIME returns "0" (12h) or "1" (24h) as a single UTF-16 digit.
        let c = char::from_u32(buf[0] as u32)?;
        Some(c == '1')
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_24h() -> Option<bool> {
    // No portable cross-platform API here that doesn't pull in icu4x. The
    // env-based fallback below is good enough for the common locales users
    // actually run — en_US/en_PH/en_CA get 12h, almost everything else 24h.
    None
}

/// Last-resort inference from environment variables. Returns true (24h) by
/// default — that's the globally more common convention.
fn default_from_env() -> bool {
    let locale = std::env::var("LC_TIME")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let locale = locale.split('.').next().unwrap_or("").to_ascii_lowercase();
    // Known 12h-clock locales. Not exhaustive — users with uncommon locales
    // can override via config in a follow-up milestone.
    !matches!(
        locale.as_str(),
        "en_us" | "en_ph" | "en_ca" | "en_au" | "en_nz" | "en_in"
    )
}
