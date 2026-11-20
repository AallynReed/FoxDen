//! Which apps run and which one is in front, for layers tied to apps. Only names are read,
//! and only while such a layer exists.

use std::collections::BTreeSet;
use std::thread;
use std::time::Duration;

use foxden_core::{Apps, When, app_name};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsWindowVisible,
};
use windows_sys::core::BOOL;

use crate::conflicts::processes;
use crate::state::{Shared, lock};

const EVERY: Duration = Duration::from_millis(750);
/// Windows parts that keep a window open but are never what anyone means.
const HIDDEN: [&str; 3] = [
    "applicationframehost",
    "shellexperiencehost",
    "textinputhost",
];

/// What the UI is told: the watched apps that run, and the app in front.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Seen {
    running: Vec<String>,
    front: Option<String>,
}

fn pid_of(window: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    pid
}

fn scan() -> Apps {
    let procs = processes();
    let front = unsafe { GetForegroundWindow() };
    let front = (!front.is_null()).then(|| pid_of(front));
    Apps {
        running: procs.iter().map(|p| app_name(&p.name)).collect(),
        front: front
            .and_then(|pid| procs.iter().find(|p| p.pid == pid))
            .map(|p| app_name(&p.name)),
    }
}

unsafe extern "system" fn shown(window: HWND, pids: LPARAM) -> BOOL {
    let pids = unsafe { &mut *(pids as *mut Vec<u32>) };
    let top = unsafe {
        IsWindowVisible(window) != 0
            && GetWindow(window, GW_OWNER).is_null()
            && GetWindowTextLengthW(window) > 0
    };
    if top {
        pids.push(pid_of(window));
    }
    1
}

/// Executables with a window open now, for picking one.
pub fn windowed() -> Vec<String> {
    let mut pids: Vec<u32> = Vec::new();
    unsafe { EnumWindows(Some(shown), &mut pids as *mut Vec<u32> as LPARAM) };
    let me = std::process::id();
    processes()
        .into_iter()
        .filter(|p| {
            p.pid != me && pids.contains(&p.pid) && !HIDDEN.contains(&app_name(&p.name).as_str())
        })
        .map(|p| p.name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn spawn(app: AppHandle, shared: Shared) {
    thread::spawn(move || {
        let mut told = Seen::default();
        loop {
            thread::sleep(EVERY);
            let watched: BTreeSet<String> = lock(&shared)
                .engine
                .layers
                .iter()
                .filter(|l| l.when != When::Always)
                .flat_map(|l| l.apps.iter().map(|a| app_name(a)))
                .collect();
            let apps = if watched.is_empty() {
                Apps::default()
            } else {
                scan()
            };
            let seen = Seen {
                running: watched
                    .iter()
                    .filter(|a| apps.running.contains(*a))
                    .cloned()
                    .collect(),
                front: apps.front.clone(),
            };
            lock(&shared).engine.apps = apps;
            if seen != told {
                let _ = app.emit("apps", &seen);
                told = seen;
            }
        }
    });
}

#[tauri::command]
pub fn list_apps() -> Vec<String> {
    windowed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_test_is_among_the_running_apps() {
        let me = std::env::current_exe().unwrap();
        let me = app_name(&me.file_name().unwrap().to_string_lossy());
        assert!(scan().running.contains(&me));
    }

    #[test]
    #[ignore = "prints what runs on this machine"]
    fn dump() {
        println!("{:?}\n{:?}", scan().front, windowed());
    }
}
