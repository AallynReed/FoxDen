//! The window: one module per tab, plus the device canvas stage.

pub mod approval;
mod audio;
mod clipboard;
mod devices;
mod effects;
pub(crate) mod effects_keys;
mod fans;
mod logs;
mod profiles;
pub(crate) mod settings;
mod stage;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use tauri::{AppHandle, Emitter, Manager};

use crate::canvas::Canvas;
use crate::state::{Jobs, Shared, lock};

slint::include_modules!();

/// Backend events waiting for the UI thread; the newest of each kind wins.
static PENDING: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);
/// A window exists; without one, events wait in `PENDING` until it opens.
static OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub struct View {
    pub snap: Value,
    pub effect: Value,
    pub canvas: Canvas,
    /// The latest payload of every other event, for tabs built after it arrived.
    pub events: HashMap<String, Value>,
    pub seed: u64,
}

struct Ui {
    win: AppWindow,
    _poll: Timer,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static VIEW: RefCell<View> = RefCell::new(View::default());
}

pub fn attach(app: &AppHandle) {
    let handle = app.clone();
    app.set_sink(move |event, value| {
        let mut pending = PENDING.lock().unwrap();
        let first = pending.is_none();
        pending.get_or_insert_with(HashMap::new).insert(event.to_string(), value);
        if first && OPEN.load(Ordering::SeqCst) {
            let app = handle.clone();
            let _ = slint::invoke_from_event_loop(move || drain(&app));
        }
    });
}

fn drain(app: &AppHandle) {
    let Some(mut events) = PENDING.lock().unwrap().take() else {
        return;
    };
    VIEW.with_borrow_mut(|view| {
        if let Some(snap) = events.remove("snapshot") {
            view.effect = snap["effect"].clone();
            view.snap = snap;
            view.canvas.settle();
            with_win(|w| apply_all(w, app, view));
        }
        if let Some(frame) = events.remove("frame") {
            with_win(|w| stage::frame(w, view, &frame));
        }
        for (name, value) in events {
            view.events.insert(name.clone(), value);
            with_win(|w| dispatch(w, app, view, &name));
        }
    });
}

fn dispatch(w: &AppWindow, app: &AppHandle, view: &mut View, name: &str) {
    fans::event(w, app, view, name);
    audio::event(w, app, view, name);
    devices::event(w, app, view, name);
    effects::event(w, app, view, name);
    settings::event(w, app, view, name);
    profiles::event(w, app, view, name);
    stage::event(w, app, view, name);
}

fn apply_all(w: &AppWindow, app: &AppHandle, view: &mut View) {
    settings::apply(w, app, view);
    effects::apply(w, app, view);
    devices::apply(w, app, view);
    profiles::apply(w, app, view);
    fans::apply(w, app, view);
    audio::apply(w, app, view);
    stage::apply(w, app, view);
}

/// Applies fan curves the person approved, through the Fans tab.
pub fn apply_proposed_fans(config: foxden_fan::FanConfig, reply: std::sync::mpsc::Sender<Result<(), String>>) {
    fans::apply_proposed(config, reply);
}

pub fn with_win(f: impl FnOnce(&AppWindow)) {
    UI.with_borrow(|ui| {
        if let Some(ui) = ui {
            f(&ui.win);
        }
    });
}

/// Runs `f` with the view, from a callback (never from inside `drain`).
pub fn with_view<R>(f: impl FnOnce(&mut View) -> R) -> R {
    VIEW.with_borrow_mut(f)
}

/// Re-applies everything after a local change, such as a selection.
pub fn refresh(app: &AppHandle) {
    VIEW.with_borrow_mut(|view| with_win(|w| apply_all(w, app, view)));
}

