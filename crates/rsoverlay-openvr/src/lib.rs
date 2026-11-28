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
    #[error("VR_InitInternal failed with code {0}: {}", init_error_hint(*.0))]
    InitFailed(i32),
    #[error("IVROverlay interface not available (code {0})")]
    InterfaceMissing(i32),
    #[error("Overlay op {op} failed with code {code}")]
    OverlayOp { op: &'static str, code: u32 },
    #[error("Invalid overlay key: contains nul byte")]
    InvalidKey,
}

/// Plain-language reasons for the EVRInitError codes users actually hit.
pub fn init_error_hint(code: i32) -> &'static str {
    match code {
        108 => "no headset found — is it plugged in and powered on?",
        110 => "SteamVR isn't running — start SteamVR first",
        126 => "headset not detected by SteamVR",
        200 => "headset driver failed to load",
        205 => "headset is in use by another runtime",
        206 => "headset needs room setup — run SteamVR room setup",
        208 | 213 => "headset display not found — check the cable / display port",
        215 => "wireless headset not connected — wake it / reconnect it, then retry",
        301 => "SteamVR is still starting up — retry in a moment",
        _ => "see SteamVR's status window for details",
    }
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

        // 1. Bundled next to the exe / on the DLL search path.
        // 2. The installed SteamVR runtime, located the way OpenVR itself
        //    does it: %LOCALAPPDATA%\openvr\openvrpaths.vrpath → "runtime".
        let lib = match unsafe { Library::new(lib_name) } {
            Ok(l) => l,
            Err(first) => {
                let candidates = runtime_dirs().into_iter().map(|d| runtime_dll(&d));
                candidates
                    .filter(|p| p.exists())
                    .find_map(|p| {
                        let l = unsafe { Library::new(&p) }.ok()?;
                        tracing::info!("loaded {} from SteamVR runtime", p.display());
                        Some(l)
                    })
                    .ok_or_else(|| OpenVrError::LoadLib(format!("{first}; SteamVR runtime not found either")))?
            }
        };
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

/// Runtime roots listed in `openvrpaths.vrpath` (SteamVR's own path
/// registry), first entry first.
fn runtime_dirs() -> Vec<std::path::PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"));
    let Some(file) = base.map(|b| b.join("openvr").join("openvrpaths.vrpath")) else { return Vec::new() };
    std::fs::read_to_string(file).map(|t| parse_runtime_paths(&t)).unwrap_or_default()
}

fn runtime_dll(root: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    return root.join("bin").join("win64").join("openvr_api.dll");
    #[cfg(not(windows))]
    return root.join("bin").join("linux64").join("libopenvr_api.so");
}

/// Pull the `"runtime": [ ... ]` string array out of the vrpath JSON.
/// Hand-rolled to avoid a serde_json dependency for one array; handles
/// the `\` escaping Windows paths carry.
fn parse_runtime_paths(json: &str) -> Vec<std::path::PathBuf> {
    let Some(k) = json.find("\"runtime\"") else { return Vec::new() };
    let rest = &json[k..];
    let (Some(open), Some(close)) = (rest.find('['), rest.find(']')) else { return Vec::new() };
    if close < open {
        return Vec::new();
    }
    rest[open + 1..close]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|s| std::path::PathBuf::from(s.replace("\\\\", "\\")))
        .collect()
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

impl OpenVr {
    /// Acquire IVRCompositor (frame timing). Only the verified 029 table
    /// is requested; `Err` = no frame stats on this runtime.
    pub fn compositor(&self) -> Result<Compositor<'_>, OpenVrError> {
        let mut err: sys::EVRInitError = 0;
        let ptr = unsafe { (self.fns.vr_get_generic_interface)(sys::IVRCompositor_FnTable_Version.as_ptr() as *const _, &mut err) };
        if ptr.is_null() || err != sys::EVRInitError_None {
            return Err(OpenVrError::InterfaceMissing(err));
        }
        Ok(Compositor { table: ptr as *const sys::IVRCompositorFnTable, _parent: std::marker::PhantomData })
    }
}

