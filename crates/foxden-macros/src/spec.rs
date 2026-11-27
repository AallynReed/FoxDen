//! Macros as plain JSON for scripts and AI assistants: keys and buttons by name, "tap" for down and up.

use serde_json::{Value, json};

use crate::keys;
use crate::model::{Hotkey, Macro, MediaKey, Mode, Step, button_name};

const BUTTONS: [(&str, u8); 5] = [("left", 1), ("right", 2), ("middle", 3), ("back", 4), ("forward", 5)];

fn button(v: &Value) -> Result<u8, String> {
    if let Some(n) = v.as_u64() {
        return u8::try_from(n).ok().filter(|b| (1..=5).contains(b)).ok_or("button must be 1 to 5.".into());
    }
    let s = v.as_str().ok_or("button must be left, right, middle, back or forward.")?;
    BUTTONS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(s))
        .map(|b| b.1)
        .ok_or_else(|| format!("{s} isn't left, right, middle, back or forward."))
}

fn presses(v: &Value) -> Result<&'static [bool], String> {
    match v.as_str().unwrap_or("tap") {
        "tap" => Ok(&[true, false]),
        "down" => Ok(&[true]),
        "up" => Ok(&[false]),
        other => Err(format!("action {other} isn't tap, down or up.")),
    }
}

/// Reads one step, which may become two (a tap is down then up).
pub fn step_from(v: &Value) -> Result<Vec<Step>, String> {
    let kind = v["type"].as_str().ok_or("Each step needs a type: key, button, wheel, delay, text or media.")?;
    Ok(match kind {
        "key" => {
            let usage = match &v["key"] {
                Value::String(s) => keys::by_name(s).ok_or_else(|| format!("{s} isn't a key FoxDen knows."))?,
                k => k.as_u64().and_then(|u| u16::try_from(u).ok()).ok_or("key must be a key name.")?,
            };
            presses(&v["action"])?.iter().map(|&down| Step::Key { usage, down }).collect()
        }
        "button" => {
            let button = button(&v["button"])?;
            presses(&v["action"])?.iter().map(|&down| Step::Button { button, down }).collect()
        }
        "wheel" => {
            let ticks = v["ticks"].as_i64().ok_or("wheel needs ticks: positive up or right, negative down or left.")?;
            vec![Step::Wheel { horizontal: v["horizontal"].as_bool().unwrap_or(false), ticks: ticks.clamp(-1000, 1000) as i32 }]
        }
        "delay" => {
            let ms = v["ms"].as_u64().ok_or("delay needs ms.")?;
            vec![Step::Delay { ms: ms.min(u32::MAX as u64) as u32 }]
        }
        "text" => vec![Step::Text { text: v["text"].as_str().ok_or("text needs text.")?.to_string() }],
        "media" => {
            let k = v["key"].as_str().unwrap_or_default();
            let codes: Vec<&str> = MediaKey::ALL.iter().map(|m| m.code()).collect();
            vec![Step::Media {
                key: MediaKey::from_code(k).ok_or_else(|| format!("media key must be one of {}.", codes.join(", ")))?,
            }]
        }
        other => return Err(format!("Step type {other} isn't key, button, wheel, delay, text or media.")),
    })
}

pub fn steps_from(v: &Value) -> Result<Vec<Step>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Array(list) => list.iter().map(step_from).collect::<Result<Vec<_>, _>>().map(|s| s.concat()),
        _ => Err("Steps must be a list.".into()),
    }
}

pub fn step_json(s: &Step) -> Value {
    let action = |d: bool| if d { "down" } else { "up" };
    match s {
        Step::Key { usage, down } => json!({"type": "key", "key": keys::name(*usage), "action": action(*down)}),
        Step::Button { button, down } => {
            let name = BUTTONS.iter().find(|b| b.1 == *button).map(|b| b.0).unwrap_or(button_name(*button));
            json!({"type": "button", "button": name, "action": action(*down)})
        }
        Step::Wheel { horizontal, ticks } => json!({"type": "wheel", "ticks": ticks, "horizontal": horizontal}),
        Step::Delay { ms } => json!({"type": "delay", "ms": ms}),
        Step::Text { text } => json!({"type": "text", "text": text}),
        Step::Media { key } => json!({"type": "media", "key": key.code()}),
    }
}

