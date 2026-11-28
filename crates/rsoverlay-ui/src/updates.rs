//! Settings › Updates: tells you when a newer RSOverlay is on GitHub,
//! shows what's new, and (only when you click Install) downloads,
//! verifies and swaps in the new version with a progress bar. After an
//! update, the new version's notes are shown once. Steam builds hide the
//! updater (Steam updates them) but still show what's new.

use crate::{icons, theme};
use rsoverlay_core::config::UpdatesCfg;
use rsoverlay_core::update::{self, Release};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Re-check this often while RSOverlay runs.
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Failed(String),
}

/// In-app install, step by step.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Install {
    #[default]
    Idle,
    Downloading { done: u64, total: Option<u64> },
    Installing,
    /// Installed; takes effect on restart.
    Ready(String),
    Failed(String),
}

#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    /// (requested, include pre-releases)
    request: Mutex<Option<bool>>,
    install: Mutex<Install>,
    install_request: Mutex<Option<Release>>,
}

/// Background worker: checks at startup, every [`RECHECK`], on "Check
/// now", and installs when asked.
pub struct UpdateWorker {
    shared: Arc<Shared>,
    last_request: Option<Instant>,
}

impl UpdateWorker {
    fn spawn() -> Self {
        let shared = Arc::new(Shared::default());
        let weak = Arc::downgrade(&shared);
        let _ = std::thread::Builder::new().name("rso-updates".into()).spawn(move || {
            while let Some(s) = weak.upgrade() {
                let req = s.request.lock().ok().and_then(|mut r| r.take());
                if let Some(pre) = req {
                    set(&s.status, Status::Checking);
                    let result = match update::check(pre) {
                        Ok(Some(r)) => Status::Available(r),
                        Ok(None) => Status::UpToDate,
                        Err(e) => {
                            tracing::info!("update check failed: {e:#}");
                            Status::Failed(e.to_string())
                        }
                    };
                    set(&s.status, result);
                }
                let install = s.install_request.lock().ok().and_then(|mut r| r.take());
                if let Some(r) = install {
                    run_install(&s, &r);
                }
                drop(s);
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        Self { shared, last_request: None }
    }

    fn request(&mut self, prereleases: bool) {
        self.last_request = Some(Instant::now());
        if let Ok(mut r) = self.shared.request.lock() {
            *r = Some(prereleases);
        }
    }

    fn status(&self) -> Status {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn install_state(&self) -> Install {
        self.shared.install.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn start_install(&self, r: Release) {
        set(&self.shared.install, Install::Downloading { done: 0, total: None });
        if let Ok(mut q) = self.shared.install_request.lock() {
            *q = Some(r);
        }
    }
}

fn set<T>(slot: &Mutex<T>, v: T) {
    if let Ok(mut s) = slot.lock() {
        *s = v;
    }
}

fn run_install(s: &Shared, r: &Release) {
    run_install_with(s, r, update::download, update::install);
}

/// The install steps, with the download / swap passed in (tests fake
/// them; the app uses the real ones).
fn run_install_with(
    s: &Shared,
    r: &Release,
    download: impl FnOnce(&Release, &mut dyn FnMut(u64, Option<u64>)) -> anyhow::Result<Vec<u8>>,
    install: impl FnOnce(&[u8]) -> anyhow::Result<()>,
) {
    let result = download(r, &mut |done, total| set(&s.install, Install::Downloading { done, total })).and_then(|bytes| {
        set(&s.install, Install::Installing);
        install(&bytes)
    });
    match result {
        Ok(()) => {
            tracing::info!("update {} installed; restart to finish", r.tag);
            set(&s.install, Install::Ready(r.tag.clone()));
        }
        Err(e) => {
            tracing::warn!("update {} failed: {e:#}", r.tag);
            set(&s.install, Install::Failed(e.to_string()));
        }
    }
}

/// Update state on the DevView.
#[derive(Default)]
pub struct Updates {
    /// This build may check / install (false for Steam builds and tests).
    pub supported: bool,
    /// Screenshots: show the real card (and defaults) but never go online.
    pub offline: bool,
    pub prefs: UpdatesCfg,
    worker: Option<UpdateWorker>,
    pub status: Status,
    pub install: Install,
    /// Tag already announced with a toast (once per version per run).
    announced: String,
    /// "Check now" pressed.
    check_now: bool,
    /// Prefs used for the last check (re-check when they change).
    checked_with: Option<bool>,
    /// "Restart RSOverlay" clicked after an install; the app relaunches.
    pub restart_requested: bool,
    /// Just updated to this build: show its notes open, once.
    whats_new_open: bool,
    after_update_checked: bool,
}

impl Updates {
    /// First run after an update? Returns the new version once, and
    /// opens "What's new" in the card. Local dev builds never announce.
    pub fn take_updated_notice(&mut self) -> Option<String> {
        if std::mem::replace(&mut self.after_update_checked, true) || update::CURRENT.ends_with("-dev") {
            return None;
        }
        let last = std::mem::replace(&mut self.prefs.last_version, update::CURRENT.to_string());
        (!last.is_empty() && last != update::CURRENT).then(|| {
            self.whats_new_open = true;
            update::CURRENT.to_string()
        })
    }

    /// Drive the worker. Returns a release to announce (newly found,
    /// not skipped) at most once per version.
    pub fn tick(&mut self) -> Option<Release> {
        if let Some(w) = &self.worker {
            self.install = w.install_state();
        }
        if !self.supported || self.offline || update::is_dev_build() || (!self.prefs.check && !self.check_now) {
            return None;
        }
        let pre = self.prefs.prereleases;
        let w = self.worker.get_or_insert_with(UpdateWorker::spawn);
        let due = w.last_request.is_none_or(|t| t.elapsed() >= RECHECK);
        if std::mem::take(&mut self.check_now) || due || self.checked_with != Some(pre) {
            self.checked_with = Some(pre);
            w.request(pre);
        }
        self.status = w.status();
        match &self.status {
            Status::Available(r) if r.tag != self.prefs.skipped && r.tag != self.announced => {
                self.announced = r.tag.clone();
                Some(r.clone())
            }
            _ => None,
        }
    }

    /// Release to show (not skipped).
    pub fn available(&self) -> Option<&Release> {
        match &self.status {
            Status::Available(r) if r.tag != self.prefs.skipped => Some(r),
            _ => None,
        }
    }

    pub fn card(&mut self, ui: &mut egui::Ui, accent: egui::Color32) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            crate::views::card_header(ui, icons::LOAD, "Updates", accent);
            ui.add_space(6.0);
            // Selectable, so it can be copied into a bug report.
            ui.horizontal(|ui| {
                ui.label(theme::body_muted("Version"));
                ui.add(egui::Label::new(egui::RichText::new(update::version_line()).monospace().color(theme::FG_PRIMARY)).selectable(true));
            });
            // This version's own notes (built in, so offline is fine).
            if let Some(notes) = update::NOTES.filter(|n| !n.trim().is_empty()) {
                egui::CollapsingHeader::new(theme::body_muted("What's new in this version"))
                    .id_source("whats-new-current")
                    .default_open(self.whats_new_open)
                    .show(ui, |ui| notes_ui(ui, notes, "notes-current"));
            }
            if update::is_dev_build() && !self.offline {
                ui.label(theme::body_muted("Local build: update checks are off (only release builds update)."));
                ui.add_space(6.0);
                if ui.button("Open logs").clicked() {
                    update::open_logs_folder();
                }
                return;
            }
            if !self.supported {
                ui.label(theme::body_muted("Updates are handled by Steam."));
                ui.add_space(6.0);
                if ui.button("Open logs").on_hover_text("Attach rsoverlay.log when you report a problem").clicked() {
                    update::open_logs_folder();
                }
                return;
            }
            ui.add_space(4.0);
            match self.status.clone() {
                Status::Available(r) if r.tag != self.prefs.skipped => self.available_ui(ui, &r, accent),
                Status::Checking => {
                    ui.label(theme::body_muted("Checking…"));
                }
                Status::Failed(_) => {
                    ui.label(theme::body_muted("Couldn't reach GitHub. Try again later."));
                }
                Status::UpToDate | Status::Available(_) => {
                    ui.label(theme::body_muted("You're up to date."));
                }
                Status::Idle => {}
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Check now").clicked() {
                    self.check_now = true;
                }
                if ui.button("All releases").clicked() {
                    update::open_release_page(update::RELEASES_URL);
                }
            });
            ui.checkbox(&mut self.prefs.check, "Check for updates automatically")
                .on_hover_text("Asks GitHub at startup and every few hours. Only a notice: you choose when to update.");
            ui.checkbox(&mut self.prefs.prereleases, "Include test builds")
                .on_hover_text("Also tell me about pre-release builds for testers (may be less stable)");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Report a problem").on_hover_text("Opens a new bug report with your version filled in").clicked() {
                    update::open_issues();
                }
                if ui.button("Open logs").on_hover_text("The log files help us find bugs: attach rsoverlay.log to your report").clicked() {
                    update::open_logs_folder();
                }
            });
        });
    }

    /// A newer version: its notes, then Install (with progress) or the
    /// release page.
    fn available_ui(&mut self, ui: &mut egui::Ui, r: &Release, accent: egui::Color32) {
        let kind = if r.prerelease { "Test build" } else { "New version" };
        ui.label(egui::RichText::new(format!("{kind} {} is available", r.tag)).color(accent).size(theme::SIZE_BODY + 1.0));
        if !r.notes.trim().is_empty() {
            egui::CollapsingHeader::new(theme::body_muted(&format!("What's new in {}", r.tag)))
                .id_source(("whats-new", r.tag.as_str()))
                .show(ui, |ui| notes_ui(ui, &r.notes, "notes-available"));
        }
        match self.install.clone() {
            Install::Downloading { done, total } => {
                let mb = |b: u64| b as f32 / (1024.0 * 1024.0);
                let (frac, text) = match total.filter(|t| *t > 0) {
                    Some(t) => ((done as f32 / t as f32).clamp(0.0, 1.0), format!("Downloading {:.1} / {:.1} MB", mb(done), mb(t))),
                    None => (0.0, format!("Downloading {:.1} MB", mb(done))),
                };
                ui.add(egui::ProgressBar::new(frac).text(text).animate(total.is_none()));
            }
            Install::Installing => {
                ui.add(egui::ProgressBar::new(1.0).text("Checking and installing…"));
            }
            Install::Ready(tag) => {
                ui.label(egui::RichText::new(format!("{tag} is installed.")).color(accent));
                ui.horizontal(|ui| {
                    if ui.button("Restart RSOverlay").on_hover_text("Closes and reopens RSOverlay on the new version. Your settings and layouts are kept.").clicked() {
                        self.restart_requested = true;
                    }
                    ui.label(theme::caption("or it switches next time you start it"));
                });
            }
            Install::Failed(e) => {
                ui.label(egui::RichText::new(format!("Update didn't finish: {e}")).color(theme::DANGER).size(theme::SIZE_CAPTION + 1.0));
                ui.label(theme::caption("Nothing was changed. You can get it from the release page instead."));
                self.update_buttons(ui, r, true);
            }
            Install::Idle => self.update_buttons(ui, r, false),
        }
    }

    fn update_buttons(&mut self, ui: &mut egui::Ui, r: &Release, failed: bool) {
        ui.horizontal(|ui| {
            if r.exe_url.is_some() {
                let label = if failed { "Try again" } else { "Install update" };
                if ui.button(label).on_hover_text("Downloads the new version, checks it, and swaps it in. You restart when it suits you.").clicked() {
                    if let Some(w) = &self.worker {
                        w.start_install(r.clone());
                        self.install = Install::Downloading { done: 0, total: None };
                    }
                }
            }
            if ui.button("Release page").on_hover_text("Opens the release in your browser").clicked() {
                update::open_release_page(&r.url);
            }
            if ui.button("Skip this version").clicked() {
                self.prefs.skipped = r.tag.clone();
            }
        });
        if r.exe_url.is_none() {
            ui.label(theme::caption("This release can only be downloaded from its page."));
        }
    }
}

