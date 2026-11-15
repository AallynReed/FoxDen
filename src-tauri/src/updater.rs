//! New releases from GitHub: checks a minute after start, then every six hours, and tells the user.
//! With `auto_update` on it installs by itself once the app is in the tray or a minute has passed.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::state::{Shared, lock};

const FIRST: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const GRACE: Duration = Duration::from_secs(60);
const DEV: &str = "development builds do not install updates";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Info {
    pub version: String,
    pub notes: String,
    /// Installs by itself unless the user says Later.
    pub auto: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

static FOUND: Mutex<Option<(Update, Info)>> = Mutex::new(None);
/// The version the user put off with Later.
static LATER: Mutex<Option<String>> = Mutex::new(None);
static INSTALLING: Mutex<bool> = Mutex::new(false);

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the automatic install goes ahead now.
pub fn due(auto: bool, put_off: bool, idle: bool, waited: Duration) -> bool {
    auto && !put_off && (idle || waited >= GRACE)
}

pub fn toast_text(version: &str, auto: bool) -> String {
    if auto {
        format!("Updating to {version}")
    } else {
        format!("Version {version} is available")
    }
}

pub fn start(app: &AppHandle, shared: Shared) {
    if cfg!(debug_assertions) {
        fake(app);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(FIRST).await;
        loop {
            if let Ok(Some(info)) = check(&app, &shared).await
                && info.auto
            {
                let toasted = Instant::now();
                loop {
                    let put_off = guard(&LATER).as_deref() == Some(info.version.as_str());
                    let idle = app
                        .get_webview_window("main")
                        .is_none_or(|w| !w.is_visible().unwrap_or(false));
                    if put_off || *guard(&INSTALLING) {
                        break;
                    }
                    if due(true, put_off, idle, toasted.elapsed()) {
                        let _ = install(&app).await;
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
            sleep(EVERY).await;
        }
    });
}

/// Test-only path for development builds: `FOXRGB_FAKE_UPDATE=<version>` shows the UI for a release that is not there.
fn fake(app: &AppHandle) {
    let Ok(version) = std::env::var("FOXRGB_FAKE_UPDATE") else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_secs(3)).await;
        let info = Info {
            version,
            notes: "A test release. Nothing will be installed.".into(),
            auto: false,
        };
        announce(&app, &info);
    });
}

fn announce(app: &AppHandle, info: &Info) {
    let _ = app.emit("update", info);
    let _ = app
        .notification()
        .builder()
        .title("FoxRGB update")
        .body(toast_text(&info.version, info.auto))
        .show();
}

/// Asks GitHub; a newer release is announced once per version.
async fn check(app: &AppHandle, shared: &Shared) -> Result<Option<Info>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let info = Info {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
        auto: lock(shared).config.settings.auto_update,
    };
    let known = guard(&FOUND)
        .replace((update, info.clone()))
        .is_some_and(|(_, old)| old == info);
    if !known {
        announce(app, &info);
    }
    Ok(Some(info))
}

async fn install(app: &AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err(DEV.into());
    }
    let update = guard(&FOUND)
        .as_ref()
        .map(|(u, _)| u.clone())
        .ok_or("no update to install")?;
    {
        let mut busy = guard(&INSTALLING);
        if *busy {
            return Err("already installing".into());
        }
        *busy = true;
    }
    let mut done = 0u64;
    let result = update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = app.emit("update-progress", Progress { done, total });
            },
            || {},
        )
        .await;
    *guard(&INSTALLING) = false;
    result.map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    shared: tauri::State<'_, Shared>,
) -> Result<Option<Info>, String> {
    if cfg!(debug_assertions) {
        return Err(DEV.into());
    }
    check(&app, shared.inner()).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app).await
}

#[tauri::command]
pub fn later_update() {
    *guard(&LATER) = guard(&FOUND).as_ref().map(|(_, info)| info.version.clone());
}

#[tauri::command]
pub fn get_update() -> Option<Info> {
    guard(&FOUND).as_ref().map(|(_, info)| info.clone())
}

async fn sleep(time: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(time)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_automatic_install_waits_for_the_tray_or_a_minute() {
        let now = Duration::ZERO;
        assert!(!due(true, false, false, now));
        assert!(due(true, false, true, now));
        assert!(due(true, false, false, GRACE));
        assert!(!due(true, true, true, GRACE));
        assert!(!due(false, false, true, GRACE));
    }

    #[test]
    fn the_toast_says_what_will_happen() {
        assert_eq!(toast_text("0.4.0", true), "Updating to 0.4.0");
        assert_eq!(toast_text("0.4.0", false), "Version 0.4.0 is available");
    }
}
