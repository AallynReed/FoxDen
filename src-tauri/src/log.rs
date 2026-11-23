//! What happened to devices and backends, for the log window. Kept in memory only.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::lock;

const KEEP: usize = 1000;
const WINDOW: &str = "logs";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    /// Unix time in milliseconds.
    pub at: u64,
    pub level: Level,
    pub source: String,
    pub text: String,
}

static LOG: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());
static APP: OnceLock<AppHandle> = OnceLock::new();

fn guard() -> MutexGuard<'static, VecDeque<Entry>> {
    lock(&LOG)
}

pub fn attach(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

pub fn add(level: Level, source: &str, text: impl Into<String>) {
    let entry = Entry {
        at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
        level,
        source: source.into(),
        text: text.into(),
    };
    if cfg!(debug_assertions) {
        eprintln!("{:?} {}: {}", entry.level, entry.source, entry.text);
    }
    keep(&mut guard(), entry.clone());
    if let Some(app) = APP.get() {
        let _ = app.emit("log", &entry);
    }
    if level == Level::Error {
        crate::report::error(&entry.source, &entry.text);
    }
}

fn keep(log: &mut VecDeque<Entry>, entry: Entry) {
    if log.len() >= KEEP {
        log.pop_front();
    }
    log.push_back(entry);
}

pub fn info(source: &str, text: impl Into<String>) {
    add(Level::Info, source, text);
}

pub fn warn(source: &str, text: impl Into<String>) {
    add(Level::Warn, source, text);
}

pub fn error(source: &str, text: impl Into<String>) {
    add(Level::Error, source, text);
}

#[tauri::command]
pub fn get_logs() -> Vec<Entry> {
    guard().iter().cloned().collect()
}

#[tauri::command]
pub fn clear_logs() {
    guard().clear();
}

#[tauri::command]
pub async fn open_logs(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        return window.set_focus().map_err(|e| e.to_string());
    }
    WebviewWindowBuilder::new(&app, WINDOW, WebviewUrl::App("logs.html".into()))
        .title("FoxDen logs")
        .inner_size(760.0, 480.0)
        .min_inner_size(420.0, 260.0)
        .data_directory(crate::config::webview_dir())
        // Every window in one data folder must share the browser flags, or WebView2 refuses it.
        .additional_browser_args(crate::BROWSER_ARGS)
        .build()
        .map(|w| crate::no_browser_keys(&w))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_newest_entries() {
        let mut logs = VecDeque::new();
        for i in 0..KEEP + 5 {
            let entry = Entry {
                at: 0,
                level: Level::Info,
                source: "test".into(),
                text: i.to_string(),
            };
            keep(&mut logs, entry);
        }
        assert_eq!(logs.len(), KEEP);
        assert_eq!(logs[0].text, "5");
        assert_eq!(logs.back().unwrap().text, (KEEP + 4).to_string());
    }
}