/// SteamVR laser-mouse event on one of our overlays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// Mouse-scale units, y down from the top.
    pub x: f32,
    pub y: f32,
    /// 1 left, 2 right, 4 middle.
    pub button: u32,
    /// The controller doing it.
    pub device: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseKind {
    Move,
    Down,
    Up,
    /// SteamVR's laser entered / left this overlay.
    Enter,
    Leave,
    /// Smooth scroll (x, y delta) from the stick / touchpad.
    Scroll(f32, f32),
}

/// One compositor frame, as far as performance stats need it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSample {
    /// Time between the game's frames, ms.
    pub interval_ms: f32,
    /// GPU time for the frame (game + compositor), ms.
    pub gpu_ms: f32,
    /// CPU time for the frame: the game's from new poses to submit, plus
    /// the compositor's (what SteamVR's own timing graph shows), ms.
    pub cpu_ms: f32,
    /// Times the compositor showed this frame (> 1 = reprojected).
    pub presents: u32,
    pub dropped: u32,
}

pub struct Compositor<'a> {
    table: *const sys::IVRCompositorFnTable,
    _parent: std::marker::PhantomData<&'a OpenVr>,
}

impl<'a> Compositor<'a> {
    /// The last `n` frames (newest first), at most 128.
    pub fn frame_samples(&self, n: u32) -> Vec<FrameSample> {
        let n = n.clamp(1, 128);
        let size = std::mem::size_of::<sys::Compositor_FrameTiming>() as u32;
        let mut buf = vec![sys::Compositor_FrameTiming { m_nSize: size, ..Default::default() }; n as usize];
        // SAFETY: `table` is the runtime's IVRCompositor_029 table (layout
        // pinned in sys.rs); `buf` holds `n` initialised entries with
        // m_nSize set, and the runtime writes at most `n` of them.
        let got = unsafe { ((*self.table).GetFrameTimings)(buf.as_mut_ptr(), n) }.min(n) as usize;
        buf[..got]
            .iter()
            .map(|t| FrameSample {
                interval_ms: t.m_flClientFrameIntervalMs,
                gpu_ms: t.m_flTotalRenderGpuMs,
                cpu_ms: (t.m_flNewFrameReadyMs - t.m_flNewPosesReadyMs).max(0.0) + t.m_flCompositorRenderCpuMs.max(0.0),
                presents: t.m_nNumFramePresents,
                dropped: t.m_nNumDroppedFrames,
            })
            .collect()
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

    /// Block / unblock the game's controller input while this overlay is
    /// visible (`MakeOverlaysInteractiveIfVisible`).
    pub fn set_input_capture(&self, handle: VROverlayHandle_t, on: bool) -> Result<(), OpenVrError> {
        // SAFETY: IVROverlay_024 slot 10 (pinned in sys.rs).
        let r = unsafe { (self.table().SetOverlayFlag)(handle, sys::VROverlayFlags_MakeOverlaysInteractiveIfVisible, on) };
        check(r, "SetOverlayFlag")
    }

    /// Let SteamVR's own laser pointer drive this overlay: SteamVR draws
    /// the beam and cursor in the compositor and sends mouse events in
    /// `w` x `h` units (see [`Overlay::mouse_events`]). Active outside
    /// the SteamVR menu only while [`Overlay::set_input_capture`] is on.
    pub fn set_mouse_input(&self, handle: VROverlayHandle_t, w: f32, h: f32) -> Result<(), OpenVrError> {
        // SAFETY: IVROverlay_024 slots 46 / 48 (pinned in sys.rs).
        check(unsafe { (self.table().SetOverlayInputMethod)(handle, sys::VROverlayInputMethod_Mouse) }, "SetOverlayInputMethod")?;
        let scale = sys::HmdVector2_t { v: [w, h] };
        check(unsafe { (self.table().SetOverlayMouseScale)(handle, &scale) }, "SetOverlayMouseScale")?;
        // SAFETY: IVROverlay_024 slot 10 (pinned in sys.rs).
        check(unsafe { (self.table().SetOverlayFlag)(handle, sys::VROverlayFlags_SendVRSmoothScrollEvents, true) }, "SetOverlayFlag")
    }

    /// Stop SteamVR's laser from hitting this overlay (faded / hidden
    /// panels must not be an invisible wall in front of others).
    pub fn set_mouse_off(&self, handle: VROverlayHandle_t) -> Result<(), OpenVrError> {
        // SAFETY: IVROverlay_024 slot 46 (pinned in sys.rs); 0 = None.
        check(unsafe { (self.table().SetOverlayInputMethod)(handle, 0) }, "SetOverlayInputMethod")
    }

    /// Drain SteamVR's laser-mouse events for this overlay. Positions are
    /// in the mouse-scale units, y down from the top (SteamVR sends y up
    /// from the bottom; flipped here with `h`).
    pub fn mouse_events(&self, handle: VROverlayHandle_t, h: f32) -> Vec<MouseEvent> {
        let mut out = Vec::new();
        let mut ev = sys::VREvent_t::default();
        // SAFETY: IVROverlay_024 slot 44 (pinned); `ev` is a full-size
        // VREvent_t (size asserted in sys.rs).
        while unsafe { (self.table().PollNextOverlayEvent)(handle, &mut ev, std::mem::size_of::<sys::VREvent_t>() as u32) } {
            // VREvent_Mouse_t { float x, y; uint32 button } or
            // VREvent_Scroll_t { float xdelta, ydelta; .. } at the union start.
            let d = ev.data[0].to_le_bytes();
            let a = f32::from_le_bytes([d[0], d[1], d[2], d[3]]);
            let b = f32::from_le_bytes([d[4], d[5], d[6], d[7]]);
            let button = (ev.data[1] & 0xFFFF_FFFF) as u32;
            let kind = match ev.eventType {
                sys::VREvent_MouseMove => MouseKind::Move,
                sys::VREvent_MouseButtonDown => MouseKind::Down,
                sys::VREvent_MouseButtonUp => MouseKind::Up,
                sys::VREvent_FocusEnter => MouseKind::Enter,
                sys::VREvent_FocusLeave => MouseKind::Leave,
                sys::VREvent_ScrollSmooth => MouseKind::Scroll(a, b),
                _ => continue,
            };
            out.push(MouseEvent { kind, x: a, y: h - b, button, device: ev.trackedDeviceIndex });
        }
        out
    }

    /// Where the laser can hit this overlay: rectangles in mouse-scale
    /// units (x, y, w, h). `y_from_top`: SteamVR's header doesn't say
    /// which way y runs for masks, so the caller passes the convention
    /// it verified (see `vr_logic::MaskCalibration`); y is converted
    /// here. Empty = no mask (the whole overlay). At most 32.
    pub fn set_hit_mask(&self, handle: VROverlayHandle_t, rects: &[[f32; 4]], h: f32, y_from_top: bool) -> Result<(), OpenVrError> {
        let prims: Vec<sys::VROverlayIntersectionMaskPrimitive_t> = rects
            .iter()
            .take(32)
            .map(|&[x, y, w, rh]| sys::VROverlayIntersectionMaskPrimitive_t {
                m_nPrimitiveType: 0,
                m_Primitive: [x, if y_from_top { y } else { h - y - rh }, w, rh],
            })
            .collect();
        let size = std::mem::size_of::<sys::VROverlayIntersectionMaskPrimitive_t>() as u32;
        // SAFETY: IVROverlay_024 slot 51 (pinned); `prims` outlives the
        // call; null + 0 clears the mask.
        let ptr = if prims.is_empty() { std::ptr::null() } else { prims.as_ptr() };
        check(unsafe { (self.table().SetOverlayIntersectionMask)(handle, ptr, prims.len() as u32, size) }, "SetOverlayIntersectionMask")
    }

    /// World pose of a point on the overlay, in mouse-scale units with
    /// y from the top (converted to SteamVR's lower-left origin here).
    pub fn point_pose(&self, handle: VROverlayHandle_t, x: f32, y: f32, h: f32) -> Option<HmdMatrix34_t> {
        let mut m = HmdMatrix34_t { m: [[0.0; 4]; 3] };
        // SAFETY: IVROverlay_024 slot 43 (pinned); valid out-pointer.
        let r = unsafe {
            (self.table().GetTransformForOverlayCoordinates)(handle, sys::ETrackingUniverseOrigin_Standing, sys::HmdVector2_t { v: [x, h - y] }, &mut m)
        };
        (r == 0).then_some(m)
    }

    /// Click tick on whichever controller SteamVR's laser is on.
    pub fn laser_haptic(&self, handle: VROverlayHandle_t, seconds: f32, amplitude: f32) {
        // SAFETY: IVROverlay_024 slot 52 (pinned).
        let _ = unsafe { (self.table().TriggerLaserMouseHapticVibration)(handle, seconds, 160.0, amplitude.clamp(0.0, 1.0)) };
    }

    /// SteamVR's own dashboard (system menu) is open.
    pub fn is_dashboard_visible(&self) -> bool {
        // SAFETY: IVROverlay_024 slot 64 (pinned in sys.rs), no arguments.
        unsafe { (self.table().IsDashboardVisible)() }
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

    /// Parent the overlay to a tracked device (controller / HMD); `m`
    /// is the device-local transform. The compositor follows the device
    /// every frame — no per-frame updates needed from us.
    pub fn set_transform_device_relative(
        &self,
        handle: VROverlayHandle_t,
        device: u32,
        m: &HmdMatrix34_t,
    ) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayTransformTrackedDeviceRelative)(handle, device, m) };
        check(r, "SetOverlayTransformTrackedDeviceRelative")
    }

    /// Texel width:height. Lets a strip overlay keep a constant
    /// thickness while its width (length) changes every frame.
    pub fn set_texel_aspect(&self, handle: VROverlayHandle_t, aspect: f32) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlayTexelAspect)(handle, aspect) };
        check(r, "SetOverlayTexelAspect")
    }

    /// Higher draws on top among overlapping overlays.
    pub fn set_sort_order(&self, handle: VROverlayHandle_t, order: u32) -> Result<(), OpenVrError> {
        let r = unsafe { (self.table().SetOverlaySortOrder)(handle, order) };
        check(r, "SetOverlaySortOrder")
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
    /// Full device-to-standing pose (for grabbing panels).
    pub pose: HmdMatrix34_t,
    /// Application-menu button (B on Index, menu on wands).
    pub menu: bool,
    pub grip: bool,
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
    /// The driver's own icon for this device ("{driver}/icons/….png").
    pub icon: Option<String>,
    /// The device's driver resource root ("indexcontroller", …).
    pub resource_root: Option<String>,
}

