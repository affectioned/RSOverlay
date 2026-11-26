//! Update notifier (non-Steam builds): asks GitHub Releases whether a
//! newer RSOverlay exists. It only *tells* the user; nothing is ever
//! downloaded or installed automatically. Steam builds don't use this
//! (Steam updates them).
//!
//! Privacy: one anonymous HTTPS GET to api.github.com with a generic
//! User-Agent. Nothing about the user or their machine is sent.

/// GitHub `owner/repo` the releases come from.
pub const REPO: &str = "affectioned/RSOverlay";

/// Every release page link must start with this (it's opened in the
/// browser, so never trust a URL from the response blindly).
pub const RELEASES_URL: &str = "https://github.com/affectioned/RSOverlay/releases";

/// Where testers report problems.
pub const ISSUES_URL: &str = "https://github.com/affectioned/RSOverlay/issues";

/// This build's version: the release tag the build was made from (set
/// by the release workflow, e.g. "0.1.0-beta.3"), else the Cargo version
/// with "-dev" for local builds. Used for display *and* the update
/// check, so a beta build is told about the next beta.
pub const CURRENT: &str = match option_env!("RSO_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};

/// Short commit the build came from (release builds), if known.
pub const COMMIT: Option<&str> = option_env!("RSO_COMMIT");

/// "RSOverlay 0.1.0-beta.3 (d8843e6)" — what to quote in a bug report.
pub fn version_line() -> String {
    match COMMIT {
        Some(c) => format!("RSOverlay {CURRENT} ({c})"),
        None => format!("RSOverlay {CURRENT}"),
    }
}

/// Issue tracker "new issue" page with the version filled in.
pub fn new_issue_url() -> String {
    let body = format!("**Version:** {}%0A**Headset:** %0A%0A**What happened:**%0A%0A**What I expected:**%0A", version_line().replace(' ', "%20"));
    format!("{ISSUES_URL}/new?body={body}")
}

/// A published release newer than this build.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Tag as published, e.g. "v0.2.0" or "v0.2.0-beta.1".
    pub tag: String,
    /// Release page (always under [`RELEASES_URL`]).
    pub url: String,
    /// A tester (pre-release) build.
    pub prerelease: bool,
}

/// Semver-ish version: major.minor.patch plus an optional pre-release
/// suffix. A pre-release sorts *before* its final release
/// (0.2.0-beta.1 < 0.2.0), and pre-releases compare by their dotted
/// parts, numerically where both are numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub core: (u64, u64, u64),
    pub pre: Vec<String>,
}

