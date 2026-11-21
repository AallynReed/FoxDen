//! Finds and closes other lighting and fan software. The list itself is in `foxden_core::conflicts`.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use foxden_client::{Client, PIPE};
use foxden_core::conflicts::{self, APPS, Area, Closed, Process, Scope};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, SYSTEMTIME};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};

use crate::config::Settings;
use crate::state::{Shared, lock};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub id: &'static str,
    pub name: &'static str,
    pub area: Area,
    /// Whether the switches in force would close it.
    pub closes: bool,
    /// The FoxDen feature that keeps it running, such as "SteelSeries GG".
    pub kept_for: Option<&'static str>,
    pub running: bool,
    /// Its executables running now or seen before, and whether each is closed or kept.
    pub exes: Vec<Exe>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Exe {
    pub name: String,
    pub closes: bool,
    pub running: bool,
    pub path: Option<String>,
}

/// A conflicting executable FoxDen has seen, kept in the config.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Known {
    pub app: String,
    pub name: String,
    pub exe: String,
    /// None when the process could not be read, such as one running elevated.
    pub path: Option<String>,
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

/// Listed software running now, from processes and services, and what was seen before.
pub fn running(app: &AppHandle, scope: Scope, keep: &[String]) -> Vec<Found> {
    let procs = processes();
    let known = record(app, observed(&procs, image_path));
    let live = running_services(Scope {
        lighting: true,
        fans: true,
        ..Scope::default()
    });
    found(&procs, &live, scope, keep, &known)
}

fn found(
    procs: &[Process],
    services: &[&str],
    scope: Scope,
    keep: &[String],
    known: &[Known],
) -> Vec<Found> {
    APPS.iter()
        .filter_map(|app| {
            let live = procs
                .iter()
                .any(|p| conflicts::app_of(&p.name) == Some(app))
                || app.services.iter().any(|s| services.contains(s));
            let exe = |name: &str, running: bool, path: Option<String>| Exe {
                name: name.to_string(),
                closes: scope.covers(app) && !kept(keep, name),
                running,
                path,
            };
            let path_of = |name: &str| {
                known
                    .iter()
                    .find(|k| k.exe.eq_ignore_ascii_case(name))
                    .and_then(|k| k.path.clone())
            };
            let mut exes: Vec<Exe> = Vec::new();
            for p in procs
                .iter()
                .filter(|p| conflicts::app_of(&p.name) == Some(app))
            {
                if !exes.iter().any(|e| e.name.eq_ignore_ascii_case(&p.name)) {
                    exes.push(exe(&p.name, true, path_of(&p.name)));
                }
            }
            for k in known.iter().filter(|k| k.app == app.id) {
                if !exes.iter().any(|e| e.name.eq_ignore_ascii_case(&k.exe)) {
                    exes.push(exe(&k.exe, false, k.path.clone()));
                }
            }
            if !live && exes.is_empty() {
                return None;
            }
            exes.sort_by_key(|e| e.name.to_lowercase());
            let closes = scope.covers(app) && (exes.is_empty() || exes.iter().any(|e| e.closes));
            Some(Found {
                id: app.id,
                name: app.name,
                area: app.area,
                closes,
                kept_for: scope.kept_for(app),
                running: live,
                exes,
            })
        })
        .collect()
}

/// The conflicting executables among `procs`, kept or not, with the paths `path_of` can read.
fn observed(procs: &[Process], path_of: impl Fn(u32) -> Option<String>) -> Vec<Known> {
    let own = std::process::id();
    let mut seen: Vec<Known> = Vec::new();
    for p in procs.iter().filter(|p| p.pid != own && p.pid > 4) {
        let Some(app) = conflicts::app_of(&p.name) else {
            continue;
        };
        let path = path_of(p.pid);
        match seen
            .iter_mut()
            .find(|k| k.exe.eq_ignore_ascii_case(&p.name))
        {
            Some(k) => {
                if k.path.is_none() {
                    k.path = path;
                }
            }
            None => seen.push(Known {
                app: app.id.into(),
                name: app.name.into(),
                exe: p.name.clone(),
                path,
            }),
        }
    }
    seen
}

/// Adds what was seen to `known`, one entry per executable; true when anything changed.
fn merge(known: &mut Vec<Known>, seen: Vec<Known>) -> bool {
    let mut changed = false;
    for s in seen {
        match known
            .iter_mut()
            .find(|k| k.exe.eq_ignore_ascii_case(&s.exe))
        {
            Some(k) => {
                let next = Known {
                    exe: k.exe.clone(),
                    path: s.path.or_else(|| k.path.clone()),
                    ..s
                };
                if *k != next {
                    *k = next;
                    changed = true;
                }
            }
            None => {
                known.push(s);
                changed = true;
            }
        }
    }
    changed
}

/// Drops what was uninstalled or is no longer listed; entries without a path stay.
fn prune(known: &mut Vec<Known>, exists: impl Fn(&Path) -> bool) -> bool {
    let before = known.len();
    known.retain(|k| {
        APPS.iter().any(|a| a.id == k.app) && k.path.as_ref().is_none_or(|p| exists(Path::new(p)))
    });
    known.len() != before
}

