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
    pub _pad_01: [*const c_void; 7], // 3..=9
    /// `VROverlayFlags` is a C enum (int-sized).
    pub SetOverlayFlag: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, eOverlayFlag: i32, bEnabled: bool) -> EVROverlayError, // 10
    pub _pad_01a: [*const c_void; 2], // 11..=12
    pub SetOverlayColor: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fRed: f32, fGreen: f32, fBlue: f32) -> EVROverlayError, // 13
    pub _pad_01b: [*const c_void; 1], // 14 GetOverlayColor
    pub SetOverlayAlpha: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fAlpha: f32) -> EVROverlayError, // 15
    pub _pad_01c: [*const c_void; 1], // 16 GetOverlayAlpha
    pub SetOverlayTexelAspect: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fTexelAspect: f32) -> EVROverlayError, // 17
    pub _pad_01d: [*const c_void; 1], // 18 GetOverlayTexelAspect
    pub SetOverlaySortOrder: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, unSortOrder: u32) -> EVROverlayError, // 19
    pub _pad_01e: [*const c_void; 1], // 20 GetOverlaySortOrder
    pub SetOverlayWidthInMeters: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fWidthInMeters: f32) -> EVROverlayError,
    pub _pad_02: [*const c_void; 1], // 22 GetOverlayWidthInMeters
    pub SetOverlayCurvature: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, fCurvature: f32) -> EVROverlayError, // 23
    pub _pad_02b: [*const c_void; 6], // 24..=29
    pub SetOverlayTransformAbsolute: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, eTrackingOrigin: ETrackingUniverseOrigin, pMatTrackingOriginToOverlayTransform: *const HmdMatrix34_t) -> EVROverlayError,
    pub _pad_03: [*const c_void; 1], // 31 GetOverlayTransformAbsolute
    pub SetOverlayTransformTrackedDeviceRelative: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, unTrackedDevice: TrackedDeviceIndex_t, pmatTrackedDeviceToOverlayTransform: *const HmdMatrix34_t) -> EVROverlayError, // 32
    pub _pad_03b: [*const c_void; 7], // 33..=39
    pub ShowOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub HideOverlay: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub _pad_04: [*const c_void; 7], // 42..=48
    pub ComputeOverlayIntersection: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, pParams: *const VROverlayIntersectionParams_t, pResults: *mut VROverlayIntersectionResults_t) -> bool, // 49
    pub _pad_04b: [*const c_void; 6], // 50..=55
    pub SetOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t, pTexture: *const Texture_t) -> EVROverlayError,
    pub ClearOverlayTexture: unsafe extern "system" fn(ulOverlayHandle: VROverlayHandle_t) -> EVROverlayError,
    pub _pad_05: [*const c_void; 6], // 58..=63
    /// Is SteamVR's own dashboard (system menu) open?
    pub IsDashboardVisible: unsafe extern "system" fn() -> bool, // 64
}

// Slot indices verified against `openvr_capi.h` at SDK tag v1.12.5,
// the release that defines IVROverlay_024. The asserts pin each bound
// function to its slot so a padding edit can't silently shift the table.
const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVROverlayFnTable, DestroyOverlay) == 2 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayFlag) == 10 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayColor) == 13 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayAlpha) == 15 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTexelAspect) == 17 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlaySortOrder) == 19 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayCurvature) == 23 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTransformTrackedDeviceRelative) == 32 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ComputeOverlayIntersection) == 49 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayWidthInMeters) == 21 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTransformAbsolute) == 30 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ShowOverlay) == 40 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, HideOverlay) == 41 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, SetOverlayTexture) == 56 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, ClearOverlayTexture) == 57 * P);
    assert!(std::mem::offset_of!(IVROverlayFnTable, IsDashboardVisible) == 64 * P);
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

