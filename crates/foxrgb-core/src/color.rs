use serde::{Deserialize, Serialize};

/// 8-bit RGB, serialised as `[r, g, b]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const BLACK: Color = Color(0, 0, 0);

    /// All components in 0..=1; hue wraps.
    pub fn hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(1.0) * 6.0;
        let sector = h.floor();
        let f = h - sector;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        // A hue a hair below zero wraps to exactly 1.0, which is sector 6.
        let (r, g, b) = match sector as u8 % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        Color(to_u8(r), to_u8(g), to_u8(b))
    }

    pub fn scale(self, k: f32) -> Self {
        let k = k.clamp(0.0, 1.0);
        let f = |c: u8| (c as f32 * k).round() as u8;
        Color(f(self.0), f(self.1), f(self.2))
    }

    pub fn lerp(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2))
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_primaries() {
        assert_eq!(Color::hsv(0.0, 1.0, 1.0), Color(255, 0, 0));
        assert_eq!(Color::hsv(1.0 / 3.0, 1.0, 1.0), Color(0, 255, 0));
        assert_eq!(Color::hsv(2.0 / 3.0, 1.0, 1.0), Color(0, 0, 255));
        assert_eq!(Color::hsv(-1.0, 1.0, 1.0), Color(255, 0, 0));
        assert_eq!(Color::hsv(-1e-9, 1.0, 1.0), Color(255, 0, 0));
    }

    #[test]
    fn lerp_and_scale() {
        let a = Color(0, 100, 200);
        assert_eq!(a.lerp(Color(100, 200, 0), 0.5), Color(50, 150, 100));
        assert_eq!(a.scale(0.0), Color::BLACK);
        assert_eq!(a.scale(1.0), a);
    }
}
