//! The device layout, drawn the way the web canvas draws it.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use slint::{Rgba8Pixel, SharedPixelBuffer};
use tiny_skia::{
    Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapMut, Rect, Stroke, Transform,
};

const PAD: f32 = 20.0;
const MIN_SIZE: f32 = 0.02;

#[derive(Clone, Debug, PartialEq)]
pub struct Dev {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub rect: [f32; 4],
    pub grid: [u32; 2],
    pub leds: Vec<[f32; 2]>,
    pub enabled: bool,
    pub brightness: f32,
}

/// View options, kept per user in a file of their own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewOpts {
    pub snap: bool,
    pub grid: f32,
    pub labels: bool,
    pub shapes: bool,
    pub keys: bool,
    pub zoom: f32,
}

impl Default for ViewOpts {
    fn default() -> Self {
        ViewOpts {
            snap: false,
            grid: 0.05,
            labels: true,
            shapes: true,
            keys: true,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Move,
    Resize,
}

struct Drag {
    id: String,
    mode: Mode,
    from: (f32, f32),
    origin: [f32; 4],
    rect: [f32; 4],
    moved: bool,
    done: bool,
}

pub struct Label {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub text: String,
    pub strong: bool,
}

#[derive(Default)]
pub struct Canvas {
    pub devices: Vec<Dev>,
    pub colors: HashMap<String, Vec<[u8; 3]>>,
    pub selected: Option<String>,
    pub hover: Option<String>,
    pub light: bool,
    pub view: ViewOpts,
    /// 0 for the default pointer, 1 over a device, 2 over the resize handle.
    pub cursor: i32,
    drag: Option<Drag>,
    w: f32,
    h: f32,
    cache: Cache,
}

/// The device bodies, drawn once per layout; each frame only adds the LEDs.
#[derive(Default)]
struct Cache {
    key: u64,
    base: Option<Pixmap>,
    masks: HashMap<String, Mask>,
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// Bodies for the image mode, which adds LEDs each frame.
    Base,
    Leds,
    All,
    /// Bodies and selection, for the window to draw the LEDs over.
    Bodies,
}

struct Ink {
    plate: Color,
    line: Color,
    stroke: Color,
    text: Color,
    accent: Color,
    unlit: Color,
    bg: Color,
}

fn hex(v: u32) -> Color {
    Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
}

impl Ink {
    fn new(light: bool) -> Self {
        if light {
            Ink {
                plate: hex(0x1b1b1f),
                line: hex(0xd0d0d6),
                stroke: hex(0x6e6f78),
                text: hex(0x1b1b1f),
                accent: hex(0xa84a00),
                unlit: hex(0x303137),
                bg: hex(0xe7e7ea),
            }
        } else {
            Ink {
                plate: hex(0x0f1011),
                line: hex(0x36373c),
                stroke: hex(0x84858e),
                text: hex(0xe7e7ea),
                accent: hex(0xe8822e),
                unlit: hex(0x232428),
                bg: hex(0x17181a),
            }
        }
    }
}

struct Painter<'a> {
    pm: PixmapMut<'a>,
    ts: Transform,
    alpha: f32,
    mask: Option<Mask>,
}

impl Painter<'_> {
    fn paint(&self, mut c: Color) -> Paint<'static> {
        c.apply_opacity(self.alpha);
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = true;
        p
    }

    fn fill_rect(&mut self, c: Color, x: f32, y: f32, w: f32, h: f32) {
        if let Some(r) = Rect::from_xywh(x, y, w.max(0.01), h.max(0.01)) {
            let p = self.paint(c);
            let path = PathBuilder::from_rect(r);
            self.pm.fill_path(&path, &p, FillRule::Winding, self.ts, self.mask.as_ref());
        }
    }

    fn fill(&mut self, c: Color, pb: PathBuilder) {
        if let Some(path) = pb.finish() {
            let p = self.paint(c);
            self.pm.fill_path(&path, &p, FillRule::Winding, self.ts, self.mask.as_ref());
        }
    }

    fn stroke(&mut self, c: Color, width: f32, pb: PathBuilder) {
        if let Some(path) = pb.finish() {
            let p = self.paint(c);
            let s = Stroke {
                width,
                ..Stroke::default()
            };
            self.pm.stroke_path(&path, &p, &s, self.ts, self.mask.as_ref());
        }
    }

    fn stroke_rect(&mut self, c: Color, x: f32, y: f32, w: f32, h: f32) {
        self.stroke(c, 1.0, rect_path(x, y, w, h));
    }
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> PathBuilder {
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w, y);
    pb.line_to(x + w, y + h);
    pb.line_to(x, y + h);
    pb.close();
    pb
}

