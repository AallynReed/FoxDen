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
    let builder = |guid: Option<u128>| {
        let mut b = TrayIconBuilder::new()
            .with_id("main")
            .with_tooltip("FoxDen")
            .with_menu(Box::new(menu(app, &first)))
            .with_menu_on_left_click(false);
        if let Ok(icon) = Icon::from_resource(1, None) {
            b = b.with_icon(icon);
        }
        if let Some(g) = guid {
            b = b.with_guid(g);
        }
        b.build()
    };
    forget_icon();
    match builder(Some(guid())).or_else(|_| builder(None)) {
        Ok(tray) => TRAY.with_borrow_mut(|t| *t = Some(tray)),
        Err(e) => crate::log::warn("tray", e.to_string()),
    }
    *SHOWN.lock().unwrap() = Some(first);

    let handle = app.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        // Hovering sends a stream of moves; only clicks are acted on.
        if !matches!(event, TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. }) {
            return;
        }
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

/// A fixed identity for the main icon, so a balloon can be shown on it. Tied to the exe path like Windows ties it.
fn guid() -> u128 {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut h: u128 = 0x2f6b3c1d9a4e4f07b1c86d5e3a917c42;
    for b in exe.to_string_lossy().to_lowercase().bytes() {
        h = (h ^ b as u128).wrapping_mul(0x0000000001000000000000000000013b);
    }
    h
}

fn notify_data() -> windows_sys::Win32::UI::Shell::NOTIFYICONDATAW {
    use windows_sys::Win32::UI::Shell::{NIF_GUID, NOTIFYICONDATAW};
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.uFlags = NIF_GUID;
    nid.guidItem = windows_sys::core::GUID::from_u128(guid());
    nid
}

/// Removes an icon a force-closed FoxDen left behind under the same identity.
fn forget_icon() {
    use windows_sys::Win32::UI::Shell::{NIM_DELETE, Shell_NotifyIconW};
    unsafe { Shell_NotifyIconW(NIM_DELETE, &notify_data()) };
}

fn copy_wide(into: &mut [u16], text: &str) {
    let n = into.len() - 1;
    for (slot, c) in into.iter_mut().zip(text.encode_utf16().take(n).chain(std::iter::repeat(0))) {
        *slot = c;
    }
}

/// A Windows notification from the tray icon.
pub fn notify(title: &str, body: &str) {
    use windows_sys::Win32::UI::Shell::{
        NIF_INFO, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIM_MODIFY, Shell_NotifyIconW,
    };
    let mut nid = notify_data();
    nid.uFlags |= NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
    copy_wide(&mut nid.szInfoTitle, title);
    copy_wide(&mut nid.szInfo, body);
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) } == 0 {
        crate::log::info("notify", format!("{title}: {body}"));
    }
}
