use crate::{AUDIO_BANDS, Color, Inputs};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// A pure function of canvas position, time and inputs, so it flows across devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// Black, for a post effect on its own.
    None,
    Static {
        color: Color,
    },
    Breathing {
        color: Color,
        speed: f32,
    },
    Cycle {
        speed: f32,
    },
    Rainbow {
        speed: f32,
        scale: f32,
        angle: f32,
    },
    Gradient {
        from: Color,
        to: Color,
        speed: f32,
        scale: f32,
    },
    Wave {
        color: Color,
        background: Color,
        speed: f32,
        width: f32,
        angle: f32,
    },
    Comet {
        color: Color,
        background: Color,
        speed: f32,
        length: f32,
        angle: f32,
    },
    Sparkle {
        color: Color,
        background: Color,
        speed: f32,
        density: f32,
    },
    Plasma {
        speed: f32,
        scale: f32,
    },
    Fire {
        speed: f32,
        intensity: f32,
    },
    Ripple {
        color: Color,
        background: Color,
        speed: f32,
        width: f32,
        rings: f32,
    },
    RadialRainbow {
        speed: f32,
        repeats: f32,
        twist: f32,
    },
    ColorShift {
        first: Color,
        second: Color,
        third: Color,
        use_third: bool,
        speed: f32,
    },
    AudioSpectrum {
        color: Color,
        rainbow: bool,
        sensitivity: f32,
    },
    AudioPulse {
        color: Color,
        sensitivity: f32,
        floor: f32,
    },
    ScreenAmbience {
        saturation: f32,
        brightness: f32,
    },
}

/// How the UI edits one parameter.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Range {
        min: f32,
        max: f32,
        step: f32,
    },
    Color,
    Toggle,
    Choice {
        options: Vec<&'static str>,
    },
    /// A set of keyboard keys.
    Keys,
    /// A colour for each of some keyboard keys.
    KeyColours,
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
const CYAN: Color = Color(0, 200, 255);
const NAVY: Color = Color(0, 12, 40);
const WHITE: Color = Color(255, 255, 255);

pub(crate) fn range(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
) -> ParamInfo {
    ParamInfo {
        name,
        label,
        control: Control::Range { min, max, step },
    }
}

pub(crate) fn color(name: &'static str, label: &'static str) -> ParamInfo {
    ParamInfo {
        name,
        label,
        control: Control::Color,
    }
}

pub(crate) fn toggle(name: &'static str, label: &'static str) -> ParamInfo {
    ParamInfo {
        name,
        label,
        control: Control::Toggle,
    }
}

pub(crate) fn smooth(k: f32) -> f32 {
    k * k * (3.0 - 2.0 * k)
}

fn mix(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

/// Distance along the direction `angle` (degrees).
pub(crate) fn along(x: f32, y: f32, angle: f32) -> f32 {
    let a = angle.to_radians();
    x * a.cos() + y * a.sin()
}

/// 1 in the middle of a repeating band `width` wide (as a share of the period), 0 outside it.
pub(crate) fn band(phase: f32, width: f32) -> f32 {
    let dist = (phase.rem_euclid(1.0) - 0.5).abs();
    smooth((1.0 - dist / (width * 0.5).max(1e-3)).clamp(0.0, 1.0))
}

/// Repeatable pseudo-random value in 0..1.
pub(crate) fn hash(x: i32, y: i32, n: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ (n as u32).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1 << 24) as f32
}

/// Smooth value noise in 0..1.
fn noise(x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let (xi, yi) = (x0 as i32, y0 as i32);
    let row = |yi: i32| mix(hash(xi, yi, 0), hash(xi.wrapping_add(1), yi, 0), fx);
    mix(row(yi), row(yi.wrapping_add(1)), fy)
}

/// Black through red and yellow to white.
fn heat_color(heat: f32) -> Color {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    Color(ch(heat * 3.0), ch(heat * 3.0 - 1.0), ch(heat * 3.0 - 2.0))
}

impl Default for Effect {
    fn default() -> Self {
        Effect::Rainbow {
            speed: 1.0,
            scale: 1.0,
            angle: 0.0,
        }
    }
}