/// Keeps what was seen in the config and gives back the whole list.
fn record(app: &AppHandle, seen: Vec<Known>) -> Vec<Known> {
    let shared = app.state::<Shared>();
    let mut state = lock(&shared);
    if merge(&mut state.config.known_conflicts, seen) {
        state.changed = Some(Instant::now());
    }
    state.config.known_conflicts.clone()
}

/// Forgets software uninstalled since the last start.
pub fn forget_removed(shared: &Shared) {
    let mut state = lock(shared);
    if prune(&mut state.config.known_conflicts, Path::exists) {
        state.changed = Some(Instant::now());
    }
}

/// Closes what `scope` covers: processes from here, the rest through the service. None when nothing ran.
pub fn sweep(app: &AppHandle, scope: Scope, keep: &[String]) -> Option<Report> {
    let procs = processes();
    record(app, observed(&procs, image_path));
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

fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(handle);
        (ok && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
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
                ..Scope::default()
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
        let list = found(&procs, &["CorsairService"], lighting, &[], &[]);
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
        let list = found(&procs, &[], lighting, &keep, &[]);
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
        assert!(!found(&procs, &[], lighting, &all, &[])[0].closes);
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

    fn known(app: &str, exe: &str, path: Option<&str>) -> Known {
        Known {
            app: app.into(),
            name: String::new(),
            exe: exe.into(),
            path: path.map(String::from),
        }
    }

    #[test]
    fn seen_executables_are_recorded_with_their_paths() {
        let procs = procs(&["lghub.exe", "lghub.exe", "notepad.exe", "lghub_agent.exe"]);
        let seen = observed(&procs, |pid| {
            (pid != 100).then(|| format!("C:\\G\\{pid}.exe"))
        });
        let got: Vec<_> = seen
            .iter()
            .map(|k| {
                (
                    k.app.as_str(),
                    k.name.as_str(),
                    k.exe.as_str(),
                    k.path.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "ghub",
                    "Logitech G HUB",
                    "lghub.exe",
                    Some("C:\\G\\104.exe")
                ),
                (
                    "ghub",
                    "Logitech G HUB",
                    "lghub_agent.exe",
                    Some("C:\\G\\112.exe")
                ),
            ]
        );
    }

    #[test]
    fn merging_keeps_one_entry_per_executable_and_never_loses_a_path() {
        let mut list = vec![known("ghub", "lghub.exe", Some("C:\\G\\lghub.exe"))];
        assert!(!merge(&mut list, vec![known("ghub", "LGHUB.exe", None)]));
        assert_eq!(list[0].path.as_deref(), Some("C:\\G\\lghub.exe"));
        assert!(merge(
            &mut list,
            vec![
                known("ghub", "lghub.exe", Some("D:\\G\\lghub.exe")),
                known("fancontrol", "FanControl.exe", None),
            ]
        ));
        assert_eq!(
            list,
            [
                known("ghub", "lghub.exe", Some("D:\\G\\lghub.exe")),
                known("fancontrol", "FanControl.exe", None),
            ]
        );
        assert!(merge(
            &mut list,
            vec![known(
                "fancontrol",
                "FanControl.exe",
                Some("C:\\F\\FanControl.exe")
            )]
        ));
        assert_eq!(list[1].path.as_deref(), Some("C:\\F\\FanControl.exe"));
    }

    #[test]
    fn uninstalled_software_is_forgotten_and_unreadable_entries_stay() {
        let mut list = vec![
            known("ghub", "lghub.exe", Some("C:\\G\\lghub.exe")),
            known("icue", "iCUE.exe", Some("C:\\Gone\\iCUE.exe")),
            known("razer", "RzSynapse.exe", None),
            known("gone", "Old.exe", None),
        ];
        assert!(prune(&mut list, |p| !p.starts_with("C:\\Gone")));
        assert_eq!(
            list,
            [
                known("ghub", "lghub.exe", Some("C:\\G\\lghub.exe")),
                known("razer", "RzSynapse.exe", None),
            ]
        );
        assert!(!prune(&mut list, |_| true));
    }

    #[test]
    fn remembered_software_is_listed_while_it_is_not_running() {
        let lighting = Scope {
            lighting: true,
            ..Scope::default()
        };
        let list_of = |procs: &[Process]| {
            let known = [
                known("steelseries", "SteelSeriesGG.exe", Some("C:\\S\\GG.exe")),
                known("steelseries", "SteelSeriesSonar.exe", None),
                known("ghub", "lghub.exe", None),
            ];
            let keep = ["SteelSeriesSonar.exe".to_string()];
            found(procs, &[], lighting, &keep, &known)
        };
        let list = list_of(&procs(&["SteelSeriesGG.exe"]));
        let apps: Vec<_> = list.iter().map(|f| (f.id, f.running, f.closes)).collect();
        assert_eq!(apps, [("ghub", false, true), ("steelseries", true, true)]);
        let exes: Vec<_> = list[1]
            .exes
            .iter()
            .map(|e| (e.name.as_str(), e.running, e.closes, e.path.as_deref()))
            .collect();
        assert_eq!(
            exes,
            [
                ("SteelSeriesGG.exe", true, true, Some("C:\\S\\GG.exe")),
                ("SteelSeriesSonar.exe", false, false, None),
            ]
        );
        assert!(list_of(&[]).iter().all(|f| !f.running));
    }
}
