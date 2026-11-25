//! Still thumbnails of top-level windows for the Desktop picker.
//!
//! `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)` asks DWM to render
//! the window's client area into our own DIB, which also works for
//! DirectX / Chromium windows that plain `BitBlt` sees as black. It's a
//! one-off CPU copy, so callers run it off the UI thread and rarely
//! (the app's thumbnail worker refreshes at most every couple of seconds).
//!
//! Minimised windows have no content to render and some protected
//! windows come back all black; both return `None` so the picker falls
//! back to the app icon.

/// Largest client area we're willing to print (per side, px).
pub const MAX_SOURCE_SIDE: u32 = 8192;

/// Box-filter a top-down BGRA image down to fit `max_w` wide (aspect
/// kept, never upscaled), producing opaque RGBA. `None` if the buffer is
/// shorter than `w * h * 4` or the image is (near) all black.
/// O(w · h) — every source pixel is read once.
pub fn downscale_bgra(src: &[u8], w: u32, h: u32, max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
    if w == 0 || h == 0 || max_w == 0 {
        return None;
    }
    let need = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let src = src.get(..need)?;
    let tw = w.min(max_w);
    let th = ((h as u64 * tw as u64) / w as u64).max(1) as u32;
    let (sw, sh) = (w as usize, h as usize);
    let (tw_, th_) = (tw as usize, th as usize);
    let mut out = vec![0u8; tw_ * th_ * 4];
    let mut brightest = 0u8;
    for ty in 0..th_ {
        let y0 = ty * sh / th_;
        let y1 = ((ty + 1) * sh / th_).max(y0 + 1).min(sh);
        for tx in 0..tw_ {
            let x0 = tx * sw / tw_;
            let x1 = ((tx + 1) * sw / tw_).max(x0 + 1).min(sw);
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = &src[(y * sw + x0) * 4..(y * sw + x1) * 4];
                for p in row.as_chunks::<4>().0 {
                    b += u32::from(p[0]);
                    g += u32::from(p[1]);
                    r += u32::from(p[2]);
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let px = [(r / n) as u8, (g / n) as u8, (b / n) as u8, 255];
            brightest = brightest.max(px[0]).max(px[1]).max(px[2]);
            let o = (ty * tw_ + tx) * 4;
            out[o..o + 4].copy_from_slice(&px);
        }
    }
    (brightest > 8).then_some((tw, th, out))
}

/// Thumbnail of `hwnd`'s client area, at most `max_w` px wide, as RGBA.
#[cfg(windows)]
pub fn window_thumbnail(hwnd: isize, max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsIconic};

    /// Memory DC with our DIB selected; Drop restores the previous
    /// selection, then frees the bitmap and the DC (in that order), on
    /// every exit path.
    struct Canvas {
        dc: HDC,
        bmp: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Canvas {
        fn drop(&mut self) {
            // SAFETY: handles were created by us below and are only
            // released here, once.
            unsafe {
                if !self.old.is_invalid() {
                    SelectObject(self.dc, self.old);
                }
                if !self.bmp.is_invalid() {
                    let _ = DeleteObject(self.bmp);
                }
                let _ = DeleteDC(self.dc);
            }
        }
    }

    const PW_CLIENTONLY: u32 = 0x1;
    const PW_RENDERFULLCONTENT: u32 = 0x2;

    let h = HWND(hwnd);
    // SAFETY: plain Win32 queries/drawing on a window handle (a stale
    // handle just makes the calls fail); the DIB pointer is only read
    // within the `w * h * 4` bytes we asked CreateDIBSection for, while
    // the Canvas guard keeps the bitmap alive.
    unsafe {
        if IsIconic(h).as_bool() {
            return None;
        }
        let mut rc = RECT::default();
        GetClientRect(h, &mut rc).ok()?;
        let (w, hgt) = ((rc.right - rc.left).max(0) as u32, (rc.bottom - rc.top).max(0) as u32);
        if w == 0 || hgt == 0 || w > MAX_SOURCE_SIDE || hgt > MAX_SOURCE_SIDE {
            return None;
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut canvas = Canvas { dc, bmp: HBITMAP::default(), old: HGDIOBJ::default() };
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(hgt as i32), // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        canvas.bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            return None;
        }
        canvas.old = SelectObject(dc, canvas.bmp);
        if !PrintWindow(h, dc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY | PW_RENDERFULLCONTENT)).as_bool() {
            return None;
        }
        let len = (w as usize) * (hgt as usize) * 4;
        let px = std::slice::from_raw_parts(bits as *const u8, len);
        downscale_bgra(px, w, hgt, max_w)
    }
}

#[cfg(not(windows))]
pub fn window_thumbnail(_hwnd: isize, _max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn downscales_keeping_aspect_and_swapping_to_rgba() {
        let (w, h, px) = downscale_bgra(&solid(1920, 1080, [10, 20, 200, 0]), 1920, 1080, 320).unwrap();
        assert_eq!((w, h), (320, 180));
        assert_eq!(&px[..4], &[200, 20, 10, 255]);
        assert_eq!(px.len(), 320 * 180 * 4);
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let (w, h, _) = downscale_bgra(&solid(100, 50, [0, 0, 255, 0]), 100, 50, 320).unwrap();
        assert_eq!((w, h), (100, 50));
    }

    #[test]
    fn averages_blocks() {
        // 2×1 → 1×1: black + white averages to mid grey.
        let src = [0, 0, 0, 0, 255, 255, 255, 0];
        let (_, _, px) = downscale_bgra(&src, 2, 1, 1).unwrap();
        assert_eq!(&px[..3], &[127, 127, 127]);
    }

    #[test]
    fn black_short_or_empty_inputs_are_rejected() {
        assert!(downscale_bgra(&solid(64, 64, [0, 0, 0, 255]), 64, 64, 32).is_none(), "all black");
        assert!(downscale_bgra(&[1, 2, 3], 64, 64, 32).is_none(), "buffer too short");
        assert!(downscale_bgra(&[], 0, 0, 32).is_none());
        assert!(downscale_bgra(&solid(4, 4, [255; 4]), 4, 4, 0).is_none());
    }

    /// Read-only: prints which open windows produce thumbnails.
    /// `cargo test -p rsoverlay-capture thumbnail -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_thumbnails() {
        let wins = crate::enumerate_windows().unwrap();
        let mut ok = 0;
        for w in &wins {
            let t = window_thumbnail(w.hwnd, 320);
            if t.is_some() {
                ok += 1;
            }
            println!("{:>5} {}", if t.is_some() { "ok" } else { "-" }, w.title);
        }
        assert!(ok > 0 || wins.is_empty());
    }
}
