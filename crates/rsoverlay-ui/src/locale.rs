//! System-locale affordances. Keeps the UI honest with the host's
//! conventions (12h vs 24h clock, eventually first-day-of-week, number
//! formatting, etc.).
//!
//! Windows: queried from the user's locale via `GetLocaleInfoEx`.
//! Linux/macOS: inferred from `LC_TIME` / `LC_ALL` / `LANG` as fallback.

/// Does the current user's locale use a 24-hour clock?
/// Clock override (Settings › Wrist toolbar): 0 = system, 1 = 12 h,
/// 2 = 24 h. Date style: 0 = "Thu, Nov 26", 1 = "Thu 26 Nov",
/// 2 = "2026-11-26".
static CLOCK_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static DATE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Clock choices as (config slug, label).
pub const CLOCK_FORMATS: [(&str, &str); 3] = [("system", "System"), ("12h", "12 h"), ("24h", "24 h")];
/// Date choices as (config slug, label).
pub const DATE_FORMATS: [(&str, &str); 3] = [("system", "Thu, Nov 26"), ("day-month", "Thu 26 Nov"), ("iso", "2026-11-26")];

fn index_of(table: &[(&str, &str)], slug: &str) -> u8 {
    table.iter().position(|(s, _)| *s == slug).unwrap_or(0) as u8
}

/// Apply the user's clock / date choices (config slugs).
pub fn set_formats(clock: &str, date: &str) {
    CLOCK_MODE.store(index_of(&CLOCK_FORMATS, clock), std::sync::atomic::Ordering::Relaxed);
    DATE_MODE.store(index_of(&DATE_FORMATS, date), std::sync::atomic::Ordering::Relaxed);
}

pub fn uses_24h_clock() -> bool {
    match CLOCK_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => detect_24h().unwrap_or_else(default_from_env),
    }
}

/// The time, in the user's clock format.
pub fn format_time(now: &chrono::DateTime<chrono::Local>) -> String {
    if uses_24h_clock() { now.format("%H:%M").to_string() } else { now.format("%-I:%M %p").to_string() }
}

/// The date, in the user's date style.
pub fn format_date(now: &chrono::DateTime<chrono::Local>) -> String {
    match DATE_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => now.format("%a %-d %b").to_string(),
        2 => now.format("%Y-%m-%d").to_string(),
        _ => now.format("%a, %b %-d").to_string(),
    }
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
