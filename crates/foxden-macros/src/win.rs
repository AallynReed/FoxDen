//! Plays macros with SendInput and, only while asked, watches keys and mouse buttons with
//! low-level hooks to start bound macros, record new ones or catch a hotkey.
//!
//! SendInput can't reach windows of apps running as administrator unless FoxDen is too, and some
//! anti-cheat systems refuse or flag injected input.

use crate::inputs::{self, Raw};
use crate::keys;
use crate::model::{Hotkey, Input, Library, Macro, Mode, Step, fold};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
    MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT,
};

const LLKHF_EXTENDED: u32 = 0x01;
const LLKHF_INJECTED: u32 = 0x10;
const LLMHF_INJECTED: u32 = 0x01;
/// Shortest gap between two rounds of a looping macro.
const LOOP_GAP: Duration = Duration::from_millis(10);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One `INPUT` for a raw event; `None` for pauses.
pub fn to_input(r: Raw) -> Option<INPUT> {
    let key = |vk: u16, scan: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let mouse = |flags: u32, data: i32| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data as _, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    Some(match r {
        Raw::Scan { code, extended, up } => {
            let mut f = KEYEVENTF_SCANCODE;
            if extended {
                f |= KEYEVENTF_EXTENDEDKEY;
            }
            if up {
                f |= KEYEVENTF_KEYUP;
            }
            key(0, code, f)
        }
        Raw::Vk { vk, up } => key(vk, 0, if up { KEYEVENTF_KEYUP } else { 0 }),
        Raw::Unicode { unit, up } => key(0, unit, KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 }),
        Raw::Button { button, up } => {
            let (down_f, up_f, data) = match button {
                1 => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
                2 => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
                3 => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
                4 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 1),
                5 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 2),
                _ => return None,
            };
            mouse(if up { up_f } else { down_f }, data)
        }
        Raw::Wheel { delta, horizontal } => mouse(if horizontal { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL }, delta),
        Raw::Sleep { .. } => return None,
    })
}

fn send(r: Raw) {
    if let Some(i) = to_input(r) {
        unsafe { SendInput(1, &i, std::mem::size_of::<INPUT>() as i32) };
    }
}

fn wait(ms: u64, stop: &AtomicBool) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(5).min(end - Instant::now()));
    }
}

/// Sends raw events, honouring `stop` between them; releases anything it left held.
fn play(raw: &[Raw], stop: &AtomicBool) {
    let mut held: HashSet<Raw> = HashSet::new();
    for r in raw {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match *r {
            Raw::Sleep { ms } => wait(ms as u64, stop),
            Raw::Scan { code, extended, up } => {
                let k = Raw::Scan { code, extended, up: false };
                if up {
                    held.remove(&k);
                } else {
                    held.insert(k);
                }
                send(*r);
            }
            Raw::Button { button, up } => {
                let k = Raw::Button { button, up: false };
                if up {
                    held.remove(&k);
                } else {
                    held.insert(k);
                }
                send(*r);
            }
            _ => send(*r),
        }
    }
    for h in held {
        match h {
            Raw::Scan { code, extended, .. } => send(Raw::Scan { code, extended, up: true }),
            Raw::Button { button, .. } => send(Raw::Button { button, up: true }),
            _ => {}
        }
    }
}

/// Modifier keys physically held, one bit per usage 0xE0..=0xE7.
static HELD_MODS: AtomicU8 = AtomicU8::new(0);

fn modifier_keys(bits: u8) -> impl Iterator<Item = u16> {
    (0..8u16).filter(move |i| bits & (1 << i) != 0).map(|i| 0xE0 + i)
}

/// Lets go of the modifiers the person holds, so they don't change what the macro types.
fn release_mods(bits: u8) {
    for usage in modifier_keys(bits) {
        if let Some((code, extended)) = keys::scancode(usage) {
            send(Raw::Scan { code, extended, up: true });
        }
    }
}

