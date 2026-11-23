use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use slint::{
    CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer,
    SharedString, Timer, TimerMode, VecModel,
};
use tauri::{AppHandle, Manager};

use crate::canvas::{Canvas, Dev};
use crate::commands;
use crate::state::{Jobs, Shared, lock};

slint::include_modules!();

const LANGS: [(&str, &str); 16] = [
    ("en", "English"),
    ("pt-PT", "Português (Portugal)"),
    ("es", "Español"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("ru", "Русский"),
    ("zh-Hans", "简体中文"),
    ("pl", "Polski"),
    ("tr", "Türkçe"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("it", "Italiano"),
    ("uk", "Українська"),
    ("zh-Hant", "繁體中文"),
    ("nl", "Nederlands"),
    ("cs", "Čeština"),
];

#[derive(Default)]
struct Pending {
    snapshot: Option<Value>,
    frame: Option<Value>,
    fans: Option<Value>,
    scheduled: bool,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    snapshot: None,
    frame: None,
    fans: None,
    scheduled: false,
});

#[derive(Default)]
struct View {
    snap: Value,
    effect: Value,
    fans: Value,
    canvas: Canvas,
    seed: u64,
}

struct Ui {
    win: AppWindow,
    effects: Rc<VecModel<EffectItem>>,
    params: Rc<VecModel<Param>>,
    devices: Rc<VecModel<DeviceItem>>,
    labels: Rc<VecModel<CanvasLabel>>,
    detected: Rc<VecModel<Line>>,
    backends: Rc<VecModel<Line>>,
    fans: Rc<VecModel<Line>>,
    profiles: Rc<VecModel<SharedString>>,
    _poll: Timer,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static VIEW: RefCell<View> = RefCell::new(View::default());
}

pub fn attach(app: &AppHandle) {
    let handle = app.clone();
    app.set_sink(move |event, value| {
        let mut p = PENDING.lock().unwrap();
        match event {
            "snapshot" => p.snapshot = Some(value),
            "frame" => p.frame = Some(value),
            "fans" => p.fans = Some(value),
            _ => return,
        }
        if !p.scheduled {
            p.scheduled = true;
            let app = handle.clone();
            let _ = slint::invoke_from_event_loop(move || drain(&app));
        }
    });
}

fn drain(app: &AppHandle) {
    let p = std::mem::take(&mut *PENDING.lock().unwrap());
    VIEW.with_borrow_mut(|view| {
        if let Some(fans) = p.fans {
            view.fans = fans;
            with_ui(|ui| apply_fans(ui, view));
        }
        if let Some(snap) = p.snapshot {
            view.effect = snap["effect"].clone();
            view.snap = snap;
            view.canvas.settle();
            with_ui(|ui| apply_snapshot(app, ui, view));
        }
        if let Some(frame) = p.frame {
            if let Some(frames) = frame.as_array() {
                let colors: std::collections::HashMap<String, Vec<[u8; 3]>> = frames
                    .iter()
                    .filter_map(|f| {
                        let id = f["device"].as_str()?.to_string();
                        let colors = serde_json::from_value(f["colors"].clone()).ok()?;
                        Some((id, colors))
                    })
                    .collect();
                if colors != view.canvas.colors {
                    view.canvas.colors = colors;
                    with_ui(|ui| redraw(ui, view));
                }
            }
        }
    });
}

fn with_ui(f: impl FnOnce(&Ui)) {
    UI.with_borrow(|ui| {
        if let Some(ui) = ui {
            f(ui);
        }
    });
}

pub fn show(app: &AppHandle) {
    let exists = UI.with_borrow(|ui| {
        if let Some(ui) = ui {
            let _ = ui.win.show();
            ui.win.window().set_minimized(false);
            true
        } else {
            false
        }
    });
    if exists {
        app.set_shown(true, false);
        return;
    }
    let ui = match build(app) {
        Ok(ui) => ui,
        Err(e) => {
            crate::log::warn("app", format!("the window could not open: {e}"));
            return;
        }
    };
    let snap = serde_json::to_value(lock(&app.state::<Shared>()).snapshot()).unwrap_or_default();
    VIEW.with_borrow_mut(|view| {
        view.effect = snap["effect"].clone();
        view.snap = snap;
        view.canvas.light = light_theme();
        apply_snapshot(app, &ui, view);
        apply_fans(&ui, view);
    });
    let _ = ui.win.show();
    UI.with_borrow_mut(|slot| *slot = Some(ui));
    app.set_shown(true, false);
    let handle = app.clone();
    std::thread::spawn(move || {
        if let Ok(state) = crate::fans::get_fans() {
            use tauri::Emitter;
            let _ = handle.emit("fans", &state);
        }
    });
}

fn light_theme() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut value = 0u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    } == 0;
    ok && value == 1
}

