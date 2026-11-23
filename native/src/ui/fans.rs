use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use foxden_fan::{Curve, FanConfig, FanRule, FanStatus, Kind, MIN_MAX_DUTY, SensorStatus};
use serde_json::Value;
use slint::platform::Key;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tauri::AppHandle;

use super::{
    AppWindow, FanCard, FanChoice, FanChoiceRow, FanCurveItem, FanPoint, FanRow, FanTile, FansPage,
    StagePage, View, fill, with_win,
};

const T_MIN: f32 = 20.0;
const T_MAX: f32 = 100.0;
/// Readings of the last five minutes, one every two seconds.
const HISTORY: usize = 150;
const SOURCES: [&str; 5] = ["CPU", "Mainboard", "Graphics card", "Drives", "Thermal zones"];
const UNSAVED: &str =
    "The changes are in use, but the service could not save them. They will be gone when it starts again.";
const OVERHEAT: &str = "a CPU or GPU is critically hot";
const NOT_ALLOWED: &str = "Only the person signed in at this PC's screen, or an administrator, can change this. From another Windows session FoxDen can only read the fans.";

#[derive(Default)]
struct Live {
    service: String,
    detail: Option<String>,
    version: Option<String>,
    sensors: Vec<SensorStatus>,
    fans: Vec<FanStatus>,
}

#[derive(Clone)]
struct Problem {
    at: &'static str,
    key: String,
    text: String,
    rejected: bool,
}

struct Drag {
    i: usize,
    before: FanConfig,
    x: f32,
    y: f32,
    moved: bool,
}

#[derive(Default)]
struct St {
    app: Option<AppHandle>,
    live: Live,
    /// The service's config.
    saved: FanConfig,
    draft: FanConfig,
    /// What the draft started from.
    base: FanConfig,
    chosen: usize,
    past: Vec<FanConfig>,
    pending: &'static str,
    problem: Option<Problem>,
    note: &'static str,
    drag: Option<Drag>,
    asking: bool,
    typing: Option<String>,
    typed: Option<(String, String)>,
    hl: Option<usize>,
    history: HashMap<String, VecDeque<Option<f32>>>,
    width: f32,
    graph_w: f32,
    focus: Option<String>,
    serial: i32,
    timer: Option<Timer>,
    fan_rows: Vec<Rc<VecModel<FanCard>>>,
    choice_rows: Vec<Rc<VecModel<FanChoice>>>,
}

thread_local! {
    static ST: RefCell<St> = RefCell::default();
}

/// Runs `f` on the state and redraws; waits for a borrow that is still held.
fn update(f: impl FnOnce(&mut St) + 'static) {
    let busy = ST.with(|s| s.try_borrow_mut().is_err());
    if busy {
        Timer::single_shot(Duration::ZERO, move || update(f));
        return;
    }
    ST.with_borrow_mut(|st| {
        f(st);
        with_win(|w| render(st, w));
    });
}

/// Stops the sampling timer once the window is gone; `bind` starts it again.
pub fn stop() {
    let _ = ST.try_with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            st.timer = None;
        }
    });
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(2), || {
        let shown = ST.with_borrow_mut(|st| {
            if st.live.service != "running" {
                return false;
            }
            sample(st);
            true
        });
        if shown {
            let mut tab = -1;
            with_win(|w| tab = w.get_tab());
            if tab == 2 {
                update(|_| {});
            }
        }
    });
    ST.with_borrow_mut(|st| {
        st.app = Some(app.clone());
        st.timer = Some(timer);
        st.fan_rows.clear();
        st.choice_rows.clear();
    });

    let g = w.global::<FansPage>();
    g.on_resized(|width| update(move |st| st.width = width));
    g.on_graph_size(|width| update(move |st| st.graph_w = width));
    g.on_service_action(|action| {
        let action = if action == "start" { "start" } else { "stop" };
        service(action);
    });
    g.on_pick_curve(|i| {
        update(move |st| {
            st.chosen = i.max(0) as usize;
            st.asking = false;
            st.problem = None;
            st.focus = Some(format!("pick:{i}"));
        })
    });
    g.on_add_curve(|| update(|st| add_curve(st, None)));
    g.on_copy_curve(|| {
        update(|st| {
            let from = st.draft.curves.get(st.chosen).cloned();
            if from.is_some() {
                add_curve(st, from);
            }
        })
    });
    g.on_ask_delete(|| {
        update(|st| {
            if st.draft.curves.get(st.chosen).is_some() {
                st.asking = true;
                st.focus = Some("ask-no".into());
            }
        })
    });
    g.on_confirm_delete(|| update(delete_curve));
    g.on_cancel_delete(|| {
        update(|st| {
            st.asking = false;
            st.focus = Some("delete-curve".into());
        })
    });
    g.on_commit(|key, text| update(move |st| commit(st, &key, &text)));
    g.on_typed(|key, text| {
        ST.with_borrow_mut(|st| st.typed = Some((key.to_string(), text.to_string())));
    });
    g.on_typing(|key, on| {
        update(move |st| {
            let row = key
                .strip_prefix("temp:")
                .or_else(|| key.strip_prefix("duty:"))
                .and_then(|i| i.parse().ok());
            if on {
                st.typing = Some(key.to_string());
                st.typed = None;
                if row.is_some() {
                    st.hl = row;
                }
            } else if st.typing.as_deref() == Some(key.as_str()) {
                st.typing = None;
                st.typed = None;
                if row.is_some() {
                    st.hl = None;
                }
                // A config that arrived while the field had focus is taken up now.
                sync(st);
            }
        })
    });
    g.on_add_point(|| update(|st| add_point(st, None, false)));
    g.on_remove_point(|i| update(move |st| remove_point(st, i.max(0) as usize, false)));
    g.on_point_key(|i, key, shift| {
        let mut done = false;
        update_now(|st| done = point_key(st, i.max(0) as usize, &key, shift));
        done
    });
    g.on_point_focus(|i, on| {
        update(move |st| st.hl = if on { Some(i.max(0) as usize) } else { None })
    });
    g.on_graph_down(|x, y| update(move |st| graph_down(st, x, y)));
    g.on_graph_move(|x, y| update(move |st| graph_move(st, x, y)));
    g.on_graph_up(|| update(drop_point));
    g.on_graph_hover(|x, y| {
        let cursor = ST.with_borrow(|st| {
            let plot = Plot::of(st.graph_w);
            match st.draft.curves.get(st.chosen) {
                Some(c) if handle_at(c, &plot, x, y).is_some() => 1,
                Some(c) if near_line(c, &plot, x, y) => 2,
                _ => 0,
            }
        });
        with_win(|w| w.global::<FansPage>().set_cursor(cursor));
    });
    g.on_toggle_sensor(|id| update(move |st| toggle_sensor(st, &id)));
    g.on_card_curve(|fan, i| update(move |st| card_curve(st, &fan, i)));
    g.on_apply(|| update(send));
    g.on_revert(|| update(revert));
    g.on_undo(|| update(undo));
    g.on_save_key(|| update(save_key));
    g.on_undo_key(|| {
        update(|st| {
            if st.past.is_empty() {
                tell(st, "No fan edit to undo");
            }
            undo(st);
        })
    });
}

/// Like `update`, for callbacks that return a value at once.
fn update_now(f: impl FnOnce(&mut St)) {
    ST.with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            f(&mut st);
            with_win(|w| render(&mut st, w));
        }
    });
}

pub fn event(w: &AppWindow, _app: &AppHandle, view: &mut View, name: &str) {
    if name != "fans" {
        return;
    }
    let payload = view.events.get("fans").cloned().unwrap_or_default();
    ST.with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            take(&mut st, &payload);
            render(&mut st, w);
        }
    });
}

pub fn apply(w: &AppWindow, _app: &AppHandle, _view: &mut View) {
    ST.with(|s| {
        if let Ok(mut st) = s.try_borrow_mut() {
            render(&mut st, w);
        }
    });
}

fn tr(st: &St, text: &str) -> String {
    st.app.as_ref().map_or_else(|| text.to_string(), |a| super::tr(a, text))
}

