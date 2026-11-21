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
mod keys;
mod links;
mod log;
mod mixer;
mod modules;
mod report;
mod service_lights;
mod state;
mod tray;
mod updater;

use std::sync::{Arc, Mutex};

use tauri::{Manager, RunEvent, WebviewWindowBuilder, WindowEvent};

use state::{AppState, Shared, lock};

/// WebView2 flags: wry's defaults, which this replaces, plus no Edge background traffic.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AutofillServerCommunication,OptimizationHints,MediaRouter      --disable-background-networking --disable-component-update --disable-domain-reliability      --disable-client-side-phishing-detection --disable-sync --no-pings --metrics-recording-only";

fn main() {
    report::catch_crashes();
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
            // Made here and not by Tauri, so that the web view's data goes where the rest does.
            for window in &app.config().app.windows {
                WebviewWindowBuilder::from_config(handle, window)?
                    .data_directory(config::webview_dir())
                    .additional_browser_args(BROWSER_ARGS)
                    .build()?;
            }
            let shared = app.state::<Shared>().inner().clone();
            let (settings, fan_backup) = {
                let state = lock(&shared);
                (
                    state.config.settings.clone(),
                    state.config.fan_backup.clone(),
                )
            };
            report::set_enabled(settings.send_reports);
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
            state::spawn_render(handle.clone(), shared.clone());
            apps::spawn(handle.clone(), shared.clone());
            battery::spawn(handle.clone(), shared.clone());
            updater::start(handle, shared.clone());
            state::spawn_saver(shared);
            fans::start(handle, settings.fans_module, fan_backup);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let shared = window.state::<Shared>();
                if lock(&shared).config.settings.close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            apps::list_apps,
            commands::get_snapshot,
            commands::set_effect,
            commands::set_layers,
            commands::set_override,
            commands::key_event,
            commands::set_brightness,
            commands::set_paused,
            commands::set_device,
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
            mixer::get_mixer,
            mixer::mixer_set,
            mixer::get_mixer_levels,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start FoxDen");

    app.run(move |handle, event| {
        if let RunEvent::Exit = event {
            if let Some(mixer) = handle.try_state::<mixer::Mixer>() {
                mixer.shutdown();
            }
            state::shutdown(&shared);
        }
    });
}