fn model<T: Clone + 'static>() -> Rc<VecModel<T>> {
    Rc::new(VecModel::default())
}

fn sync<T: Clone + PartialEq + 'static>(m: &VecModel<T>, rows: Vec<T>) {
    for (i, r) in rows.iter().enumerate() {
        if i < m.row_count() {
            if m.row_data(i).as_ref() != Some(r) {
                m.set_row_data(i, r.clone());
            }
        } else {
            m.push(r.clone());
        }
    }
    while m.row_count() > rows.len() {
        m.remove(m.row_count() - 1);
    }
}

fn report(r: Result<(), String>) {
    if let Err(e) = r {
        crate::log::warn("ui", e.clone());
        with_ui(|ui| ui.win.set_note(e.into()));
    }
}

fn shared(app: &AppHandle) -> tauri::State<'static, Shared> {
    app.state::<Shared>()
}

fn send_effect(app: &AppHandle, effect: Value) {
    match serde_json::from_value(effect) {
        Ok(effect) => report(commands::set_effect(app.clone(), shared(app), effect)),
        Err(e) => report(Err(e.to_string())),
    }
}

fn edit_effect(app: &AppHandle, name: &str, value: Value) {
    let effect = VIEW.with_borrow_mut(|view| {
        view.effect[name] = value;
        view.effect.clone()
    });
    send_effect(app, effect);
}

fn send_device(app: &AppHandle, d: &Dev, enabled: bool, brightness: f32, rect: [f32; 4]) {
    report(commands::set_device(
        app.clone(),
        shared(app),
        d.id.clone(),
        enabled,
        brightness,
        rect,
    ));
}

fn edit_settings(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    let mut settings = VIEW.with_borrow(|v| v.snap["settings"].clone());
    change(&mut settings);
    match serde_json::from_value(settings) {
        Ok(settings) => report(commands::set_settings(
            app.clone(),
            shared(app),
            app.state::<Jobs>(),
            settings,
        )),
        Err(e) => report(Err(e.to_string())),
    }
}

fn catalog_entry(view: &View) -> Option<Value> {
    let kind = view.effect["kind"].as_str()?;
    view.snap["catalog"]
        .as_array()?
        .iter()
        .find(|e| e["default"]["kind"] == kind)
        .cloned()
}

fn random(seed: &mut u64) -> f32 {
    if *seed == 0 {
        *seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1;
    }
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 40) as f32 / (1u64 << 24) as f32
}

fn hue(h: f32) -> [u8; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        (255.0 * (1.0 - (k.min(4.0 - k)).clamp(0.0, 1.0))) as u8
    };
    [f(5.0), f(3.0), f(1.0)]
}

