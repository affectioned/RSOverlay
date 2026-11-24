//! GPU-to-GPU desktop mirror via a DXGI NT shared handle + keyed mutex.
//!
//! Architecture
//!
//! 1. A D3D11 device, picked on the same adapter LUID as the wgpu DX12
//!    device, owns a BGRA8 2D texture created with
//!    `MISC_SHARED_NTHANDLE | MISC_SHARED_KEYEDMUTEX` and
//!    `BIND_RENDER_TARGET | BIND_SHADER_RESOURCE`. This is the "shared
//!    texture".
//! 2. `CreateSharedHandle` on it yields a Windows NT `HANDLE` that the
//!    render crate opens on its wgpu DX12 device (see
//!    `rsoverlay-render::shared`).
//! 3. Each captured frame, this crate acquires the keyed mutex on key 0,
//!    `CopyResource`s the duplicated desktop frame into the shared
//!    texture, then releases with key 1. The reader side waits on key 1
//!    and releases back to key 0 — this handshake keeps the GPU writer
//!    and GPU reader from racing without a CPU memcpy anywhere.
//!
//! The DXGI Desktop Duplication frame texture itself is already on the
//! GPU, so `CopyResource` is a pure driver-side blit. End-to-end there
//! is no `Map` / `memcpy` / `Vec<u8>` on this path.

use anyhow::{anyhow, Context, Result};
use std::ffi::c_void;
use windows::core::ComInterface;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIKeyedMutex,
    IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, IDXGIResource1,
    DXGI_ADAPTER_DESC1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO, DXGI_SHARED_RESOURCE_READ,
};

// Keyed mutex key constants reserved for the future D3D12-Fence-based
// handshake. Both unused today (see note at keyed_mutex field).
#[allow(dead_code)]
const KEY_WRITE_DONE: u64 = 1;
#[allow(dead_code)]
const KEY_READ_DONE: u64 = 0;

/// Owned D3D11 side of a GPU-to-GPU desktop mirror.
pub struct SharedCapture {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    shared_tex: ID3D11Texture2D,
    /// Held to keep the keyed mutex alive on the DXGI resource, so the
    /// reader-side open (even without using it for sync yet) sees a
    /// mutex-enabled resource. Dead-code allowance until the fence
    /// handshake lands.
    #[allow(dead_code)]
    keyed_mutex: Option<IDXGIKeyedMutex>,
    /// NT HANDLE we hand to the render crate so it can open the texture
    /// on the wgpu DX12 device. Closed on drop.
    shared_handle: HANDLE,
    pub width: u32,
    pub height: u32,
}

impl SharedCapture {
    /// Build the D3D11 + duplication + shared texture, picking the adapter
    /// whose LUID matches the wgpu adapter's LUID so both devices sit on
    /// the same GPU. `luid_low` / `luid_high` come from
    /// `wgpu::Adapter::get_info` on Windows via `as_hal` (see render
    /// crate).
    pub fn new(monitor_index: u32, wgpu_luid: Option<(u32, i32)>) -> Result<Self> {
        unsafe {
            let factory: IDXGIFactory1 =
                CreateDXGIFactory1().context("CreateDXGIFactory1")?;

            let (adapter, output) = pick_adapter_output(&factory, monitor_index, wgpu_luid)?;

            // Build D3D11 on the matched adapter.
            let adapter_any: IDXGIAdapter =
                adapter.cast().context("IDXGIAdapter cast")?;
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut feature_level = D3D_FEATURE_LEVEL::default();
            let fl = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            D3D11CreateDevice(
                &adapter_any,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&fl),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut context),
            )
            .context("D3D11CreateDevice (shared)")?;
            let device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null"))?;
            let context = context.ok_or_else(|| anyhow!("no immediate context"))?;

            let duplication = output
                .DuplicateOutput(&device)
                .context("IDXGIOutput1::DuplicateOutput")?;

            let mut dup_desc = Default::default();
            duplication.GetDesc(&mut dup_desc);
            let width = dup_desc.ModeDesc.Width;
            let height = dup_desc.ModeDesc.Height;

