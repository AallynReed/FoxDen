use crate::Color;
use serde::{Deserialize, Serialize};

/// An effect is a function of canvas position and time, so it flows across devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Static { color: Color },
    Breathing { color: Color, speed: f32 },
    Cycle { speed: f32 },
    Rainbow { speed: f32, scale: f32, angle: f32 },
    Gradient { from: Color, to: Color, speed: f32, scale: f32 },
}

impl Default for Effect {
    fn default() -> Self {
        Effect::Rainbow { speed: 1.0, scale: 1.0, angle: 0.0 }
    }
}

impl Effect {
    /// `x`, `y` in 0..=1 on the canvas, `t` in seconds.
    pub fn sample(&self, x: f32, y: f32, t: f32) -> Color {
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

    #[test]
    fn static_ignores_position_and_time() {
        let e = Effect::Static { color: Color(1, 2, 3) };
        assert_eq!(e.sample(0.0, 0.0, 0.0), e.sample(0.9, 0.3, 42.0));
    }

    #[test]
    fn breathing_starts_dark() {
        let e = Effect::Breathing { color: Color(255, 255, 255), speed: 1.0 };
        assert_eq!(e.sample(0.0, 0.0, 0.0), Color::BLACK);
    }

    #[test]
    fn rainbow_varies_across_canvas() {
        let e = Effect::default();
        assert_ne!(e.sample(0.0, 0.5, 0.0), e.sample(0.5, 0.5, 0.0));
    }

    #[test]
    fn serialises_with_kind_tag() {
        let json = serde_json::to_string(&Effect::Cycle { speed: 2.0 }).unwrap();
        assert_eq!(json, r#"{"kind":"cycle","speed":2.0}"#);
    }
}