fn build(app: &AppHandle) -> Result<Ui, slint::PlatformError> {
    let win = AppWindow::new()?;
    let ui = Ui {
        effects: model(),
        params: model(),
        devices: model(),
        labels: model(),
        detected: model(),
        backends: model(),
        fans: model(),
        profiles: model(),
        _poll: Timer::default(),
        win,
    };
    let w = &ui.win;
    w.set_effects(ModelRc::from(ui.effects.clone()));
    w.set_params(ModelRc::from(ui.params.clone()));
    w.set_devices(ModelRc::from(ui.devices.clone()));
    w.set_labels(ModelRc::from(ui.labels.clone()));
    w.set_detected(ModelRc::from(ui.detected.clone()));
    w.set_backends(ModelRc::from(ui.backends.clone()));
    w.set_fans(ModelRc::from(ui.fans.clone()));
    w.set_profiles(ModelRc::from(ui.profiles.clone()));
    w.global::<Theme>().set_light(light_theme());

    let a = app.clone();
    w.global::<I18n>()
        .on_tr(move |_, text| crate::i18n::tr(&a, &text).into());

    let a = app.clone();
    w.on_pick_effect(move |kind| {
        let default = VIEW.with_borrow(|v| {
            v.snap["catalog"]
                .as_array()
                .and_then(|c| c.iter().find(|e| e["default"]["kind"] == kind.as_str()))
                .map(|e| e["default"].clone())
        });
        if let Some(effect) = default {
            send_effect(&a, effect);
        }
    });
    let a = app.clone();
    w.on_set_param(move |name, v| edit_effect(&a, &name, json!(v)));
    let a = app.clone();
    w.on_set_param_bool(move |name, v| edit_effect(&a, &name, json!(v)));
    let a = app.clone();
    w.on_set_param_hex(move |name, text| {
        let t = text.trim().trim_start_matches('#');
        match u32::from_str_radix(t, 16) {
            Ok(v) if t.len() == 6 => {
                edit_effect(&a, &name, json!([(v >> 16) & 255, (v >> 8) & 255, v & 255]))
            }
            _ => report(Err(format!("{text} is not a colour like #FF6A00"))),
        }
    });
    let a = app.clone();
    w.on_set_param_choice(move |name, i| {
        let option = VIEW.with_borrow(|v| {
            catalog_entry(v).and_then(|e| {
                e["params"]
                    .as_array()?
                    .iter()
                    .find(|p| p["name"] == name.as_str())?["control"]["options"]
                    .get(i as usize)
                    .cloned()
            })
        });
        if let Some(option) = option {
            edit_effect(&a, &name, option);
        }
    });
    let a = app.clone();
    w.on_reset_effect(move || {
        if let Some(e) = VIEW.with_borrow(catalog_entry) {
            send_effect(&a, e["default"].clone());
        }
    });
    let a = app.clone();
    w.on_randomise_effect(move || {
        let effect = VIEW.with_borrow_mut(|v| {
            let entry = catalog_entry(v)?;
            let mut effect = v.effect.clone();
            for p in entry["params"].as_array()? {
                let name = p["name"].as_str()?;
                let c = &p["control"];
                let r = random(&mut v.seed);
                effect[name] = match c["type"].as_str()? {
                    "range" => {
                        let (lo, hi, step) = (
                            c["min"].as_f64()? as f32,
                            c["max"].as_f64()? as f32,
                            c["step"].as_f64()? as f32,
                        );
                        json!((lo + (r * (hi - lo) / step).round() * step).min(hi))
                    }
                    "color" => json!(hue(r)),
                    "toggle" => json!(r > 0.5),
                    "choice" => {
                        let o = c["options"].as_array()?;
                        o[((r * o.len() as f32) as usize).min(o.len() - 1)].clone()
                    }
                    _ => continue,
                };
            }
            Some(effect)
        });
        if let Some(effect) = effect {
            send_effect(&a, effect);
        }
    });
    let a = app.clone();
    w.on_set_brightness(move |v| report(commands::set_brightness(a.clone(), shared(&a), v)));
    let a = app.clone();
    w.on_toggle_pause(move || {
        let paused = VIEW.with_borrow(|v| v.snap["paused"].as_bool().unwrap_or(false));
        report(commands::set_paused(a.clone(), shared(&a), !paused));
    });
    let a = app.clone();
    w.on_select_device(move |id| {
        VIEW.with_borrow_mut(|v| {
            v.canvas.selected = Some(id.to_string());
            with_ui(|ui| {
                apply_devices(ui, v);
                redraw(ui, v);
            });
        });
        let _ = &a;
    });
    let a = app.clone();
    w.on_set_device_enabled(move |id, on| {
        if let Some(d) = VIEW.with_borrow(|v| v.canvas.find(&id).cloned()) {
            send_device(&a, &d, on, d.brightness, d.rect);
        }
    });
    let a = app.clone();
    w.on_set_device_brightness(move |id, b| {
        if let Some(d) = VIEW.with_borrow(|v| v.canvas.find(&id).cloned()) {
            send_device(&a, &d, d.enabled, b, d.rect);
        }
    });
    let a = app.clone();
    w.on_detect_now(move || report(commands::detect(a.state::<Jobs>())));
    let a = app.clone();
    w.on_rescan(move || report(commands::rescan(a.state::<Jobs>())));
    let a = app.clone();
    w.on_reset_layout(move || report(commands::reset_layout(a.clone(), shared(&a))));
    let a = app.clone();
    w.on_set_auto_detect(move |on| edit_settings(&a, |s| s["auto_detect"] = json!(on)));
    let a = app.clone();
    w.on_set_close_to_tray(move |on| edit_settings(&a, |s| s["close_to_tray"] = json!(on)));
    let a = app.clone();
    w.on_set_start_minimized(move |on| edit_settings(&a, |s| s["start_minimized"] = json!(on)));
    let a = app.clone();
    w.on_set_language(move |i| {
        let code = match i {
            0 => "auto",
            i => LANGS.get(i as usize - 1).map_or("auto", |l| l.0),
        };
        edit_settings(&a, |s| s["language"] = json!(code));
    });
    let a = app.clone();
    w.on_load_profile(move |name| {
        report(commands::load_profile(a.clone(), shared(&a), name.to_string()))
    });
    let a = app.clone();
    w.on_save_profile(move |name| {
        report(commands::save_profile(a.clone(), shared(&a), name.to_string()))
    });

    w.on_canvas_down(move |x, y| {
        VIEW.with_borrow_mut(|v| {
            v.canvas.down(x, y);
            with_ui(|ui| {
                apply_devices(ui, v);
                redraw(ui, v);
            });
        });
    });
    w.on_canvas_move(move |x, y, pressed| {
        VIEW.with_borrow_mut(|v| {
            if v.canvas.moved(x, y, pressed) {
                with_ui(|ui| redraw(ui, v));
            }
        });
    });
    let a = app.clone();
    w.on_canvas_up(move || {
        if let Some((d, rect)) = VIEW.with_borrow_mut(|v| v.canvas.up()) {
            send_device(&a, &d, d.enabled, d.brightness, rect);
        }
    });
    let a = app.clone();
    w.on_canvas_key(move |key, ctrl, shift| {
        match VIEW.with_borrow(|v| v.canvas.key(&key, ctrl, shift)) {
            Some((d, rect)) => {
                send_device(&a, &d, d.enabled, d.brightness, rect);
                true
            }
            None => false,
        }
    });

    let a = app.clone();
    w.window().on_close_requested(move || {
        let to_tray = VIEW.with_borrow(|v| v.snap["settings"]["close_to_tray"].as_bool().unwrap_or(true));
        a.set_shown(false, false);
        if to_tray {
            Timer::single_shot(Duration::ZERO, || {
                UI.with_borrow_mut(|slot| slot.take());
            });
        } else {
            crate::quit(&a);
        }
        CloseRequestResponse::HideWindow
    });

    let a = app.clone();
    let weak = w.as_weak();
    ui._poll.start(TimerMode::Repeated, Duration::from_millis(500), move || {
        if let Some(w) = weak.upgrade() {
            let minimized = w.window().is_minimized();
            a.set_shown(w.window().is_visible(), minimized);
        }
    });
    Ok(ui)
}

