//! Raw FFI types and bindings for OpenVR.
//!
//! Clean-room hand-rolled from the public headers (openvr_capi.h, openvr.h).
//! We bind only the handful of entry points and interface methods we actually use.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code)]

use std::os::raw::{c_char, c_void};

pub type EVRInitError = i32;
pub type EVROverlayError = u32;
pub type TrackedDeviceIndex_t = u32;
pub type VROverlayHandle_t = u64;
pub type EVRApplicationType = i32;

pub const EVRApplicationType_Overlay: EVRApplicationType = 2;

pub const EVRInitError_None: EVRInitError = 0;

pub const EVROverlayError_None: EVROverlayError = 0;

/// Tracking universe origin — standing room-scale frame.
pub type ETrackingUniverseOrigin = i32;
pub const ETrackingUniverseOrigin_Standing: ETrackingUniverseOrigin = 1;

pub type ETextureType = i32;
pub const ETextureType_DirectX: ETextureType = 0;
pub const ETextureType_OpenGL: ETextureType = 1;
pub const ETextureType_Vulkan: ETextureType = 2;
pub const ETextureType_DirectX12: ETextureType = 4;
pub const ETextureType_DXGISharedHandle: ETextureType = 5;

/// `D3D12TextureData_t` — the payload OpenVR expects in `Texture_t.handle`
/// when submitting a Direct3D 12 texture. The compositor needs the raw
/// resource pointer and the command queue it was last rendered on, so it
/// can synchronize with that queue before reading.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct D3D12TextureData_t {
    pub m_pResource: *mut c_void,
    pub m_pCommandQueue: *mut c_void,
    pub m_nNodeMask: u32,
}

pub type EColorSpace = i32;
pub const EColorSpace_Auto: EColorSpace = 0;
pub const EColorSpace_Gamma: EColorSpace = 1;
pub const EColorSpace_Linear: EColorSpace = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HmdMatrix34_t {
    pub m: [[f32; 4]; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Texture_t {
    pub handle: *mut c_void,
    pub eType: ETextureType,
    pub eColorSpace: EColorSpace,
}

/// Opaque interface pointer. OpenVR returns a vtable-backed struct; we only ever
/// treat it as an opaque pointer and reach into the vtable by offset via the
/// generated wrapper struct below.
#[repr(C)]
pub struct IVROverlayFnTable {
    pub FindOverlay: unsafe extern "system" fn(pchOverlayKey: *const c_char, pOverlayHandle: *mut VROverlayHandle_t) -> EVROverlayError,
    pub CreateOverlay: unsafe extern "system" fn(pchOverlayKey: *const c_char, pchOverlayName: *const c_char, pOverlayHandle: *mut VROverlayHandle_t) -> EVROverlayError,
    pub DestroyOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub _pad_01: [*const c_void; 24],
    pub SetOverlayWidthInMeters: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fWidthInMeters: f32) -> EVROverlayError,
    pub _pad_02: [*const c_void; 32],
    pub SetOverlayTransformAbsolute: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, eTrackingOrigin: ETrackingUniverseOrigin, pMatTrackingOriginToOverlayTransform: *const HmdMatrix34_t) -> EVROverlayError,
    pub _pad_03: [*const c_void; 20],
    pub ShowOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub HideOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub _pad_04: [*const c_void; 8],
    pub SetOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, pTexture: *const Texture_t) -> EVROverlayError,
    pub ClearOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
}

// Note: real vtable layout is version-specific. The padding above is a placeholder
// until we lock the OpenVR SDK version we target. In the real FFI we use the
// `FnTable:IVROverlay_NNN` interface accessor from `VR_GetGenericInterface`,
// which returns a stable function-pointer table defined per-version.
//
// For this scaffold we deliberately use the stable FnTable path, which OpenVR
// guarantees per interface-version string. The accessor pattern:
//   let p = VR_GetGenericInterface(b"FnTable:IVROverlay_024\0", &mut err);
// returns a *const IVROverlay_FnTable where every field is a function pointer
// in the exact order declared in openvr_capi.h. The full vtable is wired in
// src/overlay.rs once we pin the SDK version in the build script.

