//! Moving apps off the speakers so they are heard once, through the mixer.
//!
//! Process loopback hears an app after its own volume and mute in the Windows mixer, so muting
//! the app would silence the capture too. It hears it before the output's volume and mute, so
//! each assigned app is sent to a spare output (its per-app default) and that output is muted
//! while the mixer runs. Everything changed is written to a journal first and put back on stop,
//! on exit, or on the next start after a crash.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::system::{self, AppOutputs, Endpoint, Process, Session};

/// Apps never offered or moved: the shell and FoxDen itself.
const NEVER: [&str; 2] = ["explorer.exe", "foxden.exe"];

/// Apps known to pick an output in their own settings, whatever Windows' default is.
const OWN_PICKERS: [&str; 4] = [
    "discord.exe",
    "discordptb.exe",
    "discordcanary.exe",
    "vesktop.exe",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Muted {
    id: String,
    was_muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Route {
    /// The app's own choice before FoxDen moved it, empty for the system default.
    before: String,
    to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Journal {
    sink: Option<Muted>,
    /// Keyed by lowercase executable name.
    routes: BTreeMap<String, Route>,
}

/// What the mixer wants while it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub sink: Endpoint,
    /// Lowercase executable name to channel.
    pub apps: BTreeMap<String, Channel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AppState {
    /// Not running, or has not played anything yet.
    Idle,
    Playing,
    /// It plays on the default output and has not taken the route yet; a restart moves it.
    Elsewhere {
        output: String,
    },
    /// It picked an output in its own settings, which Windows' per-app routing never overrides.
    OwnOutput {
        output: String,
    },
    /// It started another assigned app, whose sound it would double.
    Contains {
        app: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningApp {
    pub exe: String,
    /// Playing right now.
    pub active: bool,
}

pub fn lower(exe: &str) -> String {
    exe.to_lowercase()
}

/// The spare output apps are parked on: the chosen one, or the first that is neither the
/// default, the mixer's output nor one in `avoid` (recorded outputs and the output priority
/// list, by name or id). It is muted, so it must never be anything meant to be heard.
pub fn choose_sink(
    endpoints: &[Endpoint],
    chosen: &str,
    output: &str,
    avoid: &[&str],
) -> Result<Endpoint, String> {
    let free = |e: &&Endpoint| {
        !e.default
            && e.name != output
            && !avoid.contains(&e.name.as_str())
            && !avoid.contains(&e.id.as_str())
    };
    if !chosen.is_empty() {
        let e = endpoints.iter().find(|e| e.name == chosen).ok_or_else(|| {
            format!("{chosen} is not connected; pick another output to park apps on")
        })?;
        if e.default {
            return Err(format!(
                "{chosen} is the Windows default output; parking apps there would mute everything else"
            ));
        }
        if !free(&e) {
            return Err(format!(
                "{chosen} is played on or in the output priority; pick another output to park apps on"
            ));
        }
        return Ok(e.clone());
    }
    endpoints
        .iter()
        .find(free)
        .cloned()
        .ok_or_else(|| "Apps are not moved: no spare output to park them on, besides the default, the mixer's output and the output priority".into())
}

/// Running apps that have audio sessions, for picking.
pub fn running_apps(sessions: &[Session], procs: &[Process]) -> Vec<RunningApp> {
    let exe: HashMap<u32, &str> = procs.iter().map(|p| (p.pid, p.exe.as_str())).collect();
    let own = std::process::id();
    let mut out: Vec<RunningApp> = Vec::new();
    for s in sessions.iter().filter(|s| s.pid != own) {
        let Some(&name) = exe.get(&s.pid) else {
            continue;
        };
        if NEVER.contains(&lower(name).as_str()) {
            continue;
        }
        match out.iter_mut().find(|a| lower(&a.exe) == lower(name)) {
            Some(a) => a.active |= s.active,
            None => out.push(RunningApp {
                exe: name.to_string(),
                active: s.active,
            }),
        }
    }
    out.sort_by_key(|a| lower(&a.exe));
    out
}

fn descends(pid: u32, from: u32, parent: &HashMap<u32, u32>) -> bool {
    let mut p = pid;
    for _ in 0..64 {
        match parent.get(&p) {
            Some(&up) if up == from => return true,
            Some(&up) if up != 0 && up != p => p = up,
            _ => return false,
        }
    }
    false
}

/// Which process trees to capture for each channel, and how each assigned app is doing. A
/// process is captured only while all its playing sessions are on the sink, so it is never heard twice.
pub fn captures(
    plan: &Plan,
    sessions: &[Session],
    procs: &[Process],
    endpoints: &[Endpoint],
) -> (Vec<(Channel, u32)>, BTreeMap<String, AppState>) {
    let exe: HashMap<u32, String> = procs.iter().map(|p| (p.pid, lower(&p.exe))).collect();
    let parent: HashMap<u32, u32> = procs.iter().map(|p| (p.pid, p.parent)).collect();
    let mut states: BTreeMap<String, AppState> = plan
        .apps
        .keys()
        .map(|k| (k.clone(), AppState::Idle))
        .collect();
    let mut targets: Vec<(Channel, u32, String)> = Vec::new();
    let mut away: Vec<u32> = Vec::new();
    let mut pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    for pid in pids {
        let elsewhere = sessions
            .iter()
            .find(|s| s.pid == pid && s.active && s.endpoint != plan.sink.id);
        let assigned = exe
            .get(&pid)
            .and_then(|n| plan.apps.get(n).map(|&c| (n, c)));
        match (elsewhere, assigned) {
            (Some(s), Some((name, _))) => {
                away.push(pid);
                let on = endpoints.iter().find(|e| e.id == s.endpoint);
                let output = on.map_or_else(|| "another output".to_string(), |e| e.name.clone());
                let state =
                    if on.is_some_and(|e| e.default) && !OWN_PICKERS.contains(&name.as_str()) {
                        AppState::Elsewhere { output }
                    } else {
                        AppState::OwnOutput { output }
                    };
                states.insert(name.clone(), state);
            }
            (Some(_), None) => away.push(pid),
            (None, Some((name, channel))) => targets.push((channel, pid, name.clone())),
            (None, None) => {}
        }
    }
    // A tree is captured whole, so it must not hold anything heard directly or another app.
    let mut keep: Vec<bool> = targets
        .iter()
        .map(|(_, pid, _)| !away.iter().any(|a| descends(*a, *pid, &parent)))
        .collect();
    for (i, (_, pid, name)) in targets.iter().enumerate() {
        if let Some((_, _, inner)) = targets
            .iter()
            .find(|(_, other, n)| n != name && descends(*other, *pid, &parent))
        {
            keep[i] = false;
            states.insert(name.clone(), AppState::Contains { app: inner.clone() });
        }
    }
    // A child of a captured process of the same app comes with it.
    let kept = keep.clone();
    for (i, (_, pid, _)) in targets.iter().enumerate() {
        if targets
            .iter()
            .enumerate()
            .any(|(j, (_, up, _))| kept[j] && j != i && descends(*pid, *up, &parent))
        {
            keep[i] = false;
        }
    }
    let mut out = Vec::new();
    for ((channel, pid, name), k) in targets.iter().zip(keep) {
        if k {
            out.push((*channel, *pid));
            if states.get(name) == Some(&AppState::Idle) {
                states.insert(name.clone(), AppState::Playing);
            }
        }
    }
    (out, states)
}

/// Keeps the journal and applies routes and the sink mute. Lives on one thread (COM).
pub struct Parker {
    path: PathBuf,
    journal: Journal,
    outputs: Option<AppOutputs>,
    /// When each app was last sent to the sink; it may take a moment to move.
    moved: HashMap<String, Instant>,
}

/// How long a freshly routed app may keep playing elsewhere before it is reported.
const MOVING: Duration = Duration::from_secs(4);

impl Parker {
    pub fn new(path: PathBuf) -> Parker {
        let journal = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Parker {
            path,
            journal,
            outputs: None,
            moved: HashMap::new(),
        }
    }

    /// Something still has to be put back.
    pub fn dirty(&self) -> bool {
        self.journal != Journal::default()
    }

    fn save(&self) {
        if self.journal == Journal::default() {
            let _ = fs::remove_file(&self.path);
        } else if let Ok(json) = serde_json::to_vec_pretty(&self.journal) {
            // Swapped in whole, so a crash mid-write never loses what is parked.
            let tmp = self.path.with_extension("tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
    }

    fn outputs(&mut self) -> Result<&AppOutputs, String> {
        if self.outputs.is_none() {
            self.outputs = Some(AppOutputs::new()?);
        }
        Ok(self.outputs.as_ref().unwrap())
    }

    fn restore_sink(&mut self) {
        if let Some(m) = self.journal.sink.take() {
            if system::set_endpoint_muted(&m.id, m.was_muted).is_err()
                && system::endpoint_muted(&m.id).is_ok()
            {
                // Still there but refusing: try again next time.
                self.journal.sink = Some(m);
            }
            self.save();
        }
    }

    fn mute_sink(&mut self, id: &str) -> Result<(), String> {
        if self.journal.sink.as_ref().is_some_and(|m| m.id != id) {
            self.restore_sink();
        }
        if self.journal.sink.is_none() {
            self.journal.sink = Some(Muted {
                id: id.to_string(),
                was_muted: system::endpoint_muted(id)?,
            });
            self.save();
        }
        if !system::endpoint_muted(id)? {
            system::set_endpoint_muted(id, true)?;
        }
        Ok(())
    }

    /// One pass. With a plan: parks the assigned apps and says what to capture. Without one:
    /// puts back what it can now and the rest once each app shows up again.
    pub fn tick(
        &mut self,
        plan: Option<&Plan>,
    ) -> (Vec<(Channel, u32)>, BTreeMap<String, AppState>) {
        if plan.is_none_or(|p| p.apps.is_empty()) && !self.dirty() {
            return (Vec::new(), BTreeMap::new());
        }
        let sessions = system::sessions().unwrap_or_default();
        let endpoints = system::endpoints().unwrap_or_default();
        self.tick_with(plan, &sessions, &system::processes(), &endpoints)
    }

    /// [`Parker::tick`] on a snapshot the caller already took.
    pub fn tick_with(
        &mut self,
        plan: Option<&Plan>,
        sessions: &[Session],
        procs: &[Process],
        endpoints: &[Endpoint],
    ) -> (Vec<(Channel, u32)>, BTreeMap<String, AppState>) {
        let active = plan.filter(|p| !p.apps.is_empty());
        if active.is_none() && !self.dirty() {
            return (Vec::new(), BTreeMap::new());
        }
        let mut moved = false;
        let exe: HashMap<u32, String> = procs.iter().map(|p| (p.pid, lower(&p.exe))).collect();
        let live: HashMap<String, u32> = sessions
            .iter()
            .filter_map(|s| exe.get(&s.pid).map(|e| (e.clone(), s.pid)))
            .collect();

        let mut errors: BTreeMap<String, String> = BTreeMap::new();
        if let Some(plan) = active {
            if let Err(e) = self.mute_sink(&plan.sink.id) {
                errors.extend(plan.apps.keys().map(|k| (k.clone(), e.clone())));
            }
        } else {
            self.restore_sink();
        }

        let wanted: Vec<&String> = active.map(|p| p.apps.keys().collect()).unwrap_or_default();
        let leaving: Vec<String> = self
            .journal
            .routes
            .keys()
            .filter(|k| !wanted.contains(k))
            .cloned()
            .collect();
        for name in leaving {
            let Some(&pid) = live.get(&name) else {
                continue;
            };
            let before = self.journal.routes[&name].before.clone();
            moved = true;
            if self.outputs().and_then(|o| o.set(pid, &before)).is_ok() {
                self.journal.routes.remove(&name);
                self.save();
            }
        }
        let Some(plan) = active else {
            return (Vec::new(), BTreeMap::new());
        };
        for name in wanted {
            let Some(&pid) = live.get(name) else { continue };
            if self
                .journal
                .routes
                .get(name)
                .is_some_and(|r| r.to == plan.sink.id)
            {
                continue;
            }
            let out = self.outputs().and_then(|o| {
                let before = o.get(pid)?;
                Ok(if before == plan.sink.id {
                    String::new()
                } else {
                    before
                })
            });
            let before = match (out, self.journal.routes.get(name)) {
                (_, Some(r)) => r.before.clone(),
                (Ok(b), None) => b,
                (Err(e), None) => {
                    errors.insert(name.clone(), e);
                    continue;
                }
            };
            self.journal.routes.insert(
                name.clone(),
                Route {
                    before,
                    to: plan.sink.id.clone(),
                },
            );
            self.save();
            moved = true;
            match self.outputs().and_then(|o| o.set(pid, &plan.sink.id)) {
                Ok(()) => {
                    self.moved.insert(name.clone(), Instant::now());
                }
                Err(e) => {
                    errors.insert(name.clone(), e);
                }
            }
        }

        // Sessions again: routing can have moved them.
        let again = moved.then(|| system::sessions().unwrap_or_default());
        let sessions = again.as_deref().unwrap_or(sessions);
        let (caps, mut states) = captures(plan, sessions, procs, endpoints);
        self.moved.retain(|_, at| at.elapsed() < MOVING);
        for (name, state) in states.iter_mut() {
            if self.moved.contains_key(name)
                && matches!(
                    state,
                    AppState::Elsewhere { .. } | AppState::OwnOutput { .. }
                )
            {
                *state = AppState::Idle;
            }
        }
        for (name, message) in errors {
            states.insert(name, AppState::Error { message });
        }
        (caps, states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: &str, default: bool) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: format!("{id} out"),
            default,
        }
    }

    fn proc_(pid: u32, parent: u32, exe: &str) -> Process {
        Process {
            pid,
            parent,
            exe: exe.into(),
        }
    }

    fn sess(pid: u32, endpoint: &str, active: bool) -> Session {
        Session {
            pid,
            endpoint: endpoint.into(),
            active,
        }
    }

    #[test]
    fn the_sink_is_never_the_default_the_output_or_a_recorded_output() {
        let eps = [
            ep("a", true),
            ep("b", false),
            ep("c", false),
            ep("d", false),
        ];
        assert_eq!(choose_sink(&eps, "", "b out", &["c out"]).unwrap().id, "d");
        assert!(choose_sink(&eps, "a out", "b out", &[]).is_err());
        assert!(choose_sink(&eps, "b out", "b out", &[]).is_err());
        assert_eq!(choose_sink(&eps, "c out", "b out", &[]).unwrap().id, "c");
        assert!(choose_sink(&eps[..2], "", "b out", &[]).is_err());
        // Nothing in the output priority is ever muted, by name or by id.
        assert_eq!(choose_sink(&eps, "", "b out", &["c out", "d"]).ok(), None);
        assert!(choose_sink(&eps, "c out", "b out", &["c"]).is_err());
    }

    #[test]
    fn apps_still_playing_elsewhere_are_not_captured_and_parents_never_double_children() {
        let plan = Plan {
            sink: ep("sink", false),
            apps: [
                ("game.exe".into(), Channel::Game),
                ("launcher.exe".into(), Channel::Aux),
                ("chat.exe".into(), Channel::ChatRender),
            ]
            .into(),
        };
        let procs = [
            proc_(10, 1, "Launcher.exe"),
            proc_(11, 10, "Game.exe"),
            proc_(20, 1, "Chat.exe"),
            proc_(21, 20, "Chat.exe"),
            proc_(30, 1, "Music.exe"),
        ];
        let sessions = [
            sess(10, "sink", false),
            sess(11, "sink", true),
            sess(20, "sink", false),
            sess(21, "speakers", true),
            sess(30, "speakers", true),
        ];
        let eps = [ep("speakers", true), ep("sink", false)];
        let (caps, states) = captures(&plan, &sessions, &procs, &eps);
        let own = [ep("speakers", false), ep("sink", false), ep("other", true)];
        assert_eq!(
            captures(&plan, &sessions, &procs, &own).1["chat.exe"],
            AppState::OwnOutput {
                output: "speakers out".into()
            }
        );
        assert_eq!(caps, [(Channel::Game, 11)]);
        assert_eq!(states["game.exe"], AppState::Playing);
        assert_eq!(
            states["launcher.exe"],
            AppState::Contains {
                app: "game.exe".into()
            }
        );
        assert_eq!(
            states["chat.exe"],
            AppState::Elsewhere {
                output: "speakers out".into()
            }
        );
    }

    #[test]
    fn the_picker_lists_each_app_once_without_the_shell() {
        let procs = [
            proc_(5, 1, "explorer.exe"),
            proc_(6, 1, "Chat.exe"),
            proc_(7, 6, "Chat.exe"),
        ];
        let sessions = [
            sess(5, "a", true),
            sess(6, "a", false),
            sess(7, "a", true),
            sess(0, "a", true),
        ];
        assert_eq!(
            running_apps(&sessions, &procs),
            [RunningApp {
                exe: "Chat.exe".into(),
                active: true
            }]
        );
    }

    #[test]
    fn an_empty_journal_leaves_no_file() {
        let dir = std::env::temp_dir().join(format!("foxden-park-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("parked.json");
        fs::write(
            &path,
            r#"{"sink":{"id":"x","was_muted":false},"routes":{"a.exe":{"before":"","to":"x"}}}"#,
        )
        .unwrap();
        let mut p = Parker::new(path.clone());
        assert!(p.dirty());
        assert_eq!(p.journal.routes["a.exe"].to, "x");
        p.journal = Journal::default();
        p.save();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("foxden-park-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("parked.json")
    }

    const GONE: &str = "{0.0.0.00000000}.{00000000-0000-0000-0000-00000000dead}";

    #[test]
    fn a_journal_cut_off_mid_write_loads_without_failing() {
        let path = scratch("cut");
        fs::write(&path, r#"{"sink":{"id":"x","was_mu"#).unwrap();
        let p = Parker::new(path.clone());
        assert!(!p.dirty());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_journal_missing_fields_still_restores_what_it_names() {
        let path = scratch("partial");
        fs::write(&path, r#"{"routes":{"a.exe":{"before":"speakers"}}}"#).unwrap();
        let p = Parker::new(path.clone());
        assert!(p.dirty());
        assert_eq!(p.journal.sink, None);
        assert_eq!(p.journal.routes["a.exe"].before, "speakers");
        assert_eq!(p.journal.routes["a.exe"].to, "");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_parking_output_that_is_gone_is_dropped_and_routes_wait_for_their_app() {
        let path = scratch("gone");
        let journal = format!(
            r#"{{"sink":{{"id":"{GONE}","was_muted":false}},"routes":{{"game.exe":{{"before":"","to":"{GONE}"}}}}}}"#
        );
        fs::write(&path, journal).unwrap();
        let mut p = Parker::new(path.clone());
        assert!(p.dirty());
        let (caps, states) = p.tick_with(None, &[], &[], &[]);
        assert!(caps.is_empty() && states.is_empty());
        assert_eq!(p.journal.sink, None);
        assert!(p.journal.routes.contains_key("game.exe"));
        let saved: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, p.journal);
        assert!(!path.with_extension("tmp").exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_journal_with_only_a_gone_output_is_cleared_after_one_pass() {
        let path = scratch("only-sink");
        fs::write(&path, format!(r#"{{"sink":{{"id":"{GONE}","was_muted":true}}}}"#)).unwrap();
        let mut p = Parker::new(path.clone());
        p.tick_with(None, &[], &[], &[]);
        assert!(!p.dirty());
        assert!(!path.exists());
        // Nothing left to do: later passes return at once.
        assert_eq!(p.tick_with(None, &[], &[], &[]), (Vec::new(), BTreeMap::new()));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