fn cut_path(x: f32, y: f32, w: f32, h: f32, cut: f32) -> PathBuilder {
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w - cut, y);
    pb.line_to(x + w, y + cut);
    pb.line_to(x + w, y + h);
    pb.line_to(x, y + h);
    pb.close();
    pb
}

fn mouse_path(x: f32, y: f32, w: f32, h: f32) -> PathBuilder {
    let mut pb = PathBuilder::new();
    pb.move_to(x, y + h * 0.42);
    pb.cubic_to(x, y + h * 0.08, x + w * 0.2, y, x + w / 2.0, y);
    pb.cubic_to(x + w * 0.8, y, x + w, y + h * 0.08, x + w, y + h * 0.42);
    pb.line_to(x + w * 0.96, y + h * 0.78);
    pb.cubic_to(x + w * 0.92, y + h, x + w * 0.08, y + h, x + w * 0.04, y + h * 0.78);
    pb.close();
    pb
}

fn circle(cx: f32, cy: f32, r: f32) -> PathBuilder {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r.max(0.1));
    pb
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> PathBuilder {
    let steps = (((a1 - a0).abs() * r / 3.0).ceil() as usize).clamp(2, 64);
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
        let (x, y) = (cx + a.cos() * r, cy + a.sin() * r);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb
}

fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> PathBuilder {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    pb
}

fn is_ring(leds: &[[f32; 2]]) -> bool {
    if leds.len() < 8 {
        return false;
    }
    let n = leds.len() as f32;
    let cx = leds.iter().map(|l| l[0]).sum::<f32>() / n;
    let cy = leds.iter().map(|l| l[1]).sum::<f32>() / n;
    let d: Vec<f32> = leds.iter().map(|l| (l[0] - cx).hypot(l[1] - cy)).collect();
    let mean = d.iter().sum::<f32>() / n;
    let spread = (d.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n).sqrt();
    mean > 0.2 && spread / mean < 0.15
}

fn shape_of(d: &Dev, w: f32, h: f32) -> &'static str {
    if is_ring(&d.leds) {
        return "fan";
    }
    let long = w.max(h) / w.min(h).max(1.0);
    match d.kind.as_str() {
        "keyboard" => "keyboard",
        "mouse" => "mouse",
        "memory" => "memory",
        "gpu" if long > 16.0 => "strip",
        "gpu" => "gpu",
        "motherboard" if long > 3.0 => "strip",
        "motherboard" => "board",
        "strip" => "strip",
        _ if long > 6.0 => "strip",
        _ => "plate",
    }
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi.max(lo))
}

fn tidy(r: [f32; 4]) -> [f32; 4] {
    r.map(|v| (v * 1e4).round() / 1e4)
}

impl Canvas {
    pub fn find(&self, id: &str) -> Option<&Dev> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn rect_of(&self, d: &Dev) -> [f32; 4] {
        match &self.drag {
            Some(drag) if drag.id == d.id => drag.rect,
            _ => d.rect,
        }
    }

    fn to_px(&self, [x, y, w, h]: [f32; 4]) -> [f32; 4] {
        let cw = self.w - PAD * 2.0;
        let ch = self.h - PAD * 2.0;
        [
            (PAD + x * cw).round(),
            (PAD + y * ch).round(),
            (w * cw).round().max(2.0),
            (h * ch).round().max(2.0),
        ]
    }

    fn stack(&self) -> Vec<&Dev> {
        let mut v: Vec<&Dev> = self.devices.iter().collect();
        v.sort_by_key(|d| Some(&d.id) == self.selected.as_ref());
        v
    }

