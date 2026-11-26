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

mod mirror_source;
mod device_icons;
mod preview;
mod preview_dev;
mod preview_mock;
mod preview_vr;
mod snapshot;
mod vr;

/// GitHub builds tell users about new releases; the Steam build doesn't
/// (Steam updates it).
pub(crate) const UPDATE_CHECKS: bool = !cfg!(feature = "steam");

use anyhow::Result;
use clap::Parser;
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::{spawn as spawn_notify, NotifyConfig};
use rsoverlay_osc::spawn as spawn_osc;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rsoverlay", version, about = "Open-source SteamVR overlay")]
struct Args {
    /// Force desktop preview mode (no HMD required). Default.
    #[arg(long, conflicts_with_all = ["vr", "vr_preview"])]
    preview: bool,

    /// Run in SteamVR. Requires a running compositor + HMD.
    #[arg(long, conflicts_with_all = ["preview", "vr_preview"])]
    vr: bool,

    /// Render a SteamVR-style 3D preview of the overlay — floor grid,
    /// 100° FOV camera, overlay quad at a wrist position. Mouse drag
    /// looks around. No HMD required.
    #[arg(long, conflicts_with_all = ["preview", "vr"])]
    vr_preview: bool,

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

    /// Force the CPU readback capture path instead of the default
    /// GPU-to-GPU shared-handle path. Useful for A/B latency comparison
    /// and for working around driver bugs we haven't shaken out yet.
    #[arg(long)]
    no_gpu_mirror: bool,

    /// Print detected monitors and exit.
    #[arg(long)]
    list_monitors: bool,

    /// Mirror a single window instead of a monitor — first window whose
    /// title contains this text (case-insensitive). Uses Windows
    /// Graphics Capture, so occluded windows still mirror. Switchable
    /// at runtime from the Desktop card's source picker.
    #[arg(long, value_name = "TITLE")]
    window: Option<String>,

    /// Print capturable windows and exit.
    #[arg(long)]
    list_windows: bool,

    /// VRChat OSC input address (UDP). Default matches VRChat's listener.
    #[arg(long, default_value = "127.0.0.1:9000")]
    osc_addr: SocketAddr,

    /// Disable the OSC sender entirely.
    #[arg(long)]
    no_osc: bool,

    /// Force Discord Rich Presence off even if `--discord-app-id` is
    /// given. (It's already off by default.)
    #[arg(long)]
    no_discord: bool,

    /// Turn on Discord Rich Presence with this Discord application ID
    /// (register one at discord.com/developers). Off when not given.
    #[arg(long, value_name = "ID")]
    discord_app_id: Option<String>,

    /// Save a PNG screenshot of the preview window N frames after
    /// launch. Combined with --exit-after this gives a headless
    /// render-to-PNG flow so automated runs can verify the UI without
    /// an interactive window.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<std::path::PathBuf>,

    /// Exit cleanly after this many frames. Pair with --screenshot for
    /// a non-interactive capture.
    #[arg(long, value_name = "N")]
    exit_after: Option<u64>,

    /// Override the overlay position as x,y,z metres. Letting LLMs
    /// test placement variants without a real click.
    #[arg(long, value_name = "X,Y,Z", value_parser = parse_vec3)]
    overlay_pos: Option<(f32, f32, f32)>,

    /// Override the overlay size as w,h metres.
    #[arg(long, value_name = "W,H", value_parser = parse_vec2)]
    overlay_size: Option<(f32, f32)>,

    /// Override the overlay anchor: wrist / hmd / world.
    #[arg(long, value_name = "ANCHOR")]
    overlay_anchor: Option<String>,

    /// Show the virtual keyboard card at launch.
    #[arg(long)]
    show_keyboard: bool,

    /// Show the notifications history card at launch.
    #[arg(long)]
    show_history: bool,

    /// Fill the battery card with sample devices (one low, one
    /// charging) so the widget can be screenshot-tested without an HMD.
    #[arg(long)]
    demo_batteries: bool,

    /// Test hook: request a live switch to the window matching TITLE
    /// at frame 30, exercising the pipeline teardown/rebuild path the
    /// Desktop card's source picker uses.
    #[arg(long, value_name = "TITLE")]
    switch_to_window: Option<String>,

    /// Open the dashboard on this page (home / desktop / audio / chat /
    /// macros / notifications / settings). Handy with --screenshot.
    #[arg(long, value_name = "PAGE")]
    page: Option<String>,

    /// Preview the VR dashboard layout (bottom dock instead of side rail).
    #[arg(long)]
    dock: bool,

    /// Preview the VR wrist watch-face instead of the dashboard.
    #[arg(long)]
    wrist: bool,

    /// Preview the VR wrist in edit mode (implies --wrist).
    #[arg(long)]
    wrist_edit: bool,

    /// Preview a VR widget group, e.g. `--widgets system,volume`.
    #[arg(long, value_name = "KINDS")]
    widgets: Option<String>,

    /// Preview the VR keyboard panel (labels from the live layout).
    #[arg(long)]
    vr_keyboard: bool,

    /// Pre-fill the global search box (screenshot the results list).
    #[arg(long, value_name = "QUERY")]
    search: Option<String>,

    /// Render every Settings card to `DIR/NN-name.png` offscreen and
    /// exit (input for tools/make-settings-overview.py).
    #[arg(long, value_name = "DIR")]
    settings_cards: Option<std::path::PathBuf>,