/// `VROverlayFlags_MakeOverlaysInteractiveIfVisible`: while such an
/// overlay is visible SteamVR enters laser-mouse mode and the game loses
/// input focus.
pub const VROverlayFlags_MakeOverlaysInteractiveIfVisible: i32 = 65536;

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
/// Headset refresh rate, Hz.
pub const Prop_DisplayFrequency_Float: ETrackedDeviceProperty = 2002;
pub const Prop_SecondsFromVsyncToPhotons_Float: ETrackedDeviceProperty = 2001;
/// Driver resource folder name, e.g. "indexcontroller".
pub const Prop_ResourceRoot_String: ETrackedDeviceProperty = 1035;
/// Icon SteamVR shows for the device when it's ready, e.g.
/// "{indexcontroller}/icons/left_controller_status_ready.png".
pub const Prop_NamedIconPathDeviceReady_String: ETrackedDeviceProperty = 5004;
pub const Prop_DeviceIsCharging_Bool: ETrackedDeviceProperty = 1011;
pub const Prop_DeviceBatteryPercentage_Float: ETrackedDeviceProperty = 1012;
pub const Prop_DeviceProvidesBatteryStatus_Bool: ETrackedDeviceProperty = 1026;

pub type ETrackedPropertyError = i32;
pub const ETrackedPropertyError_Success: ETrackedPropertyError = 0;

#[repr(C)]
pub struct IVRSystemFnTable {
    pub GetRecommendedRenderTargetSize: unsafe extern "system" fn(pnWidth: *mut u32, pnHeight: *mut u32), // 0
    pub _pad_00a: [*const c_void; 1], // 1 GetProjectionMatrix
    pub GetProjectionRaw: unsafe extern "system" fn(eEye: i32, pfLeft: *mut f32, pfRight: *mut f32, pfTop: *mut f32, pfBottom: *mut f32), // 2
    pub _pad_00b0: [*const c_void; 2], // 3 ComputeDistortion, 4 GetEyeToHeadTransform
    pub GetTimeSinceLastVsync: unsafe extern "system" fn(pfSecondsSinceLastVsync: *mut f32, pulFrameCounter: *mut u64) -> bool, // 5
    /// Slots 6..=10: adapter / display. Unused here.
    pub _pad_00: [*const c_void; 5],
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
    pub _pad_02: [*const c_void; 1], // 28 GetPropErrorNameFromEnum
    pub PollNextEvent: unsafe extern "system" fn(pEvent: *mut VREvent_t, uncbVREvent: u32) -> bool, // 29
    pub _pad_02b: [*const c_void; 3], // 30..=32: event with pose, event name, hidden-area mesh
    pub GetControllerState: unsafe extern "system" fn(unControllerDeviceIndex: TrackedDeviceIndex_t, pControllerState: *mut VRControllerState_t, unControllerStateSize: u32) -> bool, // 33
    pub _pad_03: [*const c_void; 1], // 34 GetControllerStateWithPose
    pub TriggerHapticPulse: unsafe extern "system" fn(unControllerDeviceIndex: TrackedDeviceIndex_t, unAxisId: u32, usDurationMicroSec: u16), // 35
    pub _pad_04: [*const c_void; 7], // 36..=42: enum names, input/pause queries, firmware
    pub AcknowledgeQuit_Exiting: unsafe extern "system" fn(), // 43
}

/// VREvent_t. Only the header is read; `data` is the 48-byte union
/// (largest member: VREvent_Reserved_t), 8-aligned, so 64 bytes total.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct VREvent_t {
    pub eventType: u32,
    pub trackedDeviceIndex: TrackedDeviceIndex_t,
    pub eventAgeSeconds: f32,
    pub data: [u64; 6],
}

pub const VREvent_Quit: u32 = 700;

const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVRSystemFnTable, PollNextEvent) == 29 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, TriggerHapticPulse) == 35 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, AcknowledgeQuit_Exiting) == 43 * P);
    assert!(std::mem::offset_of!(VREvent_t, data) == 16);
    assert!(std::mem::size_of::<VREvent_t>() == 64);
};