    fn hit(&self, px: f32, py: f32) -> Option<(String, Mode)> {
        if let Some(sel) = self.selected.as_deref().and_then(|id| self.find(id)) {
            let [x, y, w, h] = self.to_px(self.rect_of(sel));
            if (px - x - w).abs() <= 8.0 && (py - y - h).abs() <= 8.0 {
                return Some((sel.id.clone(), Mode::Resize));
            }
        }
        self.stack().into_iter().rev().find_map(|d| {
            let [x, y, w, h] = self.to_px(self.rect_of(d));
            (px >= x - 3.0 && px <= x + w + 3.0 && py >= y - 3.0 && py <= y + h + 3.0)
                .then(|| (d.id.clone(), Mode::Move))
        })
    }

    pub fn down(&mut self, px: f32, py: f32) {
        let hit = self.hit(px, py);
        self.selected = hit.as_ref().map(|(id, _)| id.clone());
        if let Some((id, mode)) = hit {
            let rect = self.find(&id).map(|d| self.rect_of(d)).unwrap_or_default();
            self.drag = Some(Drag {
                id,
                mode,
                from: (px, py),
                origin: rect,
                rect,
                moved: false,
                done: false,
            });
        }
    }

    pub fn moved(&mut self, px: f32, py: f32, pressed: bool, alt: bool) -> bool {
        let (cw, ch) = (self.w - PAD * 2.0, self.h - PAD * 2.0);
        let grid = self.view.grid.max(0.001);
        let snap = self.view.snap && !alt;
        let fit = |v: f32| if snap { (v / grid).round() * grid } else { v };
        match self.drag.as_mut() {
            Some(drag) if pressed && !drag.done => {
                if !drag.moved && (px - drag.from.0).hypot(py - drag.from.1) < 3.0 {
                    return false;
                }
                let dx = (px - drag.from.0) / cw;
                let dy = (py - drag.from.1) / ch;
                let [x, y, w, h] = drag.origin;
                drag.rect = match drag.mode {
                    Mode::Move => [
                        clamp(fit(x + dx), 0.0, 1.0 - w),
                        clamp(fit(y + dy), 0.0, 1.0 - h),
                        w,
                        h,
                    ],
                    Mode::Resize => [
                        x,
                        y,
                        clamp(fit(x + w + dx) - x, MIN_SIZE, 1.0 - x),
                        clamp(fit(y + h + dy) - y, MIN_SIZE, 1.0 - y),
                    ],
                };
                drag.moved = true;
                true
            }
            _ => {
                let hit = self.hit(px, py);
                self.cursor = match hit {
                    None => 0,
                    Some((_, Mode::Move)) => 1,
                    Some((_, Mode::Resize)) => 2,
                };
                let hover = hit.map(|(id, _)| id);
                let changed = hover != self.hover;
                self.hover = hover;
                changed
            }
        }
    }

    pub fn leave(&mut self) -> bool {
        self.cursor = 0;
        self.hover.take().is_some()
    }

    /// Selects the next device in the list, or the previous one with -1.
    pub fn select_next(&mut self, by: i32) -> Option<String> {
        let n = self.devices.len() as i32;
        if n == 0 {
            return None;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|id| self.devices.iter().position(|d| &d.id == id));
        let i = match at {
            None if by > 0 => 0,
            None => n - 1,
            Some(i) => (i as i32 + by).rem_euclid(n),
        };
        let d = &self.devices[i as usize];
        self.selected = Some(d.id.clone());
        Some(d.name.clone())
    }

    /// The selected device's name and place, as the tool bar shows it.
    pub fn readout(&self) -> String {
        let Some(d) = self.selected.as_deref().and_then(|id| self.find(id)) else {
            return String::new();
        };
        let r = self.rect_of(d);
        let parts: Vec<String> = "XYWH"
            .chars()
            .zip(r)
            .map(|(c, v)| format!("{c} {:.1}", v * 100.0))
            .collect();
        format!("{}  {}", d.name, parts.join("  "))
    }

