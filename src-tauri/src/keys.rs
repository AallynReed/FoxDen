//! Key presses for reactive effects. The hook only runs while such an effect is on and keeps
//! nothing but which key went down and when.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::state::lock;

const KEEP: Duration = Duration::from_secs(5);
const MOST: usize = 64;

struct Seen {
    presses: Vec<(u32, Instant, u32)>,
    down: [bool; 256],
    count: u32,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    presses: Vec::new(),
    down: [false; 256],
    count: 0,
});

fn seen() -> std::sync::MutexGuard<'static, Seen> {
    lock(&SEEN)
}

/// Set while the hook runs: only then are presses kept.
static ON: AtomicBool = AtomicBool::new(false);

fn seen_key(vk: u32, down: bool) {
    let i = (vk & 0xFF) as usize;
    let mut s = seen();
    if !down {
        s.down[i] = false;
    } else if !s.down[i] {
        s.down[i] = true;
        if s.presses.len() >= MOST {
            s.presses.remove(0);
        }
        s.count = s.count.wrapping_add(1);
        let n = s.count;
        s.presses.push((vk, Instant::now(), n));
    }
}

/// A key the app's own window got. Windows does not show the hook keys typed into this app.
pub fn from_window(vk: u32, down: bool) {
    if ON.load(Ordering::Relaxed) {
        seen_key(vk, down);
    }
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let vk = unsafe { (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode };
        match wparam as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => seen_key(vk, true),
            WM_KEYUP | WM_SYSKEYUP => seen_key(vk, false),
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
        ON.store(true, Ordering::Relaxed);
        Ok(KeyCapture {
            thread,
            worker: Some(worker),
        })
    }

    /// Recent presses as (virtual-key code, seconds ago, press number).
    pub fn frame(&self) -> Vec<(u32, f32, u32)> {
        let mut s = seen();
        s.presses.retain(|(_, at, _)| at.elapsed() < KEEP);
        s.presses
            .iter()
            .map(|(vk, at, n)| (*vk, at.elapsed().as_secs_f32(), *n))
            .collect()
    }
}

impl Drop for KeyCapture {
    fn drop(&mut self) {
        ON.store(false, Ordering::Relaxed);
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut s = seen();
        s.presses.clear();
        s.down = [false; 256];
    }
}
