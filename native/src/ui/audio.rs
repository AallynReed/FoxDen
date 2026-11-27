use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, Once};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tauri::{AppHandle, Manager};

use super::{AppWindow, AudioPage, I18n, View, tr, with_win};
use crate::mixer::Mixer;

#[path = "audio_render.rs"]
mod render;

const AUDIO_TAB: i32 = 3;
const HOLD: Duration = Duration::from_millis(2500);
const VOL_HOLD: Duration = Duration::from_millis(1500);
pub(super) const CHANNELS: [(&str, &str); 5] = [
    ("game", "Game"),
    ("chatRender", "Chat"),
    ("media", "Media"),
    ("aux", "Aux"),
    ("chatCapture", "Mic"),
];
pub(super) const SIDES: [(&str, &str); 3] = [("game", "Game"), ("media", "Media"), ("aux", "Aux")];
pub(crate) const FREQS: [f64; 10] = [32., 64., 125., 250., 500., 1000., 2000., 4000., 8000., 16000.];
pub(crate) const PRESETS: [[f64; 10]; 4] = [
    [0., 0., 0., 0., 0., 0., 0., 0., 0., 0.],
    [6., 5., 4., 2., 0., 0., 0., 0., 0., 0.],
    [-4., -3., -2., -1., 0., 2., 4., 4., 2., 0.],
    [0., 0., 0., 0., 0., 0., 2., 4., 5., 6.],
];

#[derive(Default)]
pub(super) struct St {
    pub mix: Value,
    pub prio: Value,
    pub vols: Value,
    pub dial: Value,
    pub problem: String,
    pub touched: HashMap<String, Instant>,
    pub eq: Option<String>,
    pub proc: Option<String>,
    pub prio_inputs: bool,
    pub pf_current: Option<String>,
    pub shown: [f32; 5],
    pub reduction: Value,
    pub peaks: HashMap<String, f32>,
    pub tune_target: usize,
    pub tune_busy: bool,
    pub tune_summary: String,
    vol_busy: bool,
    timer: Option<Timer>,
    tick: u32,
    lang: String,
}

impl St {
    pub fn held(&self, key: &str) -> bool {
        let hold = if key.starts_with("va") || key.starts_with("vd") || key.starts_with("ma") || key.starts_with("md") {
            VOL_HOLD
        } else {
            HOLD
        };
        self.touched.get(key).is_some_and(|t| t.elapsed() < hold)
    }

    fn touch(&mut self, key: &str) {
        self.touched.insert(key.to_string(), Instant::now());
    }

    pub fn cfg(&self) -> &Value {
        &self.mix["config"]
    }

    pub fn running(&self) -> bool {
        self.mix["running"].as_bool().unwrap_or(false)
    }

    pub fn chan(&self, id: &str) -> Option<&Value> {
        chan_of(&self.mix["config"], id)
    }

    fn chan_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.mix["config"]["channels"]
            .as_array_mut()?
            .iter_mut()
            .find(|c| c["channel"] == id)
    }

    pub fn proc_of(&self, target: &str) -> Option<&Value> {
        if target == "master" {
            Some(&self.mix["config"]["master_processing"]).filter(|p| p.is_object())
        } else {
            self.chan(target).map(|c| &c["processing"])
        }
    }

    fn proc_mut(&mut self, target: &str) -> Option<&mut Value> {
        if target == "master" {
            Some(&mut self.mix["config"]["master_processing"])
        } else {
            self.chan_mut(target).map(|c| &mut c["processing"])
        }
    }

    pub fn profiles(&self, kind: &str) -> Vec<&Value> {
        self.mix["profiles"]
            .as_array()
            .map(|l| l.iter().filter(|p| p["kind"] == kind).collect())
            .unwrap_or_default()
    }
}

pub(super) fn chan_of<'a>(cfg: &'a Value, id: &str) -> Option<&'a Value> {
    cfg["channels"].as_array()?.iter().find(|c| c["channel"] == id)
}

pub(super) fn kind_of(id: &str) -> &'static str {
    if id == "chatCapture" { "mic" } else { "output" }
}

pub(super) fn label_of(id: &str) -> &'static str {
    if id == "master" {
        return "Master";
    }
    CHANNELS.iter().find(|c| c.0 == id).map_or("", |c| c.1)
}

pub(super) fn short(exe: &str) -> String {
    let lower = exe.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(_) => exe[..exe.len() - 4].to_string(),
        None => exe.to_string(),
    }
}

pub(super) fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

/// Translates an English template, then fills its `{n}` slots.
pub(super) fn tf(app: &AppHandle, template: &str, args: &[&str]) -> String {
    let mut out = tr(app, template);
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
}

fn st<R>(f: impl FnOnce(&mut St) -> R) -> R {
    ST.with_borrow_mut(f)
}

fn redraw(w: &AppWindow, app: &AppHandle) {
    ST.with_borrow(|s| render::render(w, app, s));
}

fn redraw_now(app: &AppHandle) {
    with_win(|w| redraw(w, app));
}

// ---- Worker: one thread runs the commands in order; a newer job with the same key replaces a waiting one.

type Done = Box<dyn FnOnce(&AppWindow, &AppHandle) + Send>;
type Job = Box<dyn FnOnce(&AppHandle) -> Done + Send>;

static JOBS: Mutex<VecDeque<(String, Job)>> = Mutex::new(VecDeque::new());
static READY: Condvar = Condvar::new();
static START: Once = Once::new();

