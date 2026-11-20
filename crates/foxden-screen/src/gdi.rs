use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DEVMODEW, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GdiFlush, GetDC, HALFTONE,
    HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode,
    StretchBlt,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};

/// Shrinks the primary monitor into a small top-down BGRA bitmap.
/// Create, use and drop it on one thread.
pub struct Grabber {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *const u8,
    width: i32,
    height: i32,
}

impl Grabber {
    pub fn new(width: usize, height: usize) -> Result<Self, String> {
        unsafe {
            // Per thread, so the host keeps its own DPI mode and we still see real pixels.
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return Err("CreateCompatibleDC failed".into());
            }
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width as i32;
            // Negative height stores the rows top-down.
            info.bmiHeader.biHeight = -(height as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return Err("CreateDIBSection failed".into());
            }
            let previous = SelectObject(dc, bitmap);
            SetStretchBltMode(dc, HALFTONE);
            SetBrushOrgEx(dc, 0, 0, null_mut());
            Ok(Grabber {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width: width as i32,
                height: height as i32,
            })
        }
    }

    /// Captures the screen as it is now; the slice is `width * height * 4` bytes.
    pub fn grab(&mut self) -> Result<&[u8], String> {
        unsafe {
            // The primary's current mode, since the screen DC spans every monitor.
            let mut mode: DEVMODEW = zeroed();
            mode.dmSize = size_of::<DEVMODEW>() as u16;
            if EnumDisplaySettingsW(null(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
                return Err("EnumDisplaySettings failed".into());
            }
            let source_width = mode.dmPelsWidth as i32;
            let source_height = mode.dmPelsHeight as i32;
            let screen = GetDC(null_mut());
            if screen.is_null() {
                return Err("GetDC failed".into());
            }
            let copied = StretchBlt(
                self.dc,
                0,
                0,
                self.width,
                self.height,
                screen,
                0,
                0,
                source_width,
                source_height,
                SRCCOPY,
            );
            ReleaseDC(null_mut(), screen);
            if copied == 0 {
                return Err("StretchBlt failed".into());
            }
            GdiFlush();
            Ok(std::slice::from_raw_parts(
                self.bits,
                (self.width * self.height * 4) as usize,
            ))
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
