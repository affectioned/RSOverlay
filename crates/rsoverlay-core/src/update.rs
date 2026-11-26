//! Updates (non-Steam builds): asks GitHub Releases whether a newer
//! RSOverlay exists and, only when the user clicks Install, downloads the
//! new exe, checks its SHA-256 against the release's checksum file and
//! swaps it in. Nothing happens automatically. Steam builds don't use
//! this (Steam updates them).
//!
//! Privacy: anonymous HTTPS GETs to github.com with a generic User-Agent.
//! Nothing about the user or their machine is sent.

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

/// This version's release notes (Markdown), built in by the release
/// workflow so "What's new" works offline after an update.
pub const NOTES: Option<&str> = option_env!("RSO_NOTES");

/// Downloads must come from this repository's release assets.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/affectioned/RSOverlay/releases/download/";

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
    /// Release notes (Markdown), shown as "What's new".
    pub notes: String,
    /// The bare exe + its `.sha256` (releases from v0.2.0-beta.2 on).
    /// `None` → the update can only be fetched from the release page.
    pub exe_url: Option<String>,
    pub sha_url: Option<String>,
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
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Gh {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    let list: Vec<Gh> = serde_json::from_str(json).ok()?;
    let current = Version::parse(current)?;
    list.into_iter()
        .filter(|r| !r.draft && (prereleases || !r.prerelease))
        .filter(|r| r.html_url.starts_with(&format!("{RELEASES_URL}/")))
        .filter_map(|r| Version::parse(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| *v > current)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| {
            let asset = |suffix: &str| {
                r.assets
                    .iter()
                    .find(|a| a.name.ends_with(suffix) && a.browser_download_url.starts_with(DOWNLOAD_PREFIX))
                    .map(|a| a.browser_download_url.clone())
            };
            let (exe_url, sha_url) = (asset("-windows-x64.exe"), asset("-windows-x64.exe.sha256"));
            let both = exe_url.is_some() && sha_url.is_some();
            Release {
                notes: r.body.unwrap_or_default(),
                exe_url: exe_url.filter(|_| both),
                sha_url: sha_url.filter(|_| both),
                tag: r.tag_name,
                url: r.html_url,
                prerelease: r.prerelease,
            }
        })
}

/// Ask GitHub for the newest release above this build. Blocking (run it
/// off the UI thread); ~1 request, 10 s timeout.
pub fn check(prereleases: bool) -> anyhow::Result<Option<Release>> {
    let body = http_get("api.github.com", &format!("/repos/{REPO}/releases?per_page=20"), MAX_BODY, &mut |_, _| {})?;
    Ok(newest_from_json(&String::from_utf8(body)?, CURRENT, prereleases))
}

/// Largest exe we'll download.
const MAX_EXE: usize = 64 << 20;

/// Download `r`'s exe, reporting (bytes so far, total if known), and
/// check it against the release's SHA-256 file. Returns the verified
/// bytes; nothing is written to disk.
pub fn download(r: &Release, progress: &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> {
    download_via(r, &mut |url, max, p| get_url(url, max, p), progress)
}

/// A download transport: (url, max bytes, progress) → body. Real
/// downloads use HTTPS to GitHub; tests pass a fake.
pub type Transport<'a> = dyn FnMut(&str, usize, &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> + 'a;

/// [`download`] over any transport: URLs are checked against this repo's
/// release downloads *before* the transport is asked for them.
pub fn download_via(r: &Release, get: &mut Transport<'_>, progress: &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> {
    let (Some(exe_url), Some(sha_url)) = (&r.exe_url, &r.sha_url) else {
        anyhow::bail!("this release has no in-app download");
    };
    for u in [exe_url, sha_url] {
        anyhow::ensure!(u.starts_with(DOWNLOAD_PREFIX), "refusing to download from {u}");
    }
    let sha_text = String::from_utf8(get(sha_url, 4096, &mut |_, _| {})?)?;
    let want = parse_sha256(&sha_text).ok_or_else(|| anyhow::anyhow!("checksum file unreadable"))?;
    let bytes = get(exe_url, MAX_EXE, progress)?;
    let got = sha256(&bytes)?;
    anyhow::ensure!(got == want, "download is damaged (checksum mismatch)");
    anyhow::ensure!(bytes.starts_with(b"MZ"), "download isn't a Windows program");
    Ok(bytes)
}

/// First 64-hex-digit token of a `sha256sum` / `certutil` style file.
fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let hex = text.split_whitespace().find(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()))?;
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Swap the running exe for `bytes`: the current one is renamed to
/// `rsoverlay.old.exe` (Windows allows renaming a running exe) and the new
/// one written in its place. On any failure the original is put back.
pub fn install(bytes: &[u8]) -> anyhow::Result<()> {
    install_at(&std::env::current_exe()?, bytes)
}

