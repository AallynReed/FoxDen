//! Ported from OpenRGB Controllers/ (Das Keyboard, ViewSonic, Clevo, Wooting, Cougar, SRGBmods, Klipsch, Alienware) (GPL-2.0-or-later).

use std::thread;
use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use crate::registry::{Detector, Info, Open};

mod alienware;
mod clevo;
mod cougar;
mod das;
mod klipsch;
mod srgbmods;
mod viewsonic;
mod wooting;

fn sets() -> [&'static [Detector]; 9] {
    [
        &das::DETECTORS,
        &viewsonic::DETECTORS,
        &clevo::DETECTORS,
        &wooting::LEGACY,
        wooting::CURRENT.as_flattened(),
        &cougar::DETECTORS,
        &srgbmods::DETECTORS,
        &klipsch::DETECTORS,
        &alienware::DETECTORS,
    ]
}

pub fn detectors() -> Vec<&'static Detector> {
    sets().into_iter().flatten().collect()
}

/// A plain `REGISTER_HID_DETECTOR`.
const fn any(name: &'static str, vid: u16, pid: u16, open: Open) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: None,
        usage_page: None,
        usage: None,
        open,
    }
}

/// `REGISTER_HID_DETECTOR_I`.
const fn on_i(name: &'static str, vid: u16, pid: u16, interface: i32, open: Open) -> Detector {
    Detector {
        interface: Some(interface),
        ..any(name, vid, pid, open)
    }
}

/// `REGISTER_HID_DETECTOR_P`.
const fn on_p(name: &'static str, vid: u16, pid: u16, page: u16, open: Open) -> Detector {
    Detector {
        usage_page: Some(page),
        ..any(name, vid, pid, open)
    }
}

/// `REGISTER_HID_DETECTOR_IP`.
const fn on_ip(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: i32,
    page: u16,
    open: Open,
) -> Detector {
    Detector {
        interface: Some(interface),
        usage_page: Some(page),
        ..any(name, vid, pid, open)
    }
}

/// `REGISTER_HID_DETECTOR_IPU`.
const fn on_ipu(
    name: &'static str,
    vid: u16,
    pid: u16,
    interface: i32,
    page: u16,
    usage: u16,
    open: Open,
) -> Detector {
    Detector {
        interface: Some(interface),
        usage_page: Some(page),
        usage: Some(usage),
        ..any(name, vid, pid, open)
    }
}

/// `REGISTER_HID_DETECTOR_PU`.
const fn on_pu(
    name: &'static str,
    vid: u16,
    pid: u16,
    page: u16,
    usage: u16,
    open: Open,
) -> Detector {
    Detector {
        usage_page: Some(page),
        usage: Some(usage),
        ..any(name, vid, pid, open)
    }
}

/// The hidapi calls the drivers make, so tests can script a device.
pub(super) trait Io: Send {
    fn write(&mut self, b: &[u8]) -> Result<(), String>;
    fn feature(&mut self, b: &[u8]) -> Result<(), String>;
    /// `hid_get_feature_report`: `buf[0]` holds the report id on entry.
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn get_input(&mut self, buf: &mut [u8]) -> Result<usize, String>;
    fn sleep(&mut self, d: Duration);
}

struct HidIo(HidDevice);

impl Io for HidIo {
    fn write(&mut self, b: &[u8]) -> Result<(), String> {
        self.0.write(b).map(|_| ()).map_err(|e| e.to_string())
    }
    fn feature(&mut self, b: &[u8]) -> Result<(), String> {
        self.0.send_feature_report(b).map_err(|e| e.to_string())
    }
    fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.0.get_feature_report(buf).map_err(|e| e.to_string())
    }
    fn get_input(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.0.get_input_report(buf).map_err(|e| e.to_string())
    }
    fn sleep(&mut self, d: Duration) {
        thread::sleep(d);
    }
}

pub(super) fn open_io(api: &HidApi, info: &Info) -> Result<Box<dyn Io>, String> {
    Ok(Box::new(HidIo(info.open(api)?)))
}