pub struct System<'a> {
    table: *const sys::IVRSystemFnTable,
    _parent: std::marker::PhantomData<&'a OpenVr>,
}

impl<'a> System<'a> {
    fn table(&self) -> &sys::IVRSystemFnTable {
        unsafe { &*self.table }
    }

    /// Drain the system event queue; true when SteamVR asked every app
    /// to quit (it's shutting down). The quit is acknowledged here, so
    /// the caller just has to exit promptly.
    pub fn quit_requested(&self) -> bool {
        let t = self.table();
        let mut ev = sys::VREvent_t::default();
        let mut quit = false;
        while unsafe { (t.PollNextEvent)(&mut ev, std::mem::size_of::<sys::VREvent_t>() as u32) } {
            if ev.eventType == sys::VREvent_Quit {
                quit = true;
            }
        }
        if quit {
            unsafe { (t.AcknowledgeQuit_Exiting)() };
        }
        quit
    }

    /// Every connected device that reports battery status, in OpenVR
    /// index order (HMD is always index 0 when present). Lighthouse base
    /// stations are skipped — they're mains-powered.
    /// How many rendered pixels one radian covers at the centre of the
    /// left eye's view: SteamVR's render width (with its resolution
    /// setting applied) over the eye's tangent-space width. Panels sized
    /// in texels to match look as sharp as the headset can show.
    pub fn pixels_per_radian(&self) -> Option<f32> {
        let t = self.table();
        let (mut w, mut h) = (0u32, 0u32);
        let (mut l, mut r, mut top, mut bottom) = (0f32, 0f32, 0f32, 0f32);
        // SAFETY: slots 0 and 2 of IVRSystem_022 (pinned in sys.rs); all
        // out-pointers are valid locals. Eye 0 = left.
        unsafe {
            (t.GetRecommendedRenderTargetSize)(&mut w, &mut h);
            (t.GetProjectionRaw)(0, &mut l, &mut r, &mut top, &mut bottom);
        }
        let span = r - l;
        (w > 0 && span.is_finite() && span > 0.1).then(|| w as f32 / span)
    }

