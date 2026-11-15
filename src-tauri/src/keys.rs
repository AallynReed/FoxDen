//! Key presses for reactive effects. The hook only runs while such an effect is on and keeps
//! nothing but which key went down and when.

use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

const KEEP: Duration = Duration::from_secs(5);
const MOST: usize = 64;

struct Seen {
    presses: Vec<(u32, Instant)>,
    down: [bool; 256],
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    presses: Vec::new(),
    down: [false; 256],
});

fn seen() -> std::sync::MutexGuard<'static, Seen> {
    SEEN.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let vk = unsafe { (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode };
        let i = (vk & 0xFF) as usize;
        let mut s = seen();
        match wparam as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN if !s.down[i] => {
                s.down[i] = true;
                if s.presses.len() >= MOST {
                    s.presses.remove(0);
                }
                s.presses.push((vk, Instant::now()));
            }
            WM_KEYUP | WM_SYSKEYUP => s.down[i] = false,
            _ => {}
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

pub struct KeyCapture {
    thread: u32,
    worker: Option<JoinHandle<()>>,
}

impl KeyCapture {
    pub fn start() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || unsafe {
            let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), std::ptr::null_mut(), 0);
            if handle.is_null() {
                let _ = tx.send(Err("could not watch the keyboard".to_string()));
                return;
            }
            let _ = tx.send(Ok(GetCurrentThreadId()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            UnhookWindowsHookEx(handle);
        });
        let thread = rx.recv().map_err(|e| e.to_string())??;
        Ok(KeyCapture {
            thread,
            worker: Some(worker),
        })
    }

    /// Recent presses as (virtual-key code, seconds ago).
    pub fn frame(&self) -> Vec<(u32, f32)> {
        let mut s = seen();
        s.presses.retain(|(_, at)| at.elapsed() < KEEP);
        s.presses
            .iter()
            .map(|(vk, at)| (*vk, at.elapsed().as_secs_f32()))
            .collect()
    }
}

impl Drop for KeyCapture {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut s = seen();
        s.presses.clear();
        s.down = [false; 256];
    }
}