// ---------------------------------------------------------------------------
// IVRApplications_007: registering our app manifest so SteamVR lists
// RSOverlay as an overlay app and can start it with SteamVR (auto-launch).
// Slots verified against openvr_capi.h @ v2.5.1; only the prefix we call.
// ---------------------------------------------------------------------------

pub const IVRApplications_FnTable_Version: &[u8] = b"FnTable:IVRApplications_007\0";
pub type EVRApplicationError = i32;
pub const EVRApplicationError_None: EVRApplicationError = 0;

#[repr(C)]
pub struct IVRApplicationsFnTable {
    pub AddApplicationManifest: unsafe extern "system" fn(pchApplicationManifestFullPath: *const c_char, bTemporary: bool) -> EVRApplicationError, // 0
    pub _pad_00: [*const c_void; 1], // 1 RemoveApplicationManifest
    pub IsApplicationInstalled: unsafe extern "system" fn(pchAppKey: *const c_char) -> bool, // 2
    pub _pad_01: [*const c_void; 14], // 3..=16: enumerate, launch, properties
    pub SetApplicationAutoLaunch: unsafe extern "system" fn(pchAppKey: *const c_char, bAutoLaunch: bool) -> EVRApplicationError, // 17
    pub GetApplicationAutoLaunch: unsafe extern "system" fn(pchAppKey: *const c_char) -> bool, // 18
}

const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVRApplicationsFnTable, IsApplicationInstalled) == 2 * P);
    assert!(std::mem::offset_of!(IVRApplicationsFnTable, SetApplicationAutoLaunch) == 17 * P);
    assert!(std::mem::offset_of!(IVRApplicationsFnTable, GetApplicationAutoLaunch) == 18 * P);
};

pub const k_unTrackedDeviceIndexInvalid: TrackedDeviceIndex_t = 0xFFFF_FFFF;
/// Bits in VRControllerState_t::ulButtonPressed (EVRButtonId).
pub const k_EButton_ApplicationMenu: u32 = 1; // B / menu
pub const k_EButton_Grip: u32 = 2;
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
    assert!(std::mem::offset_of!(IVRSystemFnTable, GetProjectionRaw) == 2 * P);
    assert!(std::mem::offset_of!(IVRSystemFnTable, GetTimeSinceLastVsync) == 5 * P);
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

// ---------------------------------------------------------------------------
// IVRInput_010 — SteamVR Input (action manifest + per-controller bindings,
// user-rebindable in SteamVR). Slots verified against openvr_capi.h @ v2.5.1.
// Struct layouts use Windows natural packing (asserted below).
// ---------------------------------------------------------------------------

pub const IVRInput_FnTable_Version: &[u8] = b"FnTable:IVRInput_010\0";

