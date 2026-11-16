//! Finds and closes other lighting and fan software. The list itself is in `foxrgb_core::conflicts`.

use std::sync::Mutex;
use std::time::Duration;

use foxrgb_client::{Client, PIPE};
use foxrgb_core::conflicts::{self, APPS, Area, Closed, Process, Scope};
use serde::Serialize;
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, SYSTEMTIME};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::config::Settings;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub id: &'static str,
    pub name: &'static str,
    pub area: Area,
    /// Whether the switches in force would close it.
    pub closes: bool,
    /// Its executables running now, and whether each is closed or kept.
    pub exes: Vec<Exe>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Exe {
    pub name: String,
    pub closes: bool,
}

/// Whether the user keeps this executable running.
fn kept(keep: &[String], exe: &str) -> bool {
    keep.iter().any(|k| k.eq_ignore_ascii_case(exe))
}

/// The outcome of a pass, with the line the UI shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub closed: Vec<String>,
    pub left: Vec<String>,
    pub text: String,
    /// Local time of the pass, "HH:MM".
    pub at: String,
}

static LAST: Mutex<Option<Report>> = Mutex::new(None);

/// The last pass that found anything.
pub fn last() -> Option<Report> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Keeps this pass; true when it is news: something was closed, or what is left has changed.
pub fn remember(report: Option<Report>) -> bool {
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let news = is_news(last.as_ref(), report.as_ref());
    if report.is_some() {
        *last = report;
    }
    news
}

fn is_news(last: Option<&Report>, now: Option<&Report>) -> bool {
    match now {
        None => false,
        Some(now) => !now.closed.is_empty() || last.is_none_or(|l| l.left != now.left),
    }
}

pub fn scope(settings: &Settings) -> Scope {
    Scope {
        lighting: settings.lighting_module,
        fans: settings.fans_module,
        keep_openrgb: settings.openrgb_enabled,
    }
}

/// Listed software running now, from processes and services.
pub fn running(scope: Scope, keep: &[String]) -> Vec<Found> {
    let procs = processes();
    let live = running_services(Scope {
        lighting: true,
        fans: true,
        keep_openrgb: false,
    });
    found(&procs, &live, scope, keep)
}

fn found(procs: &[Process], services: &[&str], scope: Scope, keep: &[String]) -> Vec<Found> {
    APPS.iter()
        .filter(|app| {
            procs
                .iter()
                .any(|p| conflicts::app_of(&p.name) == Some(*app))
                || app.services.iter().any(|s| services.contains(s))
        })
        .map(|app| {
            let mut exes: Vec<Exe> = Vec::new();
            for p in procs
                .iter()
                .filter(|p| conflicts::app_of(&p.name) == Some(app))
            {
                if !exes.iter().any(|e| e.name.eq_ignore_ascii_case(&p.name)) {
                    exes.push(Exe {
                        name: p.name.clone(),
                        closes: scope.covers(app) && !kept(keep, &p.name),
                    });
                }
            }
            exes.sort_by_key(|e| e.name.to_lowercase());
            let closes = scope.covers(app) && (exes.is_empty() || exes.iter().any(|e| e.closes));
            Found {
                id: app.id,
                name: app.name,
                area: app.area,
                closes,
                exes,
            }
        })
        .collect()
}

/// Closes what `scope` covers: processes from here, the rest through the service. None when nothing ran.
pub fn sweep(scope: Scope, keep: &[String]) -> Option<Report> {
    let procs = processes();
    let mut targets = conflicts::targets(&procs, scope, std::process::id());
    targets.retain(|(p, _)| !kept(keep, &p.name));
    let services = running_services(scope);
    if targets.is_empty() && services.is_empty() {
        return None;
    }
    let mut out = Closed::default();
    for (process, app) in &targets {
        out.add(app, kill(process.pid));
    }
    if !out.left.is_empty() || !services.is_empty() {
        match Client::new(PIPE, Duration::from_secs(10)).close_conflicts(scope) {
            Ok(more) => out.merge(more),
            Err(_) => {
                for (name, app) in conflicts::services(scope) {
                    if services.contains(&name) {
                        out.add(app, false);
                    }
                }
            }
        }
    }
    Some(report(out))
}

pub fn report(out: Closed) -> Report {
    let mut parts = Vec::new();
    if !out.closed.is_empty() {
        parts.push(format!("Closed {}", out.closed.join(", ")));
    }
    if !out.left.is_empty() {
        parts.push(format!("Could not close {}", out.left.join(", ")));
    }
    Report {
        text: parts.join(". "),
        closed: out.closed,
        left: out.left,
        at: now(),
    }
}

