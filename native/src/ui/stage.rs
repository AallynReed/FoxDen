use std::collections::HashMap;

use serde_json::Value;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use tauri::AppHandle;

use super::devices::send_device;
use super::{AppWindow, CanvasLabel, StagePage, View, fill, refresh, report, shared, tr, with_view, with_win};
use crate::commands;

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<StagePage>();
    let a = app.clone();
    g.on_set_brightness(move |v| report(commands::set_brightness(a.clone(), shared(&a), v)));
    let a = app.clone();
    g.on_toggle_pause(move || {
        let paused = with_view(|v| v.snap["paused"].as_bool().unwrap_or(false));
        report(commands::set_paused(a.clone(), shared(&a), !paused));
    });
    let a = app.clone();
    g.on_reset_layout(move || report(commands::reset_layout(a.clone(), shared(&a))));
    let a = app.clone();
    g.on_canvas_down(move |x, y| {
        with_view(|v| v.canvas.down(x, y));
        refresh(&a);
    });
    g.on_canvas_move(move |x, y, pressed| {
        with_view(|v| {
            if v.canvas.moved(x, y, pressed) {
                with_win(|w| redraw(w, v));
            }
        });
    });
    let a = app.clone();
    g.on_canvas_up(move || {
        if let Some((d, rect)) = with_view(|v| v.canvas.up()) {
            send_device(&a, &d, d.enabled, d.brightness, rect);
        }
    });
    let a = app.clone();
    g.on_canvas_key(move |key, ctrl, shift| {
        match with_view(|v| v.canvas.key(&key, ctrl, shift)) {
            Some((d, rect)) => {
                send_device(&a, &d, d.enabled, d.brightness, rect);
                true
            }
            None => false,
        }
    });
}

pub fn event(_w: &AppWindow, _app: &AppHandle, _view: &mut View, _name: &str) {}

pub fn apply(w: &AppWindow, app: &AppHandle, view: &mut View) {
    let g = w.global::<StagePage>();
    let snap = &view.snap;
    g.set_brightness(snap["brightness"].as_f64().unwrap_or(1.0) as f32);
    let paused = snap["paused"].as_bool().unwrap_or(false);
    g.set_paused(paused);
    let mut status = vec![
        tr(app, if paused { "Paused" } else { "Running" }),
        format!("{} devices", view.canvas.devices.len()),
    ];
    if let Some(p) = snap["active_profile"].as_str() {
        status.push(p.to_string());
    }
    g.set_status(status.join("   ").into());
    redraw(w, view);
}

pub fn frame(w: &AppWindow, view: &mut View, frame: &Value) {
    let Some(frames) = frame.as_array() else {
        return;
    };
    let colors: HashMap<String, Vec<[u8; 3]>> = frames
        .iter()
        .filter_map(|f| {
            let id = f["device"].as_str()?.to_string();
            Some((id, serde_json::from_value(f["colors"].clone()).ok()?))
        })
        .collect();
    if colors != view.canvas.colors {
        view.canvas.colors = colors;
        redraw(w, view);
    }
}

pub fn redraw(w: &AppWindow, view: &mut View) {
    let (cw, ch) = (w.get_canvas_width(), w.get_canvas_height());
    if cw < 50.0 || ch < 50.0 {
        return;
    }
    let scale = w.window().scale_factor();
    let Some((pm, labels)) = view.canvas.render(cw, ch, scale) else {
        return;
    };
    let g = w.global::<StagePage>();
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pm.data(), pm.width(), pm.height());
    g.set_canvas(Image::from_rgba8_premultiplied(buf));
    fill(
        g.get_labels(),
        labels
            .into_iter()
            .map(|l| CanvasLabel {
                x: l.x,
                y: l.y,
                width: l.width,
                text: l.text.into(),
                strong: l.strong,
            })
            .collect(),
        |m| g.set_labels(m),
    );
}