/// One family's direct-mode protocol.
pub(super) trait Driver: Send {
    /// Takes software control, before the first frame.
    fn setup(&mut self, _io: &mut dyn Io) -> Result<(), String> {
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String>;
    /// How often the device must hear from us while the colours stay the same.
    fn keepalive(&self) -> Option<Duration> {
        None
    }
    fn refresh(&mut self, io: &mut dyn Io, last: &[Color]) -> Result<(), String> {
        self.frame(io, last)
    }
    /// Frames sooner than this after the last write wait for a later push.
    fn min_gap(&self) -> Duration {
        Duration::ZERO
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Skip,
    Frame,
    Refresh,
}

fn step(
    first: bool,
    changed: bool,
    since: Duration,
    keepalive: Option<Duration>,
    min_gap: Duration,
) -> Step {
    if first || (changed && since >= min_gap) {
        Step::Frame
    } else if !changed && keepalive.is_some_and(|k| since >= k) {
        Step::Refresh
    } else {
        Step::Skip
    }
}

/// One physical device driven by a driver.
pub(super) struct Hid<D: Driver> {
    io: Box<dyn Io>,
    device: Device,
    driver: D,
    label: &'static str,
    sent: Option<Vec<Color>>,
    at: Instant,
}

pub(super) fn backend<D: Driver + 'static>(
    io: Box<dyn Io>,
    device: Device,
    driver: D,
    label: &'static str,
) -> Box<dyn Backend> {
    Box::new(Hid {
        io,
        device,
        driver,
        label,
        sent: None,
        at: Instant::now(),
    })
}

impl<D: Driver> Backend for Hid<D> {
    fn name(&self) -> &str {
        self.label
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let first = self.sent.is_none();
        let changed = self.sent.as_deref() != Some(colors);
        let io = self.io.as_mut();
        let result = match step(
            first,
            changed,
            self.at.elapsed(),
            self.driver.keepalive(),
            self.driver.min_gap(),
        ) {
            Step::Skip => return Ok(()),
            Step::Refresh => self.driver.refresh(io, colors),
            Step::Frame if first => self
                .driver
                .setup(io)
                .and_then(|()| self.driver.frame(io, colors)),
            Step::Frame => self.driver.frame(io, colors),
        };
        match result {
            Ok(()) => {
                self.sent = Some(colors.to_vec());
                self.at = Instant::now();
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                Err(e)
            }
        }
    }
}

/// Colour `i`, black past the end.
pub(super) fn at(colors: &[Color], i: usize) -> Color {
    colors.get(i).copied().unwrap_or_default()
}

/// Each zone a row, LEDs spread evenly along it.
pub(super) fn rows(counts: &[usize]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let n = counts.len().max(1);
    let leds = counts
        .iter()
        .enumerate()
        .flat_map(|(r, &c)| {
            (0..c).map(move |i| [(i as f32 + 0.5) / c as f32, (r as f32 + 0.5) / n as f32])
        })
        .collect();
    let width = counts.iter().copied().max().unwrap_or(1).max(1);
    (leds, [width as u32, n as u32])
}

/// LEDs at their cells of an OpenRGB matrix map (`NA` is empty); others go to `fallback`.
pub(super) fn matrix(
    map: &[u32],
    width: usize,
    count: usize,
    fallback: impl Fn(usize) -> [f32; 2],
) -> Vec<[f32; 2]> {
    let height = map.len() / width;
    (0..count)
        .map(|led| match map.iter().position(|&m| m == led as u32) {
            Some(cell) => cell_pos(cell, width, height),
            None => fallback(led),
        })
        .collect()
}

pub(super) fn cell_pos(cell: usize, width: usize, height: usize) -> [f32; 2] {
    [
        ((cell % width) as f32 + 0.5) / width as f32,
        ((cell / width) as f32 + 0.5) / height as f32,
    ]
}

pub(super) const NA: u32 = u32::MAX;

/// Where each kind of device sits on the canvas by default.
fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.40, 0.52, 0.12],
        _ => [0.34, 0.12, 0.24, 0.24],
    }
}

pub(super) fn device(
    id: String,
    name: &str,
    kind: DeviceKind,
    grid: [u32; 2],
    leds: Vec<[f32; 2]>,
) -> Device {
    Device {
        id,
        name: name.into(),
        kind,
        rect: rect(kind),
        grid,
        leds,
    }
}

#[cfg(test)]
pub(super) mod test_util {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::Io;

