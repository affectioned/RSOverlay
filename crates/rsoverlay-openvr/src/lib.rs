//! Hand-rolled OpenVR FFI + safe wrappers.
//!
//! The OpenVR runtime ships as `openvr_api.dll` (Windows) / `libopenvr_api.so`
//! (Linux). We load it at runtime with `libloading` so:
//! - the repo builds without any OpenVR SDK installed,
//! - desktop preview mode does not need the DLL,
//! - VR mode fails gracefully with a clear error if the runtime is missing.

pub mod sys;

use libloading::{Library, Symbol};
use std::ffi::CString;
use thiserror::Error;

pub use sys::{HmdMatrix34_t, Texture_t, VROverlayHandle_t};

#[derive(Debug, Error)]
pub enum OpenVrError {
    #[error("could not load openvr_api dynamic library: {0}")]
    LoadLib(String),
    #[error("could not resolve OpenVR export `{0}`")]
    ResolveSymbol(&'static str),
    #[error("OpenVR runtime not installed")]
    RuntimeMissing,
    #[error("No HMD present")]
    NoHmd,
    #[error("VR_InitInternal failed with code {0}")]
    InitFailed(i32),
    #[error("IVROverlay interface not available (code {0})")]
    InterfaceMissing(i32),
    #[error("Overlay op {op} failed with code {code}")]
    OverlayOp { op: &'static str, code: u32 },
    #[error("Invalid overlay key: contains nul byte")]
    InvalidKey,
}

struct OpenVrFns {
    _lib: Library, // kept alive for the symbol lifetimes below
    vr_init_internal: sys::FnVR_InitInternal,
    vr_shutdown_internal: sys::FnVR_ShutdownInternal,
    vr_is_hmd_present: sys::FnVR_IsHmdPresent,
    vr_is_runtime_installed: sys::FnVR_IsRuntimeInstalled,
    vr_get_generic_interface: sys::FnVR_GetGenericInterface,
}

impl OpenVrFns {
    fn load() -> Result<Self, OpenVrError> {
        #[cfg(target_os = "windows")]
        let lib_name = "openvr_api.dll";
        #[cfg(target_os = "linux")]
        let lib_name = "libopenvr_api.so";
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let lib_name = "openvr_api";

        let lib = unsafe { Library::new(lib_name) }
            .map_err(|e| OpenVrError::LoadLib(e.to_string()))?;
        unsafe {
            let vr_init_internal: Symbol<sys::FnVR_InitInternal> = lib
                .get(b"VR_InitInternal\0")
                .map_err(|_| OpenVrError::ResolveSymbol("VR_InitInternal"))?;
            let vr_shutdown_internal: Symbol<sys::FnVR_ShutdownInternal> = lib
                .get(b"VR_ShutdownInternal\0")
                .map_err(|_| OpenVrError::ResolveSymbol("VR_ShutdownInternal"))?;
            let vr_is_hmd_present: Symbol<sys::FnVR_IsHmdPresent> = lib
                .get(b"VR_IsHmdPresent\0")
                .map_err(|_| OpenVrError::ResolveSymbol("VR_IsHmdPresent"))?;
            let vr_is_runtime_installed: Symbol<sys::FnVR_IsRuntimeInstalled> = lib
                .get(b"VR_IsRuntimeInstalled\0")
                .map_err(|_| OpenVrError::ResolveSymbol("VR_IsRuntimeInstalled"))?;
            let vr_get_generic_interface: Symbol<sys::FnVR_GetGenericInterface> = lib
                .get(b"VR_GetGenericInterface\0")
                .map_err(|_| OpenVrError::ResolveSymbol("VR_GetGenericInterface"))?;
            Ok(Self {
                vr_init_internal: *vr_init_internal,
                vr_shutdown_internal: *vr_shutdown_internal,
                vr_is_hmd_present: *vr_is_hmd_present,
                vr_is_runtime_installed: *vr_is_runtime_installed,
                vr_get_generic_interface: *vr_get_generic_interface,
                _lib: lib,
            })
        }
    }
}

/// Safe RAII handle to the OpenVR runtime. Shuts down on drop.
pub struct OpenVr {
    fns: OpenVrFns,
}

impl OpenVr {
    /// Load `openvr_api` and initialize the runtime as an Overlay application.
    pub fn init() -> Result<Self, OpenVrError> {
        let fns = OpenVrFns::load()?;
        unsafe {
            if !(fns.vr_is_runtime_installed)() {
                return Err(OpenVrError::RuntimeMissing);
            }
            if !(fns.vr_is_hmd_present)() {
                return Err(OpenVrError::NoHmd);
            }
            let mut err: sys::EVRInitError = 0;
            let _token = (fns.vr_init_internal)(&mut err, sys::EVRApplicationType_Overlay);
            if err != sys::EVRInitError_None {
                return Err(OpenVrError::InitFailed(err));
            }
        }
        Ok(Self { fns })
    }

    /// Acquire the IVROverlay interface. The returned pointer is owned by the
    /// OpenVR runtime and remains valid for the lifetime of `self`.
    pub fn overlay(&self) -> Result<Overlay<'_>, OpenVrError> {
        let iface_name = b"FnTable:IVROverlay_024\0";
        let mut err: sys::EVRInitError = 0;
        let ptr = unsafe {
            (self.fns.vr_get_generic_interface)(iface_name.as_ptr() as *const _, &mut err)
        };
        if ptr.is_null() || err != sys::EVRInitError_None {
            return Err(OpenVrError::InterfaceMissing(err));
        }
        Ok(Overlay {
            table: ptr as *const sys::IVROverlayFnTable,
            _parent: std::marker::PhantomData,
        })
    }
}

impl Drop for OpenVr {
    fn drop(&mut self) {
        unsafe { (self.fns.vr_shutdown_internal)() };
    }
}

pub struct Overlay<'a> {
    table: *const sys::IVROverlayFnTable,
    _parent: std::marker::PhantomData<&'a OpenVr>,
}

