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

impl OpenVr {
    /// Acquire the IVRSystem interface (tracked devices + properties).
    pub fn system(&self) -> Result<System<'_>, OpenVrError> {
        let mut err: sys::EVRInitError = 0;
        let ptr = unsafe {
            (self.fns.vr_get_generic_interface)(
                sys::IVRSystem_FnTable_Version.as_ptr() as *const _,
                &mut err,
            )
        };
        if ptr.is_null() || err != sys::EVRInitError_None {
            return Err(OpenVrError::InterfaceMissing(err));
        }
        Ok(System {
            table: ptr as *const sys::IVRSystemFnTable,
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

    /// Multiply the overlay's colour in the compositor (0..=1 per
    /// channel). Used for night light — zero render cost on our side.
    pub fn set_color(&self, handle: VROverlayHandle_t, rgb: [f32; 3]) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayColor)(handle, rgb[0], rgb[1], rgb[2]) };
        check(r, "SetOverlayColor")
    }

    pub fn set_alpha(&self, handle: VROverlayHandle_t, alpha: f32) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayAlpha)(handle, alpha) };
        check(r, "SetOverlayAlpha")
    }

    pub fn set_curvature(&self, handle: VROverlayHandle_t, curvature: f32) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayCurvature)(handle, curvature.clamp(0.0, 1.0)) };
        check(r, "SetOverlayCurvature")
    }

    /// Cast a ray (standing-space origin + direction) at the overlay.
    /// Returns the hit UV (as OpenVR reports it) and distance in metres.
    /// The compositor accounts for curvature and texture bounds.
    pub fn intersect(&self, handle: VROverlayHandle_t, origin: [f32; 3], dir: [f32; 3]) -> Option<([f32; 2], f32)> {
        let params = sys::VROverlayIntersectionParams_t {
            vSource: sys::HmdVector3_t { v: origin },
            vDirection: sys::HmdVector3_t { v: dir },
            eOrigin: sys::ETrackingUniverseOrigin_Standing,
        };
        let mut out = sys::VROverlayIntersectionResults_t::default();
        let hit = unsafe { (self.table().ComputeOverlayIntersection)(handle, &params, &mut out) };
        hit.then_some((out.vUVs.v, out.fDistance))
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

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

/// One hand controller's pointing ray + inputs for this frame.
#[derive(Copy, Clone, Debug)]
pub struct Controller {
    pub index: u32,
    pub hand: Hand,
    /// Standing-space ray origin (metres) and unit direction.
    pub origin: [f32; 3],
    pub dir: [f32; 3],
    /// Trigger pull 0..=1.
    pub trigger: f32,
    /// Thumbstick / trackpad (x, y), each -1..=1.
    pub stick: [f32; 2],
}

/// Pointing ray of a tracked pose: origin = translation column,
/// direction = the device's −Z axis (OpenVR's "forward").
pub fn ray_from_pose(m: &HmdMatrix34_t) -> ([f32; 3], [f32; 3]) {
    let m = &m.m;
    let origin = [m[0][3], m[1][3], m[2][3]];
    let d = [-m[0][2], -m[1][2], -m[2][2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(f32::EPSILON);
    (origin, [d[0] / len, d[1] / len, d[2] / len])
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Hmd,
    LeftHand,
    RightHand,
    /// Controller with no hand role assigned (e.g. a gamepad).
    Controller,
    Tracker,
}

/// One battery-reporting tracked device.
#[derive(Clone, Debug)]
pub struct DeviceBattery {
    pub index: u32,
    pub kind: DeviceKind,
    pub model: String,
    /// 0.0..=1.0, as OpenVR reports it.
    pub fraction: f32,
    pub charging: bool,
}

pub struct System<'a> {
    table: *const sys::IVRSystemFnTable,
    _parent: std::marker::PhantomData<&'a OpenVr>,
}

impl<'a> System<'a> {
    fn table(&self) -> &sys::IVRSystemFnTable {
        unsafe { &*self.table }
    }

    /// Every connected device that reports battery status, in OpenVR
    /// index order (HMD is always index 0 when present). Lighthouse base
    /// stations are skipped — they're mains-powered.
    pub fn batteries(&self) -> Vec<DeviceBattery> {
        let t = self.table();
        let mut out = Vec::new();
        for i in 0..sys::k_unMaxTrackedDeviceCount {
            unsafe {
                if !(t.IsTrackedDeviceConnected)(i) {
                    continue;
                }
                let kind = match (t.GetTrackedDeviceClass)(i) {
                    sys::ETrackedDeviceClass_HMD => DeviceKind::Hmd,
                    sys::ETrackedDeviceClass_Controller => {
                        match (t.GetControllerRoleForTrackedDeviceIndex)(i) {
                            sys::ETrackedControllerRole_LeftHand => DeviceKind::LeftHand,
                            sys::ETrackedControllerRole_RightHand => DeviceKind::RightHand,
                            _ => DeviceKind::Controller,
                        }
                    }
                    sys::ETrackedDeviceClass_GenericTracker => DeviceKind::Tracker,
                    _ => continue,
                };
                if self.bool_prop(i, sys::Prop_DeviceProvidesBatteryStatus_Bool) != Some(true) {
                    continue;
                }
                let Some(fraction) = self.float_prop(i, sys::Prop_DeviceBatteryPercentage_Float)
                else {
                    continue;
                };
                out.push(DeviceBattery {
                    index: i,
                    kind,
                    model: self.string_prop(i, sys::Prop_ModelNumber_String).unwrap_or_default(),
                    fraction: fraction.clamp(0.0, 1.0),
                    charging: self.bool_prop(i, sys::Prop_DeviceIsCharging_Bool).unwrap_or(false),
                });
            }
        }
        out
    }

    /// Left + right hand controllers with a valid pose this frame.
    pub fn controllers(&self) -> Vec<Controller> {
        let t = self.table();
        let mut poses = [sys::TrackedDevicePose_t {
            mDeviceToAbsoluteTracking: HmdMatrix34_t { m: [[0.0; 4]; 3] },
            vVelocity: Default::default(),
            vAngularVelocity: Default::default(),
            eTrackingResult: 0,
            bPoseIsValid: false,
            bDeviceIsConnected: false,
        }; sys::k_unMaxTrackedDeviceCount as usize];
        unsafe {
            (t.GetDeviceToAbsoluteTrackingPose)(
                sys::ETrackingUniverseOrigin_Standing,
                0.0,
                poses.as_mut_ptr(),
                poses.len() as u32,
            );
        }
        let mut out = Vec::with_capacity(2);
        for (hand, role) in [
            (Hand::Left, sys::ETrackedControllerRole_LeftHand),
            (Hand::Right, sys::ETrackedControllerRole_RightHand),
        ] {
            let idx = unsafe { (t.GetTrackedDeviceIndexForControllerRole)(role) };
            let Some(pose) = poses.get(idx as usize).filter(|p| p.bPoseIsValid) else { continue };
            let mut st = sys::VRControllerState_t::default();
            let ok = unsafe {
                (t.GetControllerState)(idx, &mut st, std::mem::size_of::<sys::VRControllerState_t>() as u32)
            };
            if !ok {
                continue;
            }
            let (origin, dir) = ray_from_pose(&pose.mDeviceToAbsoluteTracking);
            out.push(Controller {
                index: idx,
                hand,
                origin,
                dir,
                // Analog pull where reported; fall back to the digital bit
                // (some wands report 0.0 analog with the button down).
                trigger: st.rAxis[1][0].max(
                    if st.ulButtonPressed & (1u64 << sys::k_EButton_SteamVR_Trigger) != 0 { 1.0 } else { 0.0 },
                ),
                stick: st.rAxis[0],
            });
        }
        out
    }

    /// Short haptic tick. `micros` is capped at 3999 by OpenVR.
    pub fn haptic(&self, index: u32, micros: u16) {
        unsafe { (self.table().TriggerHapticPulse)(index, 0, micros.min(3999)) }
    }

    fn bool_prop(&self, i: u32, prop: sys::ETrackedDeviceProperty) -> Option<bool> {
        let mut err = 0;
        let v = unsafe { (self.table().GetBoolTrackedDeviceProperty)(i, prop, &mut err) };
        (err == sys::ETrackedPropertyError_Success).then_some(v)
    }

    fn float_prop(&self, i: u32, prop: sys::ETrackedDeviceProperty) -> Option<f32> {
        let mut err = 0;
        let v = unsafe { (self.table().GetFloatTrackedDeviceProperty)(i, prop, &mut err) };
        (err == sys::ETrackedPropertyError_Success).then_some(v)
    }

    fn string_prop(&self, i: u32, prop: sys::ETrackedDeviceProperty) -> Option<String> {
        // k_unMaxPropertyStringSize is 32 KiB, but model numbers are
        // short; 256 bytes covers every shipping device. Over-long
        // values report BufferTooSmall and we fall back to empty.
        let mut buf = [0 as std::os::raw::c_char; 256];
        let mut err = 0;
        let n = unsafe {
            (self.table().GetStringTrackedDeviceProperty)(
                i,
                prop,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut err,
            )
        };
        if err != sys::ETrackedPropertyError_Success || n == 0 {
            return None;
        }
        let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        Some(s.to_string_lossy().into_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_identity_points_down_negative_z() {
        let (o, d) = ray_from_pose(&pose_translation(1.0, 2.0, 3.0));
        assert_eq!(o, [1.0, 2.0, 3.0]);
        assert_eq!(d, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn ray_follows_yaw() {
        // 90° yaw left (rotation about +Y): -Z forward becomes -X.
        let m = HmdMatrix34_t { m: [[0.0, 0.0, 1.0, 0.0], [0.0, 1.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]] };
        let (_, d) = ray_from_pose(&m);
        assert!((d[0] + 1.0).abs() < 1e-6 && d[1].abs() < 1e-6 && d[2].abs() < 1e-6);
    }
}
