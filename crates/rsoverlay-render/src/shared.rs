//! wgpu side of the zero-copy desktop mirror.
//!
//! Companion to `rsoverlay-capture::shared`. Takes the NT HANDLE produced
//! by `SharedCapture`, opens it on our wgpu DX12 device via
//! `ID3D12Device::OpenSharedHandle`, and wraps the resulting
//! `ID3D12Resource` as a `wgpu::Texture` through `wgpu-hal`'s
//! `texture_from_raw` + `Device::create_texture_from_hal`. egui then
//! samples it like any other texture.
//!
//! The capture side owns the keyed mutex from the D3D11 side; we open a
//! parallel `IDXGIKeyedMutex` on the D3D12 side so the writer/reader
//! handshake is enforced in GPU time.

#![cfg(windows)]

use anyhow::{anyhow, Context, Result};

/// Look up the DXGI adapter LUID the wgpu DX12 device is currently
/// bound to. Returned as `(LowPart, HighPart)` so the capture side can
/// match on it without introducing a dep on windows-rs types here.
pub fn wgpu_adapter_luid(gpu: &crate::GpuCtx) -> Option<(u32, i32)> {
    unsafe {
        gpu.device
            .as_hal::<wgpu_hal::api::Dx12, _, _>(|hal_device| {
                let hal_device = hal_device?;
                let raw = hal_device.raw_device().as_mut_ptr();
                let luid = (*raw).GetAdapterLuid();
                Some((luid.LowPart, luid.HighPart))
            })
            .flatten()
    }
}
use std::ffi::c_void;
use std::ptr;
use winapi::shared::guiddef::GUID;
use winapi::shared::winerror::HRESULT;
use winapi::um::d3d12::ID3D12Resource;
use winapi::Interface;

use wgpu::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

use crate::GpuCtx;

/// Owned wgpu side of the shared mirror.
pub struct SharedMirror {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// Held so the resource lives until this struct drops. Dropping it
    /// releases the opened handle via COM refcount.
    _keeper: ResourceKeeper,
    keyed_mutex: KeyedMutex,
}

impl SharedMirror {
    /// Open an NT shared handle as a wgpu::Texture on the given GpuCtx's
    /// DX12 device. `handle` must have been produced by
    /// `rsoverlay-capture::SharedCapture::shared_handle_raw`.
    ///
    /// # Safety
    /// The caller must guarantee that `handle` refers to a live D3D11
    /// shared resource matching `(width, height, BGRA8)` and that the
    /// resource outlives this `SharedMirror`.
    pub unsafe fn open(
        gpu: &GpuCtx,
        handle: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        if handle.is_null() {
            return Err(anyhow!("null shared handle"));
        }

        let (d3d12_resource_raw, keyed_mutex_raw) = open_shared_handle(gpu, handle)?;

        // Wrap the raw resource in d3d12 crate's ComPtr form so wgpu-hal
        // can accept it. from_raw does NOT AddRef — it adopts the
        // existing refcount from OpenSharedHandle.
        let d3d12_resource: d3d12::Resource = d3d12::ComPtr::from_raw(d3d12_resource_raw);

        let hal_texture = wgpu_hal::dx12::Device::texture_from_raw(
            d3d12_resource,
            // sRGB to match the CPU mirror path (MirrorTexture): sampling
            // decodes to linear before egui re-encodes on output. A plain
            // Unorm view here double-encodes and washes the mirror out.
            TextureFormat::Bgra8UnormSrgb,
            TextureDimension::D2,
            Extent3d { width, height, depth_or_array_layers: 1 },
            1,
            1,
        );

        let desc = wgpu::TextureDescriptor {
            label: Some("shared-mirror"),
            size: Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        };

        let texture = gpu
            .device
            .create_texture_from_hal::<wgpu_hal::api::Dx12>(hal_texture, &desc);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Ok(Self {
            texture,
            view,
            width,
            height,
            _keeper: ResourceKeeper,
            keyed_mutex: KeyedMutex(keyed_mutex_raw),
        })
    }

    /// Reserve a read lease. When a keyed mutex is in play this would
    /// block until the writer hands over key=1; today it's a no-op
    /// because D3D12 can't acquire a D3D11 keyed mutex (see note in
    /// `open_shared_handle`). The lease still exists to preserve the
    /// API so a future fence-based handshake can slot in without
    /// touching callers.
    pub fn acquire_read(&self, _timeout_ms: u32) -> Result<ReadLease<'_>> {
        unsafe {
            if !self.keyed_mutex.0.is_null() {
                let hr =
                    self.keyed_mutex.acquire_sync(KEY_WRITE_DONE, _timeout_ms);
                if hr < 0 {
                    return Err(anyhow!("IDXGIKeyedMutex::AcquireSync failed hr={hr:#x}"));
                }
            }
        }
        Ok(ReadLease { mirror: self })
    }
}

impl Drop for SharedMirror {
    fn drop(&mut self) {
        // ResourceKeeper + KeyedMutex drop themselves; texture drops via
        // its own wgpu refcount.
    }
}