    /// The rect to save once a drag ends, if the device moved.
    pub fn up(&mut self) -> Option<(Dev, [f32; 4])> {
        let drag = self.drag.as_mut()?;
        if !drag.moved || drag.done {
            self.drag = None;
            return None;
        }
        drag.done = true;
        let rect = tidy(drag.rect);
        let id = drag.id.clone();
        self.find(&id).cloned().map(|d| (d, rect))
    }

    /// A finished drag gives way once the backend has the new rect.
    pub fn settle(&mut self) {
        if self.drag.as_ref().is_some_and(|d| d.done) {
            self.drag = None;
        }
        if let Some(id) = &self.selected
            && self.find(id).is_none()
        {
            self.selected = None;
        }
    }

    pub fn key(&self, key: &str, ctrl: bool, shift: bool) -> Option<(Dev, [f32; 4])> {
        let d = self.find(self.selected.as_deref()?)?;
        let (dx, dy) = match key {
            "\u{F702}" => (-1.0, 0.0),
            "\u{F703}" => (1.0, 0.0),
            "\u{F700}" => (0.0, -1.0),
            "\u{F701}" => (0.0, 1.0),
            _ => return None,
        };
        let step = if ctrl { 0.05 } else { 0.01 };
        let (dx, dy) = (dx * step, dy * step);
        let [x, y, w, h] = d.rect;
        let rect = if shift {
            [x, y, clamp(w + dx, MIN_SIZE, 1.0 - x), clamp(h + dy, MIN_SIZE, 1.0 - y)]
        } else {
            [clamp(x + dx, 0.0, 1.0 - w), clamp(y + dy, 0.0, 1.0 - h), w, h]
        };
        Some((d.clone(), tidy(rect)))
    }

