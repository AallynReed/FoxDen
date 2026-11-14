//! Client for the background service: fan status, curves, and a per-user service instance.

use foxrgb_fan::{FanConfig, FanStatus, SensorStatus};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Clone, Debug, Default, Serialize)]
pub struct FanState {
    /// "running", "stopped" or "error".
    pub service: String,
    pub detail: Option<String>,
    pub version: Option<String>,
    pub sim: bool,
    pub sensors: Vec<SensorStatus>,
    pub fans: Vec<FanStatus>,
    pub config: FanConfig,
}

const NOT_CONNECTED: &str = "the background service is not connected";

/// Starts the worker that polls the service and emits "fans".
pub fn start(_app: &AppHandle) {}

#[tauri::command]
pub fn get_fans() -> Result<FanState, String> {
    Ok(FanState { service: "stopped".into(), ..FanState::default() })
}

#[tauri::command]
pub fn set_fan_config(config: FanConfig) -> Result<(), String> {
    let _ = config;
    Err(NOT_CONNECTED.into())
}

#[tauri::command]
pub fn set_fan_sim(on: bool) -> Result<(), String> {
    let _ = on;
    Err(NOT_CONNECTED.into())
}

#[tauri::command]
pub fn start_service() -> Result<(), String> {
    Err(NOT_CONNECTED.into())
}

#[tauri::command]
pub fn stop_service() -> Result<(), String> {
    Err(NOT_CONNECTED.into())
}
