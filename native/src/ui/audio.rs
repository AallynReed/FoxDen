use tauri::AppHandle;

use super::{AppWindow, View};

pub fn bind(_w: &AppWindow, _app: &AppHandle) {}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn apply(_w: &AppWindow, _app: &AppHandle, _view: &mut View) {}