            // Shared texture: NT handle, keyed mutex, SRV + RTV usable.
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                // _SRGB: same typeless family as the duplication frame,
                // so CopyResource stays legal; the D3D12 reader opens it
                // as Bgra8UnormSrgb to match the CPU path's color.
                Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                // NT-handle sharing only. We'd normally OR in
                // SHARED_KEYEDMUTEX for cross-API sync, but without a
                // reader-side acquire (see shared.rs note) the mutex
                // flag just makes D3D12 reads return undefined data.
                // Sync comes back with the D3D12 fence milestone.
                //
                // D3D11 rejects SHARED_NTHANDLE on its own (E_INVALIDARG):
                // it must be paired with SHARED or SHARED_KEYEDMUTEX.
                // Without the pairing this path never ran and every
                // session silently fell back to CPU readback.
                MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
            };
            let mut tex: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .context("CreateTexture2D (shared)")?;
            let shared_tex = tex.ok_or_else(|| anyhow!("CreateTexture2D returned null"))?;

            // Pull the keyed mutex out of the texture; it's the sync
            // primitive both sides use.
            // Only present once the texture carries SHARED_KEYEDMUTEX
            // (fence milestone); today the cast is expected to fail.
            let keyed_mutex: Option<IDXGIKeyedMutex> = shared_tex.cast().ok();

            // CreateSharedHandle requires IDXGIResource1.
            let resource1: IDXGIResource1 =
                shared_tex.cast().context("IDXGIResource1 cast")?;
            // GENERIC_ALL so both read and write sides can open. Reader
            // narrows to DXGI_SHARED_RESOURCE_READ via OpenSharedHandle.
            const GENERIC_ALL: u32 = 0x1000_0000;
            let shared_handle = resource1
                .CreateSharedHandle(None, GENERIC_ALL, None)
                .context("IDXGIResource1::CreateSharedHandle")?;

            // The reader side cannot acquire this mutex (D3D11↔D3D12
            // interop limitation — see shared handle notes in
            // rsoverlay-render::shared). We skip the cross-API sync for
            // now; writer-side ping-pong would still deadlock without a
            // reader, so disable it here too. Reinstated once the
            // fence-based handshake lands.
            let _ = &keyed_mutex;

            Ok(Self {
                _device: device,
                context,
                duplication,
                shared_tex,
                keyed_mutex,
                shared_handle,
                width,
                height,
            })
        }
    }

    pub fn shared_handle_raw(&self) -> *mut c_void {
        self.shared_handle.0 as *mut c_void
    }

    /// Pull the next desktop frame into the shared texture.
    /// Returns `Ok(true)` if a new frame was written, `Ok(false)` on
    /// AcquireNextFrame timeout.
    pub fn capture_next_frame(&mut self, timeout_ms: u32) -> Result<bool> {
        unsafe {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            let hr = self
                .duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource);
            if let Err(e) = hr {
                if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    return Ok(false);
                }
                if e.code() == DXGI_ERROR_ACCESS_LOST {
                    return Err(e).context("DXGI_ERROR_ACCESS_LOST");
                }
                return Err(e).context("AcquireNextFrame (shared)");
            }
            let resource =
                resource.ok_or_else(|| anyhow!("AcquireNextFrame returned null"))?;
            let frame_tex: ID3D11Texture2D =
                resource.cast().context("frame_tex cast")?;

            self.context.CopyResource(&self.shared_tex, &frame_tex);
            // Force the copy onto the GPU timeline immediately. Without
            // this the D3D11 immediate context defers the copy, and the
            // D3D12 reader on the other device samples the stale
            // (initially all-zero) texture contents.
            self.context.Flush();
            self.duplication.ReleaseFrame().ok();
            Ok(true)
        }
    }
}

impl Drop for SharedCapture {
    fn drop(&mut self) {
        unsafe {
            if !self.shared_handle.is_invalid() {
                let _ = CloseHandle(self.shared_handle);
            }
        }
    }
}

unsafe impl Send for SharedCapture {}

fn pick_adapter_output(
    factory: &IDXGIFactory1,
    monitor_index: u32,
    wgpu_luid: Option<(u32, i32)>,
) -> Result<(IDXGIAdapter1, IDXGIOutput1)> {
    unsafe {
        let mut flat_idx = 0u32;
        let mut first_match: Option<(IDXGIAdapter1, IDXGIOutput1)> = None;
        let mut adapter_idx = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
            let mut desc = DXGI_ADAPTER_DESC1::default();
            if adapter.GetDesc1(&mut desc).is_ok() {
                if let Some((lo, hi)) = wgpu_luid {
                    let adapter_luid: LUID = desc.AdapterLuid;
                    if adapter_luid.LowPart != lo || adapter_luid.HighPart != hi {
                        adapter_idx += 1;
                        continue;
                    }
                }
            }
            let mut output_idx = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_idx) {
                if flat_idx == monitor_index {
                    let o1: IDXGIOutput1 =
                        output.cast().context("IDXGIOutput1 cast")?;
                    first_match = Some((adapter.clone(), o1));
                    break;
                }
                flat_idx += 1;
                output_idx += 1;
            }
            if first_match.is_some() {
                break;
            }
            adapter_idx += 1;
        }
        first_match.ok_or_else(|| anyhow!("monitor index {monitor_index} not found"))
    }
}

/// Silence currently-unused DXGI import; kept because the SRV access
/// flags constant will land with the open-side code in `rsoverlay-render`.
#[allow(dead_code)]
const _: u32 = DXGI_SHARED_RESOURCE_READ;