impl Effect {
    pub fn catalog() -> Vec<EffectInfo> {
        let speed = || range("speed", "Speed", 0.0, 5.0, 0.05);
        let scale = || range("scale", "Scale", 0.2, 4.0, 0.05);
        let angle = || range("angle", "Angle", 0.0, 360.0, 1.0);
        let width = || range("width", "Width", 0.05, 1.0, 0.05);
        let sensitivity = || range("sensitivity", "Sensitivity", 0.2, 5.0, 0.05);
        let main = || color("color", "Colour");
        let background = || color("background", "Background");
        vec![
            EffectInfo {
                label: "None",
                default: Effect::None,
                params: vec![],
            },
            EffectInfo {
                label: "Rainbow wave",
                default: Effect::default(),
                params: vec![speed(), scale(), angle()],
            },
            EffectInfo {
                label: "Gradient",
                default: Effect::Gradient {
                    from: ORANGE,
                    to: VIOLET,
                    speed: 1.0,
                    scale: 1.0,
                },
                params: vec![color("from", "From"), color("to", "To"), speed(), scale()],
            },
            EffectInfo {
                label: "Spectrum cycle",
                default: Effect::Cycle { speed: 1.0 },
                params: vec![speed()],
            },
            EffectInfo {
                label: "Breathing",
                default: Effect::Breathing {
                    color: ORANGE,
                    speed: 1.0,
                },
                params: vec![main(), speed()],
            },
            EffectInfo {
                label: "Static",
                default: Effect::Static { color: ORANGE },
                params: vec![main()],
            },
            EffectInfo {
                label: "Wave",
                default: Effect::Wave {
                    color: CYAN,
                    background: NAVY,
                    speed: 1.0,
                    width: 0.4,
                    angle: 0.0,
                },
                params: vec![main(), background(), speed(), width(), angle()],
            },
            EffectInfo {
                label: "Comet",
                default: Effect::Comet {
                    color: ORANGE,
                    background: Color::BLACK,
                    speed: 1.0,
                    length: 0.5,
                    angle: 0.0,
                },
                params: vec![
                    main(),
                    background(),
                    speed(),
                    range("length", "Tail length", 0.05, 1.0, 0.05),
                    angle(),
                ],
            },
            EffectInfo {
                label: "Sparkle",
                default: Effect::Sparkle {
                    color: WHITE,
                    background: NAVY,
                    speed: 1.0,
                    density: 0.3,
                },
                params: vec![
                    main(),
                    background(),
                    speed(),
                    range("density", "Density", 0.0, 1.0, 0.05),
                ],
            },
            EffectInfo {
                label: "Plasma",
                default: Effect::Plasma {
                    speed: 1.0,
                    scale: 1.0,
                },
                params: vec![speed(), scale()],
            },
            EffectInfo {
                label: "Fire",
                default: Effect::Fire {
                    speed: 1.0,
                    intensity: 1.0,
                },
                params: vec![speed(), range("intensity", "Intensity", 0.2, 2.0, 0.05)],
            },
            EffectInfo {
                label: "Ripple",
                default: Effect::Ripple {
                    color: CYAN,
                    background: NAVY,
                    speed: 1.0,
                    width: 0.4,
                    rings: 3.0,
                },
                params: vec![
                    main(),
                    background(),
                    speed(),
                    width(),
                    range("rings", "Rings", 1.0, 8.0, 0.5),
                ],
            },
            EffectInfo {
                label: "Radial rainbow",
                default: Effect::RadialRainbow {
                    speed: 1.0,
                    repeats: 1.0,
                    twist: 0.0,
                },
                params: vec![
                    speed(),
                    range("repeats", "Repeats", 1.0, 6.0, 1.0),
                    range("twist", "Twist", 0.0, 4.0, 0.1),
                ],
            },
            EffectInfo {
                label: "Colour shift",
                default: Effect::ColorShift {
                    first: ORANGE,
                    second: VIOLET,
                    third: CYAN,
                    use_third: true,
                    speed: 1.0,
                },
                params: vec![
                    color("first", "First colour"),
                    color("second", "Second colour"),
                    color("third", "Third colour"),
                    toggle("use_third", "Use third colour"),
                    speed(),
                ],
            },
            EffectInfo {
                label: "Audio spectrum",
                default: Effect::AudioSpectrum {
                    color: ORANGE,
                    rainbow: true,
                    sensitivity: 1.0,
                },
                params: vec![
                    toggle("rainbow", "Colour by band"),
                    color("color", "Bar colour"),
                    sensitivity(),
                ],
            },
            EffectInfo {
                label: "Audio pulse",
                default: Effect::AudioPulse {
                    color: ORANGE,
                    sensitivity: 1.0,
                    floor: 0.1,
                },
                params: vec![
                    main(),
                    sensitivity(),
                    range("floor", "Idle brightness", 0.0, 1.0, 0.05),
                ],
            },
            EffectInfo {
                label: "Screen ambience",
                default: Effect::ScreenAmbience {
                    saturation: 1.0,
                    brightness: 1.0,
                },
                params: vec![
                    range("saturation", "Saturation", 0.0, 3.0, 0.05),
                    range("brightness", "Brightness", 0.2, 3.0, 0.05),
                ],
            },
        ]
    }

