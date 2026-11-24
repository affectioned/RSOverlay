//! Per-window capture via Windows.Graphics.Capture (WGC), GPU-to-GPU.
//!
//! Same contract as [`crate::SharedCapture`]: a D3D11 device on the wgpu
//! adapter's LUID writes into an NT-shared BGRA8 texture which the render
//! crate opens on its DX12 device. No CPU readback.
//!
//! Windows resize at will, but the shared texture (and the wgpu view
//! opened on it) is fixed. So we allocate it at a *capacity* — the size
//! of the monitor the window sits on, never less than the window — and
//! copy each frame into the top-left corner. The live content size is
//! published through [`ContentSize`]; the UI samples the sub-rect
//! `uv = (0,0)..(cw/W, ch/H)`. A window larger than capacity is cropped
//! rather than forcing a pipeline rebuild mid-stream.

use anyhow::{anyhow, bail, Context, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use windows::core::{ComInterface, IInspectable};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HMODULE, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIDevice, IDXGIFactory1, IDXGIResource1, DXGI_ADAPTER_DESC1,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

/// Hard cap on the shared texture edge. 8K covers every desktop
/// monitor shipping today; beyond that the window is cropped.
const MAX_CAPACITY: u32 = 7680;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// Raw HWND value. Stable while the window lives.
    pub hwnd: isize,
    pub title: String,
}

/// Top-level, visible, titled, non-tool, non-cloaked windows from other
/// processes — what a human would call "the open windows". Cloaked
/// filters out UWP frames parked on other virtual desktops.
pub fn enumerate_windows() -> Result<Vec<WindowInfo>> {
    struct Ctx {
        out: Vec<WindowInfo>,
        own_pid: u32,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if let Some(info) = describe(hwnd, ctx.own_pid) {
            ctx.out.push(info);
        }
        TRUE
    }
    let mut ctx = Ctx { out: Vec::new(), own_pid: std::process::id() };
    unsafe { EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize)) }.context("EnumWindows")?;
    Ok(ctx.out)
}

unsafe fn describe(hwnd: HWND, own_pid: u32) -> Option<WindowInfo> {
    if !IsWindowVisible(hwnd).as_bool() {
        return None;
    }
    if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
        return None;
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut c_void,
        std::mem::size_of::<u32>() as u32,
    );
    if cloaked != 0 {
        return None;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == own_pid {
        return None;
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    (!title.trim().is_empty()).then_some(WindowInfo { hwnd: hwnd.0, title })
}

/// First window whose title contains `needle`, case-insensitively.
pub fn find_window(needle: &str) -> Result<Option<WindowInfo>> {
    let needle = needle.to_lowercase();
    Ok(enumerate_windows()?.into_iter().find(|w| w.title.to_lowercase().contains(&needle)))
}

/// On-screen rect WGC captures (DWM extended frame bounds — excludes the
/// invisible resize border Win10+ adds). Returned as
/// `(left, top, width, height)` in virtual-desktop pixels.
pub fn window_frame_rect(hwnd: isize) -> Option<(i32, i32, u32, u32)> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            HWND(hwnd),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
    }
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    (w > 0 && h > 0).then_some((r.left, r.top, w as u32, h as u32))
}

/// Bring a captured window to the front before forwarding a click, so
/// the click lands on it rather than whatever overlaps it on the real
/// desktop. Best-effort: Windows' focus-steal rules may refuse.
pub fn focus_window(hwnd: isize) {
    unsafe {
        let _ = SetForegroundWindow(HWND(hwnd));
    }
}

/// Live content size, packed `w << 32 | h` so a single atomic load is
/// tear-free between the capture thread and the UI thread.
#[derive(Clone, Default)]
pub struct ContentSize(Arc<AtomicU64>);

impl ContentSize {
    pub fn get(&self) -> (u32, u32) {
        let v = self.0.load(Ordering::Acquire);
        ((v >> 32) as u32, v as u32)
    }
    fn set(&self, w: u32, h: u32) {
        self.0.store((u64::from(w) << 32) | u64::from(h), Ordering::Release);
    }
}

pub struct WindowCapture {
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    pool_size: SizeInt32,
    shared_tex: ID3D11Texture2D,
    shared_handle: HANDLE,
    /// Shared texture dimensions (capacity), fixed for the capture's life.
    pub width: u32,
    pub height: u32,
    pub content: ContentSize,
}

