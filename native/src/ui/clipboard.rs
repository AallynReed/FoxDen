use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

/// Puts `text` on the Windows clipboard.
pub fn copy(text: &str) -> Result<(), String> {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("the clipboard is in use".into());
        }
        let done = put(&wide);
        CloseClipboard();
        done
    }
}

unsafe fn put(wide: &[u16]) -> Result<(), String> {
    unsafe {
        if EmptyClipboard() == 0 {
            return Err("the clipboard could not be cleared".into());
        }
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return Err("out of memory for the clipboard".into());
        }
        let at = GlobalLock(mem) as *mut u16;
        if at.is_null() {
            GlobalFree(mem);
            return Err("the clipboard memory could not be locked".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), at, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT as u32, mem).is_null() {
            GlobalFree(mem);
            return Err("the clipboard refused the text".into());
        }
        Ok(())
    }
}