    /// None when devices overlap, since then each one's LEDs must be drawn before the next body.
    fn layout_key(&self, pw: u32, ph: u32, scale: f32) -> Option<u64> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (pw, ph, scale.to_bits(), self.light).hash(&mut h);
        (self.view.snap, self.view.grid.to_bits(), self.view.shapes).hash(&mut h);
        (&self.selected, &self.hover).hash(&mut h);
        let mut rects: Vec<[f32; 4]> = Vec::new();
        for d in self.stack() {
            let r = self.to_px(self.rect_of(d));
            if rects.iter().any(|o| {
                r[0] < o[0] + o[2] && o[0] < r[0] + r[2] && r[1] < o[1] + o[3] && o[1] < r[1] + r[3]
            }) {
                return None;
            }
            rects.push(r);
            (&d.id, &d.kind, d.enabled, d.grid, d.leds.len()).hash(&mut h);
            r.map(f32::to_bits).hash(&mut h);
        }
        Some(h.finish())
    }

    fn shape(&self, d: &Dev) -> &'static str {
        let [_, _, dw, dh] = self.to_px(self.rect_of(d));
        if self.view.shapes { shape_of(d, dw, dh) } else { "plate" }
    }

    /// Forgets the drawn image, for a new window that has none yet.
    pub fn invalidate(&mut self) {
        self.cache = Cache::default();
    }

    pub fn render(&mut self, w: f32, h: f32, scale: f32) -> Option<Out> {
        self.w = w;
        self.h = h;
        let pw = (w * scale).round() as u32;
        let ph = (h * scale).round() as u32;
        if pw == 0 || ph == 0 {
            return None;
        }
        let rings = self.devices.iter().any(|d| self.shape(d) == "fan");
        let mut image = None;
        let mut dots = Vec::new();
        match self.layout_key(pw, ph, scale) {
            // Fan rings need arcs, so they keep the image mode.
            Some(key) if !rings => {
                if self.cache.key != key || self.cache.base.is_none() {
                    let mut base = Pixmap::new(pw, ph)?;
                    self.draw(base.as_mut(), scale, Pass::Bodies, &mut HashMap::new());
                    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(pw, ph);
                    buf.make_mut_bytes().copy_from_slice(base.data());
                    image = Some(buf);
                    self.cache = Cache {
                        key,
                        base: Some(base),
                        masks: HashMap::new(),
                    };
                }
                let ink = Ink::new(self.light);
                for d in self.stack() {
                    let [x, y, dw, dh] = self.to_px(self.rect_of(d));
                    led_dots(&mut dots, &ink, self.shape(d), d, self.colors.get(&d.id), x, y, dw, dh);
                }
            }
            key => {
                let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(pw, ph);
                let mut masks = std::mem::take(&mut self.cache.masks);
                match key.map(|k| k ^ 1) {
                    Some(key) => {
                        if self.cache.key != key || self.cache.base.is_none() {
                            let mut base = Pixmap::new(pw, ph)?;
                            masks.clear();
                            self.draw(base.as_mut(), scale, Pass::Base, &mut masks);
                            self.cache.key = key;
                            self.cache.base = Some(base);
                        }
                        let base = self.cache.base.as_ref()?;
                        buf.make_mut_bytes().copy_from_slice(base.data());
                        let pm = PixmapMut::from_bytes(buf.make_mut_bytes(), pw, ph)?;
                        self.draw(pm, scale, Pass::Leds, &mut masks);
                        self.cache.masks = masks;
                    }
                    None => {
                        self.cache = Cache::default();
                        let pm = PixmapMut::from_bytes(buf.make_mut_bytes(), pw, ph)?;
                        self.draw(pm, scale, Pass::All, &mut HashMap::new());
                    }
                }
                image = Some(buf);
            }
        }
        let labels = self
            .stack()
            .into_iter()
            .map(|d| {
                let [x, y, dw, _] = self.to_px(self.rect_of(d));
                let strong =
                    Some(&d.id) == self.selected.as_ref() || Some(&d.id) == self.hover.as_ref();
                Label {
                    x,
                    y: y - 20.0,
                    width: dw,
                    text: if d.enabled {
                        d.name.clone()
                    } else {
                        format!("{} (off)", d.name)
                    },
                    strong,
                }
            })
            .collect();
        Some(Out { image, dots, labels })
    }

    fn draw(&self, pm: PixmapMut, scale: f32, pass: Pass, masks: &mut HashMap<String, Mask>) {
        let (w, h) = (self.w, self.h);
        let (pw, ph) = (pm.width(), pm.height());
        let ink = Ink::new(self.light);
        let mut p = Painter {
            pm,
            ts: Transform::from_scale(scale, scale),
            alpha: 1.0,
            mask: None,
        };
        if pass != Pass::Leds {
            p.pm.fill(ink.bg);
            p.stroke_rect(ink.line, PAD - 0.5, PAD - 0.5, w - PAD * 2.0 + 1.0, h - PAD * 2.0 + 1.0);
            if self.view.snap {
                let (cw, ch) = (w - PAD * 2.0, h - PAD * 2.0);
                let grid = self.view.grid.max(0.001);
                let mut pb = PathBuilder::new();
                let mut i = 1;
                while i as f32 * grid < 0.9999 {
                    let x = (PAD + i as f32 * grid * cw).round() - 0.5;
                    let y = (PAD + i as f32 * grid * ch).round() - 0.5;
                    pb.move_to(x, PAD);
                    pb.line_to(x, PAD + ch);
                    pb.move_to(PAD, y);
                    pb.line_to(PAD + cw, y);
                    i += 1;
                }
                p.stroke(ink.line, 1.0, pb);
            }
        }
        for d in self.stack() {
            let [x, y, dw, dh] = self.to_px(self.rect_of(d));
            let selected = Some(&d.id) == self.selected.as_ref();
            let hovered = Some(&d.id) == self.hover.as_ref();
            let shape = if self.view.shapes { shape_of(d, dw, dh) } else { "plate" };
            let edge = if selected {
                ink.accent
            } else if hovered {
                ink.text
            } else {
                ink.stroke
            };
            p.alpha = if d.enabled { 1.0 } else { 0.35 };
            if pass != Pass::Leds {
                let clip = draw_body(&mut p, &ink, shape, x + 0.5, y + 0.5, dw - 1.0, dh - 1.0, edge);
                if let Some(path) = clip.and_then(|pb| pb.finish())
                    && let Some(mut mask) = Mask::new(pw, ph)
                {
                    mask.fill_path(&path, FillRule::Winding, true, p.ts);
                    masks.insert(d.id.clone(), mask);
                }
            }
            if pass == Pass::Leds || pass == Pass::All {
                p.mask = masks.get(&d.id).cloned();
                draw_leds(&mut p, &ink, shape, d, self.colors.get(&d.id), x, y, dw, dh);
                p.mask = None;
            }
            p.alpha = 1.0;
            if selected && pass != Pass::Base {
                p.stroke_rect(ink.accent, x - 2.5, y - 2.5, dw + 5.0, dh + 5.0);
                p.fill_rect(ink.accent, x + dw - 4.0, y + dh - 4.0, 9.0, 9.0);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_body(
    p: &mut Painter,
    ink: &Ink,
    shape: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    edge: Color,
) -> Option<PathBuilder> {
    let s = w.min(h);
    let outline = match shape {
        "mouse" => {
            p.fill(ink.plate, mouse_path(x, y, w, h));
            p.stroke(ink.line, 1.0, line(x + w / 2.0, y + 1.0, x + w / 2.0, y + h * 0.4));
            p.stroke(ink.line, 1.0, line(x + w * 0.02, y + h * 0.4, x + w * 0.98, y + h * 0.4));
            p.fill_rect(ink.line, x + w / 2.0 - w * 0.07, y + h * 0.1, w * 0.14, h * 0.2);
            mouse_path(x, y, w, h)
        }
        "memory" => {
            let c = (s * 0.3).min(6.0);
            p.fill(ink.plate, cut_path(x, y, w, h, c));
            if h > w {
                let fw = (w * 0.14).max(2.0);
                p.fill_rect(ink.stroke, x + w - fw, y + h * 0.06, fw, h * 0.4);
                p.fill_rect(ink.stroke, x + w - fw, y + h * 0.52, fw, h * 0.42);
            } else {
                let fh = (h * 0.14).max(2.0);
                p.fill_rect(ink.stroke, x + w * 0.06, y + h - fh, w * 0.4, fh);
                p.fill_rect(ink.stroke, x + w * 0.52, y + h - fh, w * 0.42, fh);
            }
            cut_path(x, y, w, h, c)
        }
        "gpu" => {
            let c = (s * 0.25).min(8.0);
            p.fill(ink.plate, cut_path(x, y, w, h, c));
            p.fill_rect(ink.line, x, y, (w * 0.03).max(2.0), h);
            let fh = (h * 0.08).max(2.0);
            p.fill_rect(ink.stroke, x + w * 0.12, y + h - fh, w * 0.3, fh);
            let fans = if w / h > 3.2 { 3 } else { 2 };
            let r = (h * 0.3).min(w * 0.8 / fans as f32 / 2.3);
            for i in 0..fans {
                let cx = x + w * 0.1 + (i as f32 + 0.5) * w * 0.86 / fans as f32;
                let cy = y + h * 0.6;
                p.stroke(ink.line, 1.0, circle(cx, cy, r));
                p.stroke(ink.line, 1.0, circle(cx, cy, r * 0.25));
            }
            cut_path(x, y, w, h, c)
        }
        "board" => {
            let c = (s * 0.08).min(10.0);
            p.fill(ink.plate, cut_path(x, y, w, h, c));
            p.stroke_rect(ink.line, x + w * 0.38, y + h * 0.12, w * 0.22, h * 0.24);
            for i in 0..4 {
                p.stroke_rect(ink.line, x + w * (0.68 + i as f32 * 0.06), y + h * 0.08, w * 0.03, h * 0.42);
            }
            for i in 0..2 {
                p.stroke_rect(ink.line, x + w * 0.08, y + h * (0.56 + i as f32 * 0.18), w * 0.6, h * 0.05);
            }
            p.stroke_rect(ink.line, x + w * 0.04, y + h * 0.06, w * 0.12, h * 0.36);
            cut_path(x, y, w, h, c)
        }
        "fan" => {
            let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, s / 2.0);
            p.fill_rect(ink.plate, cx - r, cy - r, r * 2.0, r * 2.0);
            p.stroke(ink.line, 1.0, circle(cx, cy, r * 0.78));
            for i in 0..7 {
                let a = i as f32 / 7.0 * TAU;
                let mut pb = PathBuilder::new();
                pb.move_to(cx + a.cos() * r * 0.22, cy + a.sin() * r * 0.22);
                pb.quad_to(
                    cx + (a + 0.5).cos() * r * 0.55,
                    cy + (a + 0.5).sin() * r * 0.55,
                    cx + (a + 0.9).cos() * r * 0.72,
                    cy + (a + 0.9).sin() * r * 0.72,
                );
                p.stroke(ink.line, 1.0, pb);
            }
            p.fill(ink.plate, circle(cx, cy, r * 0.22));
            p.stroke(ink.line, 1.0, circle(cx, cy, r * 0.22));
            rect_path(cx - r, cy - r, r * 2.0, r * 2.0)
        }
        "keyboard" => {
            let c = (s * 0.08).min(10.0);
            p.fill(ink.plate, cut_path(x, y, w, h, c));
            cut_path(x, y, w, h, c)
        }
        "strip" => {
            let c = (s * 0.4).min(4.0);
            p.fill(ink.plate, cut_path(x, y, w, h, c));
            cut_path(x, y, w, h, c)
        }
        _ => {
            p.fill(ink.plate, rect_path(x, y, w, h));
            rect_path(x, y, w, h)
        }
    };
    p.stroke(edge, 1.0, outline);
    (shape == "mouse").then(|| mouse_path(x, y, w, h))
}

fn led(ink: &Ink, colors: Option<&Vec<[u8; 3]>>, i: usize) -> Color {
    match colors.and_then(|c| c.get(i)) {
        Some(&[r, g, b]) if r as u32 + g as u32 + b as u32 > 24 => Color::from_rgba8(r, g, b, 255),
        _ => ink.unlit,
    }
}

/// One LED drawn by the window over the cached device image, so a frame repaints only the LEDs.
#[derive(Clone, Copy)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub round: bool,
    pub rim: bool,
    pub color: [u8; 3],
}

pub struct Out {
    /// A new canvas image, when it changed.
    pub image: Option<SharedPixelBuffer<Rgba8Pixel>>,
    pub dots: Vec<Dot>,
    pub labels: Vec<Label>,
}

fn dot_color(ink: &Ink, colors: Option<&Vec<[u8; 3]>>, i: usize, enabled: bool) -> [u8; 3] {
    let c = led(ink, colors, i);
    let rgb = |c: Color| [c.red(), c.green(), c.blue()].map(|v| (v * 255.0).round() as u8);
    let (c, under) = (rgb(c), rgb(ink.plate));
    if enabled {
        return c;
    }
    [0, 1, 2].map(|k| (c[k] as f32 * 0.35 + under[k] as f32 * 0.65).round() as u8)
}

#[allow(clippy::too_many_arguments)]
fn led_dots(
    out: &mut Vec<Dot>,
    ink: &Ink,
    shape: &str,
    d: &Dev,
    colors: Option<&Vec<[u8; 3]>>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let leds = &d.leds;
    let [cols, rows] = d.grid;
    let cw = w / cols.max(1) as f32;
    let ch = h / rows.max(1) as f32;
    let at = |i: usize| (x + leds[i][0] * w, y + leds[i][1] * h);
    let n = leds.len();
    let color = |i: usize| dot_color(ink, colors, i, d.enabled);
    let mut rect = |x: f32, y: f32, w: f32, h: f32, round: bool, rim: bool, color: [u8; 3]| {
        out.push(Dot { x, y, w, h, round, rim, color })
    };
    if shape == "memory" || (shape == "gpu" && rows == 1) {
        let tall = h > w;
        let nf = n as f32;
        for i in 0..n {
            let fi = i as f32;
            if shape == "memory" && tall {
                rect(x + w * 0.1, y + h * 0.04 + fi * h * 0.92 / nf, w * 0.45, h * 0.92 / nf + 0.5, false, false, color(i));
            } else if shape == "memory" {
                rect(x + w * 0.04 + fi * w * 0.92 / nf, y + h * 0.1, w * 0.92 / nf + 0.5, h * 0.45, false, false, color(i));
            } else {
                rect(x + w * 0.06 + fi * w * 0.9 / nf, y + h * 0.08, w * 0.9 / nf + 0.5, (h * 0.14).max(2.0), false, false, color(i));
            }
        }
        return;
    }
    if matches!(shape, "mouse" | "strip" | "board") {
        let r = if shape == "mouse" {
            w.min(h) * 0.16
        } else {
            let cap = if shape == "board" { w.min(h) * 0.08 } else { f32::INFINITY };
            (cw.min(ch).min(cap) * 0.36).max(1.0)
        };
        for i in 0..n {
            let (lx, ly) = at(i);
            rect(lx - r, ly - r, r * 2.0, r * 2.0, true, false, color(i));
        }
        return;
    }
    let gap = if shape == "keyboard" {
        (cw.min(ch) * 0.12).max(1.0)
    } else {
        (cw.min(ch) * 0.12).min(1.5)
    };
    let lw = (cw - gap * 2.0).max(1.0);
    let lh = (ch - gap * 2.0).max(1.0);
    let rim = shape == "keyboard" && lw > 6.0;
    for i in 0..n {
        let (lx, ly) = at(i);
        rect(lx - lw / 2.0, ly - lh / 2.0, lw, lh, false, rim, color(i));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_leds(
    p: &mut Painter,
    ink: &Ink,
    shape: &str,
    d: &Dev,
    colors: Option<&Vec<[u8; 3]>>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let leds = &d.leds;
    let [cols, rows] = d.grid;
    let cw = w / cols.max(1) as f32;
    let ch = h / rows.max(1) as f32;
    let at = |i: usize| (x + leds[i][0] * w, y + leds[i][1] * h);
    let n = leds.len();
    if shape == "memory" || (shape == "gpu" && rows == 1) {
        let tall = h > w;
        let nf = n as f32;
        for i in 0..n {
            let c = led(ink, colors, i);
            let fi = i as f32;
            if shape == "memory" && tall {
                p.fill_rect(c, x + w * 0.1, y + h * 0.04 + fi * h * 0.92 / nf, w * 0.45, h * 0.92 / nf + 0.5);
            } else if shape == "memory" {
                p.fill_rect(c, x + w * 0.04 + fi * w * 0.92 / nf, y + h * 0.1, w * 0.92 / nf + 0.5, h * 0.45);
            } else {
                p.fill_rect(c, x + w * 0.06 + fi * w * 0.9 / nf, y + h * 0.08, w * 0.9 / nf + 0.5, (h * 0.14).max(2.0));
            }
        }
        return;
    }
    if shape == "fan" {
        let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, w.min(h) / 2.0);
        let width = (r * 0.16).max(2.0);
        for i in 0..n {
            let (lx, ly) = at(i);
            let a = (ly - cy).atan2(lx - cx);
            let half = PI / n as f32 + 0.01;
            p.stroke(led(ink, colors, i), width, arc(cx, cy, r * 0.88, a - half, a + half));
        }
        return;
    }
    if matches!(shape, "mouse" | "strip" | "board") {
        let r = if shape == "mouse" {
            w.min(h) * 0.16
        } else {
            let cap = if shape == "board" { w.min(h) * 0.08 } else { f32::INFINITY };
            (cw.min(ch).min(cap) * 0.36).max(1.0)
        };
        for i in 0..n {
            let (lx, ly) = at(i);
            p.fill(led(ink, colors, i), circle(lx, ly, r));
        }
        return;
    }
    let gap = if shape == "keyboard" {
        (cw.min(ch) * 0.12).max(1.0)
    } else {
        (cw.min(ch) * 0.12).min(1.5)
    };
    let lw = (cw - gap * 2.0).max(1.0);
    let lh = (ch - gap * 2.0).max(1.0);
    for i in 0..n {
        let (lx, ly) = at(i);
        p.fill_rect(led(ink, colors, i), lx - lw / 2.0, ly - lh / 2.0, lw, lh);
    }
    if shape == "keyboard" && lw > 6.0 {
        for i in 0..n {
            let (lx, ly) = at(i);
            p.stroke_rect(ink.plate, lx - lw / 2.0 + 1.5, ly - lh / 2.0 + 1.5, lw - 3.0, lh - 3.0);
        }
    }
}