pub type VRActionHandle_t = u64;
pub type VRActionSetHandle_t = u64;
pub type VRInputValueHandle_t = u64;
pub type EVRInputError = i32;
pub const EVRInputError_None: EVRInputError = 0;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct VRActiveActionSet_t {
    pub ulActionSet: VRActionSetHandle_t,
    pub ulRestrictedToDevice: VRInputValueHandle_t,
    pub ulSecondaryActionSet: VRActionSetHandle_t,
    pub unPadding: u32,
    pub nPriority: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct InputDigitalActionData_t {
    pub bActive: bool,
    pub activeOrigin: VRInputValueHandle_t,
    pub bState: bool,
    pub bChanged: bool,
    pub fUpdateTime: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct InputAnalogActionData_t {
    pub bActive: bool,
    pub activeOrigin: VRInputValueHandle_t,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub deltaX: f32,
    pub deltaY: f32,
    pub deltaZ: f32,
    pub fUpdateTime: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct InputPoseActionData_t {
    pub bActive: bool,
    pub activeOrigin: VRInputValueHandle_t,
    pub pose: TrackedDevicePose_t,
}

#[repr(C)]
pub struct IVRInputFnTable {
    pub SetActionManifestPath: unsafe extern "system" fn(pchActionManifestPath: *const c_char) -> EVRInputError, // 0
    pub GetActionSetHandle: unsafe extern "system" fn(pchActionSetName: *const c_char, pHandle: *mut VRActionSetHandle_t) -> EVRInputError, // 1
    pub GetActionHandle: unsafe extern "system" fn(pchActionName: *const c_char, pHandle: *mut VRActionHandle_t) -> EVRInputError, // 2
    pub GetInputSourceHandle: unsafe extern "system" fn(pchInputSourcePath: *const c_char, pHandle: *mut VRInputValueHandle_t) -> EVRInputError, // 3
    pub UpdateActionState: unsafe extern "system" fn(pSets: *mut VRActiveActionSet_t, unSizeOfVRSelectedActionSet_t: u32, unSetCount: u32) -> EVRInputError, // 4
    pub GetDigitalActionData: unsafe extern "system" fn(action: VRActionHandle_t, pActionData: *mut InputDigitalActionData_t, unActionDataSize: u32, ulRestrictToDevice: VRInputValueHandle_t) -> EVRInputError, // 5
    pub GetAnalogActionData: unsafe extern "system" fn(action: VRActionHandle_t, pActionData: *mut InputAnalogActionData_t, unActionDataSize: u32, ulRestrictToDevice: VRInputValueHandle_t) -> EVRInputError, // 6
    pub GetPoseActionDataRelativeToNow: unsafe extern "system" fn(action: VRActionHandle_t, eOrigin: ETrackingUniverseOrigin, fPredictedSecondsFromNow: f32, pActionData: *mut InputPoseActionData_t, unActionDataSize: u32, ulRestrictToDevice: VRInputValueHandle_t) -> EVRInputError, // 7
    pub GetPoseActionDataForNextFrame: unsafe extern "system" fn(action: VRActionHandle_t, eOrigin: ETrackingUniverseOrigin, pActionData: *mut InputPoseActionData_t, unActionDataSize: u32, ulRestrictToDevice: VRInputValueHandle_t) -> EVRInputError, // 8
    pub _pad_00b: [*const c_void; 12], // 9..=20: skeletal, dominant hand
    pub TriggerHapticVibrationAction: unsafe extern "system" fn(action: VRActionHandle_t, fStartSecondsFromNow: f32, fDurationSeconds: f32, fFrequency: f32, fAmplitude: f32, ulRestrictToDevice: VRInputValueHandle_t) -> EVRInputError, // 21
}

const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVRInputFnTable, GetAnalogActionData) == 6 * P);
    assert!(std::mem::offset_of!(IVRInputFnTable, TriggerHapticVibrationAction) == 21 * P);
    assert!(std::mem::size_of::<VRActiveActionSet_t>() == 32);
    assert!(std::mem::size_of::<InputDigitalActionData_t>() == 24);
    assert!(std::mem::offset_of!(InputDigitalActionData_t, bState) == 16);
    assert!(std::mem::size_of::<InputAnalogActionData_t>() == 48);
    assert!(std::mem::offset_of!(InputAnalogActionData_t, x) == 16);
    assert!(std::mem::offset_of!(IVRInputFnTable, GetPoseActionDataForNextFrame) == 8 * P);
    assert!(std::mem::offset_of!(IVRInputFnTable, GetPoseActionDataRelativeToNow) == 7 * P);
    assert!(std::mem::offset_of!(InputPoseActionData_t, pose) == 16);
    assert!(std::mem::size_of::<InputPoseActionData_t>() == 96);
};

// ── IVRCompositor (frame timing only) ──────────────────────────────────
// Verbatim from ValveSoftware/openvr headers/openvr_capi.h,
// `struct VR_IVRCompositor_FnTable`, IVRCompositor_029. We only call
// GetFrameTimings (slot 11); earlier slots are opaque pointers so the
// offsets match. Only the 029 table is requested: if the runtime doesn't
// offer it, frame stats are simply unavailable (no layout guessing).

pub const IVRCompositor_FnTable_Version: &[u8] = b"FnTable:IVRCompositor_029\0";