impl WindowCapture {
    pub fn new(hwnd: isize, wgpu_luid: Option<(u32, i32)>) -> Result<Self> {
        let hwnd = HWND(hwnd);
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                bail!("window {:#x} no longer exists", hwnd.0);
            }
            if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
                bail!("Windows.Graphics.Capture unsupported (needs Windows 10 1903+)");
            }
            let (device, context) = create_device(wgpu_luid)?;
            let dxgi: IDXGIDevice = device.cast().context("IDXGIDevice cast")?;
            let inspectable: IInspectable =
                CreateDirect3D11DeviceFromDXGIDevice(&dxgi).context("CreateDirect3D11DeviceFromDXGIDevice")?;
            let winrt_device: IDirect3DDevice = inspectable.cast().context("IDirect3DDevice cast")?;

            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .context("GraphicsCaptureItem interop factory")?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd).context("CreateForWindow")?;
            let pool_size = item.Size().context("GraphicsCaptureItem::Size")?;

            let (width, height) = capacity_for(hwnd, pool_size);
            let (shared_tex, shared_handle) = create_shared_texture(&device, width, height)?;

            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                pool_size,
            )
            .context("CreateFreeThreaded frame pool")?;
            let session = pool.CreateCaptureSession(&item).context("CreateCaptureSession")?;
            // Win11-only knobs; older builds throw, which is fine.
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(true);
            session.StartCapture().context("StartCapture")?;

            let content = ContentSize::default();
            content.set(
                (pool_size.Width.max(1) as u32).min(width),
                (pool_size.Height.max(1) as u32).min(height),
            );
            Ok(Self {
                hwnd,
                device,
                context,
                winrt_device,
                _item: item,
                pool,
                session,
                pool_size,
                shared_tex,
                shared_handle,
                width,
                height,
                content,
            })
        }
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.0
    }

    pub fn shared_handle_raw(&self) -> *mut c_void {
        self.shared_handle.0 as *mut c_void
    }

    /// Copy the newest window frame into the shared texture.
    /// `Ok(false)` = no new frame (WGC only emits on change; we sleep
    /// `idle_ms` so the caller's loop doesn't spin). `Err` = window gone.
    pub fn capture_next_frame(&mut self, idle_ms: u32) -> Result<bool> {
        unsafe {
            if !IsWindow(self.hwnd).as_bool() {
                bail!("captured window closed");
            }
        }
        // TryGetNextFrame yields null (surfaced as Err) when the pool is
        // empty — not an error condition.
        let Ok(frame) = self.pool.TryGetNextFrame() else {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(idle_ms)));
            return Ok(false);
        };
        let size = frame.ContentSize().context("ContentSize")?;
        let surface = frame.Surface().context("frame Surface")?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().context("IDirect3DDxgiInterfaceAccess")?;
        let tex: ID3D11Texture2D = unsafe { access.GetInterface() }.context("surface → ID3D11Texture2D")?;

        let cw = (size.Width.max(1) as u32).min(self.width);
        let ch = (size.Height.max(1) as u32).min(self.height);
        let region = D3D11_BOX { left: 0, top: 0, front: 0, right: cw, bottom: ch, back: 1 };
        unsafe {
            self.context.CopySubresourceRegion(&self.shared_tex, 0, 0, 0, 0, &tex, 0, Some(&region));
            // Same reason as SharedCapture: push the copy onto the GPU
            // timeline now so the DX12 reader doesn't see stale texels.
            self.context.Flush();
        }
        self.content.set(cw, ch);
        let _ = frame.Close();

        // Window resized → the pool's buffers are the old size; recreate
        // so the next frames carry the full new content.
        if size.Width != self.pool_size.Width || size.Height != self.pool_size.Height {
            self.pool
                .Recreate(&self.winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)
                .context("frame pool Recreate")?;
            self.pool_size = size;
        }
        Ok(true)
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
        unsafe {
            if !self.shared_handle.is_invalid() {
                let _ = CloseHandle(self.shared_handle);
            }
        }
        let _ = &self.device;
    }
}

// The frame pool is free-threaded (agile) and the D3D11 immediate
// context is only touched from the owning capture thread after the move.
unsafe impl Send for WindowCapture {}

/// Capacity = monitor the window is on, grown to fit the window if it
/// spans monitors, capped at [`MAX_CAPACITY`].
fn capacity_for(hwnd: HWND, initial: SizeInt32) -> (u32, u32) {
    let (mut w, mut h) = (initial.Width.max(1) as u32, initial.Height.max(1) as u32);
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let r = mi.rcMonitor;
            w = w.max((r.right - r.left) as u32);
            h = h.max((r.bottom - r.top) as u32);
        }
    }
    (w.min(MAX_CAPACITY), h.min(MAX_CAPACITY))
}

unsafe fn create_device(wgpu_luid: Option<(u32, i32)>) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let factory: IDXGIFactory1 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
    let mut adapter: Option<IDXGIAdapter> = None;
    let mut i = 0;
    while let Ok(a) = factory.EnumAdapters1(i) {
        i += 1;
        let mut desc = DXGI_ADAPTER_DESC1::default();
        if a.GetDesc1(&mut desc).is_err() {
            continue;
        }
        let matches = wgpu_luid
            .is_none_or(|(lo, hi)| desc.AdapterLuid.LowPart == lo && desc.AdapterLuid.HighPart == hi);
        if matches {
            adapter = Some(a.cast().context("IDXGIAdapter cast")?);
            break;
        }
    }
    let adapter = adapter.ok_or_else(|| anyhow!("no DXGI adapter matches the wgpu LUID"))?;
    let mut device = None;
    let mut context = None;
    D3D11CreateDevice(
        &adapter,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )
    .context("D3D11CreateDevice (window capture)")?;
    Ok((
        device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null"))?,
        context.ok_or_else(|| anyhow!("no immediate context"))?,
    ))
}

