//! Discord Rich Presence — the "Playing X with RSOverlay" status your
//! friends see when you're using it.
//!
//! Implementation: `discord-rich-presence` crate talks to Discord's
//! local IPC socket (no network, no API keys). Spawn once on startup,
//! update periodically to refresh elapsed time, drop on shutdown.
//!
//! Discord application ID: the RSOverlay app registration on the
//! Discord Developer Portal. Users without Discord running get a
//! silent no-op — the IPC connect just fails and we log once.

use anyhow::Result;
use discord_rich_presence::activity::{Activity, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

/// Default Discord application ID for the RSOverlay project. Replace
/// with a real registered app ID before public release — this one is
/// a placeholder taken from Discord's examples so presence shows but
/// without a custom app name.
pub const DEFAULT_APP_ID: &str = "1300000000000000000";

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

/// Spawn the Discord presence worker on the given tokio runtime.
/// Returns a handle whose drop tears the connection down. Connect
/// failures (Discord not running, socket permission issue) are logged
/// once and then silently swallowed — we never want the overlay to
/// stall because the Discord daemon is unhappy.
pub fn spawn(
    rt: &tokio::runtime::Handle,
    app_id: &str,
    initial_mode: RuntimeMode,
) -> DiscordHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let app_id = app_id.to_string();
    rt.spawn(async move {
        let started_at = unix_now();
        let client = Arc::new(Mutex::new(match DiscordIpcClient::new(&app_id) {
            Ok(c) => c,
            Err(e) => {
                warn!("Discord: client construction failed: {e:#}");
                return;
            }
        }));
        let connected = connect_with_retry(&client).await;
        if !connected {
            // Give up quietly — Discord just isn't available right now.
            return;
        }
        let mut current_mode = initial_mode;
        if let Err(e) = push_activity(&client, &current_mode, started_at).await {
            warn!("Discord: initial presence update failed: {e:#}");
        } else {
            info!("Discord Rich Presence connected");
        }

        // Refresh presence periodically — Discord doesn't auto-tick the
        // elapsed time display, you have to repost. 30 s is plenty for
        // "elapsed" granularity without hammering the IPC socket.
        let mut refresh = tokio::time::interval(std::time::Duration::from_secs(30));
        refresh.tick().await; // first tick is immediate; consume it
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(Msg::SetMode(m)) => {
                            current_mode = m;
                            if let Err(e) = push_activity(&client, &current_mode, started_at).await {
                                debug!("Discord: mode update failed: {e:#}");
                            }
                        }
                        Some(Msg::Shutdown) | None => {
                            if let Err(e) = client.lock().await.clear_activity() {
                                debug!("Discord: clear_activity on shutdown: {e:#}");
                            }
                            let _ = client.lock().await.close();
                            info!("Discord Rich Presence disconnected");
                            break;
                        }
                    }
                }
                _ = refresh.tick() => {
                    if let Err(e) = push_activity(&client, &current_mode, started_at).await {
                        debug!("Discord: periodic refresh failed: {e:#}");
                    }
                }
            }
        }
    });
    DiscordHandle { tx }
}

async fn connect_with_retry(client: &Arc<Mutex<DiscordIpcClient>>) -> bool {
    for attempt in 0..3u32 {
        // Scope the lock so the !Send error is dropped before the await below.
        let outcome = { client.lock().await.connect().is_ok() };
        if outcome {
            return true;
        }
        if attempt == 0 {
            debug!("Discord IPC connect attempt {attempt} failed; retrying");
        }
        tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
    }
    debug!("Discord IPC unavailable (is Discord running?); presence disabled");
    false
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