pub fn show(app: &AppHandle) {
    let exists = UI.with_borrow(|ui| {
        ui.as_ref().is_some_and(|ui| {
            let _ = ui.win.show();
            ui.win.window().set_minimized(false);
            true
        })
    });
    if exists {
        app.set_shown(true, false);
        settings::show_whats_new(app);
        settings::prompt_update(app);
        return;
    }
    // Open before draining, so an event that lands in between still wakes the loop.
    OPEN.store(true, Ordering::SeqCst);
    drain(app);
    let win = match build(app) {
        Ok(win) => win,
        Err(e) => {
            OPEN.store(false, Ordering::SeqCst);
            crate::log::warn("app", format!("the window could not open: {e}"));
            return;
        }
    };
    let snap = serde_json::to_value(lock(&app.state::<Shared>()).snapshot()).unwrap_or_default();
    VIEW.with_borrow_mut(|view| {
        view.effect = snap["effect"].clone();
        view.snap = snap;
        view.canvas.light = light_theme();
        apply_all(&win, app, view);
    });
    let _ = win.show();
    settings::show_whats_new(app);
    settings::prompt_update(app);
    let poll = Timer::default();
    let weak = win.as_weak();
    let a = app.clone();
    poll.start(TimerMode::Repeated, Duration::from_millis(500), move || {
        if let Some(w) = weak.upgrade() {
            a.set_shown(w.window().is_visible(), w.window().is_minimized());
        }
    });
    UI.with_borrow_mut(|slot| *slot = Some(Ui { win, _poll: poll }));
    app.set_shown(true, false);
    let handle = app.clone();
    crate::state::spawn_named("foxden-fans-first", move || {
        if let Ok(state) = crate::fans::get_fans() {
            let _ = handle.emit("fans", &state);
        }
    });
}

fn build(app: &AppHandle) -> Result<AppWindow, slint::PlatformError> {
    let w = AppWindow::new()?;
    w.global::<Theme>().set_light(light_theme());
    let a = app.clone();
    w.global::<I18n>()
        .on_tr(move |_, text| crate::i18n::tr(&a, &text).into());

    effects::bind(&w, app);
    devices::bind(&w, app);
    fans::bind(&w, app);
    audio::bind(&w, app);
    profiles::bind(&w, app);
    settings::bind(&w, app);
    stage::bind(&w, app);
    approval::bind(&w);

    let a = app.clone();
    w.window().on_close_requested(move || {
        approval::window_closing();
        let to_tray = VIEW.with_borrow(|v| v.snap["settings"]["close_to_tray"].as_bool().unwrap_or(true));
        a.set_shown(false, false);
        if to_tray {
            Timer::single_shot(Duration::ZERO, || {
                OPEN.store(false, Ordering::SeqCst);
                UI.with_borrow_mut(|slot| slot.take());
                stage::stop();
                fans::stop();
            });
        } else {
            crate::quit(&a);
        }
        CloseRequestResponse::HideWindow
    });
    Ok(w)
}

pub fn light_theme() -> bool {
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

/// Updates a list model in place, so rows being dragged or focused keep their state.
pub fn fill<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    rows: Vec<T>,
    set: impl FnOnce(ModelRc<T>),
) {
    let Some(m) = current.as_any().downcast_ref::<VecModel<T>>() else {
        set(ModelRc::new(VecModel::from(rows)));
        return;
    };
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

/// Logs a failed command and shows it in the status bar.
pub fn report(r: Result<(), String>) {
    if let Err(e) = r {
        crate::log::warn("ui", e.clone());
        let e = if refused(&e) { REFUSED.to_string() } else { e };
        with_win(|w| stage::show_error(w, &e));
    }
}

const REFUSED: &str = "Only the person signed in at this PC's screen, or an administrator, can change this.";

/// The service's answer to a change asked from another Windows session.
pub fn refused(e: &str) -> bool {
    e.contains("only the person signed in at this PC")
}

pub fn shared(app: &AppHandle) -> tauri::State<'static, Shared> {
    app.state::<Shared>()
}

pub fn jobs(app: &AppHandle) -> tauri::State<'static, Jobs> {
    app.state::<Jobs>()
}

pub fn tr(app: &AppHandle, text: &str) -> String {
    crate::i18n::tr(app, text)
}

pub fn s(v: &Value) -> slint::SharedString {
    v.as_str().unwrap_or_default().into()
}

/// Changes some settings and saves them through the same command the web UI used.
pub fn edit_settings(app: &AppHandle, change: impl FnOnce(&mut Value)) {
    let mut settings = serde_json::to_value(&crate::state::lock(&shared(app)).config.settings).unwrap_or_default();
    change(&mut settings);
    match serde_json::from_value(settings) {
        Ok(settings) => report(crate::commands::set_settings(
            app.clone(),
            shared(app),
            jobs(app),
            settings,
        )),
        Err(e) => report(Err(e.to_string())),
    }
}