fn queue(app: &AppHandle, key: &str, job: impl FnOnce(&AppHandle) -> Done + Send + 'static) {
    START.call_once(|| {
        let a = app.clone();
        std::thread::Builder::new()
            .name("foxden-audio-ui".into())
            .spawn(move || loop {
                let job = {
                    let mut q = JOBS.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if let Some((_, job)) = q.pop_front() {
                            break job;
                        }
                        q = READY.wait(q).unwrap_or_else(|e| e.into_inner());
                    }
                };
                let done = job(&a);
                let b = a.clone();
                let _ = slint::invoke_from_event_loop(move || with_win(|w| done(w, &b)));
            })
            .expect("audio ui worker");
    });
    let mut q = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    let job: Job = Box::new(job);
    match q.iter_mut().find(|(k, _)| !key.is_empty() && k == key) {
        Some(slot) => slot.1 = job,
        None => q.push_back((key.to_string(), job)),
    }
    READY.notify_one();
}

/// Runs a slow command on its own thread, such as one that waits for an elevation prompt.
fn aside(app: &AppHandle, job: impl FnOnce(&AppHandle) -> Done + Send + 'static) {
    let a = app.clone();
    std::thread::Builder::new().name("foxden-audio-job".into()).spawn(move || {
        let done = job(&a);
        let b = a.clone();
        let _ = slint::invoke_from_event_loop(move || with_win(|w| done(w, &b)));
    }).expect("thread start");
}

fn json<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

fn mixer(app: &AppHandle) -> tauri::State<'static, Mixer> {
    app.state::<Mixer>()
}

// ---- Mixer data

fn take(s: &mut St, mut view: Value) {
    if s.mix.is_object() {
        if let Some(list) = view["config"]["channels"].as_array_mut() {
            for c in list {
                let id = c["channel"].as_str().unwrap_or_default().to_string();
                let Some(old) = s.chan(&id) else { continue };
                let keep: &[(&str, &[&str])] = &[
                    ("proc", &["processing"]),
                    ("eq", &["eq", "eq_on"]),
                    ("mv", &["volume"]),
                    ("mm", &["muted"]),
                    ("r", &["source", "loopback", "on"]),
                ];
                for (prefix, fields) in keep {
                    if s.held(&format!("{prefix}{id}")) {
                        for field in *fields {
                            c[*field] = old[*field].clone();
                        }
                    }
                }
            }
        }
        let keep: &[(&str, &[&str])] = &[
            ("procmaster", &["master_processing"]),
            ("mon", &["monitor", "monitor_volume"]),
            ("sides", &["game_side"]),
            ("gate", &["gate_on", "gate_db"]),
            ("vmic", &["virtual_mic"]),
            ("mvmaster", &["master"]),
            ("mmmaster", &["master_muted"]),
            ("mcm", &["chat_mix"]),
            ("output", &["output"]),
            ("park", &["park"]),
        ];
        for (key, fields) in keep {
            if s.held(key) {
                for field in *fields {
                    view["config"][*field] = s.mix["config"][*field].clone();
                }
            }
        }
    }
    s.mix = view;
}

fn problem_text(s: &St) -> String {
    let e = s.mix["error"].as_str().filter(|e| !e.is_empty());
    let park = s.mix["park_error"].as_str().filter(|_| s.running());
    e.or(park).unwrap_or_default().to_string()
}

fn send_mix(app: &AppHandle, key: &str, change: Value) {
    send(app, key, change, |_| {});
}

fn send(app: &AppHandle, key: &str, change: Value, then: impl FnOnce(&AppHandle) + Send + 'static) {
    let k = key.to_string();
    queue(app, key, move |a| {
        let r = serde_json::from_value::<crate::mixer::Change>(change)
            .map_err(|e| e.to_string())
            .and_then(|c| crate::mixer::mixer_set(mixer(a), c))
            .map(json);
        Box::new(move |w, a| {
            match r {
                Ok(v) => {
                    st(|s| {
                        take(s, v);
                        s.problem = problem_text(s);
                    });
                    redraw(w, a);
                    then(a);
                }
                Err(e) => {
                    st(|s| {
                        s.touched.remove(&k);
                        s.problem = e;
                    });
                    redraw(w, a);
                    load_mixer(a, false);
                }
            }
        })
    });
}

fn load_mixer(app: &AppHandle, rescan: bool) {
    queue(app, if rescan { "rescan" } else { "load" }, move |a| {
        let r = crate::mixer::get_mixer(mixer(a), rescan).map(json);
        Box::new(move |w, a| {
            st(|s| match r {
                Ok(v) => {
                    take(s, v);
                    s.problem = problem_text(s);
                }
                Err(e) => s.problem = e,
            });
            redraw(w, a);
        })
    });
}

// ---- Priority

fn load_priority(app: &AppHandle) {
    queue(app, "prio", |a| {
        let r = crate::priority::get_priority(a.clone()).map(json);
        Box::new(move |w, a| {
            st(|s| match r {
                Ok(v) => s.prio = v,
                Err(e) => s.problem = e,
            });
            redraw(w, a);
        })
    });
}

fn send_priority(app: &AppHandle, edit: impl FnOnce(&mut Value)) {
    let Some(mut cfg) = st(|s| s.prio.get("config").cloned()) else { return };
    edit(&mut cfg);
    queue(app, "prio-set", move |a| {
        let r = serde_json::from_value(cfg)
            .map_err(|e: serde_json::Error| e.to_string())
            .and_then(|p| crate::priority::set_priority(a.clone(), mixer(a), p))
            .map(json);
        Box::new(move |w, a| {
            let ok = st(|s| match r {
                Ok(v) => {
                    s.prio = v;
                    true
                }
                Err(e) => {
                    s.problem = e;
                    false
                }
            });
            redraw(w, a);
            if ok {
                load_mixer(a, false);
            }
        })
    });
}

fn prio_list(s: &St) -> &'static str {
    if s.prio_inputs { "inputs" } else { "outputs" }
}

// ---- Volumes panels