    /// True when the effect reads `Inputs::audio`.
    pub fn uses_audio(&self) -> bool {
        matches!(
            self,
            Effect::AudioSpectrum { .. } | Effect::AudioPulse { .. }
        )
    }

    /// True when the effect reads `Inputs::screen`.
    pub fn uses_screen(&self) -> bool {
        matches!(self, Effect::ScreenAmbience { .. })
    }

    /// `x`, `y` in 0..=1 on the canvas (`y` = 1 is the bottom), `t` in seconds.
    pub fn sample(&self, x: f32, y: f32, t: f32, inputs: &Inputs) -> Color {
        match *self {
            Effect::None => Color::BLACK,
            Effect::Static { color } => color,
            Effect::Breathing { color, speed } => color.scale(0.5 - 0.5 * (t * speed * 2.0).cos()),
            Effect::Cycle { speed } => Color::hsv(t * speed * 0.1, 1.0, 1.0),
            Effect::Rainbow {
                speed,
                scale,
                angle,
            } => Color::hsv(along(x, y, angle) * scale - t * speed * 0.2, 1.0, 1.0),
            Effect::Gradient {
                from,
                to,
                speed,
                scale,
            } => {
                let phase = (x * scale - t * speed * 0.2).rem_euclid(1.0);
                from.lerp(to, 1.0 - (phase * 2.0 - 1.0).abs())
            }
            Effect::Wave {
                color,
                background,
                speed,
                width,
                angle,
            } => background.lerp(color, band(along(x, y, angle) - t * speed * 0.2, width)),
            Effect::Comet {
                color,
                background,
                speed,
                length,
                angle,
            } => {
                let behind = (t * speed * 0.3 - along(x, y, angle)).rem_euclid(1.0);
                let k = (1.0 - behind / length.max(1e-3)).clamp(0.0, 1.0);
                background.lerp(color, k * k)
            }
            Effect::Sparkle {
                color,
                background,
                speed,
                density,
            } => {
                let (cx, cy) = ((x * 256.0).floor() as i32, (y * 256.0).floor() as i32);
                let time = t * speed * 2.0 + hash(cx, cy, -1);
                let bucket = time.floor();
                if hash(cx, cy, bucket as i32) >= density {
                    return background;
                }
                background.lerp(color, (PI * (time - bucket)).sin())
            }
            Effect::Plasma { speed, scale } => {
                let (px, py, tt) = (x * scale * 6.0, y * scale * 6.0, t * speed);
                let v = (px + tt).sin()
                    + (py * 1.3 - tt * 0.8).sin()
                    + ((px + py) * 0.7 + tt * 0.6).sin()
                    + ((px - 3.0).hypot(py - 3.0) * 1.5 - tt).sin();
                Color::hsv(v * 0.125 + tt * 0.02, 1.0, 1.0)
            }
            Effect::Fire { speed, intensity } => {
                let rise = t * speed * 2.0;
                let n = 0.65 * noise(x * 6.0, y * 4.0 + rise)
                    + 0.35 * noise(x * 13.0, y * 9.0 + rise * 1.8);
                // Cooling with height lets the noise shape the flames.
                heat_color(0.7 * n + 0.45 - (1.0 - y) * 1.1 / intensity.max(0.05))
            }
            Effect::Ripple {
                color,
                background,
                speed,
                width,
                rings,
            } => {
                let r = (x - 0.5).hypot(y - 0.5);
                background.lerp(color, band(r * rings - t * speed * 0.3, width))
            }
            Effect::RadialRainbow {
                speed,
                repeats,
                twist,
            } => {
                let (dx, dy) = (x - 0.5, y - 0.5);
                let turn = dy.atan2(dx) / TAU;
                let hue = turn * repeats.round().max(1.0) + dx.hypot(dy) * twist - t * speed * 0.2;
                Color::hsv(hue, 1.0, 1.0)
            }
            Effect::ColorShift {
                first,
                second,
                third,
                use_third,
                speed,
            } => {
                let colors = [first, second, third];
                let count = if use_third { 3 } else { 2 };
                let p = (t * speed * 0.1).rem_euclid(1.0) * count as f32;
                let i = p as usize;
                colors[i % count].lerp(colors[(i + 1) % count], smooth(p.fract()))
            }
            Effect::AudioSpectrum {
                color,
                rainbow,
                sensitivity,
            } => {
                let i = ((x.clamp(0.0, 1.0) * AUDIO_BANDS as f32) as usize).min(AUDIO_BANDS - 1);
                let height = (inputs.audio.bands[i] * sensitivity).clamp(0.0, 1.0);
                let hue = i as f32 / AUDIO_BANDS as f32 * 0.85;
                let bar = if rainbow {
                    Color::hsv(hue, 1.0, 1.0)
                } else {
                    color
                };
                // A bar fades out over a sixteenth of the canvas at its top.
                bar.scale(height * 17.0 - (1.0 - y) * 16.0)
            }
            Effect::AudioPulse {
                color,
                sensitivity,
                floor,
            } => {
                let level = (inputs.audio.level * sensitivity).clamp(0.0, 1.0);
                color.scale(mix(floor, 1.0, level))
            }
            Effect::ScreenAmbience {
                saturation,
                brightness,
            } => {
                let Color(r, g, b) = inputs.screen.sample(x, y);
                let grey = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
                let f =
                    |c: u8| ((grey + (c as f32 - grey) * saturation) * brightness).round() as u8;
                Color(f(r), f(g), f(b))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioFrame, Press, ScreenFrame};

    fn at(e: &Effect, x: f32, y: f32, t: f32) -> Color {
        e.sample(x, y, t, &Inputs::default())
    }

    fn light(c: Color) -> f32 {
        c.0 as f32 + c.1 as f32 + c.2 as f32
    }

    fn busy_inputs() -> Inputs {
        let mut audio = AudioFrame {
            level: 0.6,
            bands: [0.0; AUDIO_BANDS],
        };
        for (i, b) in audio.bands.iter_mut().enumerate() {
            *b = i as f32 / AUDIO_BANDS as f32;
        }
        let pixels = (0..8u8).map(|i| Color(i * 30, 255 - i * 30, 90)).collect();
        Inputs {
            audio,
            screen: ScreenFrame {
                width: 4,
                height: 2,
                pixels,
            },
            presses: vec![
                Press {
                    x: 0.3,
                    y: 0.7,
                    age: 0.4,
                    hw: 0.02,
                    hh: 0.04,
                    seed: 0,
                },
                Press {
                    x: 0.8,
                    y: 0.9,
                    age: 0.1,
                    hw: 0.02,
                    hh: 0.04,
                    seed: 0,
                },
            ],
            keyboards: Vec::new(),
        }
    }

    #[test]
    fn static_ignores_position_and_time() {
        let e = Effect::Static {
            color: Color(1, 2, 3),
        };
        assert_eq!(at(&e, 0.0, 0.0, 0.0), at(&e, 0.9, 0.3, 42.0));
    }

    #[test]
    fn breathing_starts_dark() {
        let e = Effect::Breathing {
            color: Color(255, 255, 255),
            speed: 1.0,
        };
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
                assert!(
                    json.get(p.name).is_some(),
                    "{} has no field {}",
                    info.label,
                    p.name
                );
            }
        }
    }

