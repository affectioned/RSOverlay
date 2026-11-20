//! SteamVR runtime path.
//!
//! This milestone: initialize OpenVR, create & position an overlay, show it,
//! then wait for Ctrl+C and clean up. Texture submission requires sharing a
//! D3D11 texture handle from wgpu to OpenVR; that interop lands next
//! milestone once we lock the SDK version and wire `wgpu::hal`. Until then
//! the VR overlay registers and positions correctly (visible as an empty
//! quad), while the Preview mode exercises the full render path.

use anyhow::{Context, Result};
use rsoverlay_notify::NotifyChannel;
use rsoverlay_openvr::{pose_translation, OpenVr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use tracing::{info, warn};

pub fn run(_notify: Option<NotifyChannel>, rt: Handle) -> Result<()> {
    let vr = OpenVr::init().context("OpenVR init")?;
    let overlay = vr.overlay().context("acquiring IVROverlay")?;

    let handle = overlay
        .create("rsoverlay.main", "RSOverlay")
        .context("CreateOverlay")?;
    overlay.set_width(handle, 1.0).ok();
    // 2m in front, 1.5m up from the standing-universe origin.
    let pose = pose_translation(0.0, 1.5, -2.0);
    overlay.set_transform_absolute(handle, &pose).ok();
    overlay.show(handle).ok();

    info!("overlay registered at 2m in front of origin. Ctrl+C to exit.");
    warn!("texture submission is stubbed this milestone — overlay will appear empty");

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    rt.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            r.store(false, Ordering::SeqCst);
        }
    });

    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    overlay.hide(handle).ok();
    overlay.destroy(handle).ok();
    info!("clean shutdown");
    Ok(())
}