fn now() -> String {
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    format!("{:02}:{:02}", time.wHour, time.wMinute)
}

fn running_services(scope: Scope) -> Vec<&'static str> {
    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    else {
        return Vec::new();
    };
    conflicts::services(scope)
        .map(|(name, _)| name)
        .filter(|name| {
            manager
                .open_service(name, ServiceAccess::QUERY_STATUS)
                .and_then(|s| s.query_status())
                .is_ok_and(|s| s.current_state != ServiceState::Stopped)
        })
        .collect()
}

pub fn processes() -> Vec<Process> {
    let mut found = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            found.push(Process {
                pid: entry.th32ProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    found
}

/// Never ends anything under `cargo test`.
fn kill(pid: u32) -> bool {
    if cfg!(test) {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let ok = TerminateProcess(handle, 1) != 0;
        // Until it has exited, the process may still hold its devices.
        if ok {
            WaitForSingleObject(handle, 2000);
        }
        CloseHandle(handle);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs(names: &[&str]) -> Vec<Process> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Process {
                pid: 100 + 4 * i as u32,
                name: n.to_string(),
            })
            .collect()
    }

    #[test]
    fn the_scope_follows_the_switches() {
        let mut settings = Settings::default();
        assert_eq!(
            scope(&settings),
            Scope {
                lighting: true,
                fans: true,
                keep_openrgb: false
            }
        );
        settings.fans_module = false;
        settings.openrgb_enabled = true;
        assert_eq!(
            scope(&settings),
            Scope {
                lighting: true,
                fans: false,
                keep_openrgb: true
            }
        );
    }

    #[test]
    fn running_software_is_listed_once_with_what_would_be_closed() {
        let procs = procs(&[
            "lghub.exe",
            "lghub_agent.exe",
            "FanControl.exe",
            "notepad.exe",
        ]);
        let lighting = Scope {
            lighting: true,
            ..Scope::default()
        };
        let list = found(&procs, &["CorsairService"], lighting, &[]);
        let names: Vec<_> = list.iter().map(|f| (f.name, f.closes)).collect();
        assert_eq!(
            names,
            [
                ("Logitech G HUB", true),
                ("Corsair iCUE", true),
                ("Fan Control", false)
            ]
        );
    }

    #[test]
    fn kept_executables_stay_and_the_rest_of_the_app_still_closes() {
        let procs = procs(&[
            "SteelSeriesGG.exe",
            "SteelSeriesSonar.exe",
            "SteelSeriesPrism.exe",
        ]);
        let lighting = Scope {
            lighting: true,
            ..Scope::default()
        };
        let keep = vec!["steelseriessonar.EXE".to_string()];
        let list = found(&procs, &[], lighting, &keep);
        let exes: Vec<_> = list[0]
            .exes
            .iter()
            .map(|e| (e.name.as_str(), e.closes))
            .collect();
        assert_eq!(
            exes,
            [
                ("SteelSeriesGG.exe", true),
                ("SteelSeriesPrism.exe", true),
                ("SteelSeriesSonar.exe", false)
            ]
        );
        assert!(list[0].closes);
        let all: Vec<String> = [
            "SteelSeriesGG.exe",
            "SteelSeriesPrism.exe",
            "SteelSeriesSonar.exe",
        ]
        .map(String::from)
        .to_vec();
        assert!(!found(&procs, &[], lighting, &all)[0].closes);
    }

    #[test]
    fn the_notice_says_what_was_closed_and_what_was_left() {
        let out = Closed {
            closed: vec!["SignalRGB".into(), "Fan Control".into()],
            left: vec!["Corsair iCUE".into()],
        };
        assert_eq!(
            report(out).text,
            "Closed SignalRGB, Fan Control. Could not close Corsair iCUE"
        );
        assert_eq!(report(Closed::default()).text, "");
    }

    #[test]
    fn the_same_leftovers_are_told_once() {
        let left = |names: &[&str]| Report {
            left: names.iter().map(|n| n.to_string()).collect(),
            ..Report::default()
        };
        let closed = Report {
            closed: vec!["SignalRGB".into()],
            ..Report::default()
        };
        assert!(is_news(None, Some(&left(&["Corsair iCUE"]))));
        assert!(!is_news(
            Some(&left(&["Corsair iCUE"])),
            Some(&left(&["Corsair iCUE"]))
        ));
        assert!(is_news(Some(&left(&["Corsair iCUE"])), Some(&left(&[]))));
        assert!(is_news(Some(&closed), Some(&closed)));
        assert!(!is_news(Some(&closed), None));
    }
}