fn take_vols(s: &mut St, mut view: Value) {
    for (list, prefix) in [("apps", 'a'), ("devices", 'd')] {
        let Some(rows) = view[list].as_array_mut() else { continue };
        for r in rows {
            let key = vol_key(prefix, r);
            if (s.held(&format!("v{key}")) || s.held(&format!("m{key}")))
                && let Some(old) = s.vols[list].as_array().and_then(|l| l.iter().find(|o| vol_key(prefix, o) == key)) {
                    if s.held(&format!("v{key}")) {
                        r["volume"] = old["volume"].clone();
                    }
                    if s.held(&format!("m{key}")) {
                        r["muted"] = old["muted"].clone();
                    }
                }
            let peak = s.peaks.get(&key).copied().unwrap_or(0.0);
            s.peaks.insert(key, f(&r["peak"]).max(peak * 0.7));
        }
    }
    s.vols = view;
}

pub(super) fn vol_key(prefix: char, r: &Value) -> String {
    if prefix == 'a' {
        format!("a{}|{}", r["exe"].as_str().unwrap_or_default(), r["endpoint"].as_str().unwrap_or_default())
    } else {
        format!("d{}", r["id"].as_str().unwrap_or_default())
    }
}

fn vol_call(app: &AppHandle, key: &str, call: impl FnOnce(&AppHandle) -> Result<crate::volumes::View, String> + Send + 'static) {
    queue(app, key, move |a| {
        let r = call(a).map(json);
        Box::new(move |w, a| {
            st(|s| match r {
                Ok(v) => {
                    take_vols(s, v);
                    s.problem_vol(None);
                }
                Err(e) => s.problem_vol(Some(e)),
            });
            redraw(w, a);
        })
    });
}

impl St {
    fn problem_vol(&mut self, e: Option<String>) {
        self.vols["problem"] = e.map_or(Value::Null, Value::from);
    }
}

fn poll_vols(app: &AppHandle) {
    let open = with_open(|w| {
        let g = w.global::<AudioPage>();
        g.get_apps_open() || g.get_devs_open()
    });
    if !open || st(|s| std::mem::replace(&mut s.vol_busy, true)) {
        return;
    }
    queue(app, "vols", |a| {
        let r = crate::volumes::get_volumes(mixer(a)).map(json);
        Box::new(move |w, a| {
            st(|s| {
                s.vol_busy = false;
                match r {
                    Ok(mut v) => {
                        v["problem"] = s.vols["problem"].clone();
                        take_vols(s, v);
                    }
                    Err(e) => s.problem_vol(Some(e)),
                }
            });
            redraw(w, a);
        })
    });
}

fn with_open(f: impl FnOnce(&AppWindow) -> bool) -> bool {
    let mut out = false;
    with_win(|w| out = f(w));
    out
}

// ---- Meters

fn tick(w: &AppWindow, app: &AppHandle) {
    let n = st(|s| {
        s.tick = s.tick.wrapping_add(1);
        s.tick
    });
    levels(w, app);
    if n % 5 == 0 || n % 5 == 2 {
        poll_vols(app);
    }
    if n % 20 == 0 && st(|s| s.running()) {
        load_mixer(app, false);
    }
}

fn levels(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<AudioPage>();
    let running = st(|s| s.running());
    if !running {
        let was = st(|s| {
            let was = s.shown.iter().any(|v| *v > 0.0);
            s.shown = [0.0; 5];
            s.reduction = Value::Null;
            was
        });
        if was {
            paint(w);
            ST.with_borrow(|s| render::paint_proc(&g, s));
        }
        return;
    }
    let lv = json(crate::mixer::get_mixer_levels(mixer(app)));
    st(|s| {
        for (n, v) in s.shown.iter_mut().enumerate() {
            *v = f(&lv["peaks"][n]).max(*v * 0.8);
            if *v < 0.002 {
                *v = 0.0;
            }
        }
        s.reduction = lv["reduction"].clone();
    });
    paint(w);
    ST.with_borrow(|s| render::paint_proc(&g, s));
}

fn paint(w: &AppWindow) {
    let g = w.global::<AudioPage>();
    let m = g.get_levels();
    ST.with_borrow(|s| {
        for (n, v) in s.shown.iter().enumerate() {
            if m.row_data(n).is_some_and(|old| (old - v).abs() > 0.001) {
                m.set_row_data(n, *v);
            }
        }
    });
}

fn start_timer(w: &AppWindow, app: &AppHandle) {
    let running = ST.with_borrow(|s| s.timer.as_ref().is_some_and(|t| t.running()));
    if running {
        return;
    }
    let t = Timer::default();
    let weak = w.as_weak();
    let a = app.clone();
    t.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let Some(w) = weak.upgrade() else {
            stop_timer();
            return;
        };
        if w.get_tab() != AUDIO_TAB {
            stop_timer();
            return;
        }
        if !w.window().is_visible() || w.window().is_minimized() {
            return;
        }
        tick(&w, &a);
    });
    st(|s| s.timer = Some(t));
}

fn stop_timer() {
    let _ = ST.try_with(|s| {
        if let Ok(s) = s.try_borrow()
            && let Some(t) = &s.timer {
                t.stop();
            }
    });
}

// ---- Wiring

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<AudioPage>();
    g.set_levels(ModelRc::new(VecModel::from(vec![0.0f32; 5])));
    let weak = w.as_weak();
    let a = app.clone();
    g.on_opened(move || {
        let Some(w) = weak.upgrade() else { return };
        load_mixer(&a, false);
        load_priority(&a);
        st(|s| s.dial = json(crate::headset::headset_dial()));
        redraw(&w, &a);
        start_timer(&w, &a);
    });
    let a = app.clone();
    g.on_act(move |what, x, y| act(&a, &what, &x, &y));
    let a = app.clone();
    g.on_slide(move |what, x, v| slide(&a, &what, &x, v));
    let a = app.clone();
    g.on_flip(move |what, x, on| flip(&a, &what, &x, on));
    let a = app.clone();
    g.on_pick(move |what, x, v| pick(&a, &what, &x, &v));
    st(|s| {
        s.lang.clear();
        s.vol_busy = false;
    });
    redraw(w, app);
    load_mixer(app, false);
    load_priority(app);
}

