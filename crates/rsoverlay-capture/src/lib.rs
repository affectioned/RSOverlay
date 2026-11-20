//! Desktop and window capture.
//!
//! Current status (M3): DXGI Desktop Duplication produces a BGRA8 frame per
//! monitor. This milestone pulls the frame through a staging CPU readback
//! into a `Vec<u8>` so the render crate can `queue.write_texture` it. The
//! follow-up milestone moves this to a zero-copy GPU-to-GPU path via shared
//! handles to meet the <16 ms latency target.
//!
//! Unix / macOS stubs out to a no-op.

#[cfg(not(windows))]
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub device_name: String,
}

/// One captured frame — BGRA8 top-left origin.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bgra: Vec<u8>,
}

#[cfg(windows)]
pub use dxgi::{enumerate_monitors, DxgiDuplication};

#[cfg(not(windows))]
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub struct DxgiDuplication;

#[cfg(not(windows))]
impl DxgiDuplication {
    pub fn new(_monitor_index: u32) -> Result<Self> {
        anyhow::bail!("desktop capture only implemented on Windows")
    }
    pub fn next_frame(&mut self, _timeout_ms: u32) -> Result<Option<Frame>> {
        Ok(None)
    }
}

#[cfg(windows)]
mod dxgi {
    use super::{Frame, MonitorInfo};
    use anyhow::{anyhow, Context, Result};
    use std::ptr;
    use windows::core::ComInterface;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
        D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
        IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
        DXGI_OUTDUPL_FRAME_INFO,
    };

    pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
            let mut monitors = Vec::new();
            let mut adapter_idx = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
                let mut output_idx = 0u32;
                while let Ok(output) = adapter.EnumOutputs(output_idx) {
                    let mut desc = Default::default();
                    output.GetDesc(&mut desc).context("IDXGIOutput::GetDesc")?;
                    let coords = desc.DesktopCoordinates;
                    let name = String::from_utf16_lossy(
                        &desc.DeviceName[..desc
                            .DeviceName
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(desc.DeviceName.len())],
                    );
                    monitors.push(MonitorInfo {
                        index: monitors.len() as u32,
                        width: (coords.right - coords.left) as u32,
                        height: (coords.bottom - coords.top) as u32,
                        device_name: name,
                    });
                    output_idx += 1;
                }
                adapter_idx += 1;
                let _ = adapter; // keep COM ref alive through loop body
            }
            Ok(monitors)
        }
    }

    pub struct DxgiDuplication {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
        staging: Option<ID3D11Texture2D>,
        staging_desc: D3D11_TEXTURE2D_DESC,
    }

    impl DxgiDuplication {
        pub fn new(monitor_index: u32) -> Result<Self> {
            unsafe {
                let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;

                // Walk adapters/outputs linearly to match the index exposed by
                // `enumerate_monitors`.
                let mut flat_idx = 0u32;
                let mut target: Option<(IDXGIAdapter1, IDXGIOutput1)> = None;
                let mut adapter_idx = 0u32;
                while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
                    let mut output_idx = 0u32;
                    while let Ok(output) = adapter.EnumOutputs(output_idx) {
                        if flat_idx == monitor_index {
                            let o1: IDXGIOutput1 = output.cast().context("IDXGIOutput1 cast")?;
                            target = Some((adapter.clone(), o1));
                            break;
                        }
                        flat_idx += 1;
                        output_idx += 1;
                    }
                    if target.is_some() {
                        break;
                    }
                    adapter_idx += 1;
                }
                let (adapter, output) = target
                    .ok_or_else(|| anyhow!("monitor index {monitor_index} not found"))?;

                let mut device: Option<ID3D11Device> = None;
                let mut context: Option<ID3D11DeviceContext> = None;
                let mut feature_level = D3D_FEATURE_LEVEL::default();
                let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
                D3D11CreateDevice(
                    &adapter.cast::<windows::Win32::Graphics::Dxgi::IDXGIAdapter>()
                        .context("adapter cast")?,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&feature_levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut context),
                )
                .context("D3D11CreateDevice")?;
                let device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null"))?;
                let context = context.ok_or_else(|| anyhow!("no immediate context"))?;

                let duplication = output
                    .DuplicateOutput(&device)
                    .context("IDXGIOutput1::DuplicateOutput")?;

                let mut dup_desc = Default::default();
                duplication.GetDesc(&mut dup_desc);
                let mode = dup_desc.ModeDesc;

                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: mode.Width,
                    Height: mode.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };

                Ok(Self {
                    _device: device,
                    context,
                    duplication,
                    staging: None,
                    staging_desc,
                })
            }
        }

        fn ensure_staging(&mut self) -> Result<()> {
            if self.staging.is_some() {
                return Ok(());
            }
            unsafe {
                let mut tex = None;
                self._device
                    .CreateTexture2D(&self.staging_desc, None, Some(&mut tex))
                    .context("CreateTexture2D staging")?;
                self.staging = tex;
            }
            Ok(())
        }

        /// Pull one frame from the compositor, blocking up to `timeout_ms`.
        /// Returns `Ok(None)` on timeout.
        pub fn next_frame(&mut self, timeout_ms: u32) -> Result<Option<Frame>> {
            self.ensure_staging()?;
            unsafe {
                let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut resource: Option<IDXGIResource> = None;
                let hr = self.duplication.AcquireNextFrame(
                    timeout_ms,
                    &mut frame_info,
                    &mut resource,
                );
                if let Err(e) = hr {
                    if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                        return Ok(None);
                    }
                    if e.code() == DXGI_ERROR_ACCESS_LOST {
                        // Recreate on the next call by failing cleanly here.
                        return Err(e).context("DXGI_ERROR_ACCESS_LOST; recreate duplication");
                    }
                    return Err(e).context("AcquireNextFrame");
                }
                let resource = resource.ok_or_else(|| anyhow!("AcquireNextFrame returned null"))?;
                let frame_tex: ID3D11Texture2D = resource.cast().context("resource cast")?;

                let staging = self
                    .staging
                    .as_ref()
                    .ok_or_else(|| anyhow!("staging missing"))?;
                self.context.CopyResource(staging, &frame_tex);

                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .context("Map staging")?;

                let height = self.staging_desc.Height;
                let width = self.staging_desc.Width;
                let row_pitch = mapped.RowPitch;
                let src = mapped.pData as *const u8;
                let byte_len = (row_pitch as usize) * (height as usize);
                let mut bgra = vec![0u8; byte_len];
                ptr::copy_nonoverlapping(src, bgra.as_mut_ptr(), byte_len);

                self.context.Unmap(staging, 0);
                self.duplication.ReleaseFrame().ok();

                Ok(Some(Frame {
                    width,
                    height,
                    row_pitch,
                    bgra,
                }))
            }
        }
    }
}