    #[test]
    fn catalog_covers_every_field_with_a_matching_control() {
        for info in Effect::catalog() {
            let json = serde_json::to_value(&info.default).unwrap();
            let fields = json.as_object().unwrap();
            assert_eq!(fields.len() - 1, info.params.len(), "{}", info.label);
            for p in &info.params {
                let value = &fields[p.name];
                match p.control {
                    Control::Range { min, max, step } => {
                        let v = value.as_f64().unwrap() as f32;
                        assert!(min < max && step > 0.0, "{} {}", info.label, p.name);
                        assert!(
                            (min..=max).contains(&v),
                            "{} {} out of range",
                            info.label,
                            p.name
                        );
                    }
                    Control::Color => assert_eq!(value.as_array().unwrap().len(), 3),
                    Control::Toggle => assert!(value.is_boolean()),
                    Control::Choice { .. } => assert!(value.is_string()),
                    Control::Keys | Control::KeyColours => assert!(value.is_array()),
                }
            }
        }
    }

    #[test]
    fn catalog_labels_and_kinds_are_unique() {
        let catalog = Effect::catalog();
        assert_eq!(catalog.len(), 17);
        for (i, a) in catalog.iter().enumerate() {
            for b in &catalog[i + 1..] {
                assert_ne!(a.label, b.label);
                assert_ne!(
                    std::mem::discriminant(&a.default),
                    std::mem::discriminant(&b.default)
                );
            }
        }
    }