pub fn event(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    match name {
        "chatmix" => {
            st(|s| s.dial = view.events.get(name).cloned().unwrap_or_default());
            redraw(w, app);
        }
        "audio-priority" => {
            let has_mix = st(|s| {
                s.prio = view.events.get(name).cloned().unwrap_or_default();
                s.mix.is_object()
            });
            redraw(w, app);
            if has_mix && w.get_tab() == AUDIO_TAB {
                load_mixer(app, false);
            }
        }
        "mixer" if st(|s| s.mix.is_object()) => load_mixer(app, false),
        _ => {}
    }
}

pub fn apply(w: &AppWindow, app: &AppHandle, _view: &mut View) {
    let lang = w.global::<I18n>().get_lang().to_string();
    if st(|s| std::mem::replace(&mut s.lang, lang.clone()) != lang) {
        redraw(w, app);
    }
}

fn act(app: &AppHandle, what: &str, x: &str, y: &str) {
    match what {
        "turn-on" => switch_on(app, x),
        "remove-app" => {
            let apps = st(|s| s.chan(x).map(|c| apps_of(c).into_iter().filter(|a| a != y).collect::<Vec<_>>()));
            if let Some(apps) = apps {
                send_mix(app, "", json!({"kind": "apps", "channel": x, "apps": apps}));
            }
        }
        "unroute" => {
            let c = st(|s| s.chan(x).cloned());
            if let Some(c) = c {
                send_mix(app, "", json!({"kind": "route", "channel": x, "source": "", "loopback": c["loopback"], "on": !apps_of(&c).is_empty()}));
            }
        }
        "eq" => {
            st(|s| {
                s.eq = (!x.is_empty() && s.eq.as_deref() != Some(x)).then(|| x.to_string());
                if s.eq.is_some() {
                    s.proc = None;
                }
            });
            redraw_now(app);
        }
        "fx" => {
            st(|s| {
                s.proc = (!x.is_empty() && s.proc.as_deref() != Some(x)).then(|| x.to_string());
                if s.proc.is_some() {
                    s.eq = None;
                }
            });
            redraw_now(app);
        }
        "side" => {
            let sides = st(|s| {
                s.touch("sides");
                let mut now: Vec<String> = s.cfg()["game_side"]
                    .as_array()
                    .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_else(|| vec!["game".into()]);
                if let Some(i) = now.iter().position(|v| v == x) {
                    now.remove(i);
                } else {
                    now.push(x.to_string());
                }
                let list: Vec<&str> = SIDES.iter().map(|s| s.0).filter(|id| now.iter().any(|n| n == id)).collect();
                s.mix["config"]["game_side"] = json!(list);
                list.iter().map(|v| v.to_string()).collect::<Vec<_>>()
            });
            redraw_now(app);
            send_mix(app, "sides", json!({"kind": "mix_sides", "channels": sides}));
        }
        "prio-open" => {
            let open = with_open(|w| {
                let g = w.global::<AudioPage>();
                g.set_prio_open(!g.get_prio_open());
                g.get_prio_open()
            });
            if open {
                load_priority(app);
            }
        }
        "prio-move" => {
            let (i, to) = (x.parse::<i64>().unwrap_or(-1), y.parse::<i64>().unwrap_or(-1));
            let list = st(|s| prio_list(s));
            send_priority(app, |c| {
                if let Some(l) = c[list].as_array_mut()
                    && i >= 0 && to >= 0 && (i as usize) < l.len() && (to as usize) < l.len() {
                        let e = l.remove(i as usize);
                        l.insert(to as usize, e);
                    }
            });
        }
        "prio-remove" => {
            let i = x.parse::<usize>().unwrap_or(usize::MAX);
            let list = st(|s| prio_list(s));
            send_priority(app, |c| {
                if let Some(l) = c[list].as_array_mut()
                    && i < l.len() {
                        l.remove(i);
                    }
            });
        }
        "sheet-cancel" => {
            st(|s| {
                s.touched.remove("vmic");
            });
            with_win(|w| w.global::<AudioPage>().set_sheet_open(false));
            redraw_now(app);
        }
        "vmic-install" => install_vmic(app),
        "vmic-donate" => {
            std::thread::Builder::new().name("foxden-link".into()).spawn(|| {
                let _ = crate::links::open_link("vbaudio".into());
            }).expect("thread start");
        }
        "vmic-remove" => remove_vmic(app),
        "rescan" => load_mixer(app, true),
        "run" => {
            let on = !st(|s| s.running());
            with_win(|w| w.global::<AudioPage>().set_run_busy(true));
            let change = json!({"kind": "run", "on": on});
            let a = app.clone();
            queue(app, "", move |b| {
                let r = serde_json::from_value::<crate::mixer::Change>(change)
                    .map_err(|e| e.to_string())
                    .and_then(|c| crate::mixer::mixer_set(mixer(b), c))
                    .map(json);
                Box::new(move |w, _| {
                    w.global::<AudioPage>().set_run_busy(false);
                    match r {
                        Ok(v) => st(|s| {
                            take(s, v);
                            s.problem = problem_text(s);
                        }),
                        Err(e) => {
                            st(|s| s.problem = e);
                            load_mixer(&a, false);
                        }
                    }
                    redraw(w, &a);
                })
            });
        }
        "preset" => {
            let n = x.parse::<usize>().unwrap_or(0).min(PRESETS.len() - 1);
            let Some(id) = st(|s| s.eq.clone()) else { return };
            let on = st(|s| {
                let c = s.chan_mut(&id)?;
                c["eq"] = json!(FREQS.iter().zip(PRESETS[n]).map(|(fq, g)| json!({"enabled": true, "filter": "PeakingEQ", "frequency": fq, "gain_db": g, "q": 1.414})).collect::<Vec<_>>());
                Some(n != 0 || c["eq_on"].as_bool().unwrap_or(false))
            });
            if let Some(on) = on {
                send_eq(app, &id, Some(on));
            }
        }
        "pf-save" => {
            let (id, name) = pf_target(app);
            if let (Some(id), false) = (id, name.is_empty()) {
                send_mix(app, "", json!({"kind": "save_profile", "channel": id, "name": name}));
            }
        }
        "pf-rename" => {
            let (_, name) = pf_target(app);
            let Some(from) = st(|s| s.pf_current.clone()) else { return };
            if !name.is_empty() {
                let to = name.clone();
                send(app, "", json!({"kind": "rename_profile", "from": from, "to": name}), move |a| {
                    st(|s| s.pf_current = Some(to));
                    redraw_now(a);
                });
            }
        }
        "pf-delete" => {
            let Some(name) = st(|s| s.pf_current.clone()) else { return };
            send(app, "", json!({"kind": "delete_profile", "name": name}), |a| {
                st(|s| s.pf_current = None);
                redraw_now(a);
            });
        }
        "pf-copy" => {
            let note = match share_code(app) {
                Ok(code) => {
                    with_win(|w| w.global::<AudioPage>().set_pf_code(code.clone().into()));
                    tr(app, if copy_text(&code) { "Share code copied." } else { "Copy the code from the box." })
                }
                Err(e) => e,
            };
            with_win(|w| w.global::<AudioPage>().set_pf_note(note.into()));
        }
        "pf-export" => {
            let note = export(app).unwrap_or_else(|e| e);
            with_win(|w| w.global::<AudioPage>().set_pf_note(note.into()));
        }
        "pf-add" => {
            let code = with_open_text(|w| w.global::<AudioPage>().get_pf_code());
            match crate::profiles::sound_decode(code.trim().to_string()) {
                Ok(p) => {
                    let name = p.name.clone();
                    let kind = json(&p)["kind"].as_str().unwrap_or("output").to_string();
                    send(app, "", json!({"kind": "add_profile", "profile": p}), move |a| {
                        let word = tr(a, if kind == "mic" { "mic" } else { "output" });
                        let note = tf(a, "Added {0} ({1}", &[&name, &format!("{word}).")]);
                        st(|s| s.pf_current = Some(name));
                        with_win(|w| {
                            let g = w.global::<AudioPage>();
                            g.set_pf_code("".into());
                            g.set_pf_note(note.into());
                        });
                        redraw_now(a);
                    });
                }
                Err(e) => with_win(|w| w.global::<AudioPage>().set_pf_note(e.into())),
            }
        }
        "make-default" => {
            let id = x.strip_prefix('d').unwrap_or(x).to_string();
            vol_call(app, "", move |a| crate::volumes::make_default_device(mixer(a), id));
        }
        "allow" => send_mix(app, "", json!({"kind": "allow_auto", "exe": x})),
        "auto-tune" => auto_tune(app),
        "tune-undo" => {
            aside(app, |a| {
                let r = crate::mic_check::undo(a);
                Box::new(move |w, a| {
                    st(|s| s.tune_summary = match r {
                        Ok(_) => tr(a, "The mic chain from before auto-tune is back."),
                        Err(e) => tr(a, &e),
                    });
                    load_mixer(a, false);
                    redraw(w, a);
                })
            });
        }
        _ => {}
    }
}

