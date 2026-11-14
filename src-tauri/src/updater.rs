//! Installs new releases from GitHub: checks a minute after start, then every six hours.

use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use crate::state::{Shared, lock};

const FIRST: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub fn start(app: &AppHandle, shared: Shared) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST).await;
        loop {
            if lock(&shared).config.settings.auto_update {
                let _ = update(&app).await;
            }
            tokio_sleep(EVERY).await;
        }
    });
}

async fn update(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

async fn tokio_sleep(time: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(time)).await;
}