#[repr(C)]
#[allow(non_snake_case)]
pub struct IVRCompositorFnTable {
    pub SetTrackingSpace: *const c_void,                 // 0
    pub GetTrackingSpace: *const c_void,                 // 1
    pub WaitGetPoses: *const c_void,                     // 2
    pub GetLastPoses: *const c_void,                     // 3
    pub GetLastPoseForTrackedDeviceIndex: *const c_void, // 4
    pub GetSubmitTexture: *const c_void,                 // 5
    pub Submit: *const c_void,                           // 6
    pub SubmitWithArrayIndex: *const c_void,             // 7
    pub ClearLastSubmittedFrame: *const c_void,          // 8
    pub PostPresentHandoff: *const c_void,               // 9
    pub GetFrameTiming: unsafe extern "system" fn(pTiming: *mut Compositor_FrameTiming, unFramesAgo: u32) -> bool, // 10
    pub GetFrameTimings: unsafe extern "system" fn(pTiming: *mut Compositor_FrameTiming, nFrames: u32) -> u32, // 11
}

/// `Compositor_FrameTiming` (natural alignment: no #pragma pack in the
/// header for it). `m_nSize` must be set before the call.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
pub struct Compositor_FrameTiming {
    pub m_nSize: u32,
    pub m_nFrameIndex: u32,
    pub m_nNumFramePresents: u32,
    pub m_nNumMisPresented: u32,
    pub m_nNumDroppedFrames: u32,
    pub m_nReprojectionFlags: u32,
    pub m_flSystemTimeInSeconds: f64,
    pub m_flPreSubmitGpuMs: f32,
    pub m_flPostSubmitGpuMs: f32,
    pub m_flTotalRenderGpuMs: f32,
    pub m_flCompositorRenderGpuMs: f32,
    pub m_flCompositorRenderCpuMs: f32,
    pub m_flCompositorIdleCpuMs: f32,
    pub m_flClientFrameIntervalMs: f32,
    pub m_flPresentCallCpuMs: f32,
    pub m_flWaitForPresentCpuMs: f32,
    pub m_flSubmitFrameMs: f32,
    pub m_flWaitGetPosesCalledMs: f32,
    pub m_flNewPosesReadyMs: f32,
    pub m_flNewFrameReadyMs: f32,
    pub m_flCompositorUpdateStartMs: f32,
    pub m_flCompositorUpdateEndMs: f32,
    pub m_flCompositorRenderStartMs: f32,
    pub m_HmdPose: TrackedDevicePose_t,
    pub m_nNumVSyncsReadyForUse: u32,
    pub m_nNumVSyncsToFirstView: u32,
    pub m_flTransferLatencyMs: f32,
}

impl Default for Compositor_FrameTiming {
    fn default() -> Self {
        // SAFETY: plain-old-data (ints, floats, bools inside the pose);
        // all-zero is a valid value for every field.
        unsafe { std::mem::zeroed() }
    }
}

const _: () = {
    const P: usize = std::mem::size_of::<*const c_void>();
    assert!(std::mem::offset_of!(IVRCompositorFnTable, GetFrameTiming) == 10 * P);
    assert!(std::mem::offset_of!(IVRCompositorFnTable, GetFrameTimings) == 11 * P);
    // 6 × u32, then the f64 at 24 (8-aligned), 16 × f32 → pose at 96,
    // 80-byte pose → 176, 2 × u32 + f32 → 188, padded to 8 → 192.
    assert!(std::mem::offset_of!(Compositor_FrameTiming, m_flSystemTimeInSeconds) == 24);
    assert!(std::mem::offset_of!(Compositor_FrameTiming, m_flClientFrameIntervalMs) == 56);
    assert!(std::mem::offset_of!(Compositor_FrameTiming, m_HmdPose) == 96);
    assert!(std::mem::offset_of!(Compositor_FrameTiming, m_flTransferLatencyMs) == 184);
    assert!(std::mem::size_of::<Compositor_FrameTiming>() == 192);
};