const TUNE_SECONDS: f64 = 6.0;

/// Records a sample the person asked for with the button, then sets the whole mic chain.
fn auto_tune(app: &AppHandle) {
    if st(|s| std::mem::replace(&mut s.tune_busy, true)) {
        return;
    }
    let n = st(|s| s.tune_target).min(render::TARGETS.len() - 1);
    let target: foxden_mixer::tune::Target = serde_json::from_value(json!(render::TARGETS[n].0)).unwrap_or(foxden_mixer::tune::Target::Discord);
    st(|s| s.tune_summary = tf(app, "Speak normally for {0} seconds…", &[&TUNE_SECONDS.to_string()]));
    redraw_now(app);
    aside(app, move |a| {
        let r = crate::mic_check::current(a).and_then(|old| {
            let (raw, rate) = crate::mic_check::record(&old, TUNE_SECONDS)?;
            let t = crate::mic_check::work_out(&raw, rate, &old, target, false);
            drop(raw);
            crate::mic_check::commit(a, &old, &t)?;
            Ok((t.before, t.after))
        });
        Box::new(move |w, a| {
            st(|s| {
                s.tune_busy = false;
                let line = |m: &foxden_mixer::analysis::Analysis| {
                    tf(a, "{0} LUFS, peak {1} dBFS, floor {2} dBFS, sibilance {3} dB", &[
                        &format!("{:.1}", m.lufs),
                        &format!("{:.1}", m.peak_dbfs),
                        &format!("{:.0}", m.noise_floor_dbfs),
                        &format!("{:.0}", m.sibilance_peak_db),
                    ])
                };
                s.tune_summary = match r {
                    Ok((before, after)) => tf(a, "Before: {0}. After: {1}.", &[&line(&before), &line(&after)]),
                    Err(e) => tr(a, &e),
                };
            });
            load_mixer(a, false);
            redraw(w, a);
        })
    });
}