impl Version {
    /// Parse "v1.2.3", "1.2.3" or "1.2.3-beta.4" (build metadata after
    /// `+` is ignored). `None` if it isn't a version.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.split('.').map(str::to_string).collect()),
            None => (s, Vec::new()),
        };
        let mut it = core.split('.').map(|n| n.parse::<u64>().ok());
        let v = (it.next()??, it.next()??, it.next()??);
        if it.next().is_some() {
            return None;
        }
        Some(Self { core: v, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Equal,
            (true, false) => Greater,
            (false, true) => Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Less,
                        (Err(_), Ok(_)) => Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if o != Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

/// From a GitHub `/releases` response (JSON array), the newest release
/// above `current`. Drafts are skipped; pre-releases only when
/// `prereleases` is on. Links outside [`RELEASES_URL`] are rejected.
pub fn newest_from_json(json: &str, current: &str, prereleases: bool) -> Option<Release> {
    #[derive(serde::Deserialize)]
    struct Gh {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    let list: Vec<Gh> = serde_json::from_str(json).ok()?;
    let current = Version::parse(current)?;
    list.into_iter()
        .filter(|r| !r.draft && (prereleases || !r.prerelease))
        .filter(|r| r.html_url.starts_with(&format!("{RELEASES_URL}/")))
        .filter_map(|r| Version::parse(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| *v > current)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| Release { tag: r.tag_name, url: r.html_url, prerelease: r.prerelease })
}

/// Ask GitHub for the newest release above this build. Blocking (run it
/// off the UI thread); ~1 request, 10 s timeout.
pub fn check(prereleases: bool) -> anyhow::Result<Option<Release>> {
    let body = fetch(&format!("/repos/{REPO}/releases?per_page=20"))?;
    Ok(newest_from_json(&body, CURRENT, prereleases))
}

/// Open a release page in the default browser. Refuses anything that
/// isn't one of RSOverlay's GitHub release pages.
pub fn open_release_page(url: &str) -> bool {
    if !url.starts_with(&format!("{RELEASES_URL}/")) && url != RELEASES_URL {
        return false;
    }
    open_url(url)
}

/// Open a new issue in the browser, with this build's version filled in.
pub fn open_issues() -> bool {
    open_url(&new_issue_url())
}

/// Open RSOverlay's log folder in Explorer (created if missing).
pub fn open_logs_folder() -> bool {
    let dir = crate::config::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    open_url(&dir.to_string_lossy())
}

#[cfg(windows)]
fn open_url(url: &str) -> bool {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // SAFETY: valid, NUL-terminated wide strings for the call's duration.
    let r = unsafe { ShellExecuteW(HWND(0), w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 > 32
}

#[cfg(not(windows))]
fn open_url(_url: &str) -> bool {
    false
}

/// Largest response accepted (the release list is a few tens of KB).
const MAX_BODY: usize = 2 << 20;

#[cfg(windows)]
fn fetch(path: &str) -> anyhow::Result<String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Networking::WinHttp::*;

    struct H(*mut core::ffi::c_void);
    impl Drop for H {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a handle WinHTTP gave us, closed once.
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }
    let nonnull = |h: *mut core::ffi::c_void, what: &str| -> anyhow::Result<H> {
        anyhow::ensure!(!h.is_null(), "{what}: {}", std::io::Error::last_os_error());
        Ok(H(h))
    };

    // SAFETY: plain WinHTTP calls; every handle is owned by an `H` and
    // closed in reverse order; buffers outlive the calls using them.
    unsafe {
        let session = nonnull(
            WinHttpOpen(w!("RSOverlay-update-check"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, 5_000, 5_000, 10_000, 10_000)?;
        let conn = nonnull(WinHttpConnect(session.0, w!("api.github.com"), INTERNET_DEFAULT_HTTPS_PORT, 0), "WinHttpConnect")?;
        let req = nonnull(
            WinHttpOpenRequest(conn.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE),
            "WinHttpOpenRequest",
        )?;
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n".encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0)?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut())?;
        let mut status = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )?;
        anyhow::ensure!(status == 200, "GitHub answered HTTP {status}");
        let mut body = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            anyhow::ensure!(body.len() <= MAX_BODY, "response too large");
        }
        Ok(String::from_utf8(body)?)
    }
}

#[cfg(not(windows))]
fn fetch(_path: &str) -> anyhow::Result<String> {
    anyhow::bail!("update check is Windows-only")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn beta_builds_are_offered_the_next_beta() {
        // The bug this guards: comparing against the bare Cargo version
        // ("0.1.0") hid every 0.1.0-beta.N from beta testers.
        let list = r#"[{"tag_name":"v0.1.0-beta.3","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v0.1.0-beta.3","draft":false,"prerelease":true}]"#;
        assert!(newest_from_json(list, "0.1.0-beta.2", true).is_some());
        assert!(newest_from_json(list, "0.1.0", true).is_none());
    }

    #[test]
    fn issue_link_carries_the_version() {
        assert!(new_issue_url().starts_with("https://github.com/affectioned/RSOverlay/issues/new?body="));
        assert!(new_issue_url().contains(CURRENT));
    }

    #[test]
    fn versions_order_like_semver() {
        assert!(v("v0.2.0") > v("0.1.9"));
        assert!(v("0.2.0") > v("0.2.0-beta.3"));
        assert!(v("0.2.0-beta.10") > v("0.2.0-beta.9"));
        assert!(v("0.2.0-rc.1") > v("0.2.0-beta.9"));
        assert!(v("1.0.0") == v("v1.0.0+build.5"));
        assert!(Version::parse("nightly").is_none());
        assert!(Version::parse("1.2").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
    }

    const LIST: &str = r#"[
        {"tag_name":"v0.3.0-beta.1","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v0.3.0-beta.1","draft":false,"prerelease":true},
        {"tag_name":"v0.2.0","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v0.2.0","draft":false,"prerelease":false},
        {"tag_name":"v9.0.0","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v9.0.0","draft":true,"prerelease":false},
        {"tag_name":"v8.0.0","html_url":"https://evil.example/RSOverlay/releases/tag/v8.0.0","draft":false,"prerelease":false},
        {"tag_name":"v0.1.0","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v0.1.0","draft":false,"prerelease":false}
    ]"#;

    #[test]
    fn picks_newest_stable_unless_testing() {
        assert_eq!(newest_from_json(LIST, "0.1.0", false).unwrap().tag, "v0.2.0");
        let beta = newest_from_json(LIST, "0.1.0", true).unwrap();
        assert_eq!(beta.tag, "v0.3.0-beta.1");
        assert!(beta.prerelease);
    }

    #[test]
    fn nothing_newer_drafts_and_foreign_links_ignored() {
        // v9 is a draft, v8 links off-site: neither is offered.
        assert_eq!(newest_from_json(LIST, "0.2.0", false), None);
        assert_eq!(newest_from_json("not json", "0.1.0", true), None);
    }
}

#[cfg(all(test, windows))]
mod live {
    /// Real request to api.github.com (run with --ignored).
    #[test]
    #[ignore]
    fn live_check_reaches_github() {
        let r = super::check(true).expect("GitHub reachable");
        println!("newest above {}: {r:?}", super::CURRENT);
    }
}