/// A macro from `{name, mode, press, hold, release, default_delay_ms}`.
pub fn macro_from(v: &Value) -> Result<Macro, String> {
    let mode = match v["mode"].as_str() {
        None => Mode::Once,
        Some(m) => Mode::from_code(m).ok_or_else(|| {
            let codes: Vec<&str> = Mode::ALL.iter().map(|m| m.code()).collect();
            format!("mode must be one of {}.", codes.join(", "))
        })?,
    };
    let m = Macro {
        name: v["name"].as_str().unwrap_or_default().trim().to_string(),
        mode,
        press: steps_from(&v["press"])?,
        hold: steps_from(&v["hold"])?,
        release: steps_from(&v["release"])?,
        default_delay_ms: v["default_delay_ms"].as_u64().map(|d| d.min(u32::MAX as u64) as u32),
    };
    m.check()?;
    if !m.playable() {
        return Err(match mode {
            Mode::PressHoldRelease => "Give the macro at least one step.".into(),
            _ => "Give the macro at least one press step.".into(),
        });
    }
    Ok(m)
}

pub fn macro_json(m: &Macro) -> Value {
    let list = |s: &[Step]| s.iter().map(step_json).collect::<Vec<_>>();
    json!({
        "name": m.name,
        "mode": m.mode.code(),
        "press": list(&m.press),
        "hold": list(&m.hold),
        "release": list(&m.release),
        "default_delay_ms": m.default_delay_ms,
    })
}

pub fn hotkey_json(h: Option<Hotkey>) -> Value {
    h.map(|h| json!(h.label())).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taps_expand_and_round_trip() {
        let v = json!({
            "name": "Hello",
            "press": [
                {"type": "key", "key": "Ctrl", "action": "down"},
                {"type": "key", "key": "c"},
                {"type": "key", "key": "Ctrl", "action": "up"},
                {"type": "delay", "ms": 50},
                {"type": "text", "text": "hi"},
                {"type": "button", "button": "back"},
                {"type": "wheel", "ticks": -3},
                {"type": "media", "key": "play_pause"},
            ],
        });
        let m = macro_from(&v).unwrap();
        assert_eq!(m.press.len(), 10);
        assert_eq!(m.press[1], Step::Key { usage: 0x06, down: true });
        assert_eq!(m.press[2], Step::Key { usage: 0x06, down: false });
        assert_eq!(m.press[7], Step::Button { button: 4, down: false });
        let back = macro_from(&macro_json(&m)).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn bad_specs_say_why() {
        let e = |v: Value| macro_from(&v).unwrap_err();
        assert!(e(json!({"name": "x"})).contains("press step"));
        assert!(e(json!({"name": "x", "mode": "loop", "press": []})).contains("mode must"));
        assert!(e(json!({"name": "x", "press": [{"type": "key", "key": "Hyper"}]})).contains("Hyper"));
        assert!(e(json!({"name": "x", "press": [{"type": "jump"}]})).contains("jump"));
        assert!(e(json!({"name": "x", "press": [{"type": "button", "button": "side"}]})).contains("side"));
        assert!(e(json!({"name": "x", "press": [{"type": "key", "key": "A", "action": "hold"}]})).contains("hold"));
        assert!(e(json!({"name": "x", "press": [{"type": "delay", "ms": 999999}]})).contains("at most"));
        assert!(e(json!({"name": "", "press": [{"type": "text", "text": "a"}]})).contains("name"));
        assert!(e(json!({"name": "x", "press": {"type": "text"}})).contains("list"));
        let hold = macro_from(&json!({"name": "x", "mode": "press_hold_release", "hold": [{"type": "button", "button": 1}]}));
        assert_eq!(hold.unwrap().hold.len(), 2);
    }
}