fn slide(app: &AppHandle, what: &str, x: &str, v: f32) {
    let v = if v.is_finite() { v } else { return };
    match what {
        "volume" => {
            let key = format!("mv{x}");
            st(|s| {
                s.touch(&key);
                if let Some(c) = s.chan_mut(x) {
                    c["volume"] = json!(v);
                }
            });
            redraw_now(app);
            send_mix(app, &key, json!({"kind": "volume", "channel": x, "volume": v}));
        }
        "master" => {
            st(|s| {
                s.touch("mvmaster");
                s.mix["config"]["master"] = json!(v);
            });
            redraw_now(app);
            send_mix(app, "mvmaster", json!({"kind": "master", "volume": v}));
        }
        "chat" => {
            let b = (v.round() / 100.0).clamp(-1.0, 1.0);
            st(|s| {
                s.touch("mcm");
                s.mix["config"]["chat_mix"] = json!(b);
            });
            redraw_now(app);
            send_mix(app, "mcm", json!({"kind": "chat_mix", "balance": b}));
        }
        "gate" => {
            let on = st(|s| {
                s.touch("gate");
                s.mix["config"]["gate_db"] = json!(v.round());
                s.cfg()["gate_on"].as_bool().unwrap_or(false)
            });
            redraw_now(app);
            send_mix(app, "gate", json!({"kind": "gate", "on": on, "threshold_db": v.round()}));
        }
        "monitor" => {
            st(|s| {
                s.mix["config"]["monitor_volume"] = json!(v);
            });
            send_monitor(app);
        }
        "band-gain" => {
            let Some(id) = st(|s| s.eq.clone()) else { return };
            let i = x.parse::<usize>().unwrap_or(usize::MAX);
            let ok = st(|s| {
                let b = s.chan_mut(&id)?.get_mut("eq")?.get_mut(i)?;
                b["gain_db"] = json!(v);
                Some(())
            });
            if ok.is_some() {
                send_eq(app, &id, None);
            }
        }
        "proc" if x == "gate.threshold_db" => slide(app, "gate", "", v),
        "proc" => {
            let Some(target) = st(|s| s.proc.clone()) else { return };
            let Some((block, field)) = x.split_once('.') else { return };
            let ok = st(|s| {
                let p = s.proc_mut(&target)?;
                p.get_mut(block)?[field] = json!(v);
                Some(())
            });
            if ok.is_some() {
                send_proc(app, &target);
            }
        }
        "app-volume" | "dev-volume" => {
            let Some(row) = vol_row(x, "volume", json!(v)) else { return };
            redraw_now(app);
            let key = format!("v{x}");
            if what == "app-volume" {
                vol_call(app, &key, move |a| {
                    crate::volumes::set_app_volume(mixer(a), s_of(&row["exe"]), s_of(&row["endpoint"]), Some(v), None)
                });
            } else {
                vol_call(app, &key, move |a| crate::volumes::set_device_volume(mixer(a), s_of(&row["id"]), Some(v), None));
            }
        }
        _ => {}
    }
}

fn flip(app: &AppHandle, what: &str, x: &str, on: bool) {
    match what {
        "mute" => {
            let key = format!("mm{x}");
            st(|s| {
                s.touch(&key);
                if let Some(c) = s.chan_mut(x) {
                    c["muted"] = json!(on);
                }
            });
            redraw_now(app);
            send_mix(app, &key, json!({"kind": "mute", "channel": x, "muted": on}));
        }
        "master-mute" => {
            st(|s| {
                s.touch("mmmaster");
                s.mix["config"]["master_muted"] = json!(on);
            });
            redraw_now(app);
            send_mix(app, "mmmaster", json!({"kind": "master_mute", "muted": on}));
        }
        "eq-on" => {
            let id = if x.is_empty() { st(|s| s.eq.clone()) } else { Some(x.to_string()) };
            if let Some(id) = id {
                send_eq(app, &id, Some(on));
            }
        }
        "vmic" => want_vmic(app, on),
        "gate" => {
            let db = st(|s| {
                s.touch("gate");
                s.mix["config"]["gate_on"] = json!(on);
                f(&s.cfg()["gate_db"])
            });
            redraw_now(app);
            send_mix(app, "gate", json!({"kind": "gate", "on": on, "threshold_db": db}));
        }
        "monitor" => {
            st(|s| s.mix["config"]["monitor"] = json!(on));
            send_monitor(app);
        }
        "prio-on" | "prio-mixer" | "prio-windows" => {
            let field = &what[5..];
            let field = if field == "on" { "on" } else { field };
            send_priority(app, |c| c[field] = json!(on));
        }
        "prio-list" => {
            st(|s| s.prio_inputs = on);
            redraw_now(app);
        }
        "block" if x == "gate" => flip(app, "gate", "", on),
        "block" | "block-check" => {
            let Some(target) = st(|s| s.proc.clone()) else { return };
            let field = if what == "block" { "on" } else { "auto_makeup" };
            let ok = st(|s| {
                let p = s.proc_mut(&target)?;
                p.get_mut(x)?[field] = json!(on);
                Some(())
            });
            if ok.is_some() {
                send_proc(app, &target);
            }
        }
        "apps-open" | "devs-open" => {
            with_win(|w| {
                let g = w.global::<AudioPage>();
                if what == "apps-open" {
                    g.set_apps_open(on);
                } else {
                    g.set_devs_open(on);
                }
            });
            if on {
                poll_vols(app);
            }
        }
        "auto" => send_mix(app, "", json!({"kind": "auto_assign", "on": on})),
        "app-mute" | "dev-mute" => {
            let Some(row) = vol_row(x, "muted", json!(on)) else { return };
            redraw_now(app);
            let key = format!("m{x}");
            if what == "app-mute" {
                vol_call(app, &key, move |a| {
                    crate::volumes::set_app_volume(mixer(a), s_of(&row["exe"]), s_of(&row["endpoint"]), None, Some(on))
                });
            } else {
                vol_call(app, &key, move |a| crate::volumes::set_device_volume(mixer(a), s_of(&row["id"]), None, Some(on)));
            }
        }
        _ => {}
    }
}

