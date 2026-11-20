//! Ported from OpenRGB Controllers/Gigabyte*Controller and Controllers/ASRockPolychromeUSBController (GPL-2.0-or-later).

mod aorus_case;
mod aorus_laptop;
mod aorus_mouse;
mod atc800;
mod castor3;
mod fusion2;
mod polychrome;

use crate::port::Port;
use crate::registry::Detector;
use foxden_core::{Backend, Color, Device};
use std::time::Duration;

pub fn detectors() -> Vec<&'static Detector> {
    [
        fusion2::DETECTORS.iter().collect::<Vec<_>>(),
        aorus_laptop::DETECTORS.iter().collect(),
        vec![
            &atc800::DETECTOR,
            &castor3::DETECTOR,
            &aorus_case::DETECTOR,
            &aorus_mouse::DETECTOR,
        ],
        polychrome::DETECTORS.iter().collect(),
    ]
    .concat()
}

/// One step of HID traffic, kept as data so tests can check it without a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Feature(Vec<u8>),
    /// A feature read whose answer is not used.
    GetFeature(Vec<u8>),
    Write(Vec<u8>),
    /// An interrupt read of this many bytes whose answer is not used.
    Read(usize),
    Sleep(u64),
}

pub fn run(hid: &Port, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Feature(b) => hid.send_feature_report(b).map_err(|e| e.to_string())?,
            Op::GetFeature(b) => {
                let mut buf = b.clone();
                let _ = hid.get_feature_report(&mut buf);
            }
            Op::Write(b) => {
                hid.write(b).map_err(|e| e.to_string())?;
            }
            Op::Read(n) => {
                let mut buf = vec![0u8; *n];
                let _ = hid.read_timeout(&mut buf, 500);
            }
            Op::Sleep(ms) => std::thread::sleep(Duration::from_millis(*ms)),
        }
    }
    Ok(())
}

/// What a family sends: once to take control, then for each new frame.
pub trait Plan: Send {
    fn first(&mut self, _colors: &[Color]) -> Vec<Op> {
        Vec::new()
    }
    fn frame(&mut self, colors: &[Color]) -> Vec<Op>;
}

/// Skips unchanged frames and takes control on the first one.
pub struct Session<P> {
    pub plan: P,
    leds: usize,
    sent: Option<Vec<Color>>,
}

impl<P: Plan> Session<P> {
    pub fn new(plan: P, leds: usize) -> Self {
        Session {
            plan,
            leds,
            sent: None,
        }
    }

    /// Reports for this frame; empty when nothing changed.
    pub fn step(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut c = colors.to_vec();
        c.resize(self.leds, Color::BLACK);
        if self.sent.as_ref() == Some(&c) {
            return Vec::new();
        }
        let mut ops = if self.sent.is_none() {
            self.plan.first(&c)
        } else {
            Vec::new()
        };
        ops.extend(self.plan.frame(&c));
        self.sent = Some(c);
        ops
    }

    pub fn reset(&mut self) {
        self.sent = None;
    }
}

pub struct Driver<P> {
    name: &'static str,
    device: Device,
    hid: Port,
    session: Session<P>,
}

impl<P: Plan> Driver<P> {
    pub fn boxed(
        name: &'static str,
        device: Device,
        hid: impl Into<Port>,
        plan: P,
    ) -> Box<dyn Backend>
    where
        P: 'static,
    {
        let leds = device.leds.len();
        Box::new(Driver {
            name,
            device,
            hid: hid.into(),
            session: Session::new(plan, leds),
        })
    }
}

impl<P: Plan> Backend for Driver<P> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let ops = self.session.step(colors);
        run(&self.hid, &ops).inspect_err(|_| self.session.reset())
    }
}

/// One row per zone, each LED at its slot centre.
pub fn rows(zones: &[usize]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let cols = zones.iter().copied().max().unwrap_or(1).max(1);
    let n = zones.len().max(1);
    let leds = zones
        .iter()
        .enumerate()
        .flat_map(|(r, &len)| {
            (0..len).map(move |i| [(i as f32 + 0.5) / cols as f32, (r as f32 + 0.5) / n as f32])
        })
        .collect();
    (leds, [cols as u32, n as u32])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Plan for Echo {
        fn first(&mut self, _: &[Color]) -> Vec<Op> {
            vec![Op::Sleep(1)]
        }
        fn frame(&mut self, c: &[Color]) -> Vec<Op> {
            vec![Op::Write(c.iter().flat_map(|c| [c.0, c.1, c.2]).collect())]
        }
    }

    #[test]
    fn session_takes_control_once_and_skips_repeats() {
        let mut s = Session::new(Echo, 2);
        let a = [Color(1, 2, 3)];
        assert_eq!(
            s.step(&a),
            vec![Op::Sleep(1), Op::Write(vec![1, 2, 3, 0, 0, 0])]
        );
        assert!(s.step(&a).is_empty());
        assert_eq!(
            s.step(&[Color(1, 2, 3), Color(4, 5, 6), Color(7, 7, 7)]),
            vec![Op::Write(vec![1, 2, 3, 4, 5, 6])]
        );
        s.reset();
        assert_eq!(s.step(&a).len(), 2);
    }

    #[test]
    fn detectors_are_unique() {
        let d = detectors();
        assert_eq!(d.len(), 5 + 2 + 4 + 2);
        for (i, a) in d.iter().enumerate() {
            for b in &d[i + 1..] {
                assert!(
                    (a.vid, a.pid, a.interface, a.usage_page, a.usage)
                        != (b.vid, b.pid, b.interface, b.usage_page, b.usage)
                );
            }
        }
    }

    #[test]
    fn rows_stay_inside_the_rect() {
        let (leds, grid) = rows(&[1, 3, 0]);
        assert_eq!(leds.len(), 4);
        assert_eq!(grid, [3, 3]);
        assert!(leds.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
    }
}