    /// Headset refresh rate (Hz), if the HMD reports it.
    pub fn display_hz(&self) -> Option<f32> {
        self.float_prop(0, sys::Prop_DisplayFrequency_Float).filter(|hz| *hz > 1.0)
    }

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
                    icon: self.string_prop(i, sys::Prop_NamedIconPathDeviceReady_String).filter(|s| !s.is_empty()),
                    resource_root: self.string_prop(i, sys::Prop_ResourceRoot_String).filter(|s| !s.is_empty()),
                });
            }
        }
        out
    }

    /// Left + right hand controllers with a valid pose this frame.
    pub fn controllers(&self) -> Vec<Controller> {
        self.tracking(0.0).1
    }

    /// Seconds since SteamVR's last vsync (frame pacing).
    pub fn since_vsync(&self) -> Option<f32> {
        let (mut since, mut frame) = (0f32, 0u64);
        // SAFETY: IVRSystem slot 5 (pinned in sys.rs); valid out-pointers.
        unsafe { (self.table().GetTimeSinceLastVsync)(&mut since, &mut frame) }.then_some(since)
    }

    /// Seconds from now until this frame's photons reach the eyes (what
    /// the compositor predicts device-relative overlays to). Hit-testing
    /// with poses predicted this far means the laser clicks where its
    /// beam is drawn — on streamed headsets the gap is tens of ms.
    pub fn seconds_to_photons(&self) -> f32 {
        let hz = self.display_hz().unwrap_or(90.0);
        let to_photons = self.float_prop(0, sys::Prop_SecondsFromVsyncToPhotons_Float).unwrap_or(0.0);
        let (mut since, mut frame) = (0f32, 0u64);
        // SAFETY: IVRSystem slot 5 (pinned in sys.rs); valid out-pointers.
        let ok = unsafe { (self.table().GetTimeSinceLastVsync)(&mut since, &mut frame) };
        let since = if ok { since } else { 0.0 };
        (1.0 / hz - since + to_photons).clamp(0.0, 0.1)
    }

    /// HMD pose (device 0) + hand controllers, from one pose fetch.
    /// `predict`: seconds ahead (see [`System::seconds_to_photons`]).
    pub fn tracking(&self, predict: f32) -> (Option<HmdMatrix34_t>, Vec<Controller>) {
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
                predict,
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
            let bit = |b: u32| st.ulButtonPressed & (1u64 << b) != 0;
            out.push(Controller {
                pose: pose.mDeviceToAbsoluteTracking,
                menu: bit(sys::k_EButton_ApplicationMenu),
                grip: bit(sys::k_EButton_Grip),
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
        let hmd = poses.first().filter(|p| p.bPoseIsValid).map(|p| p.mDeviceToAbsoluteTracking);
        (hmd, out)
    }

    /// Device index for a hand, if that controller is connected.
    pub fn hand_index(&self, hand: Hand) -> Option<u32> {
        let role = match hand {
            Hand::Left => sys::ETrackedControllerRole_LeftHand,
            Hand::Right => sys::ETrackedControllerRole_RightHand,
        };
        let i = unsafe { (self.table().GetTrackedDeviceIndexForControllerRole)(role) };
        (i != sys::k_unTrackedDeviceIndexInvalid && unsafe { (self.table().IsTrackedDeviceConnected)(i) }).then_some(i)
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

    /// A device's model name ("Knuckles Right", …), for logs.
    pub fn model_name(&self, i: u32) -> Option<String> {
        self.string_prop(i, sys::Prop_ModelNumber_String)
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

/// One hand's SteamVR Input state for this frame.
#[derive(Copy, Clone, Debug, Default)]
pub struct HandInput {
    /// Trigger pull 0..=1.
    pub trigger: f32,
    /// Trigger click (the binding's threshold).
    pub click: bool,
    pub grab: bool,
    /// Thumbstick / trackpad (x, y).
    pub stick: [f32; 2],
    /// XSO-style click modifiers (Index trackpad S/N, Touch A·X / B·Y touch).
    pub right_mod: bool,
    pub middle_mod: bool,
    /// Dedicated right / middle click buttons (press = click at the
    /// laser, no trigger needed). Unbound by default.
    pub right_click: bool,
    pub middle_click: bool,
    /// Rising edge of the edit-mode toggle (double-press left A / X).
    pub toggle_edit: bool,
    /// SteamVR's per-controller pointing pose (/pose/tip) — where the
    /// laser should come from. None if the binding/pose is inactive.
    pub aim: Option<HmdMatrix34_t>,
}

/// SteamVR Input: action manifest + per-controller default bindings,
/// rebindable by the user in SteamVR's controller-binding UI.
pub struct Input<'a> {
    table: *const sys::IVRInputFnTable,
    set: sys::VRActionSetHandle_t,
    click: sys::VRActionHandle_t,
    trigger: sys::VRActionHandle_t,
    grab: sys::VRActionHandle_t,
    stick: sys::VRActionHandle_t,
    right_mod: sys::VRActionHandle_t,
    middle_mod: sys::VRActionHandle_t,
    right_click: sys::VRActionHandle_t,
    middle_click: sys::VRActionHandle_t,
    toggle_edit: sys::VRActionHandle_t,
    haptic: sys::VRActionHandle_t,
    pointer: sys::VRActionHandle_t,
    hands: [sys::VRInputValueHandle_t; 2],
    _parent: std::marker::PhantomData<&'a OpenVr>,
}

/// What registering the app manifest did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppRegistration {
    /// First registration: auto-launch was switched on.
    pub newly_installed: bool,
    /// SteamVR will start RSOverlay with itself.
    pub auto_launch: bool,
}

impl OpenVr {
    /// Register `manifest` (absolute path to a .vrmanifest) with SteamVR
    /// permanently, so RSOverlay shows up under SteamVR's startup
    /// overlay apps. Auto-launch is switched on only the first time the
    /// app is installed; after that it's the user's choice in SteamVR
    /// (Settings → Startup / Shutdown), and we never flip it back.
    pub fn register_app(&self, manifest: &std::path::Path, app_key: &str) -> Result<AppRegistration, OpenVrError> {
        let mut err: sys::EVRInitError = 0;
        let ptr = unsafe {
            (self.fns.vr_get_generic_interface)(sys::IVRApplications_FnTable_Version.as_ptr() as *const _, &mut err)
        };
        if ptr.is_null() || err != sys::EVRInitError_None {
            return Err(OpenVrError::InterfaceMissing(err));
        }
        let t = unsafe { &*(ptr as *const sys::IVRApplicationsFnTable) };
        let key = CString::new(app_key).map_err(|_| OpenVrError::InvalidKey)?;
        let path = CString::new(manifest.to_string_lossy().as_ref()).map_err(|_| OpenVrError::InvalidKey)?;
        unsafe {
            let was_installed = (t.IsApplicationInstalled)(key.as_ptr());
            // Re-added every start: picks up a moved exe / changed args.
            let r = (t.AddApplicationManifest)(path.as_ptr(), false);
            if r != sys::EVRApplicationError_None {
                return Err(OpenVrError::OverlayOp { op: "AddApplicationManifest", code: r as u32 });
            }
            if !was_installed {
                let r = (t.SetApplicationAutoLaunch)(key.as_ptr(), true);
                if r != sys::EVRApplicationError_None {
                    return Err(OpenVrError::OverlayOp { op: "SetApplicationAutoLaunch", code: r as u32 });
                }
            }
            Ok(AppRegistration { newly_installed: !was_installed, auto_launch: (t.GetApplicationAutoLaunch)(key.as_ptr()) })
        }
    }

    /// Register `manifest` (absolute path to actions.json) and resolve
    /// the RSOverlay action set.
    pub fn input(&self, manifest: &std::path::Path) -> Result<Input<'_>, OpenVrError> {
        let mut err: sys::EVRInitError = 0;
        let ptr = unsafe {
            (self.fns.vr_get_generic_interface)(sys::IVRInput_FnTable_Version.as_ptr() as *const _, &mut err)
        };
        if ptr.is_null() || err != sys::EVRInitError_None {
            return Err(OpenVrError::InterfaceMissing(err));
        }
        let t = ptr as *const sys::IVRInputFnTable;
        let cstr = |s: &str| CString::new(s).map_err(|_| OpenVrError::InvalidKey);
        unsafe {
            let path = cstr(&manifest.to_string_lossy())?;
            let r = ((*t).SetActionManifestPath)(path.as_ptr());
            if r != sys::EVRInputError_None {
                return Err(OpenVrError::OverlayOp { op: "SetActionManifestPath", code: r as u32 });
            }
            let handle = |name: &str| -> Result<u64, OpenVrError> {
                let n = cstr(name)?;
                let mut h = 0u64;
                let r = ((*t).GetActionHandle)(n.as_ptr(), &mut h);
                if r != sys::EVRInputError_None {
                    return Err(OpenVrError::OverlayOp { op: "GetActionHandle", code: r as u32 });
                }
                Ok(h)
            };
            let mut set = 0u64;
            let n = cstr("/actions/rsoverlay")?;
            ((*t).GetActionSetHandle)(n.as_ptr(), &mut set);
            let mut hands = [0u64; 2];
            for (i, p) in ["/user/hand/left", "/user/hand/right"].iter().enumerate() {
                let n = cstr(p)?;
                ((*t).GetInputSourceHandle)(n.as_ptr(), &mut hands[i]);
            }
            Ok(Input {
                table: t,
                set,
                click: handle("/actions/rsoverlay/in/Click")?,
                trigger: handle("/actions/rsoverlay/in/TriggerPull")?,
                grab: handle("/actions/rsoverlay/in/Grab")?,
                stick: handle("/actions/rsoverlay/in/Stick")?,
                right_mod: handle("/actions/rsoverlay/in/RightClickModifier")?,
                middle_mod: handle("/actions/rsoverlay/in/MiddleClickModifier")?,
                right_click: handle("/actions/rsoverlay/in/RightClick")?,
                middle_click: handle("/actions/rsoverlay/in/MiddleClick")?,
                toggle_edit: handle("/actions/rsoverlay/in/ToggleEditMode")?,
                haptic: handle("/actions/rsoverlay/out/Haptic")?,
                pointer: handle("/actions/rsoverlay/in/Pointer")?,
                hands,
                _parent: std::marker::PhantomData,
            })
        }
    }
}

impl Input<'_> {
    fn t(&self) -> &sys::IVRInputFnTable {
        unsafe { &*self.table }
    }

    fn digital(&self, a: u64, dev: u64) -> sys::InputDigitalActionData_t {
        let mut d = sys::InputDigitalActionData_t::default();
        unsafe { (self.t().GetDigitalActionData)(a, &mut d, std::mem::size_of_val(&d) as u32, dev) };
        d
    }

    fn analog(&self, a: u64, dev: u64) -> sys::InputAnalogActionData_t {
        let mut d = sys::InputAnalogActionData_t::default();
        unsafe { (self.t().GetAnalogActionData)(a, &mut d, std::mem::size_of_val(&d) as u32, dev) };
        d
    }

    /// Pump SteamVR Input and read both hands. [left, right].
    /// `predict`: seconds ahead, the same as the device poses.
    pub fn poll(&self, predict: f32) -> [HandInput; 2] {
        let mut set = sys::VRActiveActionSet_t { ulActionSet: self.set, ..Default::default() };
        unsafe { (self.t().UpdateActionState)(&mut set, std::mem::size_of_val(&set) as u32, 1) };
        self.hands.map(|dev| {
            let on = |a| {
                let d = self.digital(a, dev);
                d.bActive && d.bState
            };
            let stick = self.analog(self.stick, dev);
            let edit = self.digital(self.toggle_edit, dev);
            let mut pose = std::mem::MaybeUninit::<sys::InputPoseActionData_t>::zeroed();
            let aim = unsafe {
                // Same prediction as the raw device poses, so tip-vs-grip
                // stays a constant offset (mixing clocks made the beam
                // drift while the hand moved).
                let r = (self.t().GetPoseActionDataRelativeToNow)(
                    self.pointer,
                    sys::ETrackingUniverseOrigin_Standing,
                    predict,
                    pose.as_mut_ptr(),
                    std::mem::size_of::<sys::InputPoseActionData_t>() as u32,
                    dev,
                );
                let p = pose.assume_init();
                (r == sys::EVRInputError_None && p.bActive && p.pose.bPoseIsValid).then_some(p.pose.mDeviceToAbsoluteTracking)
            };
            HandInput {
                trigger: self.analog(self.trigger, dev).x,
                click: on(self.click),
                grab: on(self.grab),
                stick: [stick.x, stick.y],
                right_mod: on(self.right_mod),
                middle_mod: on(self.middle_mod),
                right_click: on(self.right_click),
                middle_click: on(self.middle_click),
                toggle_edit: edit.bActive && edit.bState && edit.bChanged,
                aim,
            }
        })
    }

    /// Short vibration on one hand (0 = left, 1 = right).
    pub fn haptic(&self, hand: Hand, seconds: f32, amplitude: f32) {
        let dev = self.hands[if hand == Hand::Left { 0 } else { 1 }];
        unsafe { (self.t().TriggerHapticVibrationAction)(self.haptic, 0.0, seconds, 160.0, amplitude.clamp(0.0, 1.0), dev) };
    }
}

