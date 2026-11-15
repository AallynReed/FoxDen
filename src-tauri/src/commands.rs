use foxrgb_core::Effect;
use tauri::{AppHandle, State};

use crate::config::{DeviceConfig, Profile, Settings};
use crate::state::{Job, Jobs, Kinds, Shared, Snapshot, lock, update};
use crate::{autostart, conflicts, fans, modules};

type Reply = Result<(), String>;

#[tauri::command]
pub fn get_snapshot(shared: State<'_, Shared>) -> Result<Snapshot, String> {
    Ok(lock(&shared).snapshot())
}

#[tauri::command]
pub fn set_effect(app: AppHandle, shared: State<'_, Shared>, effect: Effect) -> Reply {
    update(&app, &shared, |s| {
        s.engine.effect = effect;
        s.check_profile();
        Ok(())
    })
}

#[tauri::command]
pub fn set_brightness(app: AppHandle, shared: State<'_, Shared>, value: f32) -> Reply {
    update(&app, &shared, |s| {
        s.engine.brightness = value.clamp(0.0, 1.0);
        s.check_profile();
        Ok(())
    })
}

#[tauri::command]
pub fn set_paused(app: AppHandle, shared: State<'_, Shared>, paused: bool) -> Reply {
    update(&app, &shared, |s| {
        s.engine.paused = paused;
        Ok(())
    })
}

#[tauri::command]
pub fn set_device(
    app: AppHandle,
    shared: State<'_, Shared>,
    id: String,
    enabled: bool,
    brightness: f32,
    rect: [f32; 4],
) -> Reply {
    update(&app, &shared, |s| {
        let devices = s.engine.devices();
        let device = devices
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| format!("unknown device {id}"))?;
        if rect.iter().any(|v| !v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return Err("invalid device rectangle".into());
        }
        // Only a rect the user actually moved becomes a saved override.
        let moved = rect
            .iter()
            .zip(device.rect)
            .any(|(a, b)| (a - b).abs() > 1e-4);
        let saved = s.config.devices.get(&id).and_then(|d| d.rect);
        let rect = if moved { Some(rect) } else { saved };
        let brightness = brightness.clamp(0.0, 1.0);
        s.engine.set_device(&id, enabled, brightness, rect);
        s.config.devices.insert(
            id,
            DeviceConfig {
                enabled,
                brightness,
                rect,
            },
        );
        Ok(())
    })
}

#[tauri::command]
pub fn reset_layout(app: AppHandle, shared: State<'_, Shared>) -> Reply {
    update(&app, &shared, |s| {
        for device in s.config.devices.values_mut() {
            device.rect = None;
        }
        s.engine.clear_rects();
        Ok(())
    })
}

#[tauri::command]
pub fn save_profile(app: AppHandle, shared: State<'_, Shared>, name: String) -> Reply {
    update(&app, &shared, |s| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("profile name is empty".into());
        }
        let profile = Profile {
            effect: s.engine.effect.clone(),
            brightness: s.engine.brightness,
        };
        s.config.profiles.insert(name.clone(), profile);
        s.config.active_profile = Some(name);
        Ok(())
    })
}

#[tauri::command]
pub fn load_profile(app: AppHandle, shared: State<'_, Shared>, name: String) -> Reply {
    update(&app, &shared, |s| s.load_profile(&name))
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, shared: State<'_, Shared>, name: String) -> Reply {
    update(&app, &shared, |s| {
        s.config
            .profiles
            .remove(&name)
            .ok_or_else(|| format!("no profile named {name}"))?;
        if s.config.active_profile.as_deref() == Some(name.as_str()) {
            s.config.active_profile = None;
        }
        Ok(())
    })
}

#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    shared: State<'_, Shared>,
    jobs: State<'_, Jobs>,
    settings: Settings,
) -> Reply {
    let settings = Settings {
        fps: settings.fps.clamp(10, 60),
        ..settings
    };
    let old = lock(&shared).config.settings.clone();
    if settings.autostart != old.autostart {
        autostart::set(settings.autostart)?;
    }
    let server = |s: &Settings| (s.openrgb_enabled, s.openrgb_host.clone(), s.openrgb_port);
    let lighting = settings.lighting_module != old.lighting_module;
    let kinds = if lighting {
        Kinds::ALL
    } else {
        Kinds {
            openrgb: server(&settings) != server(&old),
            hid: settings.native_drivers != old.native_drivers,
        }
    };
    let fans = settings.fans_module != old.fans_module;
    let wanted = |s: &Settings| s.lighting_module || s.fans_module;
    let (machine, wanted) = (wanted(&settings) != wanted(&old), wanted(&settings));
    let conflicts = settings.close_conflicts && !old.close_conflicts;
    let backup = update(&app, &shared, |s| {
        s.config.settings = settings;
        Ok(())
    })
    .map(|_| lock(&shared).config.fan_backup.clone())?;
    if fans {
        fans::set_module(lock(&shared).config.settings.fans_module, backup);
    }
    if machine {
        modules::apply(&app, wanted);
    }
    if kinds.any() {
        queue(&jobs, Job::Reload(kinds))?;
    }
    if conflicts || lighting || fans {
        queue(&jobs, Job::Detect)?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn get_conflicts(shared: State<'_, Shared>) -> Vec<conflicts::Found> {
    let settings = lock(&shared).config.settings.clone();
    conflicts::running(conflicts::scope(&settings))
}

#[tauri::command]
pub fn last_conflicts() -> Option<conflicts::Report> {
    conflicts::last()
}

#[tauri::command(async)]
pub fn close_conflicts_now(shared: State<'_, Shared>) -> conflicts::Report {
    let settings = lock(&shared).config.settings.clone();
    let report = conflicts::sweep(conflicts::scope(&settings));
    conflicts::remember(report.clone());
    report.unwrap_or_else(|| conflicts::Report {
        text: "Nothing to close".into(),
        ..Default::default()
    })
}

#[tauri::command]
pub fn rescan(jobs: State<'_, Jobs>) -> Reply {
    queue(&jobs, Job::Reload(Kinds::ALL))
}

#[tauri::command]
pub fn detect(jobs: State<'_, Jobs>) -> Reply {
    queue(&jobs, Job::Detect)
}

/// Hands the work to the hardware thread; a "snapshot" follows when it is done.
fn queue(jobs: &Jobs, job: Job) -> Reply {
    jobs.send(job)
        .map_err(|_| "the hardware thread has stopped".to_string())
}
