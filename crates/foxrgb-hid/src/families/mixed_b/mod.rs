//! Ported from OpenRGB Controllers/ (Thermaltake, NZXT, Corsair, HyperX, DRGB, Nollie, Cherry, Redragon) (GPL-2.0-or-later).

use std::thread;
use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use crate::registry::{Detector, Info, Open};

mod cherry;
mod corsair_commander_core;
mod corsair_hydro_platinum;
mod corsair_icue_link;
mod corsair_lighting_node;
mod drgb;
mod hyperx_microphone;
mod hyperx_microphone_v2;
mod hyperx_mouse;
mod hyperx_mousemat;
mod nollie;
mod nzxt_hue1;
mod nzxt_hue2;
mod nzxt_kraken;
mod nzxt_mouse;
mod redragon;
mod thermaltake_poseidon;
mod thermaltake_riing;

fn sets() -> [&'static [Detector]; 18] {
    [
        &thermaltake_riing::DETECTORS,
        &thermaltake_poseidon::DETECTORS,
        &nzxt_hue2::DETECTORS,
        &nzxt_hue1::DETECTORS,
        &nzxt_kraken::DETECTORS,
        &nzxt_mouse::DETECTORS,
        &corsair_hydro_platinum::DETECTORS,
        &corsair_lighting_node::DETECTORS,
        &corsair_commander_core::DETECTORS,
        &corsair_icue_link::DETECTORS,
        &hyperx_mouse::DETECTORS,
        &hyperx_microphone::DETECTORS,
        &hyperx_microphone_v2::DETECTORS,
        &hyperx_mousemat::DETECTORS,
        &drgb::DETECTORS,
        &nollie::DETECTORS,
        &cherry::DETECTORS,
        &redragon::DETECTORS,
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

/// One step of talking to a device, so tests can check whole sequences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Op {
    Write(Vec<u8>),
    Feature(Vec<u8>),
    /// Read one input report of this length, waiting at most this many ms.
    Read(usize, i32),
    Sleep(u64),
}

/// One family's protocol, as data.
pub(super) trait Protocol: Send {
    /// Sent once before the first frame, to take software control.
    fn setup(&mut self, _colors: &[Color]) -> Vec<Op> {
        Vec::new()
    }
    fn frame(&mut self, colors: &[Color]) -> Vec<Op>;
    /// How often the device must hear from us while the colours stay the same.
    fn keepalive(&self) -> Option<Duration> {
        None
    }
    fn refresh(&mut self, last: &[Color]) -> Vec<Op> {
        self.frame(last)
    }
    /// Frames sooner than this after the last write wait for a later push.
    fn min_gap(&self) -> Duration {
        Duration::ZERO
    }
    /// Sent when the backend is dropped after having taken control.
    fn release(&mut self) -> Vec<Op> {
        Vec::new()
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

fn run(hid: &HidDevice, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Write(b) => {
                hid.write(b).map_err(|e| e.to_string())?;
            }
            Op::Feature(b) => hid.send_feature_report(b).map_err(|e| e.to_string())?,
            Op::Read(len, ms) => {
                let mut buf = vec![0u8; *len];
                hid.read_timeout(&mut buf, *ms).map_err(|e| e.to_string())?;
            }
            Op::Sleep(ms) => thread::sleep(Duration::from_millis(*ms)),
        }
    }
    Ok(())
}

/// Writes `request` and reads replies until `ok` accepts one or `tries` reads pass.
pub(super) fn ask(
    hid: &HidDevice,
    request: &[u8],
    len: usize,
    ms: i32,
    tries: usize,
    ok: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    hid.write(request).map_err(|e| e.to_string())?;
    for _ in 0..tries {
        let mut buf = vec![0u8; len];
        let n = hid.read_timeout(&mut buf, ms).map_err(|e| e.to_string())?;
        if n > 0 && ok(&buf[..n]) {
            return Ok(buf);
        }
    }
    Err("device did not answer".into())
}

/// One physical device driven by a protocol.
pub(super) struct Hid<P: Protocol> {
    hid: HidDevice,
    device: Device,
    proto: P,
    label: &'static str,
    sent: Option<Vec<Color>>,
    at: Instant,
}

pub(super) fn backend<P: Protocol + 'static>(
    hid: HidDevice,
    device: Device,
    proto: P,
    label: &'static str,
) -> Box<dyn Backend> {
    Box::new(Hid {
        hid,
        device,
        proto,
        label,
        sent: None,
        at: Instant::now(),
    })
}

