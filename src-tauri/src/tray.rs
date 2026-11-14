use std::sync::Mutex;

use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::{self, Shared, Snapshot, lock};

const ID: &str = "main";
const PROFILE: &str = "profile:";

/// What the menu depends on: paused, profile names, active profile.
type Shown = (bool, Vec<String>, Option<String>);

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

fn shown(snapshot: &Snapshot) -> Shown {
    (snapshot.paused, snapshot.profiles.clone(), snapshot.active_profile.clone())
}

fn menu(app: &AppHandle, (paused, profiles, active): &Shown) -> tauri::Result<Menu<Wry>> {
    let mut list = SubmenuBuilder::new(app, "Profiles").enabled(!profiles.is_empty());
    for name in profiles {
        let item = CheckMenuItemBuilder::with_id(format!("{PROFILE}{name}"), name)
            .checked(active.as_ref() == Some(name))
            .build(app)?;
        list = list.item(&item);
    }
    MenuBuilder::new(app)
        .text("show", "Show")
        .text("pause", if *paused { "Resume" } else { "Pause" })
        .item(&list.build()?)
        .separator()
        .text("quit", "Quit")
        .build()
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let first = shown(&lock(&app.state::<Shared>()).snapshot());
    let mut tray = TrayIconBuilder::with_id(ID)
        .tooltip("FoxRGB")
        .menu(&menu(app, &first)?)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    *SHOWN.lock().unwrap() = Some(first);
    app.on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    Ok(())
}

/// Rebuilds the menu when what it shows has changed.
pub fn refresh(app: &AppHandle, snapshot: &Snapshot) {
    let now = shown(snapshot);
    {
        let mut last = SHOWN.lock().unwrap();
        if last.as_ref() == Some(&now) || last.is_none() {
            return;
        }
        *last = Some(now.clone());
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (handle.tray_by_id(ID), menu(&handle, &now)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let shared = app.state::<Shared>();
    let result = match id {
        "show" => {
            show_window(app);
            Ok(())
        }
        "pause" => state::update(app, &shared, |s| {
            s.engine.paused = !s.engine.paused;
            Ok(())
        }),
        "quit" => {
            app.exit(0);
            Ok(())
        }
        _ => match id.strip_prefix(PROFILE) {
            Some(name) => {
                // The click flipped the item's check mark, so the menu has to be rebuilt.
                *SHOWN.lock().unwrap() = Some(Shown::default());
                state::update(app, &shared, |s| s.load_profile(name))
            }
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        eprintln!("tray: {e}");
    }
}
