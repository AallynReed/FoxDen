#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod autostart;
mod battery;
mod commands;
mod config;
mod conflicts;
mod detect;
mod device_info;
mod fans;
mod headset;
mod i18n;
mod key_maps;
mod keys;
mod links;
mod log;
mod mixer;
mod modules;
mod priority;
mod profiles;
mod report;
mod service_lights;
mod share;
mod state;
mod tray;
mod updater;
mod volumes;

use std::sync::{Arc, Mutex};

use tauri::{Manager, RunEvent, WebviewWindowBuilder};

use state::{AppState, Shared, lock};

/// WebView2 flags: wry's defaults, which this replaces, plus no Edge background traffic and a lighter footprint.
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AutofillServerCommunication,OptimizationHints,MediaRouter,BackForwardCache,SpareRendererForSitePerProcess      --disable-background-networking --disable-component-update --disable-domain-reliability      --disable-client-side-phishing-detection --disable-sync --no-pings --metrics-recording-only      --renderer-process-limit=1 --disable-extensions --disable-gpu-shader-disk-cache      --js-flags=--max-old-space-size=256";

/// The main window, made when it is first shown. Closing it to the tray destroys it, so the
/// web view's processes and memory go with it; the app keeps running without them.
pub fn main_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("main") {
        return Some(w);
    }
    let cfg = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")?
        .clone();
    let built = WebviewWindowBuilder::from_config(app, &cfg).and_then(|b| {
        b.data_directory(config::webview_dir())
            .additional_browser_args(BROWSER_ARGS)
            .build()
    });
    match built {
        Ok(w) => {
            no_browser_keys(&w);
            Some(w)
        }
        Err(e) => {
            log::warn("app", format!("the window could not open: {e}"));
            None
        }
    }
}

/// Turns off WebView2's browser shortcuts (reload, print, find, history, dev tools) in release builds.
pub fn no_browser_keys(window: &tauri::WebviewWindow) {
    if cfg!(debug_assertions) {
        return;
    }
    let _ = window.with_webview(|view| {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows_core::Interface;
        // SAFETY: plain WebView2 COM calls on the live controller, made on its own thread.
        unsafe {
            if let Ok(settings) = view
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|s| s.cast::<ICoreWebView2Settings3>())
            {
                let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }
    });
}

fn main() {
    report::catch_crashes();
    // reqwest is built without a TLS provider; without one every HTTPS request panics.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let shared: Shared = Arc::new(Mutex::new(AppState::new(config::load(&config::dir()))));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_window(app)
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(shared.clone())
        .setup(|app| {
            let handle = app.handle();
            log::attach(handle);
            let shared = app.state::<Shared>().inner().clone();
            let (settings, fan_backup) = {
                let state = lock(&shared);
                (
                    state.config.settings.clone(),
                    state.config.fan_backup.clone(),
                )
            };
            report::set_enabled(settings.send_reports);
            i18n::set_choice(&settings.language);
            report::send_last_crash();
            log::info(
                "app",
                format!("FoxDen {} started", env!("CARGO_PKG_VERSION")),
            );
            tray::create(handle)?;
            let at_sign_in = std::env::args().any(|arg| arg == autostart::TRAY_ARG);
            if !settings.start_minimized && !at_sign_in {
                tray::show_window(handle);
            }
            // Keeps the sign-in task pointing at this copy. Development builds leave it alone.
            if !cfg!(debug_assertions) {
                let _ = autostart::set(settings.autostart);
            }
            app.manage(state::spawn_worker(handle.clone(), shared.clone()));
            app.manage(mixer::Mixer::start(shared.clone()));
            headset::spawn_dial(handle.clone());
            priority::spawn(handle.clone());
            state::spawn_render(handle.clone(), shared.clone());
            apps::spawn(handle.clone(), shared.clone());
            battery::spawn(handle.clone(), shared.clone());
            updater::start(handle, shared.clone());
            state::spawn_saver(shared);
            fans::start(handle, settings.fans_module, fan_backup);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            apps::list_apps,
            battery::get_batteries,
            report::send_feedback,
            commands::get_snapshot,
            commands::set_effect,
            commands::set_layers,
            commands::set_override,
            commands::key_event,
            commands::set_brightness,
            commands::set_paused,
            commands::set_device,
            commands::get_hardware,
            commands::set_hardware,
            commands::reset_layout,
            commands::save_profile,
            commands::load_profile,
            commands::delete_profile,
            commands::set_settings,
            commands::rescan,
            commands::detect,
            commands::scan_network,
            commands::pair_hue,
            commands::pair_nanoleaf,
            commands::serial_ports,
            commands::get_conflicts,
            commands::close_conflicts_now,
            commands::last_conflicts,
            updater::check_update,
            updater::install_update,
            updater::later_update,
            updater::get_update,
            log::get_logs,
            log::clear_logs,
            log::open_logs,
            links::open_link,
            device_info::device_info,
            device_info::send_device_info,
            fans::get_fans,
            fans::set_fan_config,
            fans::start_service,
            fans::stop_service,
            headset::headset_list,
            headset::headset_set,
            headset::headset_eq,
            headset::headset_dial,
            mixer::get_mixer,
            priority::get_priority,
            priority::set_priority,
            profiles::sound_code,
            profiles::sound_decode,
            key_maps::save_key_map,
            key_maps::delete_key_map,
            key_maps::key_map_code,
            key_maps::key_map_decode,
            mixer::mixer_set,
            mixer::get_mixer_levels,
            mixer::virtual_mic_install,
            mixer::virtual_mic_remove,
            volumes::get_volumes,
            volumes::set_app_volume,
            volumes::set_device_volume,
            volumes::make_default_device,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start FoxDen");

    app.run(move |handle, event| {
        // The last window closing is not a quit while FoxDen lives in the tray; Quit passes a code.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = &event
            && lock(&shared).config.settings.close_to_tray
        {
            api.prevent_exit();
            return;
        }
        if let RunEvent::Exit = event {
            if let Some(mixer) = handle.try_state::<mixer::Mixer>() {
                mixer.shutdown();
            }
            foxden_headset::chatmix_off();
            state::shutdown(&shared);
        }
    });
}
