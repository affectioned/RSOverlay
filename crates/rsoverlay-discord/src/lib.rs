//! Discord Rich Presence — the "Playing X with RSOverlay" status your
//! friends see when you're using it.
//!
//! Implementation: `discord-rich-presence` crate talks to Discord's
//! local IPC socket (no network, no API keys). Spawn once on startup,
//! update periodically to refresh elapsed time, drop on shutdown.
//!
//! Discord only accepts presence from a registered application: the ID is
//! [`BUILTIN_APP_ID`] (or `--discord-app-id`). Users turn it on / off live
//! in Settings ([`set_enabled`]). Without Discord running it's a silent
//! no-op that retries every few seconds, so starting Discord later works.

use anyhow::Result;
use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};


#[derive(Clone, Debug)]
pub enum RuntimeMode {
    Preview,
    VrPreview,
    Vr,
}

impl RuntimeMode {
    fn state_text(&self) -> &'static str {
        match self {
            Self::Preview => "Desktop Preview",
            Self::VrPreview => "VR Preview",
            Self::Vr => "In VR",
        }
    }
}

/// Handle returned by [`spawn`]. Drop it to tear the connection down
/// cleanly; keep it alive for the lifetime of the process.
pub struct DiscordHandle {
    tx: mpsc::UnboundedSender<Msg>,
}

enum Msg {
    SetMode(RuntimeMode),
    Shutdown,
}

impl DiscordHandle {
    /// Push a new runtime-mode update to the Discord worker. Safe to
    /// call from the UI thread. No-op if the worker has stopped.
    pub fn set_mode(&self, mode: RuntimeMode) {
        let _ = self.tx.send(Msg::SetMode(mode));
    }
}

impl Drop for DiscordHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// RSOverlay's own Discord application ID (Discord Developer Portal →
/// General Information → Application ID). Not a secret: it only names
/// the "Playing …" app. `None` = this build has no presence unless
/// `--discord-app-id` is given.
pub const BUILTIN_APP_ID: Option<&str> = Some("1575260540941635705");

/// Settings toggle, read by the worker once a second.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// A worker exists (an app ID was available at startup).
static AVAILABLE: AtomicBool = AtomicBool::new(false);
/// The worker currently holds a live presence.
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// Turn presence on / off live (Settings toggle). Cheap; call freely.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// What the Settings card shows under the toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No application ID in this build / on the command line.
    Unavailable,
    Off,
    /// On, but Discord isn't running (or refused the connection).
    Waiting,
    Connected,
}

pub fn status() -> Status {
    if !AVAILABLE.load(Ordering::Relaxed) {
        Status::Unavailable
    } else if !ENABLED.load(Ordering::Relaxed) {
        Status::Off
    } else if CONNECTED.load(Ordering::Relaxed) {
        Status::Connected
    } else {
        Status::Waiting
    }
}

/// Spawn the Discord presence worker on the given tokio runtime.
/// Returns a handle whose drop tears the connection down.
///
/// The worker follows [`set_enabled`]: on → connect (retrying every
/// [`RECONNECT`] while Discord isn't running, and again if Discord is
/// closed later), off → clear the presence and disconnect. Failures are
/// logged at debug level only; the overlay never stalls on Discord.
pub fn spawn(rt: &tokio::runtime::Handle, app_id: &str, initial_mode: RuntimeMode) -> DiscordHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let app_id = app_id.to_string();
    AVAILABLE.store(true, Ordering::Relaxed);
    rt.spawn(async move {
        let started_at = unix_now();
        let mut client: Option<Arc<Mutex<DiscordIpcClient>>> = None;
        let mut current_mode = initial_mode;
        let mut last_attempt: Option<std::time::Instant> = None;
        let mut last_push = std::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(Msg::SetMode(m)) => {
                        current_mode = m;
                        if let Some(c) = &client {
                            if push_activity(c, &current_mode, started_at).await.is_err() {
                                client = None;
                            }
                        }
                    }
                    Some(Msg::Shutdown) | None => {
                        if let Some(c) = client.take() {
                            disconnect(&c).await;
                        }
                        break;
                    }
                },
                _ = tick.tick() => {
                    let want = ENABLED.load(Ordering::Relaxed);
                    match (&client, want) {
                        (Some(c), false) => {
                            disconnect(c).await;
                            client = None;
                        }
                        (None, true) if last_attempt.is_none_or(|t| t.elapsed() >= RECONNECT) => {
                            last_attempt = Some(std::time::Instant::now());
                            client = connect(&app_id).await;
                            if let Some(c) = &client {
                                if push_activity(c, &current_mode, started_at).await.is_ok() {
                                    info!("Discord Rich Presence connected");
                                    last_push = std::time::Instant::now();
                                } else {
                                    client = None;
                                }
                            }
                        }
                        // Discord doesn't tick "elapsed" itself; repost. A
                        // failed post means Discord went away: reconnect later.
                        (Some(c), true) if last_push.elapsed() >= REFRESH => {
                            last_push = std::time::Instant::now();
                            if let Err(e) = push_activity(c, &current_mode, started_at).await {
                                debug!("Discord: refresh failed, will reconnect: {e:#}");
                                client = None;
                            }
                        }
                        _ => {}
                    }
                    CONNECTED.store(client.is_some(), Ordering::Relaxed);
                }
            }
        }
        CONNECTED.store(false, Ordering::Relaxed);
    });
    DiscordHandle { tx }
}

/// How often to retry while Discord isn't running.
const RECONNECT: std::time::Duration = std::time::Duration::from_secs(10);
/// How often to repost the presence (keeps elapsed time fresh).
const REFRESH: std::time::Duration = std::time::Duration::from_secs(30);

async fn connect(app_id: &str) -> Option<Arc<Mutex<DiscordIpcClient>>> {
    let mut c = match DiscordIpcClient::new(app_id) {
        Ok(c) => c,
        Err(e) => {
            warn!("Discord: client construction failed: {e:#}");
            return None;
        }
    };
    match c.connect() {
        Ok(()) => Some(Arc::new(Mutex::new(c))),
        Err(e) => {
            debug!("Discord IPC unavailable (is Discord running?): {e:#}");
            None
        }
    }
}

async fn disconnect(client: &Arc<Mutex<DiscordIpcClient>>) {
    let mut c = client.lock().await;
    if let Err(e) = c.clear_activity() {
        debug!("Discord: clear_activity: {e:#}");
    }
    let _ = c.close();
    info!("Discord Rich Presence disconnected");
}

async fn push_activity(
    client: &Arc<Mutex<DiscordIpcClient>>,
    mode: &RuntimeMode,
    started_at: i64,
) -> Result<()> {
    let activity = Activity::new()
        .details("Using RSOverlay")
        .state(mode.state_text())
        .timestamps(Timestamps::new().start(started_at))
        .assets(
            Assets::new()
                .large_image("rsoverlay-mark")
                .large_text("RSOverlay"),
        );
    client
        .lock()
        .await
        .set_activity(activity)
        .map_err(|e| anyhow::anyhow!("set_activity: {e}"))?;
    Ok(())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
