//! Desktop and window capture via DXGI Desktop Duplication + Windows Graphics
//! Capture. GPU-to-GPU, no readback. Scaffolding only in this milestone.

use anyhow::Result;

#[derive(Debug)]
pub struct MonitorInfo {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub device_name: String,
}

/// Placeholder enumeration used by preview UI. Real DXGI enumeration lands in
/// the capture milestone.
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    Ok(Vec::new())
}