/// Opens the collection and builds a single-protocol backend.
pub(super) fn open_with<P: Protocol + 'static>(
    api: &HidApi,
    info: &Info,
    label: &'static str,
    make: impl FnOnce(&HidDevice) -> Result<(Device, P), String>,
) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let (device, proto) = make(&hid)?;
    Ok(backend(hid, device, proto, label))
}

impl<P: Protocol> Backend for Hid<P> {
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
        let ops = match step(
            first,
            changed,
            self.at.elapsed(),
            self.proto.keepalive(),
            self.proto.min_gap(),
        ) {
            Step::Skip => return Ok(()),
            Step::Refresh => self.proto.refresh(colors),
            Step::Frame => {
                let mut ops = if first {
                    self.proto.setup(colors)
                } else {
                    Vec::new()
                };
                ops.extend(self.proto.frame(colors));
                ops
            }
        };
        match run(&self.hid, &ops) {
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

impl<P: Protocol> Drop for Hid<P> {
    fn drop(&mut self) {
        let ops = self.proto.release();
        let _ = run(&self.hid, &ops);
    }
}

/// Colour `i`, black past the end.
pub(super) fn at(colors: &[Color], i: usize) -> Color {
    colors.get(i).copied().unwrap_or_default()
}

/// `n` colours starting at `from`, padded with black.
pub(super) fn span(colors: &[Color], from: usize, n: usize) -> Vec<Color> {
    (from..from + n).map(|i| at(colors, i)).collect()
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

/// LEDs at their cells of an OpenRGB matrix map (`u32::MAX` is empty); others go to `fallback`.
pub(super) fn matrix(
    map: &[u32],
    width: usize,
    count: usize,
    fallback: impl Fn(usize) -> [f32; 2],
) -> Vec<[f32; 2]> {
    let height = map.len() / width;
    (0..count)
        .map(|led| match map.iter().position(|&m| m == led as u32) {
            Some(cell) => [
                ((cell % width) as f32 + 0.5) / width as f32,
                ((cell / width) as f32 + 0.5) / height as f32,
            ],
            None => fallback(led),
        })
        .collect()
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
    use super::Op;

    /// The bytes of a write or feature op.
    pub fn bytes(op: &Op) -> &[u8] {
        match op {
            Op::Write(b) | Op::Feature(b) => b,
            other => panic!("not a report: {other:?}"),
        }
    }

    /// A buffer of `len` zeros with `head` at the start.
    pub fn buf(len: usize, head: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[..head.len()].copy_from_slice(head);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_rules() {
        let ms = Duration::from_millis;
        assert_eq!(step(true, false, ms(0), None, ms(0)), Step::Frame);
        assert_eq!(step(false, true, ms(0), None, ms(0)), Step::Frame);
        assert_eq!(step(false, false, ms(10), None, ms(0)), Step::Skip);
        assert_eq!(step(false, false, ms(49), Some(ms(50)), ms(0)), Step::Skip);
        assert_eq!(
            step(false, false, ms(50), Some(ms(50)), ms(0)),
            Step::Refresh
        );
        assert_eq!(step(false, true, ms(20), None, ms(33)), Step::Skip);
        assert_eq!(step(false, true, ms(33), None, ms(33)), Step::Frame);
    }

    #[test]
    fn detector_counts_match_openrgb() {
        // REGISTER_* lines per Detect.cpp, Mousemat counting only its _WIN32 line, plus SignalRGB's
        // six NZXT boards and the Quadcast S 0x171D.
        let counts: Vec<usize> = sets().iter().map(|s| s.len()).collect();
        assert_eq!(
            counts,
            [51, 1, 27, 1, 2, 1, 13, 7, 7, 1, 14, 9, 1, 3, 33, 16, 25, 12]
        );
        assert_eq!(detectors().len(), counts.iter().sum::<usize>());
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
    fn rows_and_matrix_layouts() {
        let (leds, grid) = rows(&[2, 0, 1]);
        assert_eq!(grid, [2, 3]);
        assert_eq!(
            leds,
            vec![[0.25, 1.0 / 6.0], [0.75, 1.0 / 6.0], [0.5, 5.0 / 6.0]]
        );
        let m = matrix(&[1, NA, 0, NA], 2, 3, |_| [9.0, 9.0]);
        assert_eq!(m, vec![[0.25, 0.75], [0.25, 0.25], [9.0, 9.0]]);
    }
}