pub type FnVR_InitInternal = unsafe extern "system" fn(peError: *mut EVRInitError, eType: EVRApplicationType) -> isize;
pub type FnVR_ShutdownInternal = unsafe extern "system" fn();
pub type FnVR_IsHmdPresent = unsafe extern "system" fn() -> bool;
pub type FnVR_IsRuntimeInstalled = unsafe extern "system" fn() -> bool;
pub type FnVR_GetGenericInterface = unsafe extern "system" fn(pchInterfaceVersion: *const c_char, peError: *mut EVRInitError) -> *mut c_void;

// ---------------------------------------------------------------------------
// IVRSystem — tracked-device enumeration + properties (battery widget).
//
// Slot order verified against `openvr_capi.h` at SDK tag v2.5.1
// (IVRSystem_022). IVRSystem_023 shares the layout; _026 inserts
// `ComputeDistortionSet` at slot 4 and shifts everything after it, so we
// pin the request string to 022 — every SteamVR since 2019 serves it.
// ---------------------------------------------------------------------------

pub const IVRSystem_FnTable_Version: &[u8] = b"FnTable:IVRSystem_022\0";

pub const k_unMaxTrackedDeviceCount: u32 = 64;

pub type ETrackedDeviceClass = i32;
pub const ETrackedDeviceClass_Invalid: ETrackedDeviceClass = 0;
pub const ETrackedDeviceClass_HMD: ETrackedDeviceClass = 1;
pub const ETrackedDeviceClass_Controller: ETrackedDeviceClass = 2;
pub const ETrackedDeviceClass_GenericTracker: ETrackedDeviceClass = 3;
pub const ETrackedDeviceClass_TrackingReference: ETrackedDeviceClass = 4;

pub type ETrackedControllerRole = i32;
pub const ETrackedControllerRole_LeftHand: ETrackedControllerRole = 1;
pub const ETrackedControllerRole_RightHand: ETrackedControllerRole = 2;

pub type ETrackedDeviceProperty = i32;
pub const Prop_ModelNumber_String: ETrackedDeviceProperty = 1001;
pub const Prop_DeviceIsCharging_Bool: ETrackedDeviceProperty = 1011;
pub const Prop_DeviceBatteryPercentage_Float: ETrackedDeviceProperty = 1012;
pub const Prop_DeviceProvidesBatteryStatus_Bool: ETrackedDeviceProperty = 1026;

pub type ETrackedPropertyError = i32;
pub const ETrackedPropertyError_Success: ETrackedPropertyError = 0;

#[repr(C)]
pub struct IVRSystemFnTable {
    /// Slots 0..=17: render targets, projection, distortion, vsync,
    /// adapter, poses, sorted indices, activity, ApplyTransform,
    /// GetTrackedDeviceIndexForControllerRole. Unused here.
    pub _pad_00: [*const c_void; 18],
    pub GetControllerRoleForTrackedDeviceIndex: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> ETrackedControllerRole, // 18
    pub GetTrackedDeviceClass: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> ETrackedDeviceClass, // 19
    pub IsTrackedDeviceConnected: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> bool, // 20
    pub GetBoolTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pError: *mut ETrackedPropertyError) -> bool, // 21
    pub GetFloatTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pError: *mut ETrackedPropertyError) -> f32, // 22
    pub _pad_01: [*const c_void; 4], // 23..=26: Int32 / Uint64 / Matrix34 / Array
    pub GetStringTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pchValue: *mut c_char, unBufferSize: u32, pError: *mut ETrackedPropertyError) -> u32, // 27
}

// Compile-time guard: slot 27 must sit at byte offset 27 * ptr_size.
const _: () = assert!(
    std::mem::offset_of!(IVRSystemFnTable, GetStringTrackedDeviceProperty)
        == 27 * std::mem::size_of::<*const c_void>()
);
