#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

#[path = "../../src-tauri/src/apps.rs"]
mod apps;
#[path = "../../src-tauri/src/autostart.rs"]
mod autostart;
#[path = "../../src-tauri/src/battery.rs"]
mod battery;
#[path = "../../src-tauri/src/commands.rs"]
mod commands;
#[path = "../../src-tauri/src/config.rs"]
mod config;
#[path = "../../src-tauri/src/conflicts.rs"]
mod conflicts;
#[path = "../../src-tauri/src/detect.rs"]
mod detect;
#[path = "../../src-tauri/src/device_info.rs"]
mod device_info;
#[path = "../../src-tauri/src/fans.rs"]
mod fans;
#[path = "../../src-tauri/src/headset.rs"]
mod headset;
#[path = "../../src-tauri/src/i18n.rs"]
mod i18n;
#[path = "../../src-tauri/src/key_maps.rs"]
mod key_maps;
#[path = "../../src-tauri/src/keys.rs"]
mod keys;
#[path = "../../src-tauri/src/links.rs"]
mod links;
#[path = "../../src-tauri/src/mixer.rs"]
mod mixer;
#[path = "../../src-tauri/src/modules.rs"]
mod modules;
#[path = "../../src-tauri/src/priority.rs"]
mod priority;
#[path = "../../src-tauri/src/profiles.rs"]
mod profiles;
#[path = "../../src-tauri/src/report.rs"]
mod report;
#[path = "../../src-tauri/src/service_lights.rs"]
mod service_lights;
#[path = "../../src-tauri/src/share.rs"]
mod share;
#[path = "../../src-tauri/src/state.rs"]
mod state;
#[path = "../../src-tauri/src/volumes.rs"]
mod volumes;
mod canvas;
mod changelog;
mod log;
mod tray;
mod ui;
mod updater;

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use state::{AppState, Shared, lock};

macro_rules! langs {
    ($($l:literal),*) => {
        fn lang_file(path: &str) -> Option<Vec<u8>> {
            match path {
                $(concat!("lang/", $l, ".json") => Some(include_bytes!(concat!("../../ui/lang/", $l, ".json")).to_vec()),)*
                _ => None,
            }
        }
    };
}

langs!(
    "cs", "de", "en", "es", "fr", "it", "ja", "ko", "nl", "pl", "pt-PT", "ru", "tr", "uk", "zh-Hans",
    "zh-Hant"
);

const SHOW_EVENT: &str = "Local\\FoxDenNativeShow\0";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A second launch asks the first to show its window, then leaves.
fn already_running() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};
    unsafe {
        CreateMutexW(std::ptr::null(), 0, wide("Local\\FoxDenNative\0").as_ptr());
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        let event = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT).as_ptr());
        if !event.is_null() {
            SetEvent(event);
        }
        true
    }
}

fn watch_show(app: AppHandle) {
    use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT).as_ptr()) } as usize;
    if event == 0 {
        return;
    }
    state::spawn_named("foxden-show", move || {
        loop {
            unsafe { WaitForSingleObject(event as _, INFINITE) };
            tray::show_window(&app);
        }
    });
}

pub fn quit(app: &AppHandle) {
    app.fire_exit();
    if let Some(mixer) = app.try_state::<mixer::Mixer>() {
        mixer.shutdown();
    }
    foxden_headset::chatmix_off();
    state::shutdown(&app.state::<Shared>());
    let _ = slint::quit_event_loop();
}

fn main() {
    if already_running() {
        return;
    }
    report::catch_crashes();
    let _ = rustls::crypto::ring::default_provider().install_default();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
        .expect("no window backend");

    let shared: Shared = Arc::new(Mutex::new(AppState::new(config::load(&config::dir()))));
    let app = AppHandle::new();
    app.manage(shared.clone());
    app.set_main_thread(|f| {
        let _ = slint::invoke_from_event_loop(f);
    });
    app.set_assets(lang_file);
    let handle = app.clone();
    app.set_notify(move |title, body| {
        let (title, body) = (title.to_string(), body.to_string());
        let _ = handle.run_on_main_thread(move || tray::notify(&title, &body));
    });
    log::attach(&app);
    ui::attach(&app);

    let (settings, fan_backup) = {
        let state = lock(&shared);
        (state.config.settings.clone(), state.config.fan_backup.clone())
    };
    report::set_enabled(settings.send_reports);
    i18n::set_choice(&settings.language);
    report::send_last_crash();
    log::info("app", format!("FoxDen {} started (native)", env!("CARGO_PKG_VERSION")));
    tray::create(&app);
    watch_show(app.clone());
    let at_sign_in = std::env::args().any(|arg| arg == autostart::TRAY_ARG);
    if !cfg!(debug_assertions) {
        let _ = autostart::set(settings.autostart);
    }
    app.manage(state::spawn_worker(app.clone(), shared.clone()));
    app.manage(mixer::Mixer::start(shared.clone()));
    headset::spawn_dial(app.clone());
    priority::spawn(app.clone());
    state::spawn_render(app.clone(), shared.clone());
    apps::spawn(app.clone(), shared.clone());
    battery::spawn(app.clone(), shared.clone());
    updater::start(&app, shared.clone());
    state::spawn_saver(shared);
    fans::start(&app, settings.fans_module, fan_backup);
    if !settings.start_minimized && !at_sign_in {
        ui::show(&app);
    }
    let _ = slint::run_event_loop_until_quit();
}
