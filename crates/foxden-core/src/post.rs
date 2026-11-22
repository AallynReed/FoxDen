use crate::effect::{along, band, color, hash, range, smooth, toggle};
use crate::{
    Color, Control, Device, DeviceKind, Effect, Inputs, KeyTable, ParamInfo, Press, key_at,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How a layer meets what is under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    #[serde(alias = "over")]
    Normal,
    Add,
    Screen,
    Multiply,
    /// Recolours what is under it, keeping its brightness.
    Tint,
}

/// Where a layer's colour comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Colors {
    #[default]
    Single,
    /// From the colour to the second colour as it fades.
    FadeToSecond,
    /// Each press its own hue, in turn.
    Rainbow,
    Random,
    /// Hue by place on the canvas, left to right.
    ByPosition,
}

/// How a light dies away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    Linear,
    #[default]
    Smooth,
    Quick,
    Slow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lines {
    #[default]
    Both,
    Row,
    Column,
}

/// An effect drawn over the base effect.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Post {
    #[default]
    None,
    PressGhost {
        colors: Colors,
        color: Color,
        second: Color,
        hold: f32,
        fade: f32,
        curve: Curve,
        size: f32,
    },
    PressRipple {
        colors: Colors,
        color: Color,
        second: Color,
        speed: f32,
        width: f32,
        rings: f32,
        fade: f32,
        curve: Curve,
    },
    PressSplash {
        colors: Colors,
        color: Color,
        second: Color,
        speed: f32,
        fade: f32,
        curve: Curve,
    },
    PressCross {
        colors: Colors,
        color: Color,
        second: Color,
        lines: Lines,
        speed: f32,
        fade: f32,
        curve: Curve,
        size: f32,
    },
    Glitter {
        colors: Colors,
        color: Color,
        second: Color,
        speed: f32,
        density: f32,
    },
    Scanline {
        color: Color,
        speed: f32,
        width: f32,
        angle: f32,
        bounce: bool,
    },
    Breathe {
        speed: f32,
        depth: f32,
    },
    BeatFlash {
        color: Color,
        sensitivity: f32,
    },
    /// Chosen keys in one colour, as for a game's keybinds.
    KeyLights {
        color: Color,
        /// Windows virtual-key codes.
        keys: Vec<u32>,
    },
    /// A colour painted on each of some keys; the rest show what is below.
    KeyColours {
        /// Windows virtual-key code and colour.
        #[serde(default)]
        map: Vec<(u32, Color)>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PostInfo {
    pub label: &'static str,
    pub default: Post,
    pub params: Vec<ParamInfo>,
}

/// Which devices a layer is drawn on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    All,
    Keyboards,
    Chosen,
}

/// When a layer shows: always, or only while one of its apps runs or is in front.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    #[default]
    Always,
    Running,
    Front,
}

/// The apps running now and the one in front, as `app_name` gives them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Apps {
    pub running: HashSet<String>,
    pub front: Option<String>,
}

/// `Game.EXE` and `game` both become `game`.
pub fn app_name(name: &str) -> String {
    let name = name.trim().to_lowercase();
    match name.strip_suffix(".exe") {
        Some(stem) => stem.to_string(),
        None => name,
    }
}

/// What a layer draws: a key or overlay effect, or any base effect filling it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LayerEffect {
    Post(Post),
    Fill(Effect),
}

/// One layer of the stack over the base effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub enabled: bool,
    pub opacity: f32,
    pub blend: Blend,
    pub scope: Scope,
    /// Device ids, for `Scope::Chosen`.
    #[serde(default)]
    pub devices: Vec<String>,
    #[serde(default)]
    pub when: When,
    /// Executable names, for `When::Running` and `When::Front`.
    #[serde(default)]
    pub apps: Vec<String>,
    pub effect: LayerEffect,
}

impl Layer {
    pub fn new(effect: LayerEffect) -> Self {
        Layer {
            enabled: true,
            opacity: 1.0,
            blend: Blend::Normal,
            scope: Scope::All,
            devices: Vec::new(),
            when: When::Always,
            apps: Vec::new(),
            effect,
        }
    }