/// Puts back the modifiers that are still held once the macro is done.
fn restore_mods(bits: u8) {
    for usage in modifier_keys(bits & HELD_MODS.load(Ordering::Relaxed)) {
        if let Some((code, extended)) = keys::scancode(usage) {
            send(Raw::Scan { code, extended, up: false });
        }
    }
}

struct Run {
    input: Option<Input>,
    stop: Arc<AtomicBool>,
    held: Arc<AtomicBool>,
}

impl Run {
    fn new(input: Option<Input>) -> Self {
        Run { input, stop: Arc::default(), held: Arc::new(AtomicBool::new(true)) }
    }

    fn done(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn end(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.held.store(false, Ordering::Relaxed);
    }
}

fn spawn_run(m: Macro, mods: u8, delay: Duration, stop: Arc<AtomicBool>, held: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        wait(delay.as_millis() as u64, &stop);
        if stop.load(Ordering::Relaxed) {
            return;
        }
        release_mods(mods);
        let gap = LOOP_GAP.max(Duration::from_millis(m.default_delay_ms.unwrap_or(0) as u64));
        let press = inputs::expand(&m.press, m.default_delay_ms);
        let looping = |steps: &[Raw], cond: &dyn Fn() -> bool| {
            while cond() && !stop.load(Ordering::Relaxed) {
                play(steps, &stop);
                wait(gap.as_millis() as u64, &stop);
            }
        };
        match m.mode {
            Mode::Once => play(&press, &stop),
            Mode::PressHoldRelease => {
                play(&press, &stop);
                let hold = inputs::expand(&m.hold, m.default_delay_ms);
                if !hold.is_empty() {
                    looping(&hold, &|| held.load(Ordering::Relaxed));
                }
                while held.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let release = inputs::expand(&m.release, m.default_delay_ms);
                play(&release, &AtomicBool::new(false));
            }
            Mode::RepeatWhileHeld => looping(&press, &|| held.load(Ordering::Relaxed)),
            Mode::Toggle => looping(&press, &|| true),
        }
        restore_mods(mods);
        stop.store(true, Ordering::Relaxed);
    });
}

/// What a hotkey press or release does to its macro.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    LetGo,
    Nothing,
}

pub fn decide(mode: Mode, running: bool, down: bool) -> Action {
    match (down, running, mode) {
        (false, true, _) => Action::LetGo,
        (false, false, _) => Action::Nothing,
        (true, true, Mode::Toggle) => Action::Stop,
        (true, true, Mode::Once) => Action::Start,
        (true, true, _) => Action::Nothing,
        (true, false, _) => Action::Start,
    }
}

#[derive(Debug, Clone, Copy)]
enum Event {
    Input(Input, bool, u8),
    Wheel(bool, i32),
}

static SINK: Mutex<Option<Sender<(Event, Instant)>>> = Mutex::new(None);
/// Hotkeys hidden from other apps, and the inputs whose press was hidden (so is their release).
static BLOCK: Mutex<(Option<HashSet<Hotkey>>, Vec<Input>)> = Mutex::new((None, Vec::new()));

fn emit(e: Event) -> bool {
    if let Some(tx) = lock(&SINK).as_ref() {
        let _ = tx.send((e, Instant::now()));
    }
    let Event::Input(input, down, mods) = e else { return false };
    let mut b = lock(&BLOCK);
    if down {
        let hide = b.0.as_ref().is_some_and(|set| set.contains(&Hotkey { input, mods }));
        if hide && !b.1.contains(&input) {
            b.1.push(input);
        }
        hide
    } else if let Some(i) = b.1.iter().position(|x| *x == input) {
        b.1.remove(i);
        true
    } else {
        false
    }
}