fn apply_snapshot(app: &AppHandle, ui: &Ui, view: &mut View) {
    let snap = &view.snap;
    let w = &ui.win;
    let s = |v: &Value| SharedString::from(v.as_str().unwrap_or_default());

    let settings = &snap["settings"];
    let choice = settings["language"].as_str().unwrap_or("auto");
    w.global::<I18n>().set_lang(crate::i18n::language(choice).into());
    let mut langs: Vec<SharedString> = vec![crate::i18n::tr(app, "Automatic").into()];
    langs.extend(LANGS.iter().map(|l| SharedString::from(l.1)));
    w.set_languages(ModelRc::new(VecModel::from(langs)));
    w.set_language_index(
        LANGS
            .iter()
            .position(|l| l.0 == choice)
            .map_or(0, |i| i as i32 + 1),
    );
    w.set_auto_detect(settings["auto_detect"].as_bool().unwrap_or(false));
    w.set_close_to_tray(settings["close_to_tray"].as_bool().unwrap_or(true));
    w.set_start_minimized(settings["start_minimized"].as_bool().unwrap_or(false));

    let catalog = snap["catalog"].as_array().cloned().unwrap_or_default();
    sync(
        &ui.effects,
        catalog
            .iter()
            .map(|e| EffectItem {
                kind: s(&e["default"]["kind"]),
                label: s(&e["label"]),
            })
            .collect(),
    );
    w.set_effect_kind(s(&view.effect["kind"]));
    let params = catalog_entry(view)
        .and_then(|e| e["params"].as_array().cloned())
        .unwrap_or_default();
    sync(
        &ui.params,
        params
            .iter()
            .map(|p| param(p, &view.effect[p["name"].as_str().unwrap_or_default()]))
            .collect(),
    );

    w.set_brightness(snap["brightness"].as_f64().unwrap_or(1.0) as f32);
    let paused = snap["paused"].as_bool().unwrap_or(false);
    w.set_paused(paused);

    view.canvas.devices = snap["devices"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    Some(Dev {
                        id: d["id"].as_str()?.into(),
                        name: d["name"].as_str()?.into(),
                        kind: d["kind"].as_str().unwrap_or("other").into(),
                        rect: serde_json::from_value(d["rect"].clone()).ok()?,
                        grid: serde_json::from_value(d["grid"].clone()).unwrap_or([1, 1]),
                        leds: serde_json::from_value(d["leds"].clone()).unwrap_or_default(),
                        enabled: d["enabled"].as_bool().unwrap_or(true),
                        brightness: d["brightness"].as_f64().unwrap_or(1.0) as f32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    apply_devices(ui, view);

    let snap = &view.snap;
    sync(
        &ui.detected,
        snap["detected"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|d| Line {
                        title: s(&d["name"]),
                        detail: [&d["kind"], &d["state"], &d["note"]]
                            .iter()
                            .filter_map(|v| v.as_str().filter(|t| !t.is_empty()))
                            .collect::<Vec<_>>()
                            .join(" · ")
                            .into(),
                        ok: d["state"] == "active",
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    sync(
        &ui.backends,
        snap["backends"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|b| {
                        let active = b["active"].as_bool().unwrap_or(false);
                        let detail = match b["error"].as_str() {
                            Some(e) => e.to_string(),
                            None if active => format!("{} devices", b["devices"]),
                            None => "Off".into(),
                        };
                        Line {
                            title: s(&b["name"]),
                            detail: detail.into(),
                            ok: b["error"].is_null(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    sync(
        &ui.profiles,
        snap["profiles"]
            .as_array()
            .map(|l| l.iter().map(s).collect())
            .unwrap_or_default(),
    );
    w.set_active_profile(s(&snap["active_profile"]));
    let t = |x: &str| crate::i18n::tr(app, x);
    let mut status = vec![
        t(if paused { "Paused" } else { "Running" }),
        format!("{} devices", view.canvas.devices.len()),
    ];
    if let Some(p) = snap["active_profile"].as_str() {
        status.push(p.to_string());
    }
    w.set_status(status.join("   ").into());
    redraw(ui, view);
}

fn param(p: &Value, value: &Value) -> Param {
    let c = &p["control"];
    let mut out = Param {
        name: p["name"].as_str().unwrap_or_default().into(),
        label: p["label"].as_str().unwrap_or_default().into(),
        control: "other".into(),
        ..Default::default()
    };
    match c["type"].as_str().unwrap_or_default() {
        "range" => {
            out.control = "range".into();
            out.min = c["min"].as_f64().unwrap_or(0.0) as f32;
            out.max = c["max"].as_f64().unwrap_or(1.0) as f32;
            out.step = c["step"].as_f64().unwrap_or(0.01) as f32;
            out.value = value.as_f64().unwrap_or(0.0) as f32;
        }
        "color" => {
            let [r, g, b]: [u8; 3] = serde_json::from_value(value.clone()).unwrap_or_default();
            out.control = "color".into();
            out.color = slint::Color::from_rgb_u8(r, g, b);
            out.hex = format!("#{r:02X}{g:02X}{b:02X}").into();
        }
        "toggle" => {
            out.control = "toggle".into();
            out.on = value.as_bool().unwrap_or(false);
        }
        "choice" => {
            let options: Vec<SharedString> = c["options"]
                .as_array()
                .map(|o| o.iter().map(|v| v.as_str().unwrap_or_default().into()).collect())
                .unwrap_or_default();
            out.control = "choice".into();
            out.index = options
                .iter()
                .position(|o| Some(o.as_str()) == value.as_str())
                .unwrap_or(0) as i32;
            out.options = ModelRc::new(VecModel::from(options));
        }
        _ => {}
    }
    out
}

fn apply_devices(ui: &Ui, view: &View) {
    let backends: std::collections::HashMap<&str, &str> = view.snap["devices"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|d| Some((d["id"].as_str()?, d["backend"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    sync(
        &ui.devices,
        view.canvas
            .devices
            .iter()
            .map(|d| DeviceItem {
                id: d.id.as_str().into(),
                name: d.name.as_str().into(),
                backend: backends.get(d.id.as_str()).copied().unwrap_or_default().into(),
                enabled: d.enabled,
                brightness: d.brightness,
                selected: view.canvas.selected.as_ref() == Some(&d.id),
            })
            .collect(),
    );
}

fn apply_fans(ui: &Ui, view: &View) {
    let f = &view.fans;
    let status = match f["service"].as_str() {
        Some("running") => format!(
            "Fan service running{}",
            f["version"].as_str().map(|v| format!(", version {v}")).unwrap_or_default()
        ),
        Some("error") => format!("Fan service error: {}", f["detail"].as_str().unwrap_or("")),
        Some(_) => "Fan service stopped".into(),
        None => "Asking the fan service…".into(),
    };
    ui.win.set_fan_status(status.into());
    let mut rows: Vec<Line> = Vec::new();
    for s in f["sensors"].as_array().into_iter().flatten() {
        rows.push(Line {
            title: s["name"].as_str().unwrap_or_default().into(),
            detail: s["value"]
                .as_f64()
                .map_or("–".into(), |v| format!("{v:.1} °C"))
                .into(),
            ok: true,
        });
    }
    for fan in f["fans"].as_array().into_iter().flatten() {
        let rpm = fan["rpm"].as_u64().map_or("–".into(), |r| format!("{r} rpm"));
        let duty = fan["duty"]
            .as_f64()
            .map_or("auto".into(), |d| format!("{:.0}%", d * 100.0));
        rows.push(Line {
            title: fan["name"].as_str().unwrap_or_default().into(),
            detail: format!("{rpm}  {duty}").into(),
            ok: fan["error"].is_null(),
        });
    }
    sync(&ui.fans, rows);
}

fn redraw(ui: &Ui, view: &mut View) {
    let w = ui.win.get_canvas_width();
    let h = ui.win.get_canvas_height();
    if w < 50.0 || h < 50.0 {
        return;
    }
    let scale = ui.win.window().scale_factor();
    let Some((pm, labels)) = view.canvas.render(w, h, scale) else {
        return;
    };
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pm.data(), pm.width(), pm.height());
    ui.win.set_canvas(Image::from_rgba8_premultiplied(buf));
    sync(
        &ui.labels,
        labels
            .into_iter()
            .map(|l| CanvasLabel {
                x: l.x,
                y: l.y,
                width: l.width,
                text: l.text.into(),
                strong: l.strong,
            })
            .collect(),
    );
}
