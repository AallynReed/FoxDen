#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use foxrgb_core::{Device, Effect, Engine, Inputs, VirtualBackend};
use tauri::{Emitter, State};

type Shared = Arc<Mutex<Engine>>;

const FRAME_TIME: Duration = Duration::from_millis(33);

#[tauri::command]
fn devices(engine: State<'_, Shared>) -> Vec<Device> {
    engine.lock().unwrap().devices()
}

#[tauri::command]
fn get_state(engine: State<'_, Shared>) -> (Effect, f32) {
    let engine = engine.lock().unwrap();
    (engine.effect.clone(), engine.brightness)
}

#[tauri::command]
fn set_effect(engine: State<'_, Shared>, effect: Effect) {
    engine.lock().unwrap().effect = effect;
}

#[tauri::command]
fn set_brightness(engine: State<'_, Shared>, value: f32) {
    engine.lock().unwrap().brightness = value.clamp(0.0, 1.0);
}

fn main() {
    let engine: Shared = Arc::new(Mutex::new(Engine::new(vec![Box::new(VirtualBackend)])));
    let ticker = engine.clone();

    tauri::Builder::default()
        .manage(engine)
        .setup(move |app| {
            let handle = app.handle().clone();
            thread::spawn(move || {
                let start = Instant::now();
                let inputs = Inputs::default();
                loop {
                    let frames = ticker.lock().unwrap().render(start.elapsed().as_secs_f32(), &inputs);
                    let _ = handle.emit("frame", &frames);
                    thread::sleep(FRAME_TIME);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![devices, get_state, set_effect, set_brightness])
        .run(tauri::generate_context!())
        .expect("failed to run FoxRGB");
}