unsafe extern "system" fn key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if k.flags & LLKHF_INJECTED == 0 {
            let down = matches!(wparam as u32, 0x100 | 0x104);
            let usage = if k.vkCode == keys::VK_PAUSE as u32 {
                Some(keys::PAUSE_USAGE)
            } else {
                keys::usage_from_scancode(k.scanCode as u16, k.flags & LLKHF_EXTENDED != 0)
            };
            if let Some(u) = usage {
                if keys::is_modifier(u) {
                    let bit = 1u8 << (u - 0xE0);
                    if down {
                        HELD_MODS.fetch_or(bit, Ordering::Relaxed);
                    } else {
                        HELD_MODS.fetch_and(!bit, Ordering::Relaxed);
                    }
                }
                let mods = fold(HELD_MODS.load(Ordering::Relaxed));
                if emit(Event::Input(Input::Key(u), down, mods)) {
                    return 1;
                }
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if m.flags & LLMHF_INJECTED == 0 {
            let x = ((m.mouseData >> 16) & 0xFFFF) as u16;
            let mods = fold(HELD_MODS.load(Ordering::Relaxed));
            let b = |n: u8, down: bool| Some(Event::Input(Input::Button(n), down, mods));
            let side = if x == 1 { 4 } else { 5 };
            let ev = match wparam as u32 {
                0x201 => b(1, true),
                0x202 => b(1, false),
                0x204 => b(2, true),
                0x205 => b(2, false),
                0x207 => b(3, true),
                0x208 => b(3, false),
                0x20B => b(side, true),
                0x20C => b(side, false),
                0x20A => Some(Event::Wheel(false, (x as i16) as i32)),
                0x20E => Some(Event::Wheel(true, (x as i16) as i32)),
                _ => None,
            };
            if let Some(e) = ev
                && emit(e)
            {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

struct Hooks {
    thread: u32,
    join: Option<JoinHandle<()>>,
}

impl Hooks {
    fn start(tx: Sender<(Event, Instant)>) -> Result<Self, String> {
        *lock(&SINK) = Some(tx);
        HELD_MODS.store(0, Ordering::Relaxed);
        let (ready_tx, ready_rx) = channel();
        let join = std::thread::Builder::new()
            .name("foxden-macros".into())
            .spawn(move || unsafe {
                let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), std::ptr::null_mut(), 0);
                let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), std::ptr::null_mut(), 0);
                let _ = ready_tx.send((GetCurrentThreadId(), !kb.is_null() && !ms.is_null()));
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
                if !kb.is_null() {
                    UnhookWindowsHookEx(kb);
                }
                if !ms.is_null() {
                    UnhookWindowsHookEx(ms);
                }
            })
            .map_err(|e| e.to_string())?;
        let (thread, ok) = ready_rx.recv().map_err(|e| e.to_string())?;
        let hooks = Hooks { thread, join: Some(join) };
        if ok { Ok(hooks) } else { Err("Windows refused to let FoxDen watch the keyboard and mouse.".into()) }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        *lock(&SINK) = None;
        lock(&BLOCK).1.clear();
    }
}

type Caught = Box<dyn FnOnce(Option<Hotkey>) + Send>;

#[derive(Default)]
struct Shared {
    bindings_on: bool,
    recording: Option<(Vec<Step>, Option<Instant>)>,
    catching: Option<Caught>,
    runs: HashMap<String, Run>,
}

impl Shared {
    fn watching(&self) -> bool {
        self.bindings_on || self.recording.is_some() || self.catching.is_some()
    }

    fn stop_runs(&mut self) {
        for r in self.runs.values() {
            r.end();
        }
        self.runs.clear();
    }
}

/// Playback, hotkeys and recording. Nothing is hooked or sent until bindings go on, recording or
/// catching starts, or a macro is played.
pub struct Host {
    library: Arc<Mutex<Library>>,
    shared: Arc<Mutex<Shared>>,
    hooks: Mutex<Option<Hooks>>,
}

impl Host {
    pub fn new(library: Arc<Mutex<Library>>) -> Self {
        Host { library, shared: Arc::default(), hooks: Mutex::new(None) }
    }

