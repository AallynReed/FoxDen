#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod commands;
mod config;
mod state;
mod tray;

use std::sync::{Arc, Mutex};

use tauri::{Manager, RunEvent, WindowEvent};

use state::{AppState, Shared, lock};

fn main() {
    let shared: Shared = Arc::new(Mutex::new(AppState::new(config::load(&config::dir()))));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)))
        .manage(shared.clone())
        .setup(|app| {
            let handle = app.handle();
            let shared = app.state::<Shared>().inner().clone();
            let settings = lock(&shared).config.settings.clone();
            tray::create(handle)?;
            if !settings.start_minimized {
                tray::show_window(handle);
            }
            if settings.openrgb_enabled || settings.native_drivers {
                state::reload_backends(handle, &shared);
            }
            state::spawn_render(handle.clone(), shared.clone());
            state::spawn_saver(shared);
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
            commands::get_snapshot,
            commands::set_effect,
            commands::set_brightness,
            commands::set_paused,
            commands::set_device,
            commands::reset_layout,
            commands::save_profile,
            commands::load_profile,
            commands::delete_profile,
            commands::set_settings,
            commands::rescan,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start FoxRGB");

    app.run(move |_, event| {
        if let RunEvent::Exit = event {
            state::shutdown(&shared);
        }
    });
}