    /// One hidapi call as the fake saw it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Op {
        Write(Vec<u8>),
        Feature(Vec<u8>),
        /// The buffer handed to `hid_get_feature_report`, as it was on entry.
        GetFeature(Vec<u8>),
        GetInput(usize),
        Sleep(Duration),
    }

    /// A scripted device: logs every call and answers reads from `replies`, then fails.
    #[derive(Clone, Default)]
    pub struct Fake {
        pub log: Arc<Mutex<Vec<Op>>>,
        pub replies: Arc<Mutex<VecDeque<Vec<u8>>>>,
        /// The next this many feature sends fail.
        pub failing_features: Arc<Mutex<usize>>,
    }

    impl Fake {
        pub fn with(replies: Vec<Vec<u8>>) -> Fake {
            let f = Fake::default();
            *f.replies.lock().unwrap() = replies.into();
            f
        }
        pub fn ops(&self) -> Vec<Op> {
            self.log.lock().unwrap().clone()
        }
        pub fn take(&self) -> Vec<Op> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }
        fn reply(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            let r = self.replies.lock().unwrap().pop_front().ok_or("no reply")?;
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            Ok(n)
        }
    }

    impl Io for Fake {
        fn write(&mut self, b: &[u8]) -> Result<(), String> {
            self.log.lock().unwrap().push(Op::Write(b.to_vec()));
            Ok(())
        }
        fn feature(&mut self, b: &[u8]) -> Result<(), String> {
            self.log.lock().unwrap().push(Op::Feature(b.to_vec()));
            let mut failing = self.failing_features.lock().unwrap();
            if *failing > 0 {
                *failing -= 1;
                return Err("fail".into());
            }
            Ok(())
        }
        fn get_feature(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.log.lock().unwrap().push(Op::GetFeature(buf.to_vec()));
            self.reply(buf)
        }
        fn get_input(&mut self, buf: &mut [u8]) -> Result<usize, String> {
            self.log.lock().unwrap().push(Op::GetInput(buf.len()));
            self.reply(buf)
        }
        fn sleep(&mut self, d: Duration) {
            self.log.lock().unwrap().push(Op::Sleep(d));
        }
    }

    /// A buffer of `len` zeros with `head` at the start.
    pub fn buf(len: usize, head: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[..head.len()].copy_from_slice(head);
        b
    }

    pub fn ms(n: u64) -> Op {
        Op::Sleep(Duration::from_millis(n))
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::{Fake, Op};
    use super::*;

    #[test]
    fn step_rules() {
        let ms = Duration::from_millis;
        assert_eq!(step(true, false, ms(0), None, ms(0)), Step::Frame);
        assert_eq!(step(false, true, ms(0), None, ms(0)), Step::Frame);
        assert_eq!(step(false, false, ms(10), None, ms(0)), Step::Skip);
        assert_eq!(
            step(false, false, ms(50), Some(ms(50)), ms(0)),
            Step::Refresh
        );
        assert_eq!(step(false, true, ms(20), None, ms(50)), Step::Skip);
    }

    struct Count;
    impl Driver for Count {
        fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
            io.write(&[0])
        }
        fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
            io.write(&[1, colors[0].0])
        }
    }

    #[test]
    fn push_sets_up_once_and_skips_unchanged_frames() {
        let fake = Fake::default();
        let dev = device("d".into(), "D", DeviceKind::Other, [1, 1], vec![[0.5, 0.5]]);
        let mut b = backend(Box::new(fake.clone()), dev, Count, "t");
        b.push("d", &[Color(5, 0, 0)]).unwrap();
        b.push("d", &[Color(5, 0, 0)]).unwrap();
        b.push("d", &[Color(6, 0, 0)]).unwrap();
        assert_eq!(
            fake.ops(),
            vec![
                Op::Write(vec![0]),
                Op::Write(vec![1, 5]),
                Op::Write(vec![1, 6])
            ]
        );
        assert!(b.push("x", &[]).is_err());
    }

    #[test]
    fn detector_counts_match_openrgb() {
        // REGISTER_* lines per Detect.cpp; Wooting expands each REGISTER_WOOTING_DETECTOR to three.
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(counts, [3, 3, 2, 2, 33, 2, 2, 2, 2]);
        assert_eq!(detectors().len(), 51);
    }

    #[test]
    fn no_duplicate_detectors() {
        let all = detectors();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let same = a.vid == b.vid
                    && a.pid == b.pid
                    && a.interface == b.interface
                    && a.usage_page == b.usage_page
                    && a.usage == b.usage;
                assert!(!same, "{} / {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn matrix_layout() {
        let m = matrix(&[1, NA, 0, NA], 2, 3, |_| [9.0, 9.0]);
        assert_eq!(m, vec![[0.25, 0.75], [0.25, 0.25], [9.0, 9.0]]);
    }
}