    pub fn bindings_on(&self) -> bool {
        lock(&self.shared).bindings_on
    }

    pub fn is_recording(&self) -> bool {
        lock(&self.shared).recording.is_some()
    }

    pub fn is_catching(&self) -> bool {
        lock(&self.shared).catching.is_some()
    }

    pub fn hooked(&self) -> bool {
        lock(&self.hooks).is_some()
    }

    /// Ids of macros playing now.
    pub fn running(&self) -> Vec<String> {
        let mut s = lock(&self.shared);
        s.runs.retain(|_, r| !r.done());
        s.runs.keys().cloned().collect()
    }

    fn ensure_hooks(&self) -> Result<(), String> {
        let want = lock(&self.shared).watching();
        let mut hooks = lock(&self.hooks);
        let result = if want && hooks.is_none() {
            let (tx, rx) = channel();
            Hooks::start(tx).map(|h| {
                *hooks = Some(h);
                let shared = self.shared.clone();
                let library = self.library.clone();
                std::thread::spawn(move || dispatch(rx, shared, library));
            })
        } else {
            if !want {
                *hooks = None;
            }
            Ok(())
        };
        drop(hooks);
        self.refresh_bindings();
        result
    }

    /// Recomputes which hotkeys are hidden; call after the library changes.
    pub fn refresh_bindings(&self) {
        let on = {
            let s = lock(&self.shared);
            s.bindings_on && s.recording.is_none() && s.catching.is_none()
        };
        let set: HashSet<Hotkey> = if on {
            lock(&self.library)
                .entries
                .iter()
                .filter(|e| e.block && e.macro_.playable())
                .filter_map(|e| e.hotkey)
                .collect()
        } else {
            HashSet::new()
        };
        lock(&BLOCK).0 = Some(set);
    }

    pub fn set_bindings(&self, on: bool) -> Result<(), String> {
        {
            let mut s = lock(&self.shared);
            s.bindings_on = on;
            if !on {
                s.stop_runs();
            }
        }
        let r = self.ensure_hooks();
        if r.is_err() {
            lock(&self.shared).bindings_on = false;
        }
        r
    }

    pub fn start_recording(&self) -> Result<(), String> {
        lock(&self.shared).recording = Some((Vec::new(), None));
        let r = self.ensure_hooks();
        if r.is_err() {
            lock(&self.shared).recording = None;
        }
        r
    }

    /// Stops recording and returns the steps, without the click that pressed Stop.
    pub fn stop_recording(&self) -> Vec<Step> {
        let steps = lock(&self.shared).recording.take().map(|r| r.0).unwrap_or_default();
        let _ = self.ensure_hooks();
        trim_stop_click(steps)
    }

    /// Calls `done` with the next key or button pressed with its modifiers, or `None` on Esc or cancel.
    pub fn catch_hotkey(&self, done: impl FnOnce(Option<Hotkey>) + Send + 'static) -> Result<(), String> {
        if let Some(old) = lock(&self.shared).catching.replace(Box::new(done)) {
            old(None);
        }
        let r = self.ensure_hooks();
        if r.is_err()
            && let Some(f) = lock(&self.shared).catching.take()
        {
            f(None);
        }
        r
    }

    pub fn cancel_catch(&self) {
        let f = lock(&self.shared).catching.take();
        if let Some(f) = f {
            f(None);
        }
        let _ = self.ensure_hooks();
    }

    /// Plays a macro once after `delay`, whatever its mode: press, one round of hold, release.
    pub fn play_now(&self, id: &str, delay: Duration) -> Result<(), String> {
        let m = lock(&self.library).get(id).map(|e| e.macro_.clone()).ok_or("No such macro.")?;
        if m.steps() == 0 {
            return Err("This macro has no steps yet.".into());
        }
        let once = Macro {
            press: [m.press, m.hold, m.release].concat(),
            hold: Vec::new(),
            release: Vec::new(),
            mode: Mode::Once,
            ..m
        };
        let run = Run::new(None);
        run.held.store(false, Ordering::Relaxed);
        spawn_run(once, 0, delay, run.stop.clone(), run.held.clone());
        if let Some(old) = lock(&self.shared).runs.insert(id.to_string(), run) {
            old.end();
        }
        Ok(())
    }