    /// Whether the layer is on and its apps let it show.
    pub fn live(&self, apps: &Apps) -> bool {
        let named = |name: &str| self.apps.iter().any(|a| app_name(a) == name);
        self.enabled
            && match self.when {
                When::Always => true,
                When::Running => apps.running.iter().any(|r| named(r)),
                When::Front => apps.front.as_deref().is_some_and(named),
            }
    }

    /// `below` with this layer drawn over it.
    pub fn apply(&self, below: Color, x: f32, y: f32, t: f32, inputs: &Inputs) -> Color {
        let (color, k) = match &self.effect {
            LayerEffect::Post(p) => p.render(x, y, t, inputs),
            LayerEffect::Fill(e) => (e.sample(x, y, t, inputs), 1.0),
        };
        mix(below, color, k * self.opacity.clamp(0.0, 1.0), self.blend)
    }

    /// `below` with a colour this layer worked out ahead drawn over it, or left alone without one.
    pub fn over(&self, below: Color, color: Option<Color>) -> Color {
        match color {
            Some(c) => mix(below, c, self.opacity.clamp(0.0, 1.0), self.blend),
            None => below,
        }
    }

    /// The key colour map of a key colours layer, as a table.
    pub fn key_table(&self) -> Option<KeyTable> {
        match &self.effect {
            LayerEffect::Post(Post::KeyColours { map }) => Some(crate::key_table(map)),
            _ => None,
        }
    }

    pub fn uses_keys(&self) -> bool {
        self.enabled && matches!(&self.effect, LayerEffect::Post(p) if p.uses_keys())
    }

    pub fn uses_audio(&self) -> bool {
        self.enabled
            && match &self.effect {
                LayerEffect::Post(p) => p.uses_audio(),
                LayerEffect::Fill(e) => e.uses_audio(),
            }
    }

    pub fn uses_screen(&self) -> bool {
        self.enabled && matches!(&self.effect, LayerEffect::Fill(e) if e.uses_screen())
    }

    /// True when the layer is on and drawn on `device`.
    pub fn covers(&self, device: &Device) -> bool {
        self.enabled
            && match self.scope {
                Scope::All => true,
                Scope::Keyboards => device.kind == DeviceKind::Keyboard,
                Scope::Chosen => self.devices.contains(&device.id),
            }
    }
}

const WHITE: Color = Color(255, 255, 255);
const ORANGE: Color = Color(255, 106, 0);
const CYAN: Color = Color(0, 200, 255);
const VIOLET: Color = Color(150, 60, 255);

fn choice(name: &'static str, label: &'static str, options: Vec<&'static str>) -> ParamInfo {
    ParamInfo {
        name,
        label,
        control: Control::Choice { options },
    }
}

fn shape(curve: Curve, k: f32) -> f32 {
    match curve {
        Curve::Linear => k,
        Curve::Smooth => smooth(k),
        Curve::Quick => k * k,
        Curve::Slow => k.sqrt(),
    }
}

/// How lit a press still is: full for `hold` seconds, then gone over `fade`.
fn life(p: &Press, hold: f32, fade: f32, curve: Curve) -> f32 {
    let k = if p.age < hold {
        1.0
    } else {
        1.0 - (p.age - hold) / fade.max(0.05)
    };
    if k.is_nan() {
        0.0
    } else {
        shape(curve, k.clamp(0.0, 1.0))
    }
}

/// The colour a press shows when `left` of its life remains.
fn shade(colors: Colors, color: Color, second: Color, p: &Press, t: f32, left: f32) -> Color {
    match colors {
        Colors::Single => color,
        Colors::FadeToSecond => second.lerp(color, left),
        Colors::Rainbow => Color::hsv((t - p.age) * 0.37, 1.0, 1.0),
        Colors::Random => Color::hsv(hash(p.seed as i32, 7, 3), 1.0, 1.0),
        Colors::ByPosition => Color::hsv(p.x, 1.0, 1.0),
    }
}

/// 1 inside the key, `size` keys across, softening over its outer fifth.
fn on_key(x: f32, y: f32, p: &Press, size: f32) -> f32 {
    let (rx, ry) = (p.hw.max(1e-4) * size, p.hh.max(1e-4) * size);
    let d = ((x - p.x) / rx).hypot((y - p.y) / ry);
    smooth(((1.0 - d) * 5.0).clamp(0.0, 1.0))
}