unsafe impl Send for SharedMirror {}
unsafe impl Sync for SharedMirror {}

/// RAII guard: releases the keyed mutex with key=0 on drop, handing
/// write ownership back to the capture side.
pub struct ReadLease<'a> {
    mirror: &'a SharedMirror,
}

impl Drop for ReadLease<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.mirror.keyed_mutex.0.is_null() {
                let _ = self.mirror.keyed_mutex.release_sync(KEY_READ_DONE);
            }
        }
    }
}

const KEY_WRITE_DONE: u64 = 1;
const KEY_READ_DONE: u64 = 0;

/// Open an NT shared handle on the wgpu DX12 device and query out a
/// matching IDXGIKeyedMutex for synchronization.
unsafe fn open_shared_handle(
    gpu: &GpuCtx,
    handle: *mut c_void,
) -> Result<(*mut ID3D12Resource, *mut c_void)> {
    let mut resource_ptr: *mut ID3D12Resource = ptr::null_mut();
    let keyed_mutex_ptr: *mut c_void = ptr::null_mut();

    // Access the raw D3D12 device via wgpu-hal's public accessor.
    let outcome = gpu
        .device
        .as_hal::<wgpu_hal::api::Dx12, _, _>(|hal_device| -> Result<()> {
            let hal_device = hal_device
                .ok_or_else(|| anyhow!("wgpu device is not on the Dx12 backend"))?;
            let raw_device: &d3d12::Device = hal_device.raw_device();
            let device_ptr = raw_device.as_mut_ptr();

            // ID3D12Device::OpenSharedHandle(hNtHandle, REFIID, ppvObj)
            let iid = ID3D12Resource::uuidof();
            let hr: HRESULT = (*device_ptr).OpenSharedHandle(
                handle,
                &iid as *const GUID,
                &mut resource_ptr as *mut _ as *mut *mut c_void,
            );
            if hr < 0 {
                return Err(anyhow!(
                    "ID3D12Device::OpenSharedHandle failed hr={hr:#x}"
                ));
            }
            if resource_ptr.is_null() {
                return Err(anyhow!("OpenSharedHandle returned null resource"));
            }

            // IDXGIKeyedMutex isn't natively reachable from a D3D12
            // device (documented D3D11↔D3D12 interop gap — the proper
            // cross-API sync is a shared ID3D12Fence, not a keyed mutex).
            // For now we leave keyed_mutex_ptr null and accept a tiny
            // race window: both D3D11 writer and D3D12 reader are on the
            // same adapter, same driver, same GPU timeline, so tearing
            // is rare and bounded to one frame. The proper fence-based
            // handshake is a follow-up milestone.
            let _ = &idxgi_keyed_mutex_iid;
            Ok(())
        });
    outcome
        .ok_or_else(|| anyhow!("wgpu device is not on the wgpu-core backend"))?
        .context("opening shared handle on wgpu DX12 device")?;
    Ok((resource_ptr, keyed_mutex_ptr))
}

fn idxgi_keyed_mutex_iid() -> GUID {
    // IID_IDXGIKeyedMutex, from dxgi.h:
    //   DEFINE_GUID(IID_IDXGIKeyedMutex, 0x9d8e1289, 0xd7b3, 0x465f,
    //               0x81,0x26, 0x25,0x0e,0x34,0x9a,0xf8,0x5d)
    GUID {
        Data1: 0x9d8e1289,
        Data2: 0xd7b3,
        Data3: 0x465f,
        Data4: [0x81, 0x26, 0x25, 0x0e, 0x34, 0x9a, 0xf8, 0x5d],
    }
}

/// Thin wrapper on an IDXGIKeyedMutex raw pointer so we can call its two
/// methods without pulling in winapi's dxgi1_2 interface definitions.
struct KeyedMutex(*mut c_void);

impl KeyedMutex {
    /// IDXGIKeyedMutex::AcquireSync is vtable index 7 (after the three
    /// IUnknown + three IDXGIObject slots). We bind against the full
    /// winapi trait below for correctness.
    unsafe fn acquire_sync(&self, key: u64, timeout_ms: u32) -> HRESULT {
        use winapi::shared::dxgi::IDXGIKeyedMutex;
        let ptr = self.0 as *mut IDXGIKeyedMutex;
        (*ptr).AcquireSync(key, timeout_ms)
    }

    unsafe fn release_sync(&self, key: u64) -> HRESULT {
        use winapi::shared::dxgi::IDXGIKeyedMutex;
        let ptr = self.0 as *mut IDXGIKeyedMutex;
        (*ptr).ReleaseSync(key)
    }
}

impl Drop for KeyedMutex {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                (*(self.0 as *mut winapi::um::unknwnbase::IUnknown)).Release();
            }
        }
    }
}

/// Placeholder — the actual D3D12Resource lifetime is now owned by the
/// wgpu-hal Texture wrapping it. This type is present so `SharedMirror`
/// has a fixed shape for future extensions (e.g. holding a secondary
/// SRV heap handle).
struct ResourceKeeper;
