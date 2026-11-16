use crate::effect::{along, band, color, hash, range, smooth, toggle};
use crate::{Color, Control, Inputs, ParamInfo, Press};
use serde::{Deserialize, Serialize};

/// How a post effect meets the base effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    /// Covers the base.
    #[default]
    Over,
    /// Adds light to the base.
    Add,
    /// Tints the base, keeping its brightness.
    Tint,
}

/// A layer drawn over the base effect.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Post {
    #[default]
    None,
    PressRipple {
        color: Color,
        rainbow: bool,
        speed: f32,
        width: f32,
        fade: f32,
        strength: f32,
        blend: Blend,
    },
    PressGhost {
        color: Color,
        rainbow: bool,
        hold: f32,
        fade: f32,
        size: f32,
        strength: f32,
        blend: Blend,
    },
    PressSplash {
        color: Color,
        rainbow: bool,
        speed: f32,
        fade: f32,
        strength: f32,
        blend: Blend,
    },
    PressCross {
        color: Color,
        rainbow: bool,
        speed: f32,
        fade: f32,
        size: f32,
        strength: f32,
        blend: Blend,
    },
    Glitter {
        color: Color,
        rainbow: bool,
        speed: f32,
        density: f32,
        strength: f32,
        blend: Blend,
    },
    Scanline {
        color: Color,
        speed: f32,
        width: f32,
        angle: f32,
        strength: f32,
        blend: Blend,
    },
    Breathe {
        speed: f32,
        depth: f32,
    },
    BeatFlash {
        color: Color,
        sensitivity: f32,
        strength: f32,
        blend: Blend,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PostInfo {
    pub label: &'static str,
    pub default: Post,
    pub params: Vec<ParamInfo>,
}

const WHITE: Color = Color(255, 255, 255);
const ORANGE: Color = Color(255, 106, 0);
const CYAN: Color = Color(0, 200, 255);

fn blend_choice() -> ParamInfo {
    ParamInfo {
        name: "blend",
        label: "Blend",
        control: Control::Choice {
            options: vec!["over", "add", "tint"],
        },
    }
}

/// How lit a press still is: full for `hold` seconds, then gone over `fade`.
fn life(p: &Press, hold: f32, fade: f32) -> f32 {
    let k = if p.age < hold {
        1.0
    } else {
        1.0 - (p.age - hold) / fade.max(0.05)
    };
    if k.is_nan() { 0.0 } else { smooth(k.clamp(0.0, 1.0)) }
}

/// The colour a press shows: its own hue when `rainbow`, fixed for its whole life.
fn shade(color: Color, rainbow: bool, p: &Press, t: f32) -> Color {
    if rainbow {
        Color::hsv((t - p.age) * 0.37, 1.0, 1.0)
    } else {
        color
    }
}

/// 1 inside the key, `size` keys across, softening over its outer fifth.
fn on_key(x: f32, y: f32, p: &Press, size: f32) -> f32 {
    let (rx, ry) = (p.hw.max(1e-4) * size, p.hh.max(1e-4) * size);
    let d = ((x - p.x) / rx).hypot((y - p.y) / ry);
    smooth(((1.0 - d) * 5.0).clamp(0.0, 1.0))
}

/// The brightest press at this point, with its colour.
fn strongest(inputs: &Inputs, mut f: impl FnMut(&Press) -> (f32, Color)) -> (f32, Color) {
    let mut best = (0.0f32, Color::BLACK);
    for p in &inputs.presses {
        let (k, c) = f(p);
        if k > best.0 {
            best = (k, c);
        }
    }
    best
}

fn put(base: Color, over: Color, k: f32, blend: Blend) -> Color {
    let k = if k.is_nan() { 0.0 } else { k.clamp(0.0, 1.0) };
    match blend {
        Blend::Over => base.lerp(over, k),
        Blend::Add => {
            let f = |a: u8, b: u8| (a as f32 + b as f32 * k).min(255.0).round() as u8;
            Color(f(base.0, over.0), f(base.1, over.1), f(base.2, over.2))
        }
        Blend::Tint => {
            let level = base.0.max(base.1).max(base.2) as f32 / 255.0;
            base.lerp(over.scale(level), k)
        }
    }
}

impl Post {
    pub fn catalog() -> Vec<PostInfo> {
        let main = || color("color", "Colour");
        let rainbow = || toggle("rainbow", "Rainbow per key");
        let speed = || range("speed", "Speed", 0.1, 5.0, 0.05);
        let fade = || range("fade", "Fade", 0.1, 4.0, 0.05);
        let strength = || range("strength", "Strength", 0.0, 1.0, 0.05);
        vec![
            PostInfo {
                label: "None",
                default: Post::None,
                params: vec![],
            },
            PostInfo {
                label: "Key ghosting",
                default: Post::PressGhost {
                    color: WHITE,
                    rainbow: false,
                    hold: 0.1,
                    fade: 1.0,
                    size: 1.0,
                    strength: 1.0,
                    blend: Blend::Over,
                },
                params: vec![
                    main(),
                    rainbow(),
                    range("hold", "Hold", 0.0, 1.0, 0.05),
                    fade(),
                    range("size", "Size", 0.5, 4.0, 0.1),
                    strength(),
                    blend_choice(),
                ],
            },
            PostInfo {
                label: "Key ripple",
                default: Post::PressRipple {
                    color: ORANGE,
                    rainbow: false,
                    speed: 1.5,
                    width: 0.05,
                    fade: 1.0,
                    strength: 1.0,
                    blend: Blend::Over,
                },
                params: vec![
                    main(),
                    rainbow(),
                    speed(),
                    range("width", "Width", 0.01, 0.3, 0.01),
                    fade(),
                    strength(),
                    blend_choice(),
                ],
            },
            PostInfo {
                label: "Key splash",
                default: Post::PressSplash {
                    color: CYAN,
                    rainbow: false,
                    speed: 1.0,
                    fade: 0.8,
                    strength: 1.0,
                    blend: Blend::Over,
                },
                params: vec![main(), rainbow(), speed(), fade(), strength(), blend_choice()],
            },
            PostInfo {
                label: "Key crosshair",
                default: Post::PressCross {
                    color: WHITE,
                    rainbow: false,
                    speed: 2.0,
                    fade: 0.8,
                    size: 1.0,
                    strength: 1.0,
                    blend: Blend::Over,
                },
                params: vec![
                    main(),
                    rainbow(),
                    speed(),
                    fade(),
                    range("size", "Thickness", 0.5, 3.0, 0.1),
                    strength(),
                    blend_choice(),
                ],
            },
            PostInfo {
                label: "Glitter",
                default: Post::Glitter {
                    color: WHITE,
                    rainbow: false,
                    speed: 1.0,
                    density: 0.05,
                    strength: 1.0,
                    blend: Blend::Over,
                },
                params: vec![
                    main(),
                    rainbow(),
                    speed(),
                    range("density", "Density", 0.01, 0.5, 0.01),
                    strength(),
                    blend_choice(),
                ],
            },
            PostInfo {
                label: "Scanline",
                default: Post::Scanline {
                    color: WHITE,
                    speed: 1.0,
                    width: 0.15,
                    angle: 0.0,
                    strength: 0.8,
                    blend: Blend::Add,
                },
                params: vec![
                    main(),
                    speed(),
                    range("width", "Width", 0.05, 1.0, 0.05),
                    range("angle", "Angle", 0.0, 360.0, 1.0),
                    strength(),
                    blend_choice(),
                ],
            },
            PostInfo {
                label: "Breathe",
                default: Post::Breathe {
                    speed: 1.0,
                    depth: 0.7,
                },
                params: vec![speed(), range("depth", "Depth", 0.0, 1.0, 0.05)],
            },
            PostInfo {
                label: "Beat flash",
                default: Post::BeatFlash {
                    color: WHITE,
                    sensitivity: 1.5,
                    strength: 0.8,
                    blend: Blend::Add,
                },
                params: vec![
                    main(),
                    range("sensitivity", "Sensitivity", 0.2, 5.0, 0.05),
                    strength(),
                    blend_choice(),
                ],
            },
        ]
    }

    /// True when the layer reads `Inputs::presses`.
    pub fn uses_keys(&self) -> bool {
        matches!(
            self,
            Post::PressRipple { .. }
                | Post::PressGhost { .. }
                | Post::PressSplash { .. }
                | Post::PressCross { .. }
        )
    }

    /// True when the layer reads `Inputs::audio`.
    pub fn uses_audio(&self) -> bool {
        matches!(self, Post::BeatFlash { .. })
    }

    /// The base colour at `x`, `y` with this layer drawn over it.
    pub fn apply(&self, base: Color, x: f32, y: f32, t: f32, inputs: &Inputs) -> Color {
        match *self {
            Post::None => base,
            Post::PressRipple {
                color,
                rainbow,
                speed,
                width,
                fade,
                strength,
                blend,
            } => {
                let (k, c) = strongest(inputs, |p| {
                    let off = (x - p.x).hypot(y - p.y) - p.age * speed * 0.3;
                    let ring = smooth((1.0 - off.abs() / (width * 0.5).max(1e-3)).clamp(0.0, 1.0));
                    (ring * life(p, 0.0, fade), shade(color, rainbow, p, t))
                });
                put(base, c, k * strength, blend)
            }
            Post::PressGhost {
                color,
                rainbow,
                hold,
                fade,
                size,
                strength,
                blend,
            } => {
                let (k, c) = strongest(inputs, |p| {
                    (on_key(x, y, p, size) * life(p, hold, fade), shade(color, rainbow, p, t))
                });
                put(base, c, k * strength, blend)
            }
            Post::PressSplash {
                color,
                rainbow,
                speed,
                fade,
                strength,
                blend,
            } => {
                let (k, c) = strongest(inputs, |p| {
                    let reach = p.hw.max(p.hh) + p.age * speed * 0.3;
                    let edge = (x - p.x).hypot(y - p.y) / reach.max(1e-4);
                    let disc = smooth(((1.0 - edge) * 4.0).clamp(0.0, 1.0));
                    (disc * life(p, 0.0, fade), shade(color, rainbow, p, t))
                });
                put(base, c, k * strength, blend)
            }
            Post::PressCross {
                color,
                rainbow,
                speed,
                fade,
                size,
                strength,
                blend,
            } => {
                let (k, c) = strongest(inputs, |p| {
                    let reach = p.age * speed * 0.5;
                    let (dx, dy) = ((x - p.x).abs(), (y - p.y).abs());
                    let row = dy <= p.hh * size && dx <= p.hw + reach;
                    let col = dx <= p.hw * size && dy <= p.hh + reach;
                    let k = if row || col { 1.0 } else { 0.0 };
                    (k * life(p, 0.0, fade), shade(color, rainbow, p, t))
                });
                put(base, c, k * strength, blend)
            }
            Post::Glitter {
                color,
                rainbow,
                speed,
                density,
                strength,
                blend,
            } => {
                let (cx, cy) = ((x * 256.0).floor() as i32, (y * 256.0).floor() as i32);
                let time = t * speed * 2.0 + hash(cx, cy, -2);
                let bucket = time.floor();
                if hash(cx, cy, bucket as i32 ^ 0x5bd1) >= density {
                    return base;
                }
                let c = if rainbow {
                    Color::hsv(hash(cx, cy, bucket as i32), 1.0, 1.0)
                } else {
                    color
                };
                let k = (std::f32::consts::PI * (time - bucket)).sin();
                put(base, c, k * strength, blend)
            }
            Post::Scanline {
                color,
                speed,
                width,
                angle,
                strength,
                blend,
            } => {
                let k = band(along(x, y, angle) - t * speed * 0.2, width);
                put(base, color, k * strength, blend)
            }
            Post::Breathe { speed, depth } => {
                let dip = 0.5 + 0.5 * (t * speed * 2.0).cos();
                base.scale(1.0 - depth.clamp(0.0, 1.0) * (1.0 - dip))
            }
            Post::BeatFlash {
                color,
                sensitivity,
                strength,
                blend,
            } => {
                let level = (inputs.audio.level * sensitivity).clamp(0.0, 1.0);
                put(base, color, level * level * strength, blend)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioFrame;

    const BASE: Color = Color(10, 40, 90);

    fn press(x: f32, y: f32, age: f32) -> Press {
        Press {
            x,
            y,
            age,
            hw: 0.02,
            hh: 0.08,
        }
    }

    fn keys(age: f32) -> Inputs {
        Inputs {
            presses: vec![press(0.5, 0.5, age)],
            ..Inputs::default()
        }
    }

    fn busy() -> Inputs {
        Inputs {
            audio: AudioFrame {
                level: 0.8,
                ..AudioFrame::default()
            },
            presses: vec![press(0.3, 0.7, 0.3), press(0.8, 0.9, 0.05)],
            ..Inputs::default()
        }
    }

    fn ghost(fade: f32) -> Post {
        Post::PressGhost {
            color: WHITE,
            rainbow: false,
            hold: 0.0,
            fade,
            size: 1.0,
            strength: 1.0,
            blend: Blend::Over,
        }
    }

    #[test]
    fn catalog_covers_every_field_with_a_matching_control() {
        for info in Post::catalog() {
            let json = serde_json::to_value(&info.default).unwrap();
            let fields = json.as_object().unwrap();
            assert_eq!(fields.len() - 1, info.params.len(), "{}", info.label);
            for p in &info.params {
                let value = &fields[p.name];
                match &p.control {
                    Control::Range { min, max, step } => {
                        let v = value.as_f64().unwrap() as f32;
                        assert!(min < max && *step > 0.0 && (*min..=*max).contains(&v), "{} {}", info.label, p.name);
                    }
                    Control::Color => assert_eq!(value.as_array().unwrap().len(), 3),
                    Control::Toggle => assert!(value.is_boolean()),
                    Control::Choice { options } => {
                        assert!(options.contains(&value.as_str().unwrap()), "{} {}", info.label, p.name)
                    }
                }
            }
            let back: Post = serde_json::from_value(json).unwrap();
            assert_eq!(back, info.default);
        }
    }

    #[test]
    fn kinds_differ_from_base_effects() {
        let base: Vec<String> = crate::Effect::catalog()
            .iter()
            .map(|i| serde_json::to_value(&i.default).unwrap()["kind"].as_str().unwrap().to_string())
            .collect();
        for info in Post::catalog().iter().skip(1) {
            let kind = serde_json::to_value(&info.default).unwrap()["kind"].as_str().unwrap().to_string();
            assert!(!base.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn none_leaves_the_base_alone() {
        assert_eq!(Post::None.apply(BASE, 0.5, 0.5, 1.0, &busy()), BASE);
    }

    #[test]
    fn input_flags_match_what_the_layer_reads() {
        let busy = busy();
        let audio = Inputs {
            audio: busy.audio,
            ..Inputs::default()
        };
        let presses = Inputs {
            presses: busy.presses.clone(),
            ..Inputs::default()
        };
        for info in Post::catalog() {
            let e = &info.default;
            let reacts = |inputs: &Inputs| {
                (0..=40).any(|i| {
                    (0..=40).any(|j| {
                        let (x, y) = (i as f32 / 40.0, j as f32 / 40.0);
                        e.apply(BASE, x, y, 2.5, inputs) != e.apply(BASE, x, y, 2.5, &Inputs::default())
                    })
                })
            };
            assert_eq!(e.uses_audio(), reacts(&audio), "{}", info.label);
            assert_eq!(e.uses_keys(), reacts(&presses), "{}", info.label);
        }
    }

    #[test]
    fn ghosting_lights_only_the_key_then_goes_out() {
        let e = ghost(1.0);
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.0)), WHITE);
        // The next key over stays as it was.
        assert_eq!(e.apply(BASE, 0.54, 0.5, 0.0, &keys(0.0)), BASE);
        assert_eq!(e.apply(BASE, 0.5, 0.66, 0.0, &keys(0.0)), BASE);
        let half = e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.5));
        assert!(half != WHITE && half != BASE, "{half:?}");
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(1.0)), BASE);
    }

    #[test]
    fn ghosting_holds_before_fading() {
        let e = Post::PressGhost {
            color: WHITE,
            rainbow: false,
            hold: 0.5,
            fade: 1.0,
            size: 1.0,
            strength: 1.0,
            blend: Blend::Over,
        };
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.4)), WHITE);
    }

    #[test]
    fn ripple_rings_spread_from_the_key_and_fade() {
        let e = Post::PressRipple {
            color: WHITE,
            rainbow: false,
            speed: 1.0,
            width: 0.05,
            fade: 1.0,
            strength: 1.0,
            blend: Blend::Over,
        };
        let early = e.apply(BASE, 0.5 + 0.15, 0.5, 0.0, &keys(0.5));
        assert_ne!(early, BASE);
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.5)), BASE);
        assert_eq!(e.apply(BASE, 0.65, 0.5, 0.0, &keys(1.2)), BASE);
    }

    #[test]
    fn rainbow_presses_keep_their_colour() {
        let e = Post::PressGhost {
            color: WHITE,
            rainbow: true,
            hold: 2.0,
            fade: 1.0,
            size: 1.0,
            strength: 1.0,
            blend: Blend::Over,
        };
        // Pressed at t = 1 both times.
        let a = e.apply(BASE, 0.5, 0.5, 1.2, &keys(0.2));
        let b = e.apply(BASE, 0.5, 0.5, 1.7, &keys(0.7));
        assert_eq!(a, b);
        assert_ne!(a, WHITE);
    }

    #[test]
    fn strength_and_blend_shape_the_mix() {
        let half = |blend| Post::PressGhost {
            color: WHITE,
            rainbow: false,
            hold: 1.0,
            fade: 1.0,
            size: 1.0,
            strength: 0.5,
            blend,
        };
        let over = half(Blend::Over).apply(BASE, 0.5, 0.5, 0.0, &keys(0.0));
        assert_eq!(over, BASE.lerp(WHITE, 0.5));
        let add = half(Blend::Add).apply(BASE, 0.5, 0.5, 0.0, &keys(0.0));
        assert_eq!(add, Color(138, 168, 218));
        let tint = half(Blend::Tint).apply(Color::BLACK, 0.5, 0.5, 0.0, &keys(0.0));
        assert_eq!(tint, Color::BLACK);
    }

    #[test]
    fn crosshair_lights_the_row_and_column() {
        let e = Post::PressCross {
            color: WHITE,
            rainbow: false,
            speed: 2.0,
            fade: 2.0,
            size: 1.0,
            strength: 1.0,
            blend: Blend::Over,
        };
        let i = keys(0.2);
        assert_ne!(e.apply(BASE, 0.65, 0.5, 0.0, &i), BASE);
        assert_ne!(e.apply(BASE, 0.5, 0.65, 0.0, &i), BASE);
        assert_eq!(e.apply(BASE, 0.65, 0.65, 0.0, &i), BASE);
    }

    #[test]
    fn breathe_dims_the_base() {
        let e = Post::Breathe {
            speed: 1.0,
            depth: 1.0,
        };
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &Inputs::default()), BASE);
        let low = std::f32::consts::FRAC_PI_2;
        assert_eq!(e.apply(BASE, 0.5, 0.5, low, &Inputs::default()), Color::BLACK);
    }

    #[test]
    fn odd_values_never_panic() {
        let inputs = busy();
        for info in Post::catalog() {
            for v in [f32::NAN, f32::INFINITY, -1.0, 1.0e30] {
                info.default.apply(BASE, v, v, v, &inputs);
                let mut json = serde_json::to_value(&info.default).unwrap();
                for p in &info.params {
                    if matches!(p.control, Control::Range { .. }) {
                        json[p.name] = serde_json::json!(if v.is_finite() { v } else { 1.0e30 });
                    }
                }
                let e: Post = serde_json::from_value(json).unwrap();
                e.apply(BASE, 0.5, 0.5, 1.0, &inputs);
            }
        }
    }
}