/// Release notes (GitHub Markdown) as plain, readable text: headings
/// bold, bullets kept, emphasis markers dropped. Scrolls when long.
pub fn notes_ui(ui: &mut egui::Ui, md: &str, id: &str) {
    egui::ScrollArea::vertical().id_source(id).max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        for line in md.lines() {
            let t = line.trim_end();
            // Screenshots are for the release page; the panel shows text.
            if t.trim_start().starts_with("![") {
                continue;
            }
            if t.is_empty() {
                ui.add_space(4.0);
                continue;
            }
            let clean = |s: &str| s.replace("**", "").replace('`', "");
            if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("### ")) {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(clean(h)).strong().color(theme::FG_PRIMARY));
            } else if let Some(b) = t.trim_start().strip_prefix("- ").or_else(|| t.trim_start().strip_prefix("* ")) {
                let indent = (t.len() - t.trim_start().len()) as f32 * 4.0;
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(indent);
                    ui.label(theme::body_muted(&format!("• {}", clean(b))));
                });
            } else {
                ui.label(theme::body_muted(&clean(t)));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> Release {
        Release { tag: "v9.9.9".into(), url: String::new(), prerelease: true, notes: String::new(), exe_url: None, sha_url: None }
    }

    fn state(s: &Shared) -> Install {
        s.install.lock().unwrap().clone()
    }

    #[test]
    fn install_walks_download_progress_then_ready() {
        let s = Shared::default();
        let mut mid = None;
        run_install_with(
            &s,
            &release(),
            |_, progress| {
                progress(5, Some(10));
                mid = Some(state(&s));
                progress(10, Some(10));
                Ok(b"MZ new".to_vec())
            },
            |bytes| {
                assert_eq!(bytes, b"MZ new");
                assert_eq!(state(&s), Install::Installing, "installing shown while swapping");
                Ok(())
            },
        );
        assert_eq!(mid, Some(Install::Downloading { done: 5, total: Some(10) }), "progress reaches the card");
        assert_eq!(state(&s), Install::Ready("v9.9.9".into()));
    }

    #[test]
    fn install_failures_end_in_failed_and_skip_the_swap() {
        let s = Shared::default();
        let mut swapped = false;
        run_install_with(&s, &release(), |_, _| Err(anyhow::anyhow!("download is damaged (checksum mismatch)")), |_| {
            swapped = true;
            Ok(())
        });
        assert!(!swapped, "a bad download is never installed");
        assert!(matches!(state(&s), Install::Failed(e) if e.contains("checksum")));

        let s = Shared::default();
        run_install_with(&s, &release(), |_, _| Ok(b"MZ".to_vec()), |_| Err(anyhow::anyhow!("can't replace")));
        assert!(matches!(state(&s), Install::Failed(e) if e.contains("replace")));
    }

    #[test]
    fn notice_only_once_and_only_after_a_real_version_change() {
        let mut u = Updates::default();
        // Fresh install (no previous version): no notice, but remembered.
        assert_eq!(u.take_updated_notice(), None);
        if !update::CURRENT.ends_with("-dev") {
            assert_eq!(u.prefs.last_version, update::CURRENT);
        }
        // Asking again in the same run: nothing.
        assert_eq!(u.take_updated_notice(), None);
    }
}