    pub fn stop(&self, id: &str) {
        if let Some(r) = lock(&self.shared).runs.remove(id) {
            r.end();
        }
    }

    pub fn stop_all(&self) {
        lock(&self.shared).stop_runs();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop_all();
        lock(&BLOCK).0 = None;
        *lock(&self.hooks) = None;
    }
}

fn trim_stop_click(mut steps: Vec<Step>) -> Vec<Step> {
    while matches!(steps.last(), Some(Step::Delay { .. }) | Some(Step::Button { button: 1, down: true })) {
        steps.pop();
    }
    steps
}

fn dispatch(rx: Receiver<(Event, Instant)>, shared: Arc<Mutex<Shared>>, library: Arc<Mutex<Library>>) {
    for (ev, at) in rx {
        let mut s = lock(&shared);
        if let Some((steps, last)) = s.recording.as_mut() {
            if let Some(prev) = *last {
                let ms = at.duration_since(prev).as_millis().min(60_000) as u32;
                if ms > 0 {
                    steps.push(Step::Delay { ms });
                }
            }
            *last = Some(at);
            steps.push(match ev {
                Event::Input(Input::Key(usage), down, _) => Step::Key { usage, down },
                Event::Input(Input::Button(button), down, _) => Step::Button { button, down },
                Event::Wheel(horizontal, delta) => {
                    Step::Wheel { horizontal, ticks: (delta.abs() / inputs::WHEEL_DELTA).max(1) * delta.signum() }
                }
            });
            continue;
        }
        let Event::Input(input, down, mods) = ev else { continue };
        if s.catching.is_some() {
            let modifier = matches!(input, Input::Key(u) if keys::is_modifier(u));
            if !down || modifier || input == Input::Button(1) && mods == 0 {
                continue;
            }
            let f = s.catching.take();
            drop(s);
            if let Some(f) = f {
                let esc = input == Input::Key(0x29) && mods == 0;
                f((!esc).then_some(Hotkey { input, mods }));
            }
            continue;
        }
        if !s.bindings_on {
            continue;
        }
        if input == Input::Key(keys::PAUSE_USAGE) {
            if down {
                s.stop_runs();
            }
            continue;
        }
        s.runs.retain(|_, r| !r.done());
        if !down {
            for r in s.runs.values().filter(|r| r.input == Some(input)) {
                r.held.store(false, Ordering::Relaxed);
            }
            continue;
        }
        let entry = lock(&library).bound(Hotkey { input, mods }).cloned();
        let Some(entry) = entry else { continue };
        let running = s.runs.get(&entry.id);
        match decide(entry.macro_.mode, running.is_some(), true) {
            Action::Stop => {
                if let Some(r) = s.runs.remove(&entry.id) {
                    r.end();
                }
            }
            Action::Start => {
                let run = Run::new(Some(input));
                let held = HELD_MODS.load(Ordering::Relaxed);
                spawn_run(entry.macro_.clone(), held, Duration::ZERO, run.stop.clone(), run.held.clone());
                if let Some(old) = s.runs.insert(entry.id.clone(), run) {
                    old.held.store(false, Ordering::Relaxed);
                }
            }
            Action::LetGo | Action::Nothing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_have_the_right_flags() {
        let i = to_input(Raw::Scan { code: 0x48, extended: true, up: true }).unwrap();
        assert_eq!(i.r#type, INPUT_KEYBOARD);
        let ki = unsafe { i.Anonymous.ki };
        assert_eq!(ki.wScan, 0x48);
        assert_eq!(ki.dwFlags, KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP);
        let mi = unsafe { to_input(Raw::Button { button: 5, up: false }).unwrap().Anonymous.mi };
        assert_eq!(mi.dwFlags, MOUSEEVENTF_XDOWN);
        assert_eq!(mi.mouseData as i64, 2);
        let mi = unsafe { to_input(Raw::Wheel { delta: -120, horizontal: true }).unwrap().Anonymous.mi };
        assert_eq!(mi.dwFlags, MOUSEEVENTF_HWHEEL);
        assert_eq!(mi.mouseData as i32, -120);
        let ki = unsafe { to_input(Raw::Unicode { unit: 0x41, up: false }).unwrap().Anonymous.ki };
        assert_eq!(ki.dwFlags, KEYEVENTF_UNICODE);
        assert!(to_input(Raw::Sleep { ms: 5 }).is_none());
        assert!(to_input(Raw::Button { button: 9, up: false }).is_none());
    }

    #[test]
    fn stop_click_is_trimmed() {
        let s = vec![
            Step::Key { usage: 4, down: true },
            Step::Delay { ms: 9 },
            Step::Key { usage: 4, down: false },
            Step::Delay { ms: 400 },
            Step::Button { button: 1, down: true },
        ];
        assert_eq!(trim_stop_click(s).len(), 3);
    }

    #[test]
    fn hotkeys_start_stop_and_let_go() {
        assert_eq!(decide(Mode::Once, false, true), Action::Start);
        assert_eq!(decide(Mode::Once, true, true), Action::Start);
        assert_eq!(decide(Mode::Toggle, false, true), Action::Start);
        assert_eq!(decide(Mode::Toggle, true, true), Action::Stop);
        assert_eq!(decide(Mode::Toggle, true, false), Action::LetGo);
        assert_eq!(decide(Mode::RepeatWhileHeld, true, true), Action::Nothing);
        assert_eq!(decide(Mode::PressHoldRelease, true, false), Action::LetGo);
        assert_eq!(decide(Mode::PressHoldRelease, false, false), Action::Nothing);
    }

    #[test]
    fn modifier_bits_map_to_keys() {
        assert_eq!(modifier_keys(0b1000_0001).collect::<Vec<_>>(), vec![0xE0, 0xE7]);
    }

    #[test]
    fn host_starts_idle_and_unknown_macros_fail() {
        let h = Host::new(Arc::default());
        assert!(!h.bindings_on() && !h.is_recording() && !h.is_catching() && !h.hooked());
        assert!(h.play_now("m1", Duration::ZERO).is_err());
        assert!(h.running().is_empty());
    }

    #[test]
    fn hidden_presses_hide_their_release() {
        let f13 = Hotkey { input: Input::Key(0x68), mods: 0 };
        lock(&BLOCK).0 = Some([f13].into_iter().collect());
        assert!(emit(Event::Input(f13.input, true, 0)));
        assert!(emit(Event::Input(f13.input, false, 2)));
        assert!(!emit(Event::Input(f13.input, false, 0)));
        assert!(!emit(Event::Input(f13.input, true, 1)));
        lock(&BLOCK).0 = None;
    }
}

#[cfg(test)]
mod play_tests {
    use super::*;
    use crate::model::Entry;

    #[test]
    fn a_played_macro_finishes() {
        let mut lib = Library::default();
        let m = Macro { name: "w".into(), press: vec![Step::Delay { ms: 30 }], ..Default::default() };
        lib.put(Entry { id: String::new(), macro_: m, hotkey: None, block: false }).unwrap();
        let h = Host::new(Arc::new(Mutex::new(lib)));
        h.play_now("m1", Duration::from_millis(20)).unwrap();
        assert_eq!(h.running(), ["m1"]);
        std::thread::sleep(Duration::from_millis(300));
        assert!(h.running().is_empty());
    }
}
