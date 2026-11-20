//! RSOverlay application entry point.
//!
//! Two run modes:
//! - `--preview` (default): no HMD required. Opens a desktop window and
//!   presents the overlay texture into it so the full pipeline can be
//!   iterated on without SteamVR.
//! - `--vr`: initializes OpenVR, registers an overlay, positions it 2m in
//!   front of the HMD. Texture submission lands next milestone.
//!
//! A tokio runtime is started once in `main` and lives for the entire
//! process. It hosts the notification websocket server (and any other async
//! work later), independent of the UI mode.

mod preview;
mod vr;

use anyhow::Result;
use clap::Parser;
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::{spawn as spawn_notify, NotifyConfig};
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rsoverlay", version, about = "Open-source SteamVR overlay")]
struct Args {
    /// Force desktop preview mode (no HMD required). Default.
    #[arg(long, conflicts_with = "vr")]
    preview: bool,

    /// Run in SteamVR. Requires a running compositor + HMD.
    #[arg(long, conflicts_with = "preview")]
    vr: bool,

    /// Notification server bind address.
    #[arg(long, default_value = "127.0.0.1:42069")]
    notify_bind: SocketAddr,

    /// Disable the notification server entirely.
    #[arg(long)]
    no_notify: bool,

    /// Desktop monitor index to mirror into the preview overlay. Use
    /// `--list-monitors` to see the enumeration. Default: 0.
    #[arg(long, default_value_t = 0)]
    monitor: u32,

    /// Disable desktop capture.
    #[arg(long)]
    no_mirror: bool,

    /// Print detected monitors and exit.
    #[arg(long)]
    list_monitors: bool,
}

impl Args {
    fn mode(&self) -> RuntimeMode {
        if self.vr {
            RuntimeMode::Vr
        } else {
            RuntimeMode::Preview
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args = Args::parse();

    if args.list_monitors {
        for m in rsoverlay_capture::enumerate_monitors()? {
            println!("[{}] {} {}x{}", m.index, m.device_name, m.width, m.height);
        }
        return Ok(());
    }

    let mode = args.mode();
    info!("starting RSOverlay in {:?} mode", mode);

    // One multi-thread tokio runtime owns the notification server and any
    // other async background work for the process. We hold a Handle so the
    // UI thread can hand things off without touching the runtime directly.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("rsoverlay-tokio")
        .build()?;
    let rt_handle = rt.handle().clone();

    let notify_rx = if args.no_notify {
        None
    } else {
        let cfg = NotifyConfig { bind: args.notify_bind };
        Some(spawn_notify(&rt_handle, cfg))
    };

    let capture_cfg = preview::CaptureCfg {
        enabled: !args.no_mirror,
        monitor_index: args.monitor,
    };

    let result = match mode {
        RuntimeMode::Preview => preview::run(notify_rx, capture_cfg),
        RuntimeMode::Vr => vr::run(notify_rx, rt_handle.clone()),
    };

    // Give the runtime a bounded shutdown window so background tasks drain.
    rt.shutdown_timeout(std::time::Duration::from_millis(250));
    result
}
