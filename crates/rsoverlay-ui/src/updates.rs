//! Settings › Updates: tells you when a newer RSOverlay is on GitHub.
//! Never downloads or installs anything; "Get update" opens the release
//! page. Steam builds hide all of this (Steam updates them).

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

#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    /// (requested, include pre-releases)
    request: Mutex<Option<bool>>,
}

/// Background checker: one thread, woken at startup, every
/// [`RECHECK`], or when "Check now" is pressed.
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
                    if let Ok(mut st) = s.status.lock() {
                        *st = Status::Checking;
                    }
                    let result = match update::check(pre) {
                        Ok(Some(r)) => Status::Available(r),
                        Ok(None) => Status::UpToDate,
                        Err(e) => {
                            tracing::info!("update check failed: {e:#}");
                            Status::Failed(e.to_string())
                        }
                    };
                    if let Ok(mut st) = s.status.lock() {
                        *st = result;
                    }
                }
                drop(s);
                std::thread::sleep(Duration::from_millis(500));
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
}

/// Update state on the DevView.
#[derive(Default)]
pub struct Updates {
    /// This build may check (false for Steam builds and in tests).
    pub supported: bool,
    pub prefs: UpdatesCfg,
    worker: Option<UpdateWorker>,
    pub status: Status,
    /// Tag already announced with a toast (once per version per run).
    announced: String,
    /// "Check now" pressed.
    check_now: bool,
    /// Prefs used for the last check (re-check when they change).
    checked_with: Option<bool>,
}

impl Updates {
    /// Drive the worker. Returns a release to announce (newly found,
    /// not skipped) at most once per version.
    pub fn tick(&mut self) -> Option<Release> {
        if !self.supported || (!self.prefs.check && !self.check_now) {
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
                Status::Available(r) if r.tag != self.prefs.skipped => {
                    let kind = if r.prerelease { "Test build" } else { "New version" };
                    ui.label(egui::RichText::new(format!("{kind} {} is available", r.tag)).color(accent).size(theme::SIZE_BODY + 1.0));
                    ui.horizontal(|ui| {
                        if ui.button("Get update").on_hover_text("Opens the release page in your browser").clicked() {
                            update::open_release_page(&r.url);
                        }
                        if ui.button("Skip this version").clicked() {
                            self.prefs.skipped = r.tag.clone();
                        }
                    });
                    ui.label(egui::RichText::new("Nothing is installed automatically. Close RSOverlay, then run the new version.")
                        .size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
                }
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
                if ui.button("Report a problem").on_hover_text("Opens the issue tracker in your browser").clicked() {
                    update::open_issues();
                }
                if ui.button("Open logs").on_hover_text("The log files help us find bugs: attach rsoverlay.log to your report").clicked() {
                    update::open_logs_folder();
                }
            });
        });
    }
}