    #[test]
    fn defaults_round_trip_through_json() {
        for info in Effect::catalog() {
            let json = serde_json::to_string(&info.default).unwrap();
            let back: Effect = serde_json::from_str(&json).unwrap();
            assert_eq!(back, info.default, "{json}");
        }
    }

    #[test]
    fn kinds_are_snake_case() {
        let kinds: Vec<String> = Effect::catalog()
            .iter()
            .map(|i| {
                serde_json::to_value(&i.default).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let expected = [
            "none",
            "rainbow",
            "gradient",
            "cycle",
            "breathing",
            "static",
            "wave",
            "comet",
            "sparkle",
            "plasma",
            "fire",
            "ripple",
            "radial_rainbow",
            "color_shift",
            "audio_spectrum",
            "audio_pulse",
            "screen_ambience",
        ];
        assert_eq!(kinds, expected);
    }

    #[test]
    fn sampling_never_panics_and_is_deterministic() {
        let inputs = busy_inputs();
        let times = [0.0, 0.016, 1.0, 7.3, 1000.0, 1.0e7, -3.0];
        for info in Effect::catalog() {
            let e = &info.default;
            for &t in &times {
                for xi in 0..=8 {
                    for yi in 0..=8 {
                        let (x, y) = (xi as f32 / 8.0, yi as f32 / 8.0);
                        assert_eq!(
                            e.sample(x, y, t, &inputs),
                            e.sample(x, y, t, &inputs),
                            "{}",
                            info.label
                        );
                    }
                }
            }
            for v in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -1.0,
                2.0,
                1.0e30,
            ] {
                e.sample(v, v, v, &inputs);
                e.sample(v, 0.5, 1.0, &Inputs::default());
            }
        }
    }

    #[test]
    fn input_flags_match_what_sampling_reads() {
        let busy = busy_inputs();
        let audio = Inputs {
            audio: busy.audio,
            ..Inputs::default()
        };
        let screen = Inputs {
            screen: busy.screen.clone(),
            ..Inputs::default()
        };
        let keys = Inputs {
            presses: busy.presses,
            ..Inputs::default()
        };
        for info in Effect::catalog() {
            let e = &info.default;
            let reacts = |inputs: &Inputs| {
                (0..=8).any(|i| {
                    let (x, y) = (i as f32 / 8.0, 1.0 - i as f32 / 16.0);
                    e.sample(x, y, 2.5, inputs) != at(e, x, y, 2.5)
                })
            };
            assert_eq!(e.uses_audio(), reacts(&audio), "{}", info.label);
            assert_eq!(e.uses_screen(), reacts(&screen), "{}", info.label);
            assert!(!reacts(&keys), "{}", info.label);
        }
    }

    #[test]
    fn odd_parameter_values_never_panic() {
        let inputs = busy_inputs();
        for info in Effect::catalog() {
            let json = serde_json::to_value(&info.default).unwrap();
            for p in &info.params {
                let Control::Range { min, max, .. } = p.control else {
                    continue;
                };
                // 1e39 becomes infinity as an f32.
                for v in [
                    min as f64, max as f64, 0.0, -1.0, 1.0e-9, -1.0e6, 1.0e6, 1.0e39,
                ] {
                    let mut json = json.clone();
                    json[p.name] = v.into();
                    let e: Effect = serde_json::from_value(json).unwrap();
                    for (x, y, t) in [
                        (0.0, 0.0, 0.0),
                        (0.5, 0.5, 1.7),
                        (1.0, 1.0, 9.0e4),
                        (0.3, 0.9, -4.0),
                    ] {
                        e.sample(x, y, t, &inputs);
                    }
                }
            }
        }
    }

    #[test]
    fn wave_band_sits_on_the_background() {
        let (c, bg) = (Color(255, 0, 0), Color(0, 0, 40));
        let e = Effect::Wave {
            color: c,
            background: bg,
            speed: 1.0,
            width: 0.2,
            angle: 0.0,
        };
        assert_eq!(at(&e, 0.5, 0.5, 0.0), c);
        assert_eq!(at(&e, 0.1, 0.5, 0.0), bg);
        // One fifth of a period later the band has moved along x.
        assert_eq!(at(&e, 0.7, 0.2, 1.0), c);
        let up = Effect::Wave {
            color: c,
            background: bg,
            speed: 1.0,
            width: 0.2,
            angle: 90.0,
        };
        assert_eq!(at(&up, 0.1, 0.5, 0.0), c);
        assert_eq!(at(&up, 0.5, 0.1, 0.0), bg);
    }

    #[test]
    fn comet_head_leads_a_fading_tail() {
        let c = Color(255, 255, 255);
        let e = Effect::Comet {
            color: c,
            background: Color::BLACK,
            speed: 1.0,
            length: 0.5,
            angle: 0.0,
        };
        // At t = 1 the head is at x = 0.3.
        let head = light(at(&e, 0.3, 0.5, 1.0));
        let mid = light(at(&e, 0.15, 0.5, 1.0));
        let end = light(at(&e, 0.0, 0.5, 1.0));
        assert!(head > mid && mid > end, "{head} {mid} {end}");
        assert_eq!(at(&e, 0.4, 0.5, 1.0), Color::BLACK);
    }

    #[test]
    fn sparkle_density_sets_how_many_points_twinkle() {
        let (c, bg) = (Color(255, 255, 255), Color(0, 0, 20));
        let lit = |density: f32| {
            let e = Effect::Sparkle {
                color: c,
                background: bg,
                speed: 1.0,
                density,
            };
            let mut n = 0;
            for xi in 0..32 {
                for yi in 0..32 {
                    n += (at(&e, xi as f32 / 32.0, yi as f32 / 32.0, 3.1) != bg) as u32;
                }
            }
            n
        };
        assert_eq!(lit(0.0), 0);
        assert!((150..450).contains(&lit(0.3)), "{}", lit(0.3));
        assert!(lit(1.0) > 900, "{}", lit(1.0));
        // Every point comes and goes.
        let e = Effect::Sparkle {
            color: c,
            background: bg,
            speed: 1.0,
            density: 0.3,
        };
        for xi in 0..8 {
            let on = (0..40)
                .filter(|&i| at(&e, xi as f32 / 8.0, 0.4, i as f32 * 0.5) != bg)
                .count();
            assert!((1..40).contains(&on), "{on}");
        }
    }

    #[test]
    fn plasma_moves_and_varies() {
        let e = Effect::Plasma {
            speed: 1.0,
            scale: 1.0,
        };
        assert_ne!(at(&e, 0.1, 0.1, 0.0), at(&e, 0.6, 0.8, 0.0));
        assert_ne!(at(&e, 0.4, 0.4, 0.0), at(&e, 0.4, 0.4, 1.0));
    }

    #[test]
    fn fire_is_brighter_at_the_bottom() {
        let e = Effect::Fire {
            speed: 1.0,
            intensity: 1.0,
        };
        let mean = |y: f32| {
            let mut sum = 0.0;
            for xi in 0..16 {
                for ti in 0..40 {
                    sum += light(at(&e, xi as f32 / 16.0, y, ti as f32 * 0.13));
                }
            }
            sum / 640.0
        };
        let (top, middle, bottom) = (mean(0.05), mean(0.5), mean(0.95));
        assert!(bottom > middle && middle > top, "{top} {middle} {bottom}");
        assert!(bottom > 300.0 && top < 100.0, "{top} {bottom}");
    }

    #[test]
    fn fire_flickers() {
        let e = Effect::Fire {
            speed: 1.0,
            intensity: 1.0,
        };
        assert_ne!(at(&e, 0.5, 0.8, 0.0), at(&e, 0.5, 0.8, 0.4));
    }

    #[test]
    fn ripple_rings_are_round_and_expand() {
        let e = Effect::Ripple {
            color: CYAN,
            background: NAVY,
            speed: 1.0,
            width: 0.4,
            rings: 3.0,
        };
        for t in [0.0, 0.7, 2.0] {
            assert_eq!(at(&e, 0.8, 0.5, t), at(&e, 0.2, 0.5, t));
            assert_eq!(at(&e, 0.8, 0.5, t), at(&e, 0.5, 0.8, t));
        }
        // The ring at radius 0.25 has grown to 0.4 one second later.
        let ring = Effect::Ripple {
            color: CYAN,
            background: NAVY,
            speed: 1.0,
            width: 0.2,
            rings: 2.0,
        };
        assert_eq!(at(&ring, 0.75, 0.5, 0.0), CYAN);
        assert_eq!(at(&ring, 0.9, 0.5, 0.0), NAVY);
        assert_eq!(at(&ring, 0.9, 0.5, 1.0), CYAN);
        assert_eq!(at(&ring, 0.75, 0.5, 1.0), NAVY);
    }

    #[test]
    fn radial_rainbow_follows_the_angle_and_rotates() {
        let e = Effect::RadialRainbow {
            speed: 1.0,
            repeats: 1.0,
            twist: 0.0,
        };
        assert_eq!(at(&e, 0.9, 0.5, 0.0), Color(255, 0, 0));
        assert_eq!(at(&e, 0.9, 0.5, 0.0), at(&e, 0.6, 0.5, 0.0));
        assert_ne!(at(&e, 0.9, 0.5, 0.0), at(&e, 0.5, 0.9, 0.0));
        assert_ne!(at(&e, 0.9, 0.5, 0.0), at(&e, 0.9, 0.5, 1.0));
        let twice = Effect::RadialRainbow {
            speed: 1.0,
            repeats: 2.0,
            twist: 0.0,
        };
        assert_eq!(at(&twice, 0.9, 0.5, 0.0), at(&twice, 0.1, 0.5, 0.0));
    }

    #[test]
    fn colour_shift_visits_each_colour_in_turn() {
        let (a, b, c) = (Color(255, 0, 0), Color(0, 255, 0), Color(0, 0, 255));
        let three = Effect::ColorShift {
            first: a,
            second: b,
            third: c,
            use_third: true,
            speed: 1.0,
        };
        // Speed 1 is a ten second cycle.
        assert_eq!(at(&three, 0.2, 0.9, 0.0), a);
        assert_eq!(at(&three, 0.2, 0.9, 10.0 / 3.0 + 0.001), b);
        assert_eq!(at(&three, 0.2, 0.9, 20.0 / 3.0 + 0.001), c);
        assert_eq!(at(&three, 0.2, 0.9, 5.0), Color(0, 128, 128));
        let two = Effect::ColorShift {
            first: a,
            second: b,
            third: c,
            use_third: false,
            speed: 1.0,
        };
        assert_eq!(at(&two, 0.0, 0.0, 5.0), b);
        assert_eq!(at(&two, 0.0, 0.0, 2.5), Color(128, 128, 0));
        for i in 0..200 {
            assert_eq!(at(&two, 0.0, 0.0, i as f32 * 0.05).2, 0);
        }
    }

    #[test]
    fn spectrum_lights_the_columns_of_the_loud_band() {
        let mut inputs = Inputs::default();
        inputs.audio.bands[4] = 1.0;
        let green = Color(0, 255, 0);
        let e = Effect::AudioSpectrum {
            color: green,
            rainbow: false,
            sensitivity: 1.0,
        };
        for band in 0..AUDIO_BANDS {
            let x = (band as f32 + 0.5) / AUDIO_BANDS as f32;
            for y in [0.05, 0.5, 0.95] {
                let expected = if band == 4 { green } else { Color::BLACK };
                assert_eq!(e.sample(x, y, 0.0, &inputs), expected, "band {band} y {y}");
            }
        }
    }

    #[test]
    fn spectrum_bars_grow_from_the_bottom() {
        let mut inputs = Inputs::default();
        inputs.audio.bands = [0.5; AUDIO_BANDS];
        let e = Effect::AudioSpectrum {
            color: WHITE,
            rainbow: false,
            sensitivity: 1.0,
        };
        assert_eq!(e.sample(0.3, 0.9, 0.0, &inputs), WHITE);
        assert_eq!(e.sample(0.3, 0.2, 0.0, &inputs), Color::BLACK);
        let loud = Effect::AudioSpectrum {
            color: WHITE,
            rainbow: false,
            sensitivity: 2.0,
        };
        assert_eq!(loud.sample(0.3, 0.2, 0.0, &inputs), WHITE);
    }

    #[test]
    fn spectrum_colours_bars_by_band() {
        let mut inputs = Inputs::default();
        inputs.audio.bands = [1.0; AUDIO_BANDS];
        let e = Effect::AudioSpectrum {
            color: WHITE,
            rainbow: true,
            sensitivity: 1.0,
        };
        assert_eq!(e.sample(0.0, 0.5, 0.0, &inputs), Color(255, 0, 0));
        assert_ne!(
            e.sample(0.0, 0.5, 0.0, &inputs),
            e.sample(0.5, 0.5, 0.0, &inputs)
        );
        assert_ne!(
            e.sample(0.5, 0.5, 0.0, &inputs),
            e.sample(1.0, 0.5, 0.0, &inputs)
        );
    }

    #[test]
    fn pulse_follows_the_level() {
        let c = Color(200, 100, 0);
        let e = Effect::AudioPulse {
            color: c,
            sensitivity: 1.0,
            floor: 0.0,
        };
        let mut inputs = Inputs::default();
        assert_eq!(e.sample(0.5, 0.5, 0.0, &inputs), Color::BLACK);
        inputs.audio.level = 0.5;
        assert_eq!(e.sample(0.5, 0.5, 0.0, &inputs), Color(100, 50, 0));
        inputs.audio.level = 1.0;
        assert_eq!(e.sample(0.1, 0.9, 5.0, &inputs), c);
        let idle = Effect::AudioPulse {
            color: c,
            sensitivity: 1.0,
            floor: 0.5,
        };
        assert_eq!(
            idle.sample(0.5, 0.5, 0.0, &Inputs::default()),
            Color(100, 50, 0)
        );
    }

    #[test]
    fn ambience_returns_the_screen_pixel() {
        let inputs = busy_inputs();
        let e = Effect::ScreenAmbience {
            saturation: 1.0,
            brightness: 1.0,
        };
        for xi in 0..=10 {
            for yi in 0..=10 {
                let (x, y) = (xi as f32 / 10.0, yi as f32 / 10.0);
                assert_eq!(e.sample(x, y, 4.0, &inputs), inputs.screen.sample(x, y));
            }
        }
        assert_eq!(e.sample(0.5, 0.5, 0.0, &Inputs::default()), Color::BLACK);
    }

    #[test]
    fn ambience_boosts_saturation_and_brightness() {
        let pixel = Color(120, 100, 80);
        let screen = ScreenFrame {
            width: 1,
            height: 1,
            pixels: vec![pixel],
        };
        let inputs = Inputs {
            screen,
            ..Inputs::default()
        };
        let sample = |saturation, brightness| {
            Effect::ScreenAmbience {
                saturation,
                brightness,
            }
            .sample(0.5, 0.5, 0.0, &inputs)
        };
        let vivid = sample(2.0, 1.0);
        assert!(vivid.0 > pixel.0 && vivid.2 < pixel.2, "{vivid:?}");
        let grey = sample(0.0, 1.0);
        assert!(grey.0 == grey.1 && grey.1 == grey.2, "{grey:?}");
        assert_eq!(sample(1.0, 2.0), Color(240, 200, 160));
        assert_eq!(sample(1.0, 3.0), Color(255, 255, 240));
    }
}