    /// VR preview layout preset: work / gaming / social / dashboard /
    /// wrist-compact / wrist-expanded (keys 1-6 switch live).
    #[arg(long, value_name = "PRESET")]
    scene: Option<String>,
}

fn parse_vec3(s: &str) -> Result<(f32, f32, f32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return Err("expected X,Y,Z".into());
    }
    let x = parts[0].trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
    let y = parts[1].trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
    let z = parts[2].trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
    Ok((x, y, z))
}

fn parse_vec2(s: &str) -> Result<(f32, f32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err("expected W,H".into());
    }
    let w = parts[0].trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
    let h = parts[1].trim().parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
    Ok((w, h))
}

impl Args {
    fn mode(&self) -> RuntimeMode {
        if self.vr {
            RuntimeMode::Vr
        } else if self.vr_preview {
            RuntimeMode::VrPreview
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

    // Per-monitor DPI awareness before anything reads coordinates: on a
    // scaled display (125 %/150 %) an unaware process gets virtualised
    // cursor and screen metrics, so mirrored-window clicks and the drawn
    // cursor land off target. (VR mode has no window to do this for us.)
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Load persistent config (accent / placement / toggles). Missing
    // file → defaults. Malformed file → log and fall back so a corrupt
    // config can never brick launch.
    let config_path = rsoverlay_core::Config::default_path();
    let config = match rsoverlay_core::Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("config load failed, using defaults: {e:#}");
            rsoverlay_core::Config::default()
        }
    };

    if let Some(dir) = &args.settings_cards {
        // Default config, not the user's: the overview shows RSOverlay as
        // shipped and stays the same on every machine.
        return snapshot::settings_cards(dir, &rsoverlay_core::Config::default());
    }

    if args.list_monitors {
        for m in rsoverlay_capture::enumerate_monitors()? {
            println!("[{}] {} {}x{}", m.index, m.device_name, m.width, m.height);
        }
        return Ok(());
    }

    #[cfg(windows)]
    if args.list_windows {
        for w in rsoverlay_capture::enumerate_windows()? {
            println!("[{:#x}] {}", w.hwnd, w.title);
        }
        return Ok(());
    }

    // Opt-in elevation (Settings → Administrator). Relaunch through the
    // UAC prompt; if the user declines, carry on unelevated.
    if config.vr.run_as_admin && !rsoverlay_input::elevation::current_process_elevated() {
        let argv: Vec<String> = std::env::args().skip(1).collect();
        if rsoverlay_input::elevation::relaunch_elevated(&argv) {
            info!("relaunched elevated; exiting this instance");
            return Ok(());
        }
        tracing::warn!("elevation declined — admin windows will be view-only");
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
        window: args.window.clone(),
        force_cpu: args.no_gpu_mirror,
    };

    let osc = if args.no_osc {
        None
    } else {
        Some(spawn_osc(&rt_handle, args.osc_addr))
    };

    // Discord Rich Presence: spawn once, carry the handle through the
    // chosen mode so it lives for the process lifetime. The handle's
    // Drop sends a tidy shutdown to the worker.
    let discord_mode = match mode {
        RuntimeMode::Preview => rsoverlay_discord::RuntimeMode::Preview,
        RuntimeMode::VrPreview => rsoverlay_discord::RuntimeMode::VrPreview,
        RuntimeMode::Vr => rsoverlay_discord::RuntimeMode::Vr,
    };
    // Toggle from Settings; the running UI keeps it in sync after this.
    rsoverlay_discord::set_enabled(config.appearance.discord_presence);
    let app_id = args.discord_app_id.as_deref().or(rsoverlay_discord::BUILTIN_APP_ID).filter(|id| !id.trim().is_empty());
    let _discord_handle = match (app_id, args.no_discord) {
        (Some(id), false) => Some(rsoverlay_discord::spawn(&rt_handle, id, discord_mode)),
        _ => None,
    };

    let test_cfg = preview::TestHooks {
        screenshot: args.screenshot.clone(),
        exit_after: args.exit_after,
        overlay_pos: args.overlay_pos,
        overlay_size: args.overlay_size,
        overlay_anchor: args.overlay_anchor.clone(),
        show_keyboard: args.show_keyboard,
        show_history: args.show_history,
        demo_batteries: args.demo_batteries,
        switch_to_window: args.switch_to_window.clone(),
        page: args.page.clone(),
        dock: args.dock,
        wrist: args.wrist || args.wrist_edit,
        wrist_edit: args.wrist_edit,
        widgets: args.widgets.clone(),
        vr_keyboard: args.vr_keyboard,
        search: args.search.clone(),
        scene: args.scene.clone(),
    };

    let result = match mode {
        RuntimeMode::Preview => preview::run(
            notify_rx,
            capture_cfg,
            osc,
            test_cfg,
            config.clone(),
            config_path.clone(),
        ),
        RuntimeMode::VrPreview => {
            preview_vr::run(notify_rx, test_cfg, config.clone(), config_path.clone())
        }
        RuntimeMode::Vr => vr::run(notify_rx, rt_handle.clone(), config.clone(), config_path.clone(), osc),
    };

    // Give the runtime a bounded shutdown window so background tasks drain.
    rt.shutdown_timeout(std::time::Duration::from_millis(250));
    result
}
