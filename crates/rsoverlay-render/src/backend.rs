//! Presentation backends.
//!
//! [`PresentBackend`] abstracts over the two targets we support:
//! - [`VrBackend`] (feature-gated in the app) hands the overlay texture to OpenVR.
//! - [`PreviewBackend`] blits the overlay texture to a wgpu surface on a winit
//!   window, so you can iterate without a headset.

use crate::{GpuCtx, OverlayTarget};
use anyhow::Result;

/// A target that can receive this frame's overlay texture.
pub trait PresentBackend {
    /// Called once per frame after the panels have been rendered into `target`.
    fn present(&mut self, gpu: &GpuCtx, target: &OverlayTarget) -> Result<()>;
}

/// Null backend — useful for benchmarks and tests.
pub struct NullBackend;

impl PresentBackend for NullBackend {
    fn present(&mut self, _gpu: &GpuCtx, _target: &OverlayTarget) -> Result<()> {
        Ok(())
    }
}