/// A translated template with its parts filled in, each translated where it can be.
fn tf(st: &St, template: &str, args: &[&str]) -> String {
    let mut out = tr(st, template);
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &tr(st, a));
    }
    out
}

fn tell(st: &St, text: &str) {
    let text = tr(st, text);
    with_win(|w| w.global::<StagePage>().set_note(text.into()));
}

fn tidy(config: &FanConfig) -> FanConfig {
    let mut c = config.clone();
    c.rules.sort_by(|a, b| a.fan.cmp(&b.fan));
    c
}

fn trim(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

fn percent(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i64)
}

fn reading(st: &St, v: Option<f32>, kind: Kind) -> String {
    match v {
        None => tr(st, "no reading"),
        Some(v) if kind == Kind::Power => format!("{} W", v.round() as i64),
        Some(v) => format!("{v:.1} °C"),
    }
}

fn speed(st: &St, rpm: Option<u32>) -> String {
    let Some(r) = rpm else {
        return tr(st, "no speed signal");
    };
    let digits = r.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    tf(st, "{0} rpm", &[&grouped])
}

fn pump(name: &str) -> bool {
    name.split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("pump"))
}

fn source(id: &str) -> &'static str {
    let gpu = id
        .strip_prefix("gpu")
        .map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('/'));
    let disk = id
        .strip_prefix("disk")
        .map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('/'));
    if id.starts_with("cpu/") {
        "CPU"
    } else if id.starts_with("board/") {
        "Mainboard"
    } else if gpu == Some(true) {
        "Graphics card"
    } else if disk == Some(true) {
        "Drives"
    } else if id.starts_with("acpi/") {
        "Thermal zones"
    } else {
        "Other"
    }
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn quoted(name: &str) -> String {
    format!("\"{name}\"")
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn parse(text: &str) -> Option<f32> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

impl St {
    fn running(&self) -> bool {
        self.live.service == "running"
    }

    fn edited(&self) -> bool {
        tidy(&self.draft) != self.base
    }

    fn locked(&self) -> bool {
        self.drag.is_some() || self.typing.is_some()
    }

    fn told(&self, at: &str, key: &str) -> String {
        match &self.problem {
            Some(p) if p.at == at && p.key == key => p.text.clone(),
            _ => String::new(),
        }
    }

    /// Only temperatures can drive a curve.
    fn temps(&self) -> impl Iterator<Item = &SensorStatus> {
        self.live.sensors.iter().filter(|s| s.kind != Kind::Power)
    }

    fn sensor(&self, id: &str) -> Option<&SensorStatus> {
        self.live.sensors.iter().find(|s| s.id == id)
    }

    fn fan_name(&self, id: &str) -> String {
        let name = self
            .live
            .fans
            .iter()
            .find(|f| f.id == id)
            .map_or(id, |f| f.name.as_str());
        tr(self, name)
    }

    /// The sensors of a curve that give no reading, by name.
    fn lost(&self, curve: &Curve) -> Vec<String> {
        curve
            .sensors
            .iter()
            .filter(|id| self.sensor(id).is_none_or(|s| s.value.is_none()))
            .map(|id| tr(self, self.sensor(id).map_or(id.as_str(), |s| s.name.as_str())))
            .collect()
    }

    fn silent(&self, names: &[String]) -> String {
        let verb = if names.len() == 1 { " gives" } else { " give" };
        format!("{}{verb} no reading", list(names))
    }

    fn hottest(&self, curve: &Curve) -> Option<&SensorStatus> {
        self.temps()
            .filter(|s| curve.sensors.contains(&s.id) && s.value.is_some())
            .fold(None, |best: Option<&SensorStatus>, s| match best {
                Some(b) if b.value >= s.value => Some(b),
                _ => Some(s),
            })
    }

    /// What is wrong with a fan right now, in words, or nothing.
    fn trouble(&self, fan: &FanStatus) -> String {
        if let Some(e) = &fan.error {
            if e.starts_with(OVERHEAT) {
                return tf(self, "Fail-safe: full speed, because {0}", &[OVERHEAT]);
            }
            return tf(self, "Error: {0}", &[e]);
        }
        if !fan.failsafe {
            return String::new();
        }
        let curve_name = fan.curve.clone().unwrap_or_default();
        let gone = self
            .saved
            .curves
            .iter()
            .find(|c| c.name == curve_name)
            .map(|c| self.lost(c))
            .unwrap_or_default();
        let why = if gone.is_empty() {
            tf(self, "a sensor of {0} gives no reading", &[&curve_name])
        } else {
            self.silent(&gone)
        };
        tf(self, "Fail-safe: full speed, because {0}", &[&why])
    }

    /// Fans the service reports, then fans that only a rule still names.
    fn rule_fans(&self) -> Vec<(String, String, Option<FanStatus>)> {
        let mut fans: Vec<_> = self
            .live
            .fans
            .iter()
            .map(|f| (f.id.clone(), f.name.clone(), Some(f.clone())))
            .collect();
        for rule in &self.draft.rules {
            if !fans.iter().any(|f| f.0 == rule.fan) {
                fans.push((rule.fan.clone(), rule.fan.clone(), None));
            }
        }
        fans
    }

    fn clamp_chosen(&mut self) {
        self.chosen = self.chosen.min(self.draft.curves.len().saturating_sub(1));
    }
}

fn take(st: &mut St, v: &Value) {
    let text = |v: &Value| v.as_str().map(str::to_string);
    st.live = Live {
        service: v["service"].as_str().unwrap_or_default().to_string(),
        detail: text(&v["detail"]),
        version: text(&v["version"]),
        sensors: serde_json::from_value(v["sensors"].clone()).unwrap_or_default(),
        fans: serde_json::from_value(v["fans"].clone()).unwrap_or_default(),
    };
    // Without a running service the config in the state is only a placeholder.
    if !st.running() {
        st.note = "";
        return;
    }
    st.saved = tidy(&serde_json::from_value(v["config"].clone()).unwrap_or_default());
    sync(st);
}

/// The draft follows the service until the user changes something.
fn sync(st: &mut St) {
    let mine = tidy(&st.draft);
    if mine == st.saved {
        st.base = st.saved.clone();
    } else if mine == st.base && !st.locked() {
        let name = st.draft.curves.get(st.chosen).map(|c| c.name.clone());
        st.draft = st.saved.clone();
        st.base = st.saved.clone();
        st.past.clear();
        st.asking = false;
        match name.and_then(|n| st.draft.curves.iter().position(|c| c.name == n)) {
            Some(i) => st.chosen = i,
            None => st.clamp_chosen(),
        }
    }
}

/// Every change to the draft goes through here so that it can be undone.
fn edit(st: &mut St, f: impl FnOnce(&mut St)) {
    let before = tidy(&st.draft);
    f(st);
    let changed = tidy(&st.draft) != before;
    if changed {
        st.past.push(before);
        if st.past.len() > 100 {
            st.past.remove(0);
        }
        st.note = "";
    }
    // What the service objected to stays on screen until the draft changes.
    if changed || !st.problem.as_ref().is_some_and(|p| p.rejected) {
        st.problem = None;
    }
}

fn undo(st: &mut St) {
    if !st.pending.is_empty() {
        return;
    }
    let Some(last) = st.past.pop() else {
        return;
    };
    st.draft = last;
    st.clamp_chosen();
    st.asking = false;
    st.problem = None;
    st.note = "Undone";
}

fn revert(st: &mut St) {
    if !st.edited() || !st.pending.is_empty() {
        return;
    }
    st.past.push(tidy(&st.draft));
    st.draft = st.saved.clone();
    st.base = st.saved.clone();
    st.clamp_chosen();
    st.asking = false;
    st.problem = None;
    st.note = "Reverted to what the service has";
}

fn problem(st: &mut St, at: &'static str, key: String, text: String) {
    st.problem = Some(Problem { at, key, text, rejected: false });
}

/// Takes a typed number into the draft, within its range, and says so when it was outside it.
fn commit_number(
    st: &mut St,
    text: &str,
    (lo, hi): (f32, f32),
    what: &str,
    at: (&'static str, String),
    put: impl FnOnce(&mut St, f32),
) {
    let v = parse(text);
    if let Some(v) = v {
        edit(st, |st| put(st, v.clamp(lo, hi)));
    }
    if !v.is_some_and(|v| v >= lo && v <= hi) {
        let text = tf(st, "{0} must be a number from {1} to {2}.", &[what, &lo.to_string(), &hi.to_string()]);
        problem(st, at.0, at.1, text);
    }
}

fn commit(st: &mut St, key: &str, text: &str) {
    let chosen = st.chosen;
    let here = chosen.to_string();
    if key == "name" {
        rename(st, text);
    } else if key == "hyst" {
        if chosen >= st.draft.curves.len() {
            return;
        }
        commit_number(st, text, (0.0, 30.0), "Hysteresis", ("hysteresis", here), move |st, v| {
            st.draft.curves[chosen].hysteresis = (v * 10.0).round() / 10.0;
        });
    } else if let Some(i) = key.strip_prefix("temp:").and_then(|i| i.parse::<usize>().ok()) {
        if st.draft.curves.get(chosen).is_none_or(|c| i >= c.points.len()) {
            return;
        }
        commit_number(st, text, (T_MIN, T_MAX), "Temperature", ("points", here.clone()), move |st, v| {
            let curve = &mut st.draft.curves[chosen];
            let duty = curve.points[i].1;
            set_point(curve, i, v, duty);
        });
        let now = st.draft.curves[chosen].points[i].0;
        if let Some(typed) = parse(text).filter(|v| (T_MIN..=T_MAX).contains(v))
            && (now - typed).abs() > 0.05
        {
            let text = tf(
                st,
                "Point {0} stays between its neighbours, so it is at {1} °C.",
                &[&(i + 1).to_string(), &trim(now)],
            );
            problem(st, "points", here, text);
        }
    } else if let Some(i) = key.strip_prefix("duty:").and_then(|i| i.parse::<usize>().ok()) {
        if st.draft.curves.get(chosen).is_none_or(|c| i >= c.points.len()) {
            return;
        }
        commit_number(st, text, (0.0, 100.0), "Duty", ("points", here), move |st, v| {
            let curve = &mut st.draft.curves[chosen];
            let temp = curve.points[i].0;
            set_point(curve, i, temp, v / 100.0);
        });
    } else {
        let floor = MIN_MAX_DUTY * 100.0;
        for (prefix, lo, what) in [("min:", 0.0, "Minimum duty"), ("max:", floor, "Maximum duty"), ("ramp:", 1.0, "Ramp")] {
            let Some(fan) = key.strip_prefix(prefix) else {
                continue;
            };
            let Some(r) = st.draft.rules.iter().position(|r| r.fan == fan) else {
                return;
            };
            commit_number(st, text, (lo, 100.0), what, ("rule", fan.to_string()), move |st, v| {
                let v = (v * 10.0).round() / 1000.0;
                let rule = &mut st.draft.rules[r];
                match prefix {
                    "min:" => rule.min_duty = v.min(rule.max_duty),
                    "max:" => {
                        rule.max_duty = v;
                        rule.min_duty = rule.min_duty.min(v);
                    }
                    _ => rule.ramp = v,
                }
            });
            if prefix == "min:"
                && let Some(typed) = parse(text).filter(|v| (0.0..=100.0).contains(v))
                && typed / 100.0 > st.draft.rules[r].max_duty + 0.0005
            {
                let text = tf(st, "{0}: the minimum must not be above the maximum.", &[&st.fan_name(fan)]);
                problem(st, "rule", fan.to_string(), text);
            }
            return;
        }
    }
}

fn duty_at(points: &[(f32, f32)], temp: f32) -> f32 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    if temp <= first.0 {
        return first.1;
    }
    for w in points.windows(2) {
        let ((t0, d0), (t1, d1)) = (w[0], w[1]);
        if temp <= t1 {
            return d0 + (d1 - d0) * (temp - t0) / (t1 - t0);
        }
    }
    points[points.len() - 1].1
}

/// A point stays between its neighbours, so the order never changes.
fn set_point(curve: &mut Curve, i: usize, temp: f32, duty: f32) {
    let points = &mut curve.points;
    let lo = if i > 0 { points[i - 1].0 + 1.0 } else { T_MIN };
    let hi = if i + 1 < points.len() { points[i + 1].0 - 1.0 } else { T_MAX };
    if lo <= hi {
        points[i].0 = ((temp * 10.0).round() / 10.0).clamp(lo, hi);
    }
    points[i].1 = ((duty * 100.0).round() / 100.0).clamp(0.0, 1.0);
}

/// The middle of the widest gap, or nothing when the curve is full.
fn room_for(points: &[(f32, f32)]) -> Option<f32> {
    let mut temps = vec![T_MIN - 1.0];
    temps.extend(points.iter().map(|p| p.0));
    temps.push(T_MAX + 1.0);
    let mut at = None;
    let mut widest = 1.99;
    for w in temps.windows(2) {
        if w[1] - w[0] > widest {
            widest = w[1] - w[0];
            at = Some(((w[1] + w[0]) / 2.0).round());
        }
    }
    at
}

fn add_point(st: &mut St, temp: Option<f32>, graph: bool) {
    let chosen = st.chosen;
    let Some(curve) = st.draft.curves.get(chosen) else {
        return;
    };
    let Some(t) = temp.or_else(|| room_for(&curve.points)) else {
        return;
    };
    let t = t.round().clamp(T_MIN, T_MAX);
    if curve.points.iter().any(|p| (p.0 - t).abs() < 1.0) {
        let text = tf(st, "There is already a point at {0} °C.", &[&t.to_string()]);
        problem(st, "points", chosen.to_string(), text);
        return;
    }
    let duty = (duty_at(&curve.points, t) * 100.0).round() / 100.0;
    edit(st, |st| {
        let points = &mut st.draft.curves[chosen].points;
        points.push((t, duty));
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
    });
    let i = st.draft.curves[chosen].points.iter().position(|p| p.0 == t).unwrap_or(0);
    st.focus = Some(if graph { format!("handle:{i}") } else { format!("temp:{i}") });
}

fn remove_point(st: &mut St, i: usize, graph: bool) {
    let chosen = st.chosen;
    let Some(curve) = st.draft.curves.get(chosen) else {
        return;
    };
    if curve.points.len() < 2 {
        let text = tr(st, "A curve needs at least one point.");
        problem(st, "points", chosen.to_string(), text);
        return;
    }
    if i >= curve.points.len() {
        return;
    }
    edit(st, |st| {
        st.draft.curves[chosen].points.remove(i);
    });
    let next = i.min(st.draft.curves[chosen].points.len() - 1);
    st.focus = Some(if graph { format!("handle:{next}") } else { format!("remove:{next}") });
}

fn free_name(st: &St, stem: &str, bare: bool) -> String {
    let taken = |name: &str| st.draft.curves.iter().any(|c| c.name == name);
    if bare && !taken(stem) {
        return stem.to_string();
    }
    (if bare { 2 } else { 1 }..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

fn add_curve(st: &mut St, from: Option<Curve>) {
    let sensor = st.temps().find(|s| s.value.is_some()).map(|s| s.id.clone());
    let curve = match from {
        Some(from) => {
            let stem: String = from.name.chars().take(32).collect();
            Curve {
                name: free_name(st, &format!("{stem} copy"), true),
                ..from
            }
        }
        None => Curve {
            name: free_name(st, "Curve", false),
            sensors: sensor.into_iter().collect(),
            points: vec![(30.0, 0.25), (50.0, 0.4), (70.0, 0.75), (85.0, 1.0)],
            hysteresis: 2.0,
        },
    };
    edit(st, |st| {
        st.draft.curves.push(curve);
        st.chosen = st.draft.curves.len() - 1;
    });
    st.focus = Some("name".into());
}

fn delete_curve(st: &mut St) {
    st.asking = false;
    if st.chosen >= st.draft.curves.len() {
        return;
    }
    edit(st, |st| {
        let gone = st.draft.curves.remove(st.chosen);
        st.draft.rules.retain(|r| r.curve != gone.name);
        st.clamp_chosen();
    });
    st.focus = Some(if st.draft.curves.is_empty() { "add-curve" } else { "delete-curve" }.into());
}

fn rename(st: &mut St, text: &str) {
    let chosen = st.chosen;
    let Some(curve) = st.draft.curves.get(chosen) else {
        return;
    };
    let old = curve.name.clone();
    let name: String = text.trim().chars().take(40).collect();
    let wrong = if name.is_empty() {
        tr(st, "A curve needs a name.")
    } else if st.draft.curves.iter().enumerate().any(|(i, c)| i != chosen && c.name == name) {
        tf(st, "Another curve is already called {0}.", &[&name])
    } else {
        String::new()
    };
    if !wrong.is_empty() {
        problem(st, "name", chosen.to_string(), wrong);
        return;
    }
    edit(st, |st| {
        for rule in st.draft.rules.iter_mut().filter(|r| r.curve == old) {
            rule.curve = name.clone();
        }
        st.draft.curves[chosen].name = name;
    });
}

fn toggle_sensor(st: &mut St, id: &str) {
    let chosen = st.chosen;
    if chosen >= st.draft.curves.len() {
        return;
    }
    edit(st, |st| {
        let curve = &mut st.draft.curves[chosen];
        let on = !curve.sensors.iter().any(|s| s == id);
        curve.sensors.retain(|s| s != id);
        if on {
            curve.sensors.push(id.to_string());
        }
        // Back in the saved order when the same sensors are chosen again.
        if let Some(was) = st.saved.curves.iter().find(|c| c.name == curve.name)
            && was.sensors.len() == curve.sensors.len()
            && was.sensors.iter().all(|s| curve.sensors.contains(s))
        {
            curve.sensors = was.sensors.clone();
        }
    });
}

fn card_curve(st: &mut St, fan: &str, i: i32) {
    let name = (i > 0)
        .then(|| st.draft.curves.get(i as usize - 1).map(|c| c.name.clone()))
        .flatten();
    edit(st, |st| {
        let at = st.draft.rules.iter().position(|r| r.fan == fan);
        match (at, name) {
            (Some(r), Some(name)) => st.draft.rules[r].curve = name,
            (Some(r), None) => {
                st.draft.rules.remove(r);
            }
            (None, Some(name)) => st.draft.rules.push(FanRule {
                fan: fan.to_string(),
                curve: name,
                min_duty: 0.0,
                max_duty: 1.0,
                ramp: 0.2,
            }),
            (None, None) => {}
        }
    });
}

fn service(action: &'static str) {
    update(move |st| {
        st.pending = action;
        st.problem = None;
    });
    crate::state::spawn_named("foxden-fan-service", move || {
        let done = if action == "start" {
            crate::fans::start_service()
        } else {
            crate::fans::stop_service()
        };
        let state = done
            .is_ok()
            .then(crate::fans::get_fans)
            .and_then(Result::ok)
            .and_then(|s| serde_json::to_value(s).ok());
        let _ = slint::invoke_from_event_loop(move || {
            update(move |st| {
                if let Some(state) = &state {
                    take(st, state);
                }
                if let Err(e) = done {
                    let text = if super::refused(&e) {
                        tr(st, NOT_ALLOWED)
                    } else {
                        tr(st, &sentence(&e))
                    };
                    problem(st, "service", String::new(), text);
                }
                st.pending = "";
                let state = st.live.service.as_str();
                if action == "start" && (state.is_empty() || state == "running") {
                    st.focus = Some("stop".into());
                } else if action == "stop" && (state.is_empty() || state == "stopped") {
                    st.focus = Some("start".into());
                }
            })
        });
    });
}

/// What the service would refuse in the draft, with the control that puts it right.
fn fault(st: &St) -> Option<(Option<usize>, String, String)> {
    if let Some(bare) = st.draft.curves.iter().position(|c| c.sensors.is_empty()) {
        let text = tf(
            st,
            "Curve {0} has no sensor. Choose at least one.",
            &[&quoted(&st.draft.curves[bare].name)],
        );
        let target = if st.temps().next().is_some() { "sensor:0" } else { "name" };
        return Some((Some(bare), text, target.into()));
    }
    let rule = st.draft.rules.iter().find(|r| r.min_duty > r.max_duty)?;
    let text = tf(st, "{0}: the minimum must not be above the maximum.", &[&st.fan_name(&rule.fan)]);
    Some((None, text, format!("min:{}", rule.fan)))
}

fn send(st: &mut St) {
    if !st.edited() || !st.pending.is_empty() || !st.running() {
        return;
    }
    // Nothing is sent while the draft has a fault this tab can see. The key or button leads to it instead.
    if let Some((curve, text, target)) = fault(st) {
        if let Some(c) = curve {
            st.chosen = c;
        }
        let text = tf(st, "Not applied. {0}", &[&text]);
        st.problem = Some(Problem { at: "apply", key: String::new(), text, rejected: true });
        st.focus = Some(target);
        return;
    }
    let sent = tidy(&st.draft);
    apply_config(st, sent, None);
}

/// A config the person approved on screen, applied like Apply. The draft it replaces stays for Undo.
pub fn apply_proposed(config: FanConfig, reply: Sender<Result<(), String>>) {
    update(move |st| {
        if !st.pending.is_empty() {
            let _ = reply.send(Err(crate::fan_proposal::TAB_BUSY.into()));
            return;
        }
        let new = tidy(&config);
        let before = tidy(&st.draft);
        if before != new {
            st.past.push(before);
            if st.past.len() > 100 {
                st.past.remove(0);
            }
        }
        st.draft = new.clone();
        st.clamp_chosen();
        st.asking = false;
        apply_config(st, new, Some(reply));
    });
}

/// Sends `sent` to the service, then shows how that went.
fn apply_config(st: &mut St, sent: FanConfig, reply: Option<Sender<Result<(), String>>>) {
    st.pending = "apply";
    st.problem = None;
    let config = sent.clone();
    crate::state::spawn_named("foxden-fan-apply", move || {
        let done = crate::fans::set_fan_config(config);
        if let Some(reply) = reply {
            let _ = reply.send(done.clone());
        }
        let state = done
            .is_ok()
            .then(crate::fans::get_fans)
            .and_then(Result::ok)
            .and_then(|s| serde_json::to_value(s).ok());
        let _ = slint::invoke_from_event_loop(move || {
            update(move |st| {
                if let Err(e) = done {
                    st.pending = "";
                    reject(st, &e);
                    return;
                }
                if tidy(&st.draft) == sent {
                    st.base = sent;
                }
                st.note = "Applied";
                if let Some(state) = &state {
                    take(st, state);
                }
                st.pending = "";
            })
        });
    });
}

/// The service's reasons for refusing a config, in the words and units of this tab.
fn plain(st: &St, text: &str) -> String {
    // The one refusal that comes after the config has taken effect.
    if text.starts_with("config applied") {
        return tr(st, UNSAVED);
    }
    let between = |pre: &str, post: &str| text.strip_prefix(pre)?.strip_suffix(post).map(str::to_string);
    let curve = |post: &str| between("curve '", post).map(|c| quoted(&c));
    let said = if let Some(c) = between("curve name '", "' is used more than once") {
        Some(tf(st, "Two curves are called {0}. Give one of them another name.", &[&quoted(&c)]))
    } else if let Some(c) = curve("' has no sensors") {
        Some(tf(st, "Curve {0} has no sensor. Choose at least one.", &[&c]))
    } else if let Some(c) = curve("' has no points") {
        Some(tf(st, "Curve {0} has no points. Add at least one.", &[&c]))
    } else if let Some(c) = curve("' has a point that is not a finite number") {
        Some(tf(st, "A point of curve {0} is not a number.", &[&c]))
    } else if let Some(c) = curve("' has a duty outside 0 to 1") {
        Some(tf(st, "A duty of curve {0} is outside 0 to 100%.", &[&c]))
    } else if let Some(c) = curve("' points must be sorted by rising temperature") {
        Some(tf(st, "Each point of curve {0} must be hotter than the one before it.", &[&c]))
    } else if let Some(c) = curve("' hysteresis must be from 0 to 30") {
        Some(tf(st, "The hysteresis of curve {0} must be from 0 to 30 °C.", &[&c]))
    } else if let Some(f) = between("fan '", "' has more than one rule") {
        Some(tf(st, "{0} has more than one rule.", &[&st.fan_name(&f)]))
    } else if let Some((f, c)) = between("fan '", "'").and_then(|m| {
        let (f, c) = m.rsplit_once("' uses unknown curve '")?;
        Some((f.to_string(), c.to_string()))
    }) {
        Some(tf(st, "{0} follows {1}, which is not among the curves.", &[&st.fan_name(&f), &quoted(&c)]))
    } else if let Some(f) = text
        .strip_prefix("fan '")
        .and_then(|m| m.rfind("' needs duties from 0 to 1, ").map(|end| &m[..end]))
    {
        Some(tf(
            st,
            "{0}: the minimum must not be above the maximum, and both must be from 0 to 100%.",
            &[&st.fan_name(f)],
        ))
    } else if let Some(f) = fan_before(text, "' needs a maximum duty of at least ") {
        Some(tf(st, "{0}: the maximum duty must be at least {1}%.", &[&st.fan_name(f), &trim(MIN_MAX_DUTY * 100.0)]))
    } else if let Some(f) = fan_before(text, "' would never run above ") {
        Some(tf(
            st,
            "{0}: the curve or the minimum must reach at least {1}%.",
            &[&st.fan_name(f), &trim(MIN_MAX_DUTY * 100.0)],
        ))
    } else if let Some(f) = fan_before(text, "' ramp must be ") {
        Some(tf(st, "{0}: the ramp must be at least 1% per second.", &[&st.fan_name(f)]))
    } else if super::refused(text) {
        Some(tr(st, NOT_ALLOWED))
    } else {
        None
    };
    match said {
        Some(said) => tf(st, "Not applied. {0}", &[&said]),
        None => tf(st, "Not applied. {0}.", &[&sentence(text)]),
    }
}

fn fan_before<'a>(text: &'a str, rest: &str) -> Option<&'a str> {
    let m = text.strip_prefix("fan '")?;
    m.rfind(rest).map(|end| &m[..end])
}

/// Puts the service's objection beside the control it is about and goes there.
fn reject(st: &mut St, text: &str) {
    let said = plain(st, text);
    let longest = |a: &String, b: &String| a.len().cmp(&b.len());
    let fan = st
        .rule_fans()
        .into_iter()
        .map(|f| f.0)
        .filter(|id| text.contains(&format!("fan '{id}'")))
        .max_by(longest);
    let name = st
        .draft
        .curves
        .iter()
        .map(|c| c.name.clone())
        .filter(|n| text.contains(&format!("curve '{n}'")) || text.contains(&format!("curve name '{n}'")))
        .max_by(longest);
    // Read without the name, so that a curve called "CPU sensor" is not taken for a sensor problem.
    let named = fan.as_ref().or(name.as_ref()).cloned().unwrap_or_default();
    let rest = text.replacen(&format!("'{named}'"), "", 1);
    let mut target = "apply".to_string();
    if let Some(fan) = fan {
        target = if rest.contains("ramp") {
            format!("ramp:{fan}")
        } else if rest.contains("maximum duty") {
            format!("max:{fan}")
        } else if rest.contains("duty") {
            format!("min:{fan}")
        } else {
            format!("curve:{fan}")
        };
        st.problem = Some(Problem { at: "rule", key: fan, text: said, rejected: true });
    } else if let Some(name) = name {
        st.chosen = st.draft.curves.iter().rposition(|c| c.name == name).unwrap_or(0);
        let (at, to) = if rest.contains("hysteresis") {
            ("hysteresis", "hyst")
        } else if rest.contains("sensor") {
            ("sensors", if st.temps().next().is_some() { "sensor:0" } else { "name" })
        } else if rest.contains("curve name") {
            ("name", "name")
        } else {
            ("points", "temp:0")
        };
        target = to.into();
        st.problem = Some(Problem { at, key: st.chosen.to_string(), text: said, rejected: true });
    } else {
        st.problem = Some(Problem { at: "apply", key: String::new(), text: said, rejected: true });
    }
    st.focus = Some(target);
}

/// Ctrl+S: what is being typed goes in first. When it could not be taken as typed, nothing is sent.
fn save_key(st: &mut St) {
    if let (Some(key), Some((typed_key, text))) = (st.typing.clone(), st.typed.take())
        && key == typed_key
    {
        st.problem = None;
        commit(st, &key, &text);
        st.focus = Some(key);
        if st.problem.is_some() {
            return;
        }
    }
    if !st.running() {
        tell(st, if st.edited() { "Start the service to apply the fan changes" } else { "No fan changes to apply" });
    } else if !st.edited() {
        tell(st, "No fan changes to apply");
    }
    send(st);
}

fn sample(st: &mut St) {
    let mut add = |id: String, v: Option<f32>| {
        let h = st.history.entry(id).or_default();
        h.push_back(v);
        if h.len() > HISTORY {
            h.pop_front();
        }
    };
    for s in &st.live.sensors {
        add(format!("s:{}", s.id), s.value);
    }
    for f in &st.live.fans {
        add(format!("f:{}", f.id), f.rpm.map(|r| r as f32));
    }
}

const SPARK_W: u32 = 300;
const SPARK_H: u32 = 64;

/// A sparkline as an image: Slint paths left stray pixels on partial repaints.
fn spark(h: Option<&VecDeque<Option<f32>>>, span: f32, ink: [u8; 3]) -> slint::Image {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let light = super::light_theme();
    let bg = if light { [0xf3, 0xf3, 0xf5] } else { [0x1f, 0x20, 0x23] };
    let Some(mut pm) = Pixmap::new(SPARK_W, SPARK_H) else {
        return slint::Image::default();
    };
    pm.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
    let got: Vec<f32> = h.into_iter().flatten().flatten().copied().collect();
    if let (Some(h), true) = (h, got.len() >= 2) {
        let mut lo = got.iter().copied().fold(f32::INFINITY, f32::min);
        let mut hi = got.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if hi - lo < span {
            let mid = (hi + lo) / 2.0;
            lo = mid - span / 2.0;
            hi = mid + span / 2.0;
        }
        let (sx, sy) = (SPARK_W as f32 / HISTORY as f32, SPARK_H as f32 / 32.0);
        let step = HISTORY as f32 / (h.len() - 1).max(1) as f32;
        let mix = |a: u8, b: u8, f: f32| (a as f32 * f + b as f32 * (1.0 - f)).round() as u8;
        let mut fill = Paint::default();
        fill.set_color_rgba8(mix(ink[0], bg[0], 0.22), mix(ink[1], bg[1], 0.22), mix(ink[2], bg[2], 0.22), 255);
        fill.anti_alias = true;
        let mut pen = Paint::default();
        pen.set_color_rgba8(ink[0], ink[1], ink[2], 255);
        pen.anti_alias = true;
        let stroke = Stroke { width: 3.0, line_join: tiny_skia::LineJoin::Round, ..Stroke::default() };
        let mut run: Vec<(f32, f32)> = Vec::new();
        let draw = |run: &mut Vec<(f32, f32)>, pm: &mut Pixmap| {
            if run.len() > 1 {
                let mut area = PathBuilder::new();
                let mut line = PathBuilder::new();
                for (i, &(x, y)) in run.iter().enumerate() {
                    if i == 0 {
                        area.move_to(x, y);
                        line.move_to(x, y);
                    } else {
                        area.line_to(x, y);
                        line.line_to(x, y);
                    }
                }
                area.line_to(run[run.len() - 1].0, SPARK_H as f32);
                area.line_to(run[0].0, SPARK_H as f32);
                area.close();
                if let Some(p) = area.finish() {
                    pm.fill_path(&p, &fill, FillRule::Winding, Transform::identity(), None);
                }
                if let Some(p) = line.finish() {
                    pm.stroke_path(&p, &pen, &stroke, Transform::identity(), None);
                }
            }
            run.clear();
        };
        for (i, v) in h.iter().enumerate() {
            match v {
                Some(v) => run.push((i as f32 * step * sx, (30.0 - (v - lo) / (hi - lo) * 26.0) * sy)),
                None => draw(&mut run, &mut pm),
            }
        }
        draw(&mut run, &mut pm);
    }
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(pm.data(), SPARK_W, SPARK_H);
    slint::Image::from_rgba8_premultiplied(buf)
}

struct Plot {
    w: f32,
    h: f32,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

impl Plot {
    fn of(w: f32) -> Plot {
        let h = (w * 0.8).round().clamp(200.0, 320.0);
        Plot { w, h, x0: 38.0, x1: w - 14.0, y0: 12.0, y1: h - 24.0 }
    }
    fn px(&self, temp: f32) -> f32 {
        self.x0 + (temp.clamp(T_MIN, T_MAX) - T_MIN) / (T_MAX - T_MIN) * (self.x1 - self.x0)
    }
    fn py(&self, duty: f32) -> f32 {
        self.y1 - duty.clamp(0.0, 1.0) * (self.y1 - self.y0)
    }
    /// A position in the graph as temperature and duty.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let temp = T_MIN + (x - self.x0) / (self.x1 - self.x0) * (T_MAX - T_MIN);
        (temp, (self.y1 - y) / (self.y1 - self.y0))
    }
    fn line(&self, points: &[(f32, f32)]) -> Vec<(f32, f32)> {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return Vec::new();
        };
        let mut out = vec![(self.x0, self.py(first.1))];
        out.extend(points.iter().map(|p| (self.px(p.0), self.py(p.1))));
        out.push((self.x1, self.py(last.1)));
        out
    }
}

