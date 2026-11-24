//! Offscreen renders for docs and store art: UI drawn into a texture
//! (no window, no headset) and written to PNG.

use anyhow::{Context, Result};
use rsoverlay_core::RuntimeMode;
use rsoverlay_render::{GpuCtx, OverlayTarget};
use rsoverlay_ui::{views::DevView, EguiPanel};
use std::path::Path;
use std::time::Duration;
use tracing::info;

/// Card width in points matches one Settings-page column; 2× for crisp
/// text. Height is generous: the overview tool crops to the card.
const PPP: f32 = 2.0;
const SIZE: (u32, u32) = (1100, 2000);
/// Frames per card: lets animations settle and the stats sampler report.
const FRAMES: u32 = 24;
const FRAME_GAP: Duration = Duration::from_millis(60);

/// Every Settings card to `dir/NN-name.png`.
pub fn settings_cards(dir: &Path, config: &rsoverlay_core::Config) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let gpu = pollster::block_on(GpuCtx::new(RuntimeMode::Preview))?;
    let hooks = crate::preview::TestHooks { demo_batteries: true, ..Default::default() };
    let mut dev = DevView::default();
    crate::preview::seed_dev_view(&mut dev, config, &hooks);
    dev.set_demo_media("Northern Lights", "Ambient Works");
    let points = egui::vec2(SIZE.0 as f32, SIZE.1 as f32) / PPP;
    let mut time = 0.0;
    for index in 0.. {
        let target = OverlayTarget::new(&gpu, SIZE.0, SIZE.1);
        let mut panel = EguiPanel::new(&gpu.device, target.format, SIZE.0, SIZE.1);
        panel.set_pixels_per_point(PPP);
        let mut name = None;
        for _ in 0..FRAMES {
            time += FRAME_GAP.as_secs_f64();
            // Offscreen has no display: show the rate the overlay runs at
            // on a 90 Hz headset (representative, not measured here).
            dev.last_fps = 90.0;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
                time: Some(time),
                predicted_dt: FRAME_GAP.as_secs_f32(),
                ..Default::default()
            };
            // Real, measured UI frame time feeds the Stats card.
            let t0 = std::time::Instant::now();
            panel.render(&gpu.device, &gpu.queue, &target.view, raw, |ctx| name = dev.settings_card_ui(ctx, index));
            dev.record_frame_ms(t0.elapsed().as_secs_f32() * 1000.0);
            std::thread::sleep(FRAME_GAP);
        }
        let Some(name) = name else { break };
        let path = dir.join(format!("{index:02}-{name}.png"));
        crate::preview::save_surface_as_png(&gpu, &target.texture, SIZE.0, SIZE.1, &path)?;
        info!("wrote {}", path.display());
    }
    Ok(())
}