/// The brightest press at this point: how lit, and how much of its life is left.
fn strongest(inputs: &Inputs, f: impl Fn(&Press) -> (f32, f32)) -> Option<(f32, f32, &Press)> {
    inputs
        .presses
        .iter()
        .map(|p| {
            let (k, left) = f(p);
            (k, left, p)
        })
        .filter(|(k, _, _)| *k > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
}

/// `over` laid on `base` with `blend`, showing `k` of it.
pub fn mix(base: Color, over: Color, k: f32, blend: Blend) -> Color {
    let k = if k.is_nan() { 0.0 } else { k.clamp(0.0, 1.0) };
    match blend {
        Blend::Normal => base.lerp(over, k),
        Blend::Add => {
            let f = |a: u8, b: u8| (a as f32 + b as f32 * k).min(255.0).round() as u8;
            Color(f(base.0, over.0), f(base.1, over.1), f(base.2, over.2))
        }
        Blend::Screen => {
            let f = |a: u8, b: u8| 255 - ((255 - a) as u16 * (255 - b) as u16 / 255) as u8;
            base.lerp(
                Color(f(base.0, over.0), f(base.1, over.1), f(base.2, over.2)),
                k,
            )
        }
        Blend::Multiply => {
            let f = |a: u8, b: u8| (a as u16 * b as u16 / 255) as u8;
            base.lerp(
                Color(f(base.0, over.0), f(base.1, over.1), f(base.2, over.2)),
                k,
            )
        }
        Blend::Tint => {
            let level = base.0.max(base.1).max(base.2) as f32 / 255.0;
            base.lerp(over.scale(level), k)
        }
    }
}

impl Post {
    pub fn catalog() -> Vec<PostInfo> {
        let colors = || {
            choice(
                "colors",
                "Colours",
                vec![
                    "single",
                    "fade_to_second",
                    "rainbow",
                    "random",
                    "by_position",
                ],
            )
        };
        let main = || color("color", "Colour");
        let second = || color("second", "Second colour");
        let speed = || range("speed", "Speed", 0.1, 5.0, 0.05);
        let fade = || range("fade", "Fade time", 0.1, 4.0, 0.05);
        let curve = || {
            choice(
                "curve",
                "Fade curve",
                vec!["linear", "smooth", "quick", "slow"],
            )
        };
        vec![
            PostInfo {
                label: "Key ghosting",
                default: Post::PressGhost {
                    colors: Colors::Single,
                    color: WHITE,
                    second: VIOLET,
                    hold: 0.1,
                    fade: 1.0,
                    curve: Curve::Smooth,
                    size: 1.0,
                },
                params: vec![
                    colors(),
                    main(),
                    second(),
                    range("hold", "Hold", 0.0, 2.0, 0.05),
                    fade(),
                    curve(),
                    range("size", "Size", 0.5, 4.0, 0.1),
                ],
            },
            PostInfo {
                label: "Key ripple",
                default: Post::PressRipple {
                    colors: Colors::Single,
                    color: ORANGE,
                    second: VIOLET,
                    speed: 1.5,
                    width: 0.05,
                    rings: 1.0,
                    fade: 1.0,
                    curve: Curve::Smooth,
                },
                params: vec![
                    colors(),
                    main(),
                    second(),
                    speed(),
                    range("width", "Width", 0.01, 0.3, 0.01),
                    range("rings", "Rings", 1.0, 5.0, 1.0),
                    fade(),
                    curve(),
                ],
            },
            PostInfo {
                label: "Key splash",
                default: Post::PressSplash {
                    colors: Colors::Single,
                    color: CYAN,
                    second: VIOLET,
                    speed: 1.0,
                    fade: 0.8,
                    curve: Curve::Smooth,
                },
                params: vec![colors(), main(), second(), speed(), fade(), curve()],
            },
            PostInfo {
                label: "Key crosshair",
                default: Post::PressCross {
                    colors: Colors::Single,
                    color: WHITE,
                    second: VIOLET,
                    lines: Lines::Both,
                    speed: 2.0,
                    fade: 0.8,
                    curve: Curve::Smooth,
                    size: 1.0,
                },
                params: vec![
                    colors(),
                    main(),
                    second(),
                    choice("lines", "Lines", vec!["both", "row", "column"]),
                    speed(),
                    fade(),
                    curve(),
                    range("size", "Thickness", 0.5, 3.0, 0.1),
                ],
            },
            PostInfo {
                label: "Glitter",
                default: Post::Glitter {
                    colors: Colors::Single,
                    color: WHITE,
                    second: VIOLET,
                    speed: 1.0,
                    density: 0.05,
                },
                params: vec![
                    colors(),
                    main(),
                    second(),
                    speed(),
                    range("density", "Density", 0.01, 0.5, 0.01),
                ],
            },
            PostInfo {
                label: "Scanline",
                default: Post::Scanline {
                    color: WHITE,
                    speed: 1.0,
                    width: 0.15,
                    angle: 0.0,
                    bounce: false,
                },
                params: vec![
                    main(),
                    speed(),
                    range("width", "Width", 0.05, 1.0, 0.05),
                    range("angle", "Angle", 0.0, 360.0, 1.0),
                    toggle("bounce", "Bounce back"),
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
                },
                params: vec![main(), range("sensitivity", "Sensitivity", 0.2, 5.0, 0.05)],
            },
            PostInfo {
                label: "Chosen keys",
                default: Post::KeyLights {
                    color: ORANGE,
                    keys: vec![0x57, 0x41, 0x53, 0x44],
                },
                params: vec![
                    main(),
                    ParamInfo {
                        name: "keys",
                        label: "Keys",
                        control: Control::Keys,
                    },
                ],
            },
            PostInfo {
                label: "Key colours",
                default: Post::KeyColours { map: Vec::new() },
                params: vec![ParamInfo {
                    name: "map",
                    label: "Key colours",
                    control: Control::KeyColours,
                }],
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

    /// The layer's colour at `x`, `y` and how much of it shows, 0..=1.
    pub fn render(&self, x: f32, y: f32, t: f32, inputs: &Inputs) -> (Color, f32) {
        match *self {
            Post::None => (Color::BLACK, 0.0),
            Post::PressRipple {
                colors,
                color,
                second,
                speed,
                width,
                rings,
                fade,
                curve,
            } => {
                let rings = rings.clamp(1.0, 8.0).round();
                let half = (width * 0.5).max(1e-3);
                let hit = strongest(inputs, |p| {
                    let off = (x - p.x).hypot(y - p.y) - p.age * speed * 0.3;
                    // Later rings trail the first, one ring width and a gap apart.
                    let gap = width * 2.5;
                    let n = (-off / gap).round().clamp(0.0, rings - 1.0);
                    let ring = smooth((1.0 - (off + n * gap).abs() / half).clamp(0.0, 1.0));
                    let left = life(p, 0.0, fade, Curve::Linear);
                    (ring * shape(curve, left), left)
                });
                match hit {
                    Some((k, left, p)) => (shade(colors, color, second, p, t, left), k),
                    None => (Color::BLACK, 0.0),
                }
            }
            Post::PressGhost {
                colors,
                color,
                second,
                hold,
                fade,
                curve,
                size,
            } => {
                let hit = strongest(inputs, |p| {
                    let left = life(p, hold, fade, Curve::Linear);
                    (on_key(x, y, p, size) * shape(curve, left), left)
                });
                match hit {
                    Some((k, left, p)) => (shade(colors, color, second, p, t, left), k),
                    None => (Color::BLACK, 0.0),
                }
            }
            Post::PressSplash {
                colors,
                color,
                second,
                speed,
                fade,
                curve,
            } => {
                let hit = strongest(inputs, |p| {
                    let reach = p.hw.max(p.hh) + p.age * speed * 0.3;
                    let edge = (x - p.x).hypot(y - p.y) / reach.max(1e-4);
                    let disc = smooth(((1.0 - edge) * 4.0).clamp(0.0, 1.0));
                    let left = life(p, 0.0, fade, Curve::Linear);
                    (disc * shape(curve, left), left)
                });
                match hit {
                    Some((k, left, p)) => (shade(colors, color, second, p, t, left), k),
                    None => (Color::BLACK, 0.0),
                }
            }
            Post::PressCross {
                colors,
                color,
                second,
                lines,
                speed,
                fade,
                curve,
                size,
            } => {
                let hit = strongest(inputs, |p| {
                    let reach = p.age * speed * 0.5;
                    let (dx, dy) = ((x - p.x).abs(), (y - p.y).abs());
                    let row = lines != Lines::Column && dy <= p.hh * size && dx <= p.hw + reach;
                    let col = lines != Lines::Row && dx <= p.hw * size && dy <= p.hh + reach;
                    let left = life(p, 0.0, fade, Curve::Linear);
                    let k = if row || col { shape(curve, left) } else { 0.0 };
                    (k, left)
                });
                match hit {
                    Some((k, left, p)) => (shade(colors, color, second, p, t, left), k),
                    None => (Color::BLACK, 0.0),
                }
            }
            Post::Glitter {
                colors,
                color,
                second,
                speed,
                density,
            } => {
                let (cx, cy) = ((x * 256.0).floor() as i32, (y * 256.0).floor() as i32);
                let time = t * speed * 2.0 + hash(cx, cy, -2);
                let bucket = time.floor();
                if hash(cx, cy, bucket as i32 ^ 0x5bd1) >= density {
                    return (Color::BLACK, 0.0);
                }
                let k = (std::f32::consts::PI * (time - bucket)).sin();
                let c = match colors {
                    Colors::Single => color,
                    Colors::FadeToSecond => second.lerp(color, k),
                    Colors::Rainbow | Colors::Random => {
                        Color::hsv(hash(cx, cy, bucket as i32), 1.0, 1.0)
                    }
                    Colors::ByPosition => Color::hsv(x, 1.0, 1.0),
                };
                (c, k)
            }
            Post::Scanline {
                color,
                speed,
                width,
                angle,
                bounce,
            } => {
                let mut phase = t * speed * 0.2;
                if bounce {
                    // Back and forth across the canvas, the band fully off it at each end.
                    phase = 1.0 - (phase.rem_euclid(2.0) - 1.0).abs();
                    let d = along(x, y, angle) - (phase * (1.0 + width) - width * 0.5);
                    let k = smooth((1.0 - d.abs() / (width * 0.5).max(1e-3)).clamp(0.0, 1.0));
                    return (color, k);
                }
                let k = band(along(x, y, angle) - phase, width);
                (color, k)
            }
            Post::Breathe { speed, depth } => {
                let dip = 0.5 + 0.5 * (t * speed * 2.0).cos();
                (Color::BLACK, depth.clamp(0.0, 1.0) * (1.0 - dip))
            }
            Post::BeatFlash { color, sensitivity } => {
                let level = (inputs.audio.level * sensitivity).clamp(0.0, 1.0);
                (color, level * level)
            }
            Post::KeyLights { color, ref keys } => {
                let lit = keys.iter().any(|&vk| key_at(vk, x, y, &inputs.keyboards));
                (color, if lit { 1.0 } else { 0.0 })
            }
            Post::KeyColours { ref map } => map
                .iter()
                .find(|&&(vk, _)| key_at(vk, x, y, &inputs.keyboards))
                .map_or((Color::BLACK, 0.0), |&(_, c)| (c, 1.0)),
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
            seed: 1,
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

    trait On {
        fn apply(&self, base: Color, x: f32, y: f32, t: f32, inputs: &Inputs) -> Color;
    }

    impl On for Post {
        fn apply(&self, base: Color, x: f32, y: f32, t: f32, inputs: &Inputs) -> Color {
            let (c, k) = self.render(x, y, t, inputs);
            mix(base, c, k, Blend::Normal)
        }
    }

    fn ghost(colors: Colors, hold: f32, curve: Curve) -> Post {
        Post::PressGhost {
            colors,
            color: WHITE,
            second: Color(255, 0, 0),
            hold,
            fade: 1.0,
            curve,
            size: 1.0,
        }
    }

    fn plain_ghost() -> Post {
        ghost(Colors::Single, 0.0, Curve::Smooth)
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
                        assert!(
                            min < max && *step > 0.0 && (*min..=*max).contains(&v),
                            "{} {}",
                            info.label,
                            p.name
                        );
                    }
                    Control::Color => assert_eq!(value.as_array().unwrap().len(), 3),
                    Control::Toggle => assert!(value.is_boolean()),
                    Control::Keys | Control::KeyColours | Control::Stops => {
                        assert!(value.is_array())
                    }
                    Control::Sensor => assert!(value.is_string()),
                    Control::Choice { options } => {
                        assert!(
                            options.contains(&value.as_str().unwrap()),
                            "{} {}",
                            info.label,
                            p.name
                        );
                        for o in options {
                            let mut v = json.clone();
                            v[p.name] = serde_json::json!(o);
                            serde_json::from_value::<Post>(v).unwrap();
                        }
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
            .map(|i| {
                serde_json::to_value(&i.default).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        for info in Post::catalog() {
            let kind = serde_json::to_value(&info.default).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string();
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
                        e.apply(BASE, x, y, 2.5, inputs)
                            != e.apply(BASE, x, y, 2.5, &Inputs::default())
                    })
                })
            };
            assert_eq!(e.uses_audio(), reacts(&audio), "{}", info.label);
            assert_eq!(e.uses_keys(), reacts(&presses), "{}", info.label);
        }
    }

    #[test]
    fn ghosting_lights_only_the_key_then_goes_out() {
        let e = plain_ghost();
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.0)), WHITE);
        assert_eq!(e.apply(BASE, 0.54, 0.5, 0.0, &keys(0.0)), BASE);
        assert_eq!(e.apply(BASE, 0.5, 0.66, 0.0, &keys(0.0)), BASE);
        let half = e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.5));
        assert!(half != WHITE && half != BASE, "{half:?}");
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(1.0)), BASE);
    }

    #[test]
    fn ghosting_holds_before_fading() {
        let e = ghost(Colors::Single, 0.5, Curve::Smooth);
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.4)), WHITE);
    }

    #[test]
    fn curves_shape_the_fade() {
        let at = |curve| {
            let c =
                ghost(Colors::Single, 0.0, curve).apply(Color::BLACK, 0.5, 0.5, 0.0, &keys(0.5));
            c.0
        };
        assert!(at(Curve::Quick) < at(Curve::Linear));
        assert!(at(Curve::Linear) < at(Curve::Slow));
    }

    #[test]
    fn fade_to_second_ends_on_the_second_colour() {
        let e = ghost(Colors::FadeToSecond, 0.0, Curve::Linear);
        let start = e.apply(Color::BLACK, 0.5, 0.5, 0.0, &keys(0.0));
        let late = e.apply(Color::BLACK, 0.5, 0.5, 0.0, &keys(0.9));
        assert_eq!(start, WHITE);
        assert!(late.0 > late.1 && late.1 == late.2, "{late:?}");
    }

    #[test]
    fn rainbow_and_random_presses_keep_their_colour() {
        for colors in [Colors::Rainbow, Colors::Random] {
            let e = ghost(colors, 2.0, Curve::Smooth);
            let a = e.apply(BASE, 0.5, 0.5, 1.2, &keys(0.2));
            let b = e.apply(BASE, 0.5, 0.5, 1.7, &keys(0.7));
            assert_eq!(a, b);
            assert_ne!(a, WHITE);
        }
        let random = ghost(Colors::Random, 2.0, Curve::Smooth);
        let mut other = keys(0.2);
        other.presses[0].seed = 2;
        assert_ne!(
            random.apply(BASE, 0.5, 0.5, 1.2, &keys(0.2)),
            random.apply(BASE, 0.5, 0.5, 1.2, &other)
        );
    }

    #[test]
    fn ripple_rings_spread_from_the_key_and_fade() {
        let ripple = |rings| Post::PressRipple {
            colors: Colors::Single,
            color: WHITE,
            second: WHITE,
            speed: 1.0,
            width: 0.05,
            rings,
            fade: 1.0,
            curve: Curve::Smooth,
        };
        let e = ripple(1.0);
        assert_ne!(e.apply(BASE, 0.65, 0.5, 0.0, &keys(0.5)), BASE);
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &keys(0.5)), BASE);
        assert_eq!(e.apply(BASE, 0.65, 0.5, 0.0, &keys(1.2)), BASE);
        // A second ring trails the first.
        let inner = 0.65 - 0.05 * 2.5;
        assert_eq!(e.apply(BASE, inner, 0.5, 0.0, &keys(0.5)), BASE);
        assert_ne!(ripple(2.0).apply(BASE, inner, 0.5, 0.0, &keys(0.5)), BASE);
    }

    #[test]
    fn blends_mix_like_layers() {
        let (b, w) = (BASE, WHITE);
        assert_eq!(mix(b, w, 0.5, Blend::Normal), b.lerp(w, 0.5));
        assert_eq!(mix(b, w, 0.5, Blend::Add), Color(138, 168, 218));
        assert_eq!(mix(b, w, 1.0, Blend::Screen), w);
        assert_eq!(mix(b, w, 1.0, Blend::Multiply), b);
        assert_eq!(mix(Color::BLACK, w, 1.0, Blend::Tint), Color::BLACK);
        assert_eq!(mix(b, Color::BLACK, 1.0, Blend::Screen), b);
    }

    #[test]
    fn a_layer_applies_its_opacity_and_blend() {
        let mut layer = Layer::new(LayerEffect::Fill(Effect::Static { color: WHITE }));
        assert_eq!(layer.apply(BASE, 0.5, 0.5, 0.0, &Inputs::default()), WHITE);
        layer.opacity = 0.5;
        assert_eq!(
            layer.apply(BASE, 0.5, 0.5, 0.0, &Inputs::default()),
            BASE.lerp(WHITE, 0.5)
        );
        layer.enabled = false;
        assert!(!layer.uses_keys());
    }

    #[test]
    fn layers_read_back_either_kind_of_effect() {
        for layer in [
            Layer::new(LayerEffect::Post(ghost(Colors::Single, 0.1, Curve::Smooth))),
            Layer::new(LayerEffect::Fill(Effect::default())),
        ] {
            let json = serde_json::to_string(&layer).unwrap();
            assert_eq!(
                serde_json::from_str::<Layer>(&json).unwrap(),
                layer,
                "{json}"
            );
        }
    }

    #[test]
    fn crosshair_lights_the_chosen_lines() {
        let cross = |lines| Post::PressCross {
            colors: Colors::Single,
            color: WHITE,
            second: WHITE,
            lines,
            speed: 2.0,
            fade: 2.0,
            curve: Curve::Smooth,
            size: 1.0,
        };
        let i = keys(0.2);
        let both = cross(Lines::Both);
        assert_ne!(both.apply(BASE, 0.65, 0.5, 0.0, &i), BASE);
        assert_ne!(both.apply(BASE, 0.5, 0.65, 0.0, &i), BASE);
        assert_eq!(both.apply(BASE, 0.65, 0.65, 0.0, &i), BASE);
        assert_eq!(cross(Lines::Row).apply(BASE, 0.5, 0.65, 0.0, &i), BASE);
        assert_eq!(cross(Lines::Column).apply(BASE, 0.65, 0.5, 0.0, &i), BASE);
    }

    #[test]
    fn scanline_can_bounce() {
        let line = Post::Scanline {
            color: WHITE,
            speed: 1.0,
            width: 0.1,
            angle: 0.0,
            bounce: true,
        };
        // Halfway there at t = 2.5 and halfway back at t = 7.5.
        assert_eq!(line.apply(BASE, 0.5, 0.5, 2.5, &Inputs::default()), WHITE);
        assert_eq!(line.apply(BASE, 0.5, 0.5, 7.5, &Inputs::default()), WHITE);
        assert_eq!(line.apply(BASE, 0.9, 0.5, 2.5, &Inputs::default()), BASE);
    }

    #[test]
    fn breathe_dims_the_base() {
        let e = Post::Breathe {
            speed: 1.0,
            depth: 1.0,
        };
        assert_eq!(e.apply(BASE, 0.5, 0.5, 0.0, &Inputs::default()), BASE);
        let low = std::f32::consts::FRAC_PI_2;
        assert_eq!(
            e.apply(BASE, 0.5, 0.5, low, &Inputs::default()),
            Color::BLACK
        );
    }

    #[test]
    fn layers_cover_their_scope() {
        let board = Device::grid("kb", "Board", DeviceKind::Keyboard, [0.0; 4], 1, 1);
        let ram = Device::grid("ram", "RAM", DeviceKind::Memory, [0.0; 4], 1, 1);
        let mut layer = Layer::new(LayerEffect::Post(plain_ghost()));
        assert!(layer.covers(&board) && layer.covers(&ram));
        layer.scope = Scope::Keyboards;
        assert!(layer.covers(&board) && !layer.covers(&ram));
        layer.scope = Scope::Chosen;
        layer.devices = vec!["ram".into()];
        assert!(!layer.covers(&board) && layer.covers(&ram));
        layer.enabled = false;
        assert!(!layer.covers(&ram));
    }

    #[test]
    fn chosen_keys_light_only_on_keyboards() {
        let keys = Post::KeyLights {
            color: WHITE,
            keys: vec![0x41],
        };
        let board = Inputs {
            keyboards: vec![([0.0, 0.0, 1.0, 1.0], false)],
            ..Inputs::default()
        };
        let (ax, ay) = crate::key_spot(0x41, false).unwrap();
        assert_eq!(keys.render(ax, ay, 0.0, &board), (WHITE, 1.0));
        assert_eq!(keys.render(ax + 0.1, ay, 0.0, &board).1, 0.0);
        assert_eq!(keys.render(ax, ay, 0.0, &Inputs::default()).1, 0.0);
        assert!(!keys.uses_keys());
    }

    #[test]
    fn key_colours_load_without_a_map_and_light_painted_keys() {
        let v = serde_json::json!({"enabled": true, "opacity": 1, "blend": "normal", "scope": "keyboards", "effect": {"kind": "key_colours"}});
        let layer: Layer = serde_json::from_value(v).unwrap();
        assert_eq!(
            layer.effect,
            LayerEffect::Post(Post::KeyColours { map: vec![] })
        );
        let red = Color(255, 0, 0);
        let keys = Post::KeyColours {
            map: vec![(0x41, red)],
        };
        let board = Inputs {
            keyboards: vec![([0.0, 0.0, 1.0, 1.0], false)],
            ..Inputs::default()
        };
        let (ax, ay) = crate::key_spot(0x41, false).unwrap();
        assert_eq!(keys.render(ax, ay, 0.0, &board), (red, 1.0));
        assert_eq!(keys.render(ax + 0.1, ay, 0.0, &board).1, 0.0);
        let back: Post = serde_json::from_value(serde_json::to_value(&keys).unwrap()).unwrap();
        assert_eq!(back, keys);
    }

    #[test]
    fn layers_follow_their_apps() {
        let mut layer = Layer::new(LayerEffect::Post(plain_ghost()));
        let none = Apps::default();
        let game = Apps {
            running: ["game".into(), "discord".into()].into(),
            front: Some("discord".into()),
        };
        assert!(layer.live(&none));
        layer.when = When::Running;
        assert!(!layer.live(&game));
        layer.apps = vec!["Game.EXE".into()];
        assert!(layer.live(&game) && !layer.live(&none));
        layer.when = When::Front;
        assert!(!layer.live(&game));
        layer.apps.push("discord".into());
        assert!(layer.live(&game));
        layer.enabled = false;
        assert!(!layer.live(&game));
        assert_eq!(app_name(" Game.exe "), "game");
    }

    #[test]
    fn older_layers_read_as_always() {
        let mut v = serde_json::to_value(Layer::new(LayerEffect::Post(plain_ghost()))).unwrap();
        v.as_object_mut()
            .unwrap()
            .retain(|k, _| k != "when" && k != "apps");
        let layer: Layer = serde_json::from_value(v).unwrap();
        assert_eq!((layer.when, layer.apps.len()), (When::Always, 0));
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