fn handle_at(curve: &Curve, plot: &Plot, x: f32, y: f32) -> Option<usize> {
    curve
        .points
        .iter()
        .position(|p| (plot.px(p.0) - x).abs() <= 12.0 && (plot.py(p.1) - y).abs() <= 12.0)
}

fn near_line(curve: &Curve, plot: &Plot, x: f32, y: f32) -> bool {
    plot.line(&curve.points).windows(2).any(|s| {
        let ((ax, ay), (bx, by)) = (s[0], s[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len = dx * dx + dy * dy;
        let t = if len > 0.0 { (((x - ax) * dx + (y - ay) * dy) / len).clamp(0.0, 1.0) } else { 0.0 };
        let (cx, cy) = (ax + t * dx, ay + t * dy);
        (x - cx).hypot(y - cy) <= 8.0
    })
}

fn graph_down(st: &mut St, x: f32, y: f32) {
    let plot = Plot::of(st.graph_w);
    let Some(curve) = st.draft.curves.get(st.chosen) else {
        return;
    };
    if let Some(i) = handle_at(curve, &plot, x, y) {
        st.drag = Some(Drag { i, before: tidy(&st.draft), x, y, moved: false });
        st.focus = Some(format!("handle:{i}"));
    } else if near_line(curve, &plot, x, y) {
        add_point(st, Some(plot.at(x, y).0), true);
    }
}

fn graph_move(st: &mut St, x: f32, y: f32) {
    let plot = Plot::of(st.graph_w);
    let chosen = st.chosen;
    let Some(drag) = st.drag.as_mut() else {
        return;
    };
    if !drag.moved && (x - drag.x).hypot(y - drag.y) < 3.0 {
        return;
    }
    drag.moved = true;
    let i = drag.i;
    let (temp, duty) = plot.at(x, y);
    if let Some(curve) = st.draft.curves.get_mut(chosen)
        && i < curve.points.len()
    {
        set_point(curve, i, temp.round(), duty);
    }
}

fn drop_point(st: &mut St) {
    let Some(drag) = st.drag.take() else {
        return;
    };
    if tidy(&st.draft) != drag.before {
        st.past.push(drag.before);
        st.problem = None;
        st.note = "";
    }
    sync(st);
}

fn point_key(st: &mut St, i: usize, key: &str, shift: bool) -> bool {
    let chosen = st.chosen;
    let Some(curve) = st.draft.curves.get(chosen) else {
        return false;
    };
    if i >= curve.points.len() {
        return false;
    }
    let is = |k: Key| key == SharedString::from(k).as_str();
    if is(Key::Escape) {
        let Some(drag) = st.drag.take() else {
            return false;
        };
        st.draft = drag.before;
        return true;
    }
    if is(Key::Delete) || is(Key::Backspace) {
        remove_point(st, i, true);
        return true;
    }
    let step = if shift { 5.0 } else { 1.0 };
    let (dt, dd) = if is(Key::LeftArrow) {
        (-step, 0.0)
    } else if is(Key::RightArrow) {
        (step, 0.0)
    } else if is(Key::UpArrow) {
        (0.0, step)
    } else if is(Key::DownArrow) {
        (0.0, -step)
    } else {
        return false;
    };
    let (t, d) = curve.points[i];
    edit(st, |st| set_point(&mut st.draft.curves[chosen], i, t + dt, d + dd / 100.0));
    true
}

/// Rows of `cols` items, each row keeping its own model so focused fields in it survive.
fn rows<T: Clone + PartialEq + 'static>(
    keep: &mut Vec<Rc<VecModel<T>>>,
    items: Vec<T>,
    cols: usize,
) -> Vec<(ModelRc<T>, i32)> {
    let chunks: Vec<Vec<T>> = items.chunks(cols.max(1)).map(<[T]>::to_vec).collect();
    keep.truncate(chunks.len());
    while keep.len() < chunks.len() {
        keep.push(Rc::new(VecModel::default()));
    }
    let mut out = Vec::new();
    for (model, chunk) in keep.iter().zip(chunks) {
        let pad = (cols - chunk.len()) as i32;
        let rc = ModelRc::from(model.clone());
        fill(rc.clone(), chunk, |_| {});
        out.push((rc, pad));
    }
    out
}

fn columns(width: f32, min: f32, gap: f32) -> (usize, f32) {
    let cols = (((width + gap) / (min + gap)).floor() as usize).max(1);
    (cols, ((width - (cols - 1) as f32 * gap) / cols as f32).max(0.0))
}

fn render(st: &mut St, w: &AppWindow) {
    let g = w.global::<FansPage>();
    let running = st.running();
    let edited = st.edited();
    let open = running || edited;
    let state = st.live.service.clone();
    let has_state = !state.is_empty();

    let word = match (st.pending, state.as_str()) {
        ("start", _) => "Starting",
        ("stop", _) => "Stopping",
        (_, "running") => "Running",
        (_, "stopped") => "Stopped",
        (_, "error") => "Error",
        _ => "Checking",
    };
    g.set_service(state.clone().into());
    g.set_word(tr(st, word).into());
    g.set_version(st.live.version.as_ref().map(|v| tf(st, "version {0}", &[v])).unwrap_or_default().into());
    let detail = match state.as_str() {
        "stopped" => tr(st, "Fan control runs in the FoxDen background service, which is not running."),
        "error" => match &st.live.detail {
            Some(d) => tr(st, &sentence(d)),
            None => tr(st, "The service did not answer as expected."),
        },
        _ => String::new(),
    };
    g.set_detail(detail.into());
    g.set_show_start(has_state && !running);
    g.set_show_stop(has_state && state != "stopped");
    g.set_busy(!st.pending.is_empty());
    g.set_service_problem(st.told("service", "").into());
    g.set_show_notice(has_state && !(running && !st.live.fans.is_empty()));
    g.set_show_offline(has_state && !running && !edited);
    g.set_open(open);
    g.set_running(running);

    // One column, then two, then three as the panel widens.
    let page = (st.width - 32.0).max(0.0);
    let cols = if page >= 1440.0 { 3 } else if page >= 1000.0 { 2 } else { 1 };
    let (sensors_w, fans_w, curves_w) = match cols {
        3 => {
            let third = (page - 32.0) / 3.0;
            (third, page - third - 16.0, page)
        }
        2 => (page, (page - 16.0) / 2.0, (page - 16.0) / 2.0),
        _ => (page, page, page),
    };
    g.set_columns(cols);
    g.set_sensors_w(sensors_w);
    g.set_fans_w(fans_w);
    g.set_curves_w(curves_w);

    render_sensors(st, w, sensors_w - 26.0);
    render_fans(st, w, fans_w - 26.0);
    render_curves(st, w, curves_w - 26.0);

    let dirty = edited;
    let text = if st.pending == "apply" {
        tr(st, "Applying")
    } else if let Some(p) = st.problem.as_ref().filter(|p| p.at == "apply") {
        p.text.clone()
    } else if st.problem.as_ref().is_some_and(|p| p.rejected) {
        tr(st, "Not applied")
    } else if dirty && !running {
        tr(st, "Unsaved changes. Start the service to apply them.")
    } else if dirty {
        tr(st, "Unsaved changes")
    } else if !st.note.is_empty() {
        tr(st, st.note)
    } else {
        tr(st, "No unsaved changes")
    };
    // Kept while there is something to undo, even when there is nothing to edit.
    g.set_show_bar(open || !st.past.is_empty());
    g.set_save_text(text.into());
    g.set_dirty(dirty);
    g.set_can_apply(dirty && running && st.pending.is_empty());
    g.set_can_revert(dirty && st.pending.is_empty());
    g.set_can_undo(!st.past.is_empty() && st.pending.is_empty());

    if let Some(target) = st.focus.take() {
        st.serial += 1;
        g.set_focus_target(target.into());
        g.set_focus_serial(st.serial);
        let serial = st.serial;
        // Rows built a moment later still find it; after that it is spent.
        Timer::single_shot(Duration::from_millis(300), move || {
            with_win(|w| {
                let g = w.global::<FansPage>();
                if g.get_focus_serial() == serial {
                    g.set_focus_target(SharedString::new());
                }
            })
        });
    }
}

fn render_sensors(st: &St, w: &AppWindow, inner: f32) {
    let g = w.global::<FansPage>();
    let mut groups: Vec<(&'static str, Vec<&SensorStatus>)> = Vec::new();
    for s in &st.live.sensors {
        let name = source(&s.id);
        match groups.iter_mut().find(|g| g.0 == name) {
            Some(group) => group.1.push(s),
            None => groups.push((name, vec![s])),
        }
    }
    let rank = |name: &str| SOURCES.iter().position(|s| *s == name).unwrap_or(SOURCES.len());
    groups.sort_by_key(|g| rank(g.0));
    // Headings only when there is more than one source.
    let named = groups.len() > 1;
    let tile_w = 168f32.min(inner.max(0.0));
    let (mut x, mut y, mut line) = (0f32, 0f32, 0f32);
    let mut tiles = Vec::new();
    for (name, sensors) in &groups {
        let fit = (((inner + 4.0) / 172.0).floor() as usize).max(1);
        let per_row = sensors.len().min(fit);
        let gw = per_row as f32 * (tile_w + 4.0) - 4.0;
        let rows = sensors.len().div_ceil(per_row);
        let head = if named { 20.0 } else { 0.0 };
        let gh = head + rows as f32 * 100.0 - 4.0;
        if x > 0.0 && x + gw > inner {
            x = 0.0;
            y += line + 8.0;
            line = 0.0;
        }
        if named {
            tiles.push(FanTile {
                x,
                y,
                w: gw,
                head: true,
                name: tr(st, name).into(),
                ..Default::default()
            });
        }
        for (i, s) in sensors.iter().enumerate() {
            let accent = if super::light_theme() { [0xa8, 0x4a, 0x00] } else { [0xe8, 0x82, 0x2e] };
            let graph = spark(st.history.get(&format!("s:{}", s.id)), 4.0, accent);
            // The group heading already says what kind of reading it is.
            let short = s
                .name
                .strip_suffix(" temperature")
                .or_else(|| s.name.strip_suffix(" Temperature"))
                .unwrap_or(&s.name);
            tiles.push(FanTile {
                x: x + (i % per_row) as f32 * (tile_w + 4.0),
                y: y + head + (i / per_row) as f32 * 100.0,
                w: tile_w,
                head: false,
                name: tr(st, short).into(),
                value: reading(st, s.value, s.kind).into(),
                none: s.value.is_none(),
                graph,
            });
        }
        x += gw + 16.0;
        line = line.max(gh);
    }
    g.set_tiles_height(if tiles.is_empty() { 0.0 } else { y + line });
    fill(g.get_tiles(), tiles, |m| g.set_tiles(m));
}

fn render_fans(st: &mut St, w: &AppWindow, inner: f32) {
    let g = w.global::<FansPage>();
    let names: Vec<String> = st.draft.curves.iter().map(|c| c.name.clone()).collect();
    let mut options = vec![SharedString::from(tr(st, "Not controlled"))];
    options.extend(names.iter().map(SharedString::from));
    fill(g.get_curve_options(), options, |m| g.set_curve_options(m));

    let cards: Vec<FanCard> = st
        .rule_fans()
        .into_iter()
        .map(|(id, name, now)| {
            let rule = st.draft.rules.iter().find(|r| r.fan == id);
            let was = st.saved.rules.iter().find(|r| r.fan == id);
            let blue = if super::light_theme() { [0x1f, 0x6f, 0xb2] } else { [0x5a, 0xa9, 0xe6] };
            let graph = spark(st.history.get(&format!("f:{id}")), 200.0, blue);
            let duty = now.as_ref().and_then(|n| n.duty);
            let warn = rule.is_some_and(|r| r.min_duty > r.max_duty);
            FanCard {
                id: id.clone().into(),
                name: tr(st, &name).into(),
                tag: if was == rule { String::new() } else { tr(st, "Edited") }.into(),
                spin: match &now {
                    Some(n) => speed(st, n.rpm),
                    None => tr(st, "not connected"),
                }
                .into(),
                spin_none: now.as_ref().is_none_or(|n| n.rpm.is_none()),
                duty: match (&now, duty) {
                    (Some(_), Some(d)) => percent(d),
                    (Some(_), None) => tr(st, "firmware"),
                    (None, _) => tr(st, "unknown"),
                }
                .into(),
                duty_none: duty.is_none(),
                fill: duty.unwrap_or(0.0).clamp(0.0, 1.0),
                pump: pump(&name),
                wrong: now.as_ref().map(|n| st.trouble(n)).unwrap_or_default().into(),
                warn: if warn {
                    tr(st, "The minimum is above the maximum, which the service does not accept.")
                } else {
                    String::new()
                }
                .into(),
                said: st.told("rule", &id).into(),
                curve: rule
                    .and_then(|r| names.iter().position(|n| *n == r.curve))
                    .map_or(0, |i| i as i32 + 1),
                ruled: rule.is_some(),
                min: trim(rule.map_or(0.0, |r| r.min_duty) * 100.0).into(),
                max: trim(rule.map_or(1.0, |r| r.max_duty) * 100.0).into(),
                ramp: trim(rule.map_or(0.2, |r| r.ramp) * 100.0).into(),
                graph,
            }
        })
        .collect();
    let (cols, card_w) = columns(inner, 236.0, 8.0);
    g.set_card_w(card_w);
    let rows: Vec<FanRow> = rows(&mut st.fan_rows, cards, cols)
        .into_iter()
        .map(|(cards, pad)| FanRow { cards, pad })
        .collect();
    fill(g.get_fan_rows(), rows, |m| g.set_fan_rows(m));
}

fn render_curves(st: &mut St, w: &AppWindow, inner: f32) {
    let g = w.global::<FansPage>();
    let items: Vec<FanCurveItem> = st
        .draft
        .curves
        .iter()
        .map(|c| {
            let tag = match st.saved.curves.iter().find(|s| s.name == c.name) {
                None => tr(st, "New"),
                Some(was) if was != c => tr(st, "Edited"),
                Some(_) => String::new(),
            };
            FanCurveItem { name: c.name.clone().into(), tag: tag.into() }
        })
        .collect();
    fill(g.get_curves(), items, |m| g.set_curves(m));
    g.set_chosen(st.chosen as i32);
    g.set_asking(st.asking);

    let curve_wide = inner >= 880.0;
    let editor_w = if curve_wide { inner - 260.0 - 16.0 } else { inner };
    let editor_wide = inner >= 1000.0;
    let shape_w = (editor_w - 16.0 - 240.0).clamp(0.0, 720.0);
    g.set_curve_wide(curve_wide);
    g.set_editor_wide(editor_wide);
    g.set_shape_w(shape_w);

    let Some(curve) = st.draft.curves.get(st.chosen).cloned() else {
        g.set_has_curve(false);
        fill(g.get_points(), Vec::new(), |m| g.set_points(m));
        return;
    };
    g.set_has_curve(true);
    if st.asking {
        let users: Vec<String> = st
            .draft
            .rules
            .iter()
            .filter(|r| r.curve == curve.name)
            .map(|r| st.fan_name(&r.fan))
            .collect();
        let rest = if users.is_empty() {
            String::new()
        } else {
            let follow = if users.len() == 1 { " follows it and" } else { " follow it and" };
            format!(" {}{follow} will no longer be controlled.", list(&users))
        };
        g.set_ask_text(tf(st, "Delete {0}?{1}", &[&curve.name, &rest]).into());
    }
    let here = st.chosen.to_string();
    g.set_name(curve.name.clone().into());
    g.set_name_problem(st.told("name", &here).into());

    let plot = Plot::of(st.graph_w);
    g.set_graph_height(plot.h);
    let points: Vec<FanPoint> = curve
        .points
        .iter()
        .enumerate()
        .map(|(i, &(t, d))| {
            let n = (i + 1).to_string();
            FanPoint {
                x: plot.px(t),
                y: plot.py(d),
                temp: trim(t).into(),
                duty: ((d * 100.0).round() as i64).to_string().into(),
                label: tf(st, "Point {0}", &[&n]).into(),
                value: format!("{} °C, {}", trim(t), percent(d)).into(),
                temp_label: tf(st, "Point {0} temperature in degrees Celsius", &[&n]).into(),
                duty_label: tf(st, "Point {0} duty in per cent", &[&n]).into(),
                remove_label: tf(st, "Remove point {0}", &[&n]).into(),
                percent: (d * 100.0).round() as i32,
                on: st.hl == Some(i),
            }
        })
        .collect();
    fill(g.get_points(), points, |m| g.set_points(m));
    g.set_can_add_point(room_for(&curve.points).is_some());
    g.set_points_problem(st.told("points", &here).into());

    let line = plot.line(&curve.points);
    let d: String = line
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{} {:.2} {:.2} ", if i == 0 { "M" } else { "L" }, p.0, p.1))
        .collect();
    let ready = plot.w > 0.0 && !line.is_empty();
    g.set_line_path(if ready { d.clone() } else { String::new() }.into());
    g.set_area_path(
        if ready {
            format!("{d}L {:.2} {:.2} L {:.2} {:.2} Z", plot.x1, plot.y1, plot.x0, plot.y1)
        } else {
            String::new()
        }
        .into(),
    );

    // One lost sensor is enough for full speed, as in the service.
    let gone = st.lost(&curve);
    let hot = if gone.is_empty() { st.hottest(&curve) } else { None };
    g.set_now_shown(hot.is_some() && ready);
    let now = match hot {
        Some(hot) => {
            let value = hot.value.unwrap_or_default();
            let asked = duty_at(&curve.points, value);
            g.set_now_x(plot.px(value).round() + 0.5);
            g.set_now_y(plot.py(asked));
            tf(
                st,
                "{0} is at {1}, so the curve asks for {2}.",
                &[&hot.name, &reading(st, hot.value, hot.kind), &percent(asked)],
            )
        }
        None if !gone.is_empty() => tf(st, "Fans on this curve run at full speed, because {0}.", &[&st.silent(&gone)]),
        None => tr(st, "No sensor chosen."),
    };
    g.set_now_text(now.into());

    let mut ids: Vec<String> = st.temps().map(|s| s.id.clone()).collect();
    for id in &curve.sensors {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let choices: Vec<FanChoice> = ids
        .iter()
        .map(|id| {
            let s = st.sensor(id);
            FanChoice {
                id: id.clone().into(),
                name: tr(st, s.map_or(id.as_str(), |s| s.name.as_str())).into(),
                value: match s {
                    Some(s) => reading(st, s.value, s.kind),
                    None => tr(st, "not available"),
                }
                .into(),
                none: s.is_none_or(|s| s.value.is_none()),
                checked: curve.sensors.contains(id),
            }
        })
        .collect();
    let inputs_w = if editor_wide { editor_w - shape_w - 16.0 } else { editor_w };
    let (cols, choice_w) = columns(inputs_w, 220.0, 16.0);
    g.set_choice_w(choice_w);
    let rows: Vec<FanChoiceRow> = rows(&mut st.choice_rows, choices, cols)
        .into_iter()
        .map(|(items, pad)| FanChoiceRow { items, pad })
        .collect();
    fill(g.get_choice_rows(), rows, |m| g.set_choice_rows(m));
    let sensors_problem = match st.told("sensors", &here) {
        t if !t.is_empty() => t,
        _ if curve.sensors.is_empty() => {
            tr(st, "Choose at least one sensor. The service does not accept a curve without one.")
        }
        _ => String::new(),
    };
    g.set_sensors_problem(sensors_problem.into());
    g.set_hyst(trim(curve.hysteresis).into());
    g.set_hyst_problem(st.told("hysteresis", &here).into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(max: f32, peak: f32) -> St {
        St {
            draft: FanConfig {
                curves: vec![Curve { name: "c".into(), sensors: vec!["s".into()], points: vec![(40.0, peak)], hysteresis: 2.0 }],
                rules: vec![FanRule { fan: "f".into(), curve: "c".into(), min_duty: 0.1, max_duty: max, ramp: 0.1 }],
            },
            ..St::default()
        }
    }

    #[test]
    fn the_maximum_duty_never_goes_below_the_floor_and_the_minimum_never_above_it() {
        let mut s = st(0.8, 1.0);
        commit(&mut s, "max:f", "0");
        assert_eq!(s.draft.rules[0].max_duty, MIN_MAX_DUTY);
        assert!(s.problem.as_ref().is_some_and(|p| p.text.contains("from 20 to 100")));
        commit(&mut s, "min:f", "90");
        assert_eq!(s.draft.rules[0].min_duty, MIN_MAX_DUTY);
        assert!(s.problem.as_ref().is_some_and(|p| p.text.contains("minimum must not be above")));
        commit(&mut s, "max:f", "60");
        commit(&mut s, "min:f", "50");
        commit(&mut s, "max:f", "30");
        assert_eq!((s.draft.rules[0].min_duty, s.draft.rules[0].max_duty), (0.3, 0.3));
    }

    #[test]
    fn the_services_duty_and_access_refusals_read_as_sentences() {
        let s = st(0.1, 1.0);
        let low_max = s.draft.validate().unwrap_err();
        assert_eq!(plain(&s, &low_max), "Not applied. f: the maximum duty must be at least 20%.");
        let s = st(1.0, 0.1);
        let never = s.draft.validate().unwrap_err();
        assert_eq!(plain(&s, &never), "Not applied. f: the curve or the minimum must reach at least 20%.");
        let refused = "only the person signed in at this PC's screen, or an administrator, can change this; \
            from another Windows session FoxDen can only read the fans";
        assert_eq!(plain(&s, refused), format!("Not applied. {NOT_ALLOWED}"));
    }
}
