use std::cell::RefCell;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::state::{self, Shared, Snapshot, lock};

const PROFILE: &str = "profile:";

type Shown = (bool, Vec<String>, Option<String>, String);

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

fn shown(snapshot: &Snapshot) -> Shown {
    (
        snapshot.paused,
        snapshot.profiles.clone(),
        snapshot.active_profile.clone(),
        snapshot.language(),
    )
}

fn menu(app: &AppHandle, (paused, profiles, active, _): &Shown) -> Menu {
    let t = |s: &str| crate::i18n::tr(app, s);
    let list = Submenu::new(t("Profiles"), !profiles.is_empty());
    for name in profiles {
        let item = CheckMenuItem::with_id(
            format!("{PROFILE}{name}"),
            name,
            true,
            active.as_ref() == Some(name),
            None,
        );
        let _ = list.append(&item);
    }
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &MenuItem::with_id("show", t("Show"), true, None),
        &MenuItem::with_id("pause", t(if *paused { "Resume" } else { "Pause" }), true, None),
        &list,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("discord", t("Join the Discord"), true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", t("Quit"), true, None),
    ]);
    menu
}

pub fn create(app: &AppHandle) {
    let first = shown(&lock(&app.state::<Shared>()).snapshot());
    let mut builder = TrayIconBuilder::new()
        .with_id("main")
        .with_tooltip("FoxDen")
        .with_menu(Box::new(menu(app, &first)))
        .with_menu_on_left_click(false);
    if let Ok(icon) = Icon::from_resource(1, None) {
        builder = builder.with_icon(icon);
    }
    match builder.build() {
        Ok(tray) => TRAY.with_borrow_mut(|t| *t = Some(tray)),
        Err(e) => crate::log::warn("tray", e.to_string()),
    }
    *SHOWN.lock().unwrap() = Some(first);

    let handle = app.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        let app = handle.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let TrayIconEvent::Click {
                id,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = &event
                && id.as_ref() == "main"
            {
                crate::ui::show(&app);
            }
            app.fire_tray(event);
        });
    }));
    let handle = app.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let app = handle.clone();
        let _ = slint::invoke_from_event_loop(move || on_menu(&app, event.id().as_ref()));
    }));
}

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
        let menu = menu(&handle, &now);
        TRAY.with_borrow(|t| {
            if let Some(tray) = t {
                tray.set_menu(Some(Box::new(menu)));
            }
        });
    });
}

pub fn show_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::ui::show(&handle));
}

fn on_menu(app: &AppHandle, id: &str) {
    let shared = app.state::<Shared>();
    let result = match id {
        "show" => {
            crate::ui::show(app);
            Ok(())
        }
        "pause" => state::update(app, &shared, |s| {
            s.engine.paused = !s.engine.paused;
            Ok(())
        }),
        "discord" => crate::links::open(crate::links::DISCORD),
        "quit" => {
            crate::quit(app);
            Ok(())
        }
        _ => match id.strip_prefix(PROFILE) {
            Some(name) => {
                *SHOWN.lock().unwrap() = Some(Shown::default());
                state::update(app, &shared, |s| s.load_profile(name))
            }
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        crate::log::warn("tray", e.to_string());
    }
}