/// Device-local pose: translate `(x, y, z)` metres, then pitch about
/// the device's X axis by `pitch_deg` (negative tilts the panel's face
/// up toward the wearer). Row-major 3x4.
pub fn pose_offset_pitch(x: f32, y: f32, z: f32, pitch_deg: f32) -> HmdMatrix34_t {
    let (s, c) = pitch_deg.to_radians().sin_cos();
    HmdMatrix34_t {
        m: [
            [1.0, 0.0, 0.0, x],
            [0.0, c, -s, y],
            [0.0, s, c, z],
        ],
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
    fn init_errors_read_as_english() {
        let e = OpenVrError::InitFailed(215).to_string();
        assert!(e.contains("215") && e.contains("wireless headset not connected"), "{e}");
    }

    #[test]
    fn parses_vrpath_runtime() {
        let json = r#"{ "config" : [ "C:\\x" ], "runtime" : [ "C:\\Program Files (x86)\\Steam\\steamapps\\common\\SteamVR" ], "version" : 1 }"#;
        let p = parse_runtime_paths(json);
        assert_eq!(p, vec![std::path::PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\SteamVR")]);
        assert!(parse_runtime_paths("{}").is_empty());
    }

    #[test]
    fn pitch_tilts_normal_up() {
        // Overlay faces +Z; pitching -90° about X turns that to +Y (up).
        let m = pose_offset_pitch(0.0, 0.0, 0.0, -90.0).m;
        let normal = [m[0][2], m[1][2], m[2][2]];
        assert!(normal[0].abs() < 1e-6 && (normal[1] - 1.0).abs() < 1e-6 && normal[2].abs() < 1e-6);
    }

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