fn pick(app: &AppHandle, what: &str, x: &str, v: &str) {
    match what {
        "add-app" => {
            if v.is_empty() {
                return;
            }
            let Some(mut apps) = st(|s| s.chan(x).map(apps_of)) else { return };
            apps.push(v.to_string());
            let id = x.to_string();
            send(app, "", json!({"kind": "apps", "channel": x, "apps": apps}), move |a| {
                if st(|s| s.chan(&id).is_some_and(|c| !c["on"].as_bool().unwrap_or(false))) {
                    switch_on(a, &id);
                }
            });
        }
        "mic" | "source" => {
            let id = if what == "mic" { "chatCapture" } else { x };
            let (kind, source) = v.split_once(':').unwrap_or(("", ""));
            let key = format!("r{id}");
            let loopback = kind == "out";
            let on = st(|s| {
                s.touch(&key);
                let c = s.chan_mut(id)?;
                c["source"] = json!(source);
                c["loopback"] = json!(loopback);
                let on = !v.is_empty() || (what == "source" && !apps_of(c).is_empty());
                c["on"] = json!(on);
                Some(on)
            });
            let Some(on) = on else { return };
            redraw_now(app);
            send_mix(app, &key, json!({"kind": "route", "channel": id, "source": source, "loopback": loopback, "on": on}));
        }
        "profile" => {
            let id = if x.is_empty() { st(|s| s.eq.clone()) } else { Some(x.to_string()) };
            let Some(id) = id else { return };
            if x.is_empty() {
                st(|s| s.pf_current = Some(v.to_string()));
            }
            let p = st(|s| s.profiles(kind_of(&id)).into_iter().find(|p| p["name"] == v).cloned());
            if let Some(p) = p {
                send_mix(app, "", json!({"kind": "apply_profile", "channel": id, "profile": p}));
            }
        }
        "output" => {
            st(|s| {
                s.touch("output");
                s.mix["config"]["output"] = json!(v);
            });
            redraw_now(app);
            send_mix(app, "", json!({"kind": "output", "name": v}));
        }
        "prio-add" => {
            let list = st(|s| prio_list(s));
            let choices = if list == "outputs" { "choices_out" } else { "choices_in" };
            let e = st(|s| s.prio[choices].as_array().and_then(|l| l.iter().find(|e| e["id"] == v).cloned()));
            if let Some(e) = e {
                send_priority(app, |c| {
                    if let Some(l) = c[list].as_array_mut() {
                        l.push(json!({"id": e["id"], "name": e["name"]}));
                    }
                });
            }
        }
        "band-filter" | "band-freq" | "band-q" => {
            let Some(id) = st(|s| s.eq.clone()) else { return };
            let i = x.parse::<usize>().unwrap_or(usize::MAX);
            let ok = st(|s| {
                let b = s.chan_mut(&id)?.get_mut("eq")?.get_mut(i)?;
                match what {
                    "band-filter" => b["filter"] = json!(v),
                    "band-freq" => b["frequency"] = json!(num(v, 20.0, 20000.0).unwrap_or(f(&b["frequency"]) as f64)),
                    _ => b["q"] = json!(num(v, 0.1, 10.0).unwrap_or(f(&b["q"]) as f64)),
                }
                Some(())
            });
            if ok.is_some() {
                send_eq(app, &id, None);
            }
        }
        "proc-preset" => {
            let Some(target) = st(|s| s.proc.clone()) else { return };
            let p = st(|s| {
                s.profiles(kind_of(&target))
                    .into_iter()
                    .find(|p| p["name"] == v && p["processing"].is_object())
                    .cloned()
            });
            let Some(p) = p else { return };
            if target == "master" {
                st(|s| s.mix["config"]["master_processing"] = p["processing"].clone());
                send_proc(app, &target);
            } else {
                send_mix(app, "", json!({"kind": "apply_profile", "channel": target, "profile": p}));
            }
        }
        "proc-engine" | "proc-order" => {
            let Some(target) = st(|s| s.proc.clone()) else { return };
            let ok = st(|s| {
                let p = s.proc_mut(&target)?;
                if what == "proc-engine" {
                    p.get_mut("denoise")?["engine"] = json!(v);
                } else {
                    p["order"] = json!(v.split(',').collect::<Vec<_>>());
                }
                Some(())
            });
            if ok.is_some() {
                send_proc(app, &target);
            }
        }
        "tune-target" => {
            let n = render::TARGETS.iter().position(|t| t.0 == v).unwrap_or(0);
            st(|s| s.tune_target = n);
            redraw_now(app);
        }
        "park" => {
            st(|s| {
                s.touch("park");
                s.mix["config"]["park"] = json!(v);
            });
            redraw_now(app);
            send_mix(app, "", json!({"kind": "park", "name": v}));
        }
        _ => {}
    }
}

fn num(text: &str, lo: f64, hi: f64) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(lo, hi))
}

