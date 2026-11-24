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
    pub _pad_01: [*const c_void; 10], // 3..=12
    pub SetOverlayColor: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fRed: f32, fGreen: f32, fBlue: f32) -> EVROverlayError, // 13
    pub _pad_01b: [*const c_void; 1], // 14 GetOverlayColor
    pub SetOverlayAlpha: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fAlpha: f32) -> EVROverlayError, // 15
    pub _pad_01c: [*const c_void; 5], // 16..=20
    pub SetOverlayWidthInMeters: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fWidthInMeters: f32) -> EVROverlayError,
    pub _pad_02: [*const c_void; 1], // 22 GetOverlayWidthInMeters
    pub SetOverlayCurvature: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fCurvature: f32) -> EVROverlayError, // 23
    pub _pad_02b: [*const c_void; 6], // 24..=29
    pub SetOverlayTransformAbsolute: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, eTrackingOrigin: ETrackingUniverseOrigin, pMatTrackingOriginToOverlayTransform: *const HmdMatrix34_t) -> EVROverlayError,
    pub _pad_03: [*const c_void; 9], // 31..=39
    pub ShowOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub HideOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub _pad_04: [*const c_void; 7], // 42..=48
    pub ComputeOverlayIntersection: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, pParams: *const VROverlayIntersectionParams_t, pResults: *mut VROverlayIntersectionResults_t) -> bool, // 49
    pub _pad_04b: [*const c_void; 6], // 50..=55
    pub SetOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, pTexture: *const Texture_t) -> EVROverlayError,
    pub ClearOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
}

// Slot indices verified against `openvr_capi.h` at SDK tag v1.12.5,
// the release that defines IVROverlay_024. The asserts pin each bound
// function to its slot so a padding edit can't silently shift the table.
const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVROverlayFnTable, DestroyOverlay) == 2 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayColor) == 13 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayAlpha) == 15 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayCurvature) == 23 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ComputeOverlayIntersection) == 49 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayWidthInMeters) == 21 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTransformAbsolute) == 30 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ShowOverlay) == 40 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, HideOverlay) == 41 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTexture) == 56 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ClearOverlayTexture) == 57 * P);
};

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
    /// Slots 0..=10: render targets, projection, distortion, vsync,
    /// adapter / display. Unused here.
    pub _pad_00: [*const c_void; 11],
    pub GetDeviceToAbsoluteTrackingPose: unsafe extern "system" fn(eOrigin: ETrackingUniverseOrigin, fPredictedSecondsToPhotonsFromNow: f32, pTrackedDevicePoseArray: *mut TrackedDevicePose_t, unTrackedDevicePoseArrayCount: u32), // 11
    pub _pad_00b: [*const c_void; 5], // 12..=16
    pub GetTrackedDeviceIndexForControllerRole: unsafe extern "system" fn(unDeviceType: ETrackedControllerRole) -> TrackedDeviceIndex_t, // 17
    pub GetControllerRoleForTrackedDeviceIndex: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> ETrackedControllerRole, // 18
    pub GetTrackedDeviceClass: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> ETrackedDeviceClass, // 19
    pub IsTrackedDeviceConnected: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t) -> bool, // 20
    pub GetBoolTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pError: *mut ETrackedPropertyError) -> bool, // 21
    pub GetFloatTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pError: *mut ETrackedPropertyError) -> f32, // 22
    pub _pad_01: [*const c_void; 4], // 23..=26: Int32 / Uint64 / Matrix34 / Array
    pub GetStringTrackedDeviceProperty: unsafe extern "system" fn(unDeviceIndex: TrackedDeviceIndex_t, prop: ETrackedDeviceProperty, pchValue: *mut c_char, unBufferSize: u32, pError: *mut ETrackedPropertyError) -> u32, // 27
    pub _pad_02: [*const c_void; 5], // 28..=32: prop error name, poll events, hidden-area mesh
    pub GetControllerState: unsafe extern "system" fn(unControllerDeviceIndex: TrackedDeviceIndex_t, pControllerState: *mut VRControllerState_t, unControllerStateSize: u32) -> bool, // 33
    pub _pad_03: [*const c_void; 1], // 34 GetControllerStateWithPose
    pub TriggerHapticPulse: unsafe extern "system" fn(unControllerDeviceIndex: TrackedDeviceIndex_t, unAxisId: u32, usDurationMicroSec: u16), // 35
}

pub const k_unTrackedDeviceIndexInvalid: TrackedDeviceIndex_t = 0xFFFF_FFFF;
/// Bit in VRControllerState_t::ulButtonPressed (EVRButtonId 33).
pub const k_EButton_SteamVR_Trigger: u32 = 33;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct HmdVector3_t {
    pub v: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct HmdVector2_t {
    pub v: [f32; 2],
}

/// Natural (8-byte) packing on Windows — the header only forces
/// pack(4) on Linux/macOS — so there's a 4-byte hole after
/// unPacketNum and the struct is 64 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct VRControllerState_t {
    pub unPacketNum: u32,
    pub ulButtonPressed: u64,
    pub ulButtonTouched: u64,
    /// [0] = trackpad / thumbstick, [1] = trigger (x = pull 0..1).
    pub rAxis: [[f32; 2]; 5],
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TrackedDevicePose_t {
    pub mDeviceToAbsoluteTracking: HmdMatrix34_t,
    pub vVelocity: HmdVector3_t,
    pub vAngularVelocity: HmdVector3_t,
    pub eTrackingResult: i32,
    pub bPoseIsValid: bool,
    pub bDeviceIsConnected: bool,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VROverlayIntersectionParams_t {
    pub vSource: HmdVector3_t,
    pub vDirection: HmdVector3_t,
    pub eOrigin: ETrackingUniverseOrigin,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct VROverlayIntersectionResults_t {
    pub vPoint: HmdVector3_t,
    pub vNormal: HmdVector3_t,
    pub vUVs: HmdVector2_t,
    pub fDistance: f32,
}

const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVRSystemFnTable, GetDeviceToAbsoluteTrackingPose) == 11 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, GetTrackedDeviceIndexForControllerRole) == 17 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, GetControllerState) == 33 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, TriggerHapticPulse) == 35 * P);
    assert!(std::mem::size_of::<VRControllerState_t>() == 64);
    assert!(std::mem::offset_of!(VRControllerState_t, ulButtonPressed) == 8);
    assert!(std::mem::size_of::<TrackedDevicePose_t>() == 80);
    assert!(std::mem::size_of::<VROverlayIntersectionParams_t>() == 28);
    assert!(std::mem::size_of::<VROverlayIntersectionResults_t>() == 36);
};

// Compile-time guard: slot 27 must sit at byte offset 27 * ptr_size.
const _: () = assert!(
    std::mem::offset_of!(IVRSystemFnTable, GetStringTrackedDeviceProperty)
        == 27 * std::mem::size_of::<*const c_void>()
);