impl<'a> Overlay<'a> {
    fn table(&self) -> &sys::IVROverlayFnTable {
        unsafe { &*self.table }
    }

    pub fn create(&self, key: &str, name: &str) -> Result<VROverlayHandle_t, OpenVrError> {
        let k = CString::new(key).map_err(|_| OpenVrError::InvalidKey)?;
        let n = CString::new(name).map_err(|_| OpenVrError::InvalidKey)?;
        let mut handle: VROverlayHandle_t = 0;
        let r = unsafe { (self.table().CreateOverlay)(k.as_ptr(), n.as_ptr(), &mut handle) };
        check(r, "CreateOverlay")?;
        Ok(handle)
    }

    pub fn destroy(&self, handle: VROverlayHandle_t) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().DestroyOverlay)(handle) };
        check(r, "DestroyOverlay")
    }

    pub fn show(&self, handle: VROverlayHandle_t) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().ShowOverlay)(handle) };
        check(r, "ShowOverlay")
    }

    pub fn hide(&self, handle: VROverlayHandle_t) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().HideOverlay)(handle) };
        check(r, "HideOverlay")
    }

    pub fn set_width(&self, handle: VROverlayHandle_t, meters: f32) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayWidthInMeters)(handle, meters) };
        check(r, "SetOverlayWidthInMeters")
    }

    pub fn set_transform_absolute(
        &self,
        handle: VROverlayHandle_t,
        m: &HmdMatrix34_t,
    ) -> Result<(), OpenVrError> {
        let r = unsafe {
            (self.table().SetOverlayTransformAbsolute)(
                handle,
                sys::ETrackingUniverseOrigin_Standing,
                m,
            )
        };
        check(r, "SetOverlayTransformAbsolute")
    }

    /// Submit a DirectX 11 shared texture to the overlay.
    ///
    /// # Safety
    /// `d3d11_texture_ptr` must be a valid `ID3D11Texture2D*` whose lifetime
    /// extends past this call. OpenVR will take an internal reference.
    pub unsafe fn set_texture_d3d11(
        &self,
        handle: VROverlayHandle_t,
        d3d11_texture_ptr: *mut std::os::raw::c_void,
    ) -> Result<(), OpenVrError> {
        let tex = Texture_t {
            handle: d3d11_texture_ptr,
            eType: sys::ETextureType_DirectX,
            eColorSpace: sys::EColorSpace_Auto,
        };
        let r = (self.table().SetOverlayTexture)(handle, &tex);
        check(r, "SetOverlayTexture")
    }

    /// Submit a Direct3D 12 texture to the overlay.
    ///
    /// # Safety
    /// `resource_ptr` must be a valid `ID3D12Resource*` whose lifetime
    /// extends past this call. `command_queue_ptr` must be the
    /// `ID3D12CommandQueue*` the resource was most recently rendered on
    /// — OpenVR synchronizes with that queue before sampling.
    pub unsafe fn set_texture_d3d12(
        &self,
        handle: VROverlayHandle_t,
        resource_ptr: *mut std::os::raw::c_void,
        command_queue_ptr: *mut std::os::raw::c_void,
    ) -> Result<(), OpenVrError> {
        // D3D12TextureData_t is what OpenVR actually reads from
        // `Texture_t.handle` when eType == DirectX12. See sys.rs for the
        // layout.
        let data = sys::D3D12TextureData_t {
            m_pResource: resource_ptr,
            m_pCommandQueue: command_queue_ptr,
            m_nNodeMask: 0,
        };
        let tex = Texture_t {
            handle: &data as *const _ as *mut std::os::raw::c_void,
            eType: sys::ETextureType_DirectX12,
            eColorSpace: sys::EColorSpace_Auto,
        };
        let r = (self.table().SetOverlayTexture)(handle, &tex);
        check(r, "SetOverlayTexture (D3D12)")
    }

    pub fn clear_texture(&self, handle: VROverlayHandle_t) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().ClearOverlayTexture)(handle) };
        check(r, "ClearOverlayTexture")
    }
}

fn check(code: sys::EVROverlayError, op: &'static str) -> Result<(), OpenVrError> {
    if code == sys::EVROverlayError_None {
        Ok(())
    } else {
        Err(OpenVrError::OverlayOp { op, code })
    }
}

/// Build a row-major 3x4 pose matrix placing a point at `(x, y, z)` facing -Z.
pub fn pose_translation(x: f32, y: f32, z: f32) -> HmdMatrix34_t {
    HmdMatrix34_t {
        m: [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, y],
            [0.0, 0.0, 1.0, z],
        ],
    }
}