fn s_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub(super) fn apps_of(c: &Value) -> Vec<String> {
    c["apps"]
        .as_array()
        .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn switch_on(app: &AppHandle, id: &str) {
    let Some(c) = st(|s| s.chan(id).cloned()) else { return };
    send_mix(app, "", json!({"kind": "route", "channel": id, "source": c["source"], "loopback": c["loopback"], "on": true}));
}

fn send_eq(app: &AppHandle, id: &str, on: Option<bool>) {
    let key = format!("eq{id}");
    let change = st(|s| {
        s.touch(&key);
        let c = s.chan_mut(id)?;
        if let Some(on) = on {
            c["eq_on"] = json!(on);
        }
        Some(json!({"kind": "eq", "channel": id, "enabled": c["eq_on"], "bands": c["eq"]}))
    });
    redraw_now(app);
    if let Some(change) = change {
        send_mix(app, &key, change);
    }
}

fn send_proc(app: &AppHandle, target: &str) {
    let key = format!("proc{target}");
    let change = st(|s| {
        s.touch(&key);
        let p = s.proc_of(target)?.clone();
        let channel = if target == "master" { Value::Null } else { json!(target) };
        Some(json!({"kind": "processing", "channel": channel, "processing": p}))
    });
    redraw_now(app);
    if let Some(change) = change {
        send_mix(app, &key, change);
    }
}

fn send_monitor(app: &AppHandle) {
    let change = st(|s| {
        s.touch("mon");
        let c = s.cfg();
        json!({"kind": "monitor", "on": c["monitor"].as_bool().unwrap_or(false), "volume": c["monitor_volume"].as_f64().unwrap_or(0.5)})
    });
    redraw_now(app);
    send_mix(app, "mon", change);
}

/// Changes a row of the Apps or Devices panel locally; `field` is "volume" or "muted".
fn vol_row(key: &str, field: &str, value: Value) -> Option<Value> {
    let list = if key.starts_with('a') { "apps" } else { "devices" };
    let prefix = if list == "apps" { 'a' } else { 'd' };
    st(|s| {
        let row = s.vols[list].as_array_mut()?.iter_mut().find(|r| vol_key(prefix, r) == key)?;
        row[field] = value;
        let out = row.clone();
        s.touch(&format!("{}{key}", &field[..1]));
        Some(out)
    })
}

// ---- Virtual mic

fn want_vmic(app: &AppHandle, on: bool) {
    st(|s| s.touch("vmic"));
    if !on {
        st(|s| s.mix["config"]["virtual_mic"] = json!(false));
        redraw_now(app);
        send_mix(app, "vmic", json!({"kind": "virtual_mic", "on": false}));
        return;
    }
    if st(|s| s.mix["vmic"]["installed"].as_bool().unwrap_or(false)) {
        st(|s| s.mix["config"]["virtual_mic"] = json!(true));
        redraw_now(app);
        send_mix(app, "vmic", json!({"kind": "virtual_mic", "on": true}));
        return;
    }
    with_win(|w| {
        let g = w.global::<AudioPage>();
        g.set_vmic(true);
        g.set_sheet_progress("".into());
        g.set_sheet_open(true);
    });
}

fn install_vmic(app: &AppHandle) {
    with_win(|w| {
        let g = w.global::<AudioPage>();
        g.set_installing(true);
        g.set_sheet_progress(tr(app, "Looking for VB-CABLE…").into());
    });
    aside(app, |a| {
        let r = crate::mixer::virtual_mic_install(mixer(a)).map(json);
        Box::new(move |w, a| {
            let g = w.global::<AudioPage>();
            g.set_installing(false);
            match r {
                Ok(v) => {
                    st(|s| {
                        s.touched.remove("vmic");
                        take(s, v);
                    });
                    g.set_sheet_open(false);
                }
                Err(e) => g.set_sheet_progress(e.into()),
            }
            redraw(w, a);
        })
    });
}

fn remove_vmic(app: &AppHandle) {
    with_win(|w| {
        let g = w.global::<AudioPage>();
        g.set_removing(true);
        g.set_vmic_outcome(tr(app, "Removing VB-CABLE. Windows asks for administrator rights.").into());
    });
    aside(app, |a| {
        let r = crate::mixer::virtual_mic_remove(mixer(a)).map(json);
        Box::new(move |w, a| {
            let g = w.global::<AudioPage>();
            g.set_removing(false);
            match r {
                Ok(v) => {
                    st(|s| take(s, v));
                    g.set_vmic_outcome(tr(a, "Removed. Restart Windows to finish.").into());
                }
                Err(e) => g.set_vmic_outcome(e.into()),
            }
            redraw(w, a);
        })
    });
}

// ---- Sound profiles

fn with_open_text(f: impl FnOnce(&AppWindow) -> SharedString) -> String {
    let mut out = String::new();
    with_win(|w| out = f(w).to_string());
    out
}

/// The channel the EQ drawer edits and the trimmed profile name typed there.
fn pf_target(_app: &AppHandle) -> (Option<String>, String) {
    let name: String = with_open_text(|w| w.global::<AudioPage>().get_pf_name()).trim().chars().take(40).collect();
    (st(|s| s.eq.clone()), name)
}

/// The profile sharing works on: the one last applied or saved, else the current settings.
fn shareable(app: &AppHandle) -> Option<Value> {
    let (id, name) = pf_target(app);
    let id = id?;
    st(|s| {
        let kind = kind_of(&id);
        if let Some(p) = s.profiles(kind).into_iter().find(|p| Some(p["name"].as_str().unwrap_or_default()) == s.pf_current.as_deref()) {
            return Some(p.clone());
        }
        let c = s.chan(&id)?;
        let cfg = s.cfg();
        let name = if name.is_empty() { format!("{} sound", label_of(&id)) } else { name };
        Some(json!({
            "name": name,
            "kind": kind,
            "eq": {"on": c["eq_on"], "bands": c["eq"]},
            "volume": c["volume"],
            "gate": if kind == "mic" { json!({"on": cfg["gate_on"], "db": cfg["gate_db"]}) } else { Value::Null },
            "processing": c["processing"],
        }))
    })
}

fn share_code(app: &AppHandle) -> Result<String, String> {
    let p = shareable(app).ok_or_else(String::new)?;
    let profile = serde_json::from_value(p).map_err(|e| e.to_string())?;
    Ok(crate::profiles::sound_code(profile))
}

/// Saves the code to Downloads, as the browser did.
fn export(app: &AppHandle) -> Result<String, String> {
    let p = shareable(app).ok_or_else(String::new)?;
    let name: String = p["name"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == ' ' || *c == '-')
        .collect();
    let code = crate::profiles::sound_code(serde_json::from_value(p).map_err(|e| e.to_string())?);
    let dir = std::path::PathBuf::from(std::env::var_os("USERPROFILE").ok_or("no user folder")?).join("Downloads");
    let path = dir.join(format!("{}.foxsound", if name.is_empty() { "profile" } else { &name }));
    std::fs::write(&path, format!("{code}\n")).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

fn copy_text(text: &str) -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow()) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let mut ok = false;
        if !mem.is_null() {
            let p = GlobalLock(mem) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                GlobalUnlock(mem);
                ok = !SetClipboardData(CF_UNICODETEXT as u32, mem).is_null();
            }
            if !ok {
                windows_sys::Win32::Foundation::GlobalFree(mem);
            }
        }
        CloseClipboard();
        ok
    }
}