/// [`install`] for any exe path (tests use a throwaway copy).
pub fn install_at(exe: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    let exe = exe.to_path_buf();
    let old = exe.with_extension("old.exe");
    let tmp = exe.with_extension("new.exe");
    std::fs::write(&tmp, bytes).map_err(|e| anyhow::anyhow!("can't write next to {}: {e}", exe.display()))?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&exe, &old) {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("can't replace {}: {e}", exe.display());
    }
    if let Err(e) = std::fs::rename(&tmp, &exe) {
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("can't install the new version: {e}");
    }
    Ok(())
}

/// Remove the previous exe left by an update (call at startup).
pub fn cleanup_after_update() {
    if let Ok(exe) = std::env::current_exe() {
        cleanup_at(&exe);
    }
}

/// [`cleanup_after_update`] for any exe path.
pub fn cleanup_at(exe: &std::path::Path) {
    let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    let _ = std::fs::remove_file(exe.with_extension("new.exe"));
}

/// Start the (new) exe with this process's arguments plus
/// `--after-update <pid>`, so it waits for us to exit before starting
/// (two copies would fight over the SteamVR overlay keys).
pub fn spawn_updated() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).args(restart_args(std::env::args().skip(1), std::process::id())).spawn()?;
    Ok(())
}

/// The arguments the restarted copy gets: ours, minus any earlier
/// `--after-update <pid>`, plus `--after-update <our pid>`.
pub fn restart_args(args: impl IntoIterator<Item = String>, pid: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--after-update" {
            it.next();
        } else {
            out.push(a);
        }
    }
    out.push("--after-update".into());
    out.push(pid.to_string());
    out
}

/// `--after-update <pid>`: wait (up to 15 s) for the old process to exit.
pub fn wait_for_previous(pid: u32) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        // SAFETY: a process handle we own and close.
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                WaitForSingleObject(h, 15_000);
                let _ = CloseHandle(h);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}

/// GET a full https:// URL (only this repo's release downloads).
fn get_url(url: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(url.starts_with(DOWNLOAD_PREFIX), "refusing to download from {url}");
    let path = &url["https://github.com".len()..];
    http_get("github.com", path, max, progress)
}

#[cfg(windows)]
fn sha256(data: &[u8]) -> anyhow::Result<[u8; 32]> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    // SAFETY: CNG one-shot hash with the SHA-256 pseudo-handle (Win10+).
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    anyhow::ensure!(st.is_ok(), "SHA-256 failed: {st:?}");
    Ok(out)
}

#[cfg(not(windows))]
fn sha256(_data: &[u8]) -> anyhow::Result<[u8; 32]> {
    anyhow::bail!("updates are Windows-only")
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

/// Largest API response accepted (the release list is a few tens of KB).
const MAX_BODY: usize = 2 << 20;

/// HTTPS GET `host` + `path` (redirects followed, e.g. to GitHub's
/// download CDN), up to `max` bytes, reporting progress.
#[cfg(windows)]
fn http_get(host: &str, path: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> {
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
        WinHttpSetTimeouts(session.0, 5_000, 5_000, 15_000, 15_000)?;
        let conn = nonnull(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0), "WinHttpConnect")?;
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
        // Content-Length, when the server sends it (for the progress bar).
        let mut total = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        let total = WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut total as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .ok()
        .map(|()| u64::from(total));
        let mut body = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        progress(0, total);
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            anyhow::ensure!(body.len() <= max, "response too large");
            progress(body.len() as u64, total);
        }
        Ok(body)
    }
}

