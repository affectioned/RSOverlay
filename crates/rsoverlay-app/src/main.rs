//! RSOverlay application entry point.
//!
//! Two run modes:
//! - `--preview` (default): no HMD required. Opens a desktop window and
//!   presents the overlay texture into it so the full pipeline can be
//!   iterated on without SteamVR.
//! - `--vr`: initializes OpenVR, registers an overlay, positions it 2m in
//!   front of the HMD. Texture submission lands next milestone.

mod preview;
mod vr;

use anyhow::Result;
use clap::Parser;
use rsoverlay_core::RuntimeMode;
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
}

impl Args {
    fn mode(&self) -> RuntimeMode {
        if self.vr {
            RuntimeMode::Vr
        } else {
            // Preview is the default to make iteration painless.
            RuntimeMode::Preview
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();

    let args = Args::parse();
    let mode = args.mode();
    info!("starting RSOverlay in {:?} mode", mode);

    match mode {
        RuntimeMode::Preview => preview::run(),
        RuntimeMode::Vr => vr::run(),
    }
}
