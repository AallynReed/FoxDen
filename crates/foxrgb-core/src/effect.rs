use crate::{Color, Inputs};
use serde::{Deserialize, Serialize};

/// A pure function of canvas position, time and inputs, so it flows across devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Static { color: Color },
    Breathing { color: Color, speed: f32 },
    Cycle { speed: f32 },
    Rainbow { speed: f32, scale: f32, angle: f32 },
    Gradient { from: Color, to: Color, speed: f32, scale: f32 },
}

/// How the UI edits one parameter.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Range { min: f32, max: f32, step: f32 },
    Color,
    Toggle,
    Choice { options: Vec<&'static str> },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParamInfo {
    /// Field name in the serialised effect.
    pub name: &'static str,
    pub label: &'static str,
    pub control: Control,
}

/// One catalogue entry. The UI builds its controls from these.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffectInfo {
    pub label: &'static str,
    pub default: Effect,
    pub params: Vec<ParamInfo>,
}

const ORANGE: Color = Color(255, 106, 0);
const VIOLET: Color = Color(150, 60, 255);

fn range(name: &'static str, label: &'static str, min: f32, max: f32, step: f32) -> ParamInfo {
    ParamInfo { name, label, control: Control::Range { min, max, step } }
}

fn color(name: &'static str, label: &'static str) -> ParamInfo {
    ParamInfo { name, label, control: Control::Color }
}

impl Default for Effect {
    fn default() -> Self {
        Effect::Rainbow { speed: 1.0, scale: 1.0, angle: 0.0 }
    }
}

impl Effect {
    pub fn catalog() -> Vec<EffectInfo> {
        let speed = || range("speed", "Speed", 0.0, 5.0, 0.05);
        let scale = || range("scale", "Scale", 0.2, 4.0, 0.05);
        vec![
            EffectInfo {
                label: "Rainbow wave",
                default: Effect::default(),
                params: vec![speed(), scale(), range("angle", "Angle", 0.0, 360.0, 1.0)],
            },
            EffectInfo {
                label: "Gradient",
                default: Effect::Gradient { from: ORANGE, to: VIOLET, speed: 1.0, scale: 1.0 },
                params: vec![color("from", "From"), color("to", "To"), speed(), scale()],
            },
            EffectInfo {
                label: "Spectrum cycle",
                default: Effect::Cycle { speed: 1.0 },
                params: vec![speed()],
            },
            EffectInfo {
                label: "Breathing",
                default: Effect::Breathing { color: ORANGE, speed: 1.0 },
                params: vec![color("color", "Colour"), speed()],
            },
            EffectInfo {
                label: "Static",
                default: Effect::Static { color: ORANGE },
                params: vec![color("color", "Colour")],
            },
        ]
    }

    /// True when the effect reads `Inputs::audio`.
    pub fn uses_audio(&self) -> bool {
        false
    }

    /// True when the effect reads `Inputs::screen`.
    pub fn uses_screen(&self) -> bool {
        false
    }

    /// `x`, `y` in 0..=1 on the canvas, `t` in seconds.
    pub fn sample(&self, x: f32, y: f32, t: f32, _inputs: &Inputs) -> Color {
        match *self {
            Effect::Static { color } => color,
            Effect::Breathing { color, speed } => color.scale(0.5 - 0.5 * (t * speed * 2.0).cos()),
            Effect::Cycle { speed } => Color::hsv(t * speed * 0.1, 1.0, 1.0),
            Effect::Rainbow { speed, scale, angle } => {
                let a = angle.to_radians();
                let d = x * a.cos() + y * a.sin();
                Color::hsv(d * scale - t * speed * 0.2, 1.0, 1.0)
            }
            Effect::Gradient { from, to, speed, scale } => {
                let phase = (x * scale - t * speed * 0.2).rem_euclid(1.0);
                from.lerp(to, 1.0 - (phase * 2.0 - 1.0).abs())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(e: &Effect, x: f32, y: f32, t: f32) -> Color {
        e.sample(x, y, t, &Inputs::default())
    }

    #[test]
    fn static_ignores_position_and_time() {
        let e = Effect::Static { color: Color(1, 2, 3) };
        assert_eq!(at(&e, 0.0, 0.0, 0.0), at(&e, 0.9, 0.3, 42.0));
    }

    #[test]
    fn breathing_starts_dark() {
        let e = Effect::Breathing { color: Color(255, 255, 255), speed: 1.0 };
        assert_eq!(at(&e, 0.0, 0.0, 0.0), Color::BLACK);
    }

    #[test]
    fn rainbow_varies_across_canvas() {
        let e = Effect::default();
        assert_ne!(at(&e, 0.0, 0.5, 0.0), at(&e, 0.5, 0.5, 0.0));
    }

    #[test]
    fn serialises_with_kind_tag() {
        let json = serde_json::to_string(&Effect::Cycle { speed: 2.0 }).unwrap();
        assert_eq!(json, r#"{"kind":"cycle","speed":2.0}"#);
    }

    #[test]
    fn catalog_params_are_real_fields() {
        for info in Effect::catalog() {
            let json = serde_json::to_value(&info.default).unwrap();
            for p in &info.params {
                assert!(json.get(p.name).is_some(), "{} has no field {}", info.label, p.name);
            }
        }
    }
}