#[cfg(not(windows))]
fn http_get(_host: &str, _path: &str, _max: usize, _progress: &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("updates are Windows-only")
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
    fn release_assets_and_notes_are_read_only_from_this_repo() {
        let list = r##"[{"tag_name":"v9.0.0","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v9.0.0","body":"New\n- thing",
            "assets":[
              {"name":"RSOverlay-v9.0.0-windows-x64.exe","browser_download_url":"https://github.com/affectioned/RSOverlay/releases/download/v9.0.0/RSOverlay-v9.0.0-windows-x64.exe"},
              {"name":"RSOverlay-v9.0.0-windows-x64.exe.sha256","browser_download_url":"https://github.com/affectioned/RSOverlay/releases/download/v9.0.0/RSOverlay-v9.0.0-windows-x64.exe.sha256"}]},
          {"tag_name":"v8.0.0","html_url":"https://github.com/affectioned/RSOverlay/releases/tag/v8.0.0",
            "assets":[
              {"name":"RSOverlay-v8.0.0-windows-x64.exe","browser_download_url":"https://evil.example/x.exe"},
              {"name":"RSOverlay-v8.0.0-windows-x64.exe.sha256","browser_download_url":"https://github.com/affectioned/RSOverlay/releases/download/v8.0.0/x.sha256"}]}]"##;
        let r = newest_from_json(list, "1.0.0", false).unwrap();
        assert_eq!(r.tag, "v9.0.0");
        assert!(r.notes.contains("thing"));
        assert!(r.exe_url.is_some() && r.sha_url.is_some());
        // An off-site exe means no in-app install at all for that release.
        let l8 = list.replace("v9.0.0", "v7.0.0");
        let r8 = newest_from_json(&l8, "1.0.0", false).unwrap();
        assert_eq!(r8.tag, "v8.0.0");
        assert!(r8.exe_url.is_none() && r8.sha_url.is_none());
    }

    #[test]
    fn checksum_files_parse() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256(&format!("{h}  RSOverlay.exe\n")).unwrap(), [0xaa; 32]);
        assert!(parse_sha256("nope").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc").
        let d = sha256(b"abc").unwrap();
        assert_eq!(d[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(d[28..], [0xf2, 0x00, 0x15, 0xad]);
    }

    #[test]
    fn downloads_refuse_other_hosts() {
        assert!(get_url("https://evil.example/x.exe", 10, &mut |_, _| {}).is_err());
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

    /// Real release-asset download through GitHub's redirect (run with --ignored).
    #[test]
    #[ignore]
    fn live_download_follows_the_redirect() {
        let url = "https://github.com/affectioned/RSOverlay/releases/download/v0.2.0-beta.1/RSOverlay-v0.2.0-beta.1-windows-x64.zip.sha256";
        let mut seen = Vec::new();
        let body = super::get_url(url, 4096, &mut |d, t| seen.push((d, t))).expect("downloaded");
        let text = String::from_utf8(body).unwrap();
        println!("{text}");
        assert!(super::parse_sha256(&text).is_some());
        assert!(!seen.is_empty());
    }
}


/// Sandbox tests for the whole updater: fake release server (transport),
/// throwaway exe in a temp folder, real hashing / swapping / waiting.
#[cfg(all(test, windows))]
mod sandbox {
    use super::*;
    use std::path::PathBuf;

    const BASE: &str = "https://github.com/affectioned/RSOverlay/releases/download/v9.9.9/";

    fn release() -> Release {
        Release {
            tag: "v9.9.9".into(),
            url: format!("{RELEASES_URL}/tag/v9.9.9"),
            prerelease: true,
            notes: String::new(),
            exe_url: Some(format!("{BASE}RSOverlay-v9.9.9-windows-x64.exe")),
            sha_url: Some(format!("{BASE}RSOverlay-v9.9.9-windows-x64.exe.sha256")),
        }
    }

    /// A "new version": MZ header + filler, and its checksum file.
    fn new_exe() -> (Vec<u8>, String) {
        let mut exe = b"MZ".to_vec();
        exe.extend((0..200_000u32).map(|i| (i % 251) as u8));
        let hex: String = sha256(&exe).unwrap().iter().map(|b| format!("{b:02x}")).collect();
        (exe, format!("{hex} *RSOverlay-v9.9.9-windows-x64.exe\n"))
    }

    /// Fake release server: serves `exe` / `sha` in 64 KB chunks with
    /// progress, records what was asked for.
    fn server<'a>(exe: &'a [u8], sha: &'a str, asked: &'a mut Vec<String>) -> Box<Transport<'a>> {
        Box::new(move |url: &str, max: usize, progress: &mut dyn FnMut(u64, Option<u64>)| {
            asked.push(url.to_string());
            let body = if url.ends_with(".sha256") { sha.as_bytes() } else { exe };
            anyhow::ensure!(body.len() <= max, "too large");
            let total = Some(body.len() as u64);
            for end in (0..=body.len()).step_by(64 * 1024).chain([body.len()]) {
                progress(end as u64, total);
            }
            Ok(body.to_vec())
        })
    }

    fn temp_install() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rso-updater-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("rsoverlay.exe");
        std::fs::write(&exe, b"MZ old version").unwrap();
        (dir, exe)
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn full_update_flow_download_verify_swap_cleanup() {
        let (exe_bytes, sha) = new_exe();
        let mut asked = Vec::new();
        let mut seen = Vec::new();
        let bytes = {
            let mut get = server(&exe_bytes, &sha, &mut asked);
            download_via(&release(), &mut *get, &mut |d, t| seen.push((d, t))).expect("verified download")
        };
        assert_eq!(bytes, exe_bytes);
        assert_eq!(asked.len(), 2, "checksum + exe");
        assert!(asked[0].ends_with(".sha256"), "checksum fetched first");
        // Progress ran from 0 to the full size, with the total known.
        assert_eq!(seen.first(), Some(&(0, Some(exe_bytes.len() as u64))));
        assert_eq!(seen.last(), Some(&(exe_bytes.len() as u64, Some(exe_bytes.len() as u64))));

        let (dir, exe) = temp_install();
        install_at(&exe, &bytes).expect("installed");
        assert_eq!(std::fs::read(&exe).unwrap(), exe_bytes, "new version in place");
        assert_eq!(std::fs::read(exe.with_extension("old.exe")).unwrap(), b"MZ old version", "old kept until restart");
        assert!(!exe.with_extension("new.exe").exists());
        // Next start tidies up.
        cleanup_at(&exe);
        assert!(!exe.with_extension("old.exe").exists());
        assert!(exe.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tampered_or_wrong_downloads_are_rejected() {
        let (exe_bytes, sha) = new_exe();
        // One byte changed in transit.
        let mut bad = exe_bytes.clone();
        bad[1000] ^= 1;
        let mut asked = Vec::new();
        let mut get = server(&bad, &sha, &mut asked);
        let e = download_via(&release(), &mut *get, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("checksum"), "{e}");

        // Matching checksum, but not a Windows program.
        let junk = b"not an exe".to_vec();
        let hex: String = sha256(&junk).unwrap().iter().map(|b| format!("{b:02x}")).collect();
        let mut asked = Vec::new();
        let mut get = server(&junk, &hex, &mut asked);
        assert!(download_via(&release(), &mut *get, &mut |_, _| {}).is_err());

        // Unreadable checksum file.
        let mut asked = Vec::new();
        let mut get = server(&exe_bytes, "garbage", &mut asked);
        assert!(download_via(&release(), &mut *get, &mut |_, _| {}).is_err());
    }

    #[test]
    fn off_site_urls_never_reach_the_transport() {
        let mut r = release();
        r.exe_url = Some("https://evil.example/RSOverlay.exe".into());
        let called = std::cell::Cell::new(false);
        let mut get = |_: &str, _: usize, _: &mut dyn FnMut(u64, Option<u64>)| -> anyhow::Result<Vec<u8>> {
            called.set(true);
            Ok(Vec::new())
        };
        assert!(download_via(&r, &mut get, &mut |_, _| {}).is_err());
        // No assets at all: nothing to download.
        let mut r = release();
        r.exe_url = None;
        assert!(download_via(&r, &mut get, &mut |_, _| {}).is_err());
        assert!(!called.get(), "the transport was never asked");
    }

    #[test]
    fn failed_swap_leaves_the_original_untouched() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, exe) = temp_install();
        // Hold the exe open without FILE_SHARE_DELETE: renaming it fails,
        // like an exe locked by antivirus or another process.
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&exe).unwrap();
        let e = install_at(&exe, b"MZ new").unwrap_err();
        drop(lock);
        assert!(e.to_string().contains("can't replace"), "{e}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old version", "original intact");
        assert!(!exe.with_extension("new.exe").exists(), "temp file removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restart_args_carry_ours_and_replace_the_wait_pid() {
        let args = ["--vr", "--after-update", "111", "--no-osc"].map(String::from);
        assert_eq!(restart_args(args, 222), ["--vr", "--no-osc", "--after-update", "222"].map(String::from));
    }

    #[test]
    fn restarted_copy_waits_for_the_old_one_to_exit() {
        // A real process that lives ~1 s.
        let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 2 127.0.0.1 >NUL"]).spawn().unwrap();
        let t = std::time::Instant::now();
        wait_for_previous(child.id());
        let waited = t.elapsed();
        let _ = child.wait();
        assert!(waited >= std::time::Duration::from_millis(500), "returned before the process exited: {waited:?}");
        assert!(waited < std::time::Duration::from_secs(10));
        // A pid that's gone: no wait at all.
        let t = std::time::Instant::now();
        wait_for_previous(u32::MAX - 7);
        assert!(t.elapsed() < std::time::Duration::from_millis(200));
    }
}