unsafe fn create_shared_texture(device: &ID3D11Device, width: u32, height: u32) -> Result<(ID3D11Texture2D, HANDLE)> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        // _SRGB: same typeless family as the UNORM source, so the copy
        // is legal; the D3D12 reader opens it as Bgra8UnormSrgb.
        Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        // NTHANDLE must be paired with SHARED (see SharedCapture).
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
    };
    let mut tex = None;
    device.CreateTexture2D(&desc, None, Some(&mut tex)).context("CreateTexture2D (window shared)")?;
    let tex: ID3D11Texture2D = tex.ok_or_else(|| anyhow!("CreateTexture2D returned null"))?;
    let res1: IDXGIResource1 = tex.cast().context("IDXGIResource1 cast")?;
    const GENERIC_ALL: u32 = 0x1000_0000;
    let handle = res1.CreateSharedHandle(None, GENERIC_ALL, None).context("CreateSharedHandle (window)")?;
    Ok((tex, handle))
}

/// Minimised windows produce no frames; callers can show a hint.
pub fn is_minimized(hwnd: isize) -> bool {
    unsafe { IsIconic(HWND(hwnd)).as_bool() }
}

/// Where the OS cursor is right now, in virtual-desktop pixels —
/// `None` while hidden (games, video players). DXGI desktop duplication
/// leaves the cursor out of captured frames, so monitor mirrors draw it
/// themselves from this.
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorInfo, CURSORINFO, CURSOR_SHOWING};
    let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    unsafe { GetCursorInfo(&mut ci) }.ok()?;
    (ci.flags.0 & CURSOR_SHOWING.0 != 0).then_some((ci.ptScreenPos.x, ci.ptScreenPos.y))
}

/// RGBA pixels of a window's app icon: (width, height, rgba). Asks the
/// window (WM_GETICON big → small) with a short timeout so a hung app
/// can't stall us, then falls back to its window-class icon.
pub fn window_icon(hwnd: isize) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassLongPtrW, GetIconInfo, SendMessageTimeoutW, GCLP_HICON, GCLP_HICONSM, HICON, ICONINFO,
        SMTO_ABORTIFHUNG, WM_GETICON,
    };
    const ICON_BIG: usize = 1;
    const ICON_SMALL2: usize = 2;
    unsafe {
        let h = HWND(hwnd);
        let ask = |kind: usize| {
            let mut out = 0usize;
            SendMessageTimeoutW(h, WM_GETICON, WPARAM(kind), LPARAM(0), SMTO_ABORTIFHUNG, 50, Some(&mut out));
            out
        };
        let raw = [ask(ICON_BIG), ask(ICON_SMALL2), GetClassLongPtrW(h, GCLP_HICON), GetClassLongPtrW(h, GCLP_HICONSM)]
            .into_iter()
            .find(|&v| v != 0)?;
        let mut info = ICONINFO::default();
        GetIconInfo(HICON(raw as isize), &mut info).ok()?;
        let mut bm = BITMAP::default();
        let color = info.hbmColor;
        let ok = !color.is_invalid()
            && GetObjectW(color, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) != 0;
        let result = ok.then(|| {
            let (w, hgt) = (bm.bmWidth.max(1) as u32, bm.bmHeight.max(1) as u32);
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(hgt as i32), // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * hgt * 4) as usize];
            let dc = GetDC(HWND(0));
            let lines = GetDIBits(dc, color, 0, hgt, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
            ReleaseDC(HWND(0), dc);
            (lines > 0).then(|| {
                // BGRA → RGBA. Legacy icons carry no alpha (all zero):
                // treat them as opaque rather than invisible.
                let has_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
                for p in px.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                    if !has_alpha {
                        p[3] = 255;
                    }
                }
                (w, hgt, px)
            })
        });
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask);
        }
        result.flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only: `cargo test -p rsoverlay-capture -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_icons_and_cursor() {
        let wins = enumerate_windows().unwrap();
        let with_icon = wins.iter().filter(|w| window_icon(w.hwnd).is_some()).count();
        println!("{} windows, {} with icons; cursor {:?}", wins.len(), with_icon, cursor_pos());
        if let Some((w, h, px)) = wins.iter().find_map(|w| window_icon(w.hwnd)) {
            assert_eq!(px.len() as u32, w * h * 4);
            assert!(px.as_chunks::<4>().0.iter().any(|p| p[3] > 0), "icon fully transparent");
        }
        assert!(with_icon > 0);
    }
}
