//! Ported from OpenRGB Controllers/CoolerMasterController/CMARGBGen2A1Controller (GPL-2.0-or-later).

use std::thread::sleep;
use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Driver, Lights, det, device, read, rows, write};
use crate::registry::{Detector, Info};

const VID: u16 = 0x2516;
const I1: Option<i32> = Some(1);
const PAGE: Option<u16> = Some(0xFF01);
const USAGE: Option<u16> = Some(1);
const MINI: u16 = 0x01CB;
const PAGE_CMD: u8 = 0x80;
const READ: u8 = 0x01;
const WRITE: u8 = 0x02;
const RESPONSE: u8 = 0x03;
const CMD_REALTIME_MODE: u8 = 0x01;
const CMD_GEN1_GEOMETRY: u8 = 0x06;
const CMD_GEN2_TOPOLOGY: u8 = 0x07;
const CMD_REALTIME_RGB: u8 = 0x09;
const CMD_GENERATION: u8 = 0x0B;
const GEN2: u8 = 0x00;
const GEN1: u8 = 0x01;
const MAX_LEDS: usize = 60;
const GEN1_MIN_LEDS: usize = 6;
const SUBPORTS: usize = 5;
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(150);
/// The realtime worker streams the frame on this period.
const PERIOD: Duration = Duration::from_millis(40);
const PACKET_GAP: Duration = Duration::from_millis(2);
const SECOND_ON_TO_TARGET: Duration = Duration::from_millis(104);
const EXIT_QUIET: Duration = Duration::from_millis(10);

pub static DETECTORS: &[Detector] = &[
    det!(
        "Cooler Master ARGB Gen 2 A1",
        VID,
        0x0173,
        I1,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Cooler Master ARGB Gen 2 A1 V2",
        VID,
        0x01C9,
        I1,
        PAGE,
        USAGE,
        open
    ),
    det!(
        "Cooler Master ARGB Gen 2 A1 Mini",
        VID,
        MINI,
        I1,
        PAGE,
        USAGE,
        open
    ),
];

pub type Report = [u8; 65];

/// SendCommandPage request.
pub fn command(command: u8, operation: u8, payload: &[u8]) -> Report {
    let mut r = [0u8; 65];
    r[1..4].copy_from_slice(&[PAGE_CMD, command, operation]);
    r[4..4 + payload.len()].copy_from_slice(payload);
    r
}

/// The logical reply to a command, if `raw` is one: page, command, then RESPONSE.
pub fn reply(command: u8, prefix: &[u8], raw: &[u8]) -> Option<Result<[u8; 64], ()>> {
    let offset = if raw.len() > 1 && raw[0] == 0x00 && raw[1] == PAGE_CMD {
        1
    } else if raw.first() == Some(&PAGE_CMD) {
        0
    } else {
        return None;
    };
    let body = &raw[offset..];
    if body.len() < 3 || body[1] != command {
        return None;
    }
    let mut out = [0u8; 64];
    let n = body.len().min(64);
    out[..n].copy_from_slice(&body[..n]);
    if !prefix.is_empty() && out[3..3 + prefix.len()] != *prefix {
        return None;
    }
    if out[2] != RESPONSE {
        return Some(Err(()));
    }
    Some(Ok(out))
}

/// SendRealtimeFrameLocked: 60 LEDs of RGB over three packets.
pub fn realtime(outer: u8, sub: u8, colors: &[Color]) -> [Report; 3] {
    let mut data = [0u8; MAX_LEDS * 3];
    for (i, c) in colors.iter().take(MAX_LEDS).enumerate() {
        data[i * 3..i * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    let mut a = [0u8; 65];
    a[2..6].copy_from_slice(&[CMD_REALTIME_RGB, outer, sub, MAX_LEDS as u8]);
    a[6..].copy_from_slice(&data[..59]);
    let mut b = [0u8; 65];
    b[1..3].copy_from_slice(&[0x01, CMD_REALTIME_RGB]);
    b[3..].copy_from_slice(&data[59..121]);
    let mut c = [0u8; 65];
    c[1..3].copy_from_slice(&[0x82, CMD_REALTIME_RGB]);
    c[3..62].copy_from_slice(&data[121..180]);
    [a, b, c]
}

/// One realtime target: a port mask and a Gen2 subport mask (0 for Gen1), and its LED count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub outer: u8,
    pub sub: u8,
    pub leds: usize,
}

/// Targets of one port from its topology reply (Gen2) or geometry reply (Gen1).
pub fn port_targets(outer: u8, generation: u8, answer: &[u8; 64]) -> Vec<Target> {
    if generation == GEN1 {
        let per_device = answer[5] as usize;
        return vec![Target {
            outer,
            sub: 0,
            leds: per_device.clamp(GEN1_MIN_LEDS, MAX_LEDS),
        }];
    }
    let count = (answer[4] as usize).min(SUBPORTS);
    (0..count)
        .map(|i| (answer[5 + i * 2], answer[6 + i * 2]))
        .filter(|&(mask, n)| mask != 0 && n != 0)
        .map(|(mask, n)| Target {
            outer,
            sub: mask,
            leds: (n as usize).min(MAX_LEDS),
        })
        .collect()
}

/// Each Gen1 port at the user's length; Gen2 subports report theirs.
fn sized(id: &str, targets: &mut [Target]) {
    for (i, t) in targets.iter_mut().enumerate().filter(|(_, t)| t.sub == 0) {
        let name = format!("Port {}", t.outer.trailing_zeros() + 1);
        t.leds = crate::lengths::length(id, i, &name, t.leds, GEN1_MIN_LEDS, MAX_LEDS);
    }
}

fn drain(hid: &HidDevice) -> Result<(), String> {
    while !read(hid, 1)?.is_empty() {}
    Ok(())
}

/// SendCommandPage with a response: drain, write, then wait for the matching reply.
fn request(
    hid: &HidDevice,
    cmd: u8,
    op: u8,
    payload: &[u8],
    prefix: &[u8],
) -> Result<[u8; 64], String> {
    drain(hid)?;
    write(hid, &command(cmd, op, payload))?;
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let raw = read(hid, (left.as_millis() as i32).max(1))?;
        if raw.is_empty() {
            break;
        }
        match reply(cmd, prefix, &raw) {
            Some(Ok(r)) => return Ok(r),
            Some(Err(())) => return Err(format!("command {cmd:02x} refused")),
            None => {}
        }
    }
    Err(format!("no answer to command {cmd:02x}"))
}

fn send_realtime(hid: &HidDevice, outer: u8, sub: u8, colors: &[Color]) -> Result<(), String> {
    for r in realtime(outer, sub, colors) {
        write(hid, &r)?;
        sleep(PACKET_GAP);
    }
    Ok(())
}

/// SendRealtimePreamble: realtime on, two black frames, on again, five black frames.
fn preamble(hid: &HidDevice) -> Result<(), String> {
    request(hid, CMD_REALTIME_MODE, WRITE, &[0x01], &[0x01])?;
    for _ in 0..2 {
        send_realtime(hid, 0xFF, 0xFF, &[])?;
    }
    let second = Instant::now();
    request(hid, CMD_REALTIME_MODE, WRITE, &[0x01], &[0x01])?;
    for _ in 0..5 {
        send_realtime(hid, 0xFF, 0xFF, &[])?;
    }
    sleep((second + SECOND_ON_TO_TARGET).saturating_duration_since(Instant::now()));
    Ok(())
}

/// RealtimeWorker sends at most one cycle per period.
pub fn cycle_due(since_last: Option<Duration>) -> bool {
    since_last.is_none_or(|d| d >= PERIOD)
}

struct Gen2 {
    hid: HidDevice,
    targets: Vec<Target>,
    realtime: bool,
    last: Option<Instant>,
}

impl Driver for Gen2 {
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first || !self.realtime {
            preamble(&self.hid)?;
            self.realtime = true;
        } else if !cycle_due(self.last.map(|t| t.elapsed())) {
            return Ok(());
        }
        self.last = Some(Instant::now());
        let mut at = 0;
        for t in &self.targets {
            send_realtime(&self.hid, t.outer, t.sub, &colors[at..at + t.leds])?;
            at += t.leds;
        }
        Ok(())
    }
}

impl Drop for Gen2 {
    fn drop(&mut self) {
        if self.realtime {
            sleep(EXIT_QUIET);
            for _ in 0..2 {
                if request(&self.hid, CMD_REALTIME_MODE, WRITE, &[0x00], &[0x00]).is_ok() {
                    break;
                }
                sleep(EXIT_QUIET);
            }
        }
    }
}

/// RefreshTopology: each port's generation, then its Gen1 geometry or Gen2 topology.
fn topology(hid: &HidDevice, ports: u32) -> Vec<Target> {
    let mut out = Vec::new();
    for port in 0..ports {
        let outer = 1u8 << port;
        let Ok(r) = request(hid, CMD_GENERATION, READ, &[outer], &[outer]) else {
            continue;
        };
        let generation = r[4];
        if r[3] != outer || (generation != GEN1 && generation != GEN2) {
            continue;
        }
        let cmd = if generation == GEN1 {
            CMD_GEN1_GEOMETRY
        } else {
            CMD_GEN2_TOPOLOGY
        };
        let answer = match request(hid, cmd, READ, &[outer], &[outer]) {
            Ok(a) if a[3] == outer => a,
            _ => [0u8; 64],
        };
        out.extend(port_targets(outer, generation, &answer));
    }
    out
}

fn open(api: &HidApi, info: &Info, det: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let ports = if info.pid == MINI { 1 } else { 3 };
    let mut targets = topology(&hid, ports);
    if targets.is_empty() {
        return Err("no ports answered".into());
    }
    let id = info.device_id("coolermaster");
    sized(&id, &mut targets);
    let counts: Vec<usize> = targets.iter().map(|t| t.leds).collect();
    let widest = counts.iter().copied().max().unwrap_or(1) as u32;
    let dev = device(
        id,
        det.name,
        DeviceKind::Strip,
        rows(&counts),
        [widest, counts.len() as u32],
    );
    let driver = Gen2 {
        hid,
        targets,
        realtime: false,
        last: None,
    };
    Ok(Lights::boxed(
        "cooler master argb a1",
        dev,
        Some(PERIOD),
        driver,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CMARGBGen2A1controller::SendCommandPage request framing (SendRealtimePreamble's realtime on)
    #[test]
    fn command_bytes() {
        let r = command(CMD_REALTIME_MODE, WRITE, &[0x01]);
        assert_eq!(&r[..6], &[0x00, 0x80, 0x01, 0x02, 0x01, 0x00]);
        let g = command(CMD_GENERATION, READ, &[0x04]);
        assert_eq!(&g[..6], &[0x00, 0x80, 0x0B, 0x01, 0x04, 0x00]);
    }

    // CMARGBGen2A1controller::SendCommandPage response matching
    #[test]
    fn reply_matching() {
        let ok = [0x80, 0x0B, 0x03, 0x02, 0x00];
        assert_eq!(reply(0x0B, &[0x02], &ok).unwrap().unwrap()[4], 0x00);
        let with_id = [0x00, 0x80, 0x0B, 0x03, 0x02, 0x01];
        assert_eq!(reply(0x0B, &[0x02], &with_id).unwrap().unwrap()[4], 0x01);
        assert!(reply(0x0B, &[0x04], &ok).is_none());
        assert!(reply(0x07, &[], &ok).is_none());
        assert!(reply(0x0B, &[], &[0x81, 0x0B, 0x03]).is_none());
        assert_eq!(reply(0x0B, &[], &[0x80, 0x0B, 0x02]), Some(Err(())));
    }

    // CMARGBGen2A1controller::SendRealtimeFrameLocked
    #[test]
    fn realtime_bytes() {
        let colors: Vec<Color> = (0..60u8).map(|i| Color(i, i, i)).collect();
        let [a, b, c] = realtime(0x02, 0x04, &colors);
        assert_eq!(&a[..9], &[0x00, 0x00, 0x09, 0x02, 0x04, 60, 0, 0, 0]);
        // data[58] is LED 19's green
        assert_eq!(a[64], 19);
        assert_eq!(&b[..4], &[0x00, 0x01, 0x09, 19]);
        // data[120] is LED 40's red, the last byte of packet two
        assert_eq!(b[64], 40);
        assert_eq!(&c[..4], &[0x00, 0x82, 0x09, 40]);
        assert_eq!(c[61], 59);
        assert_eq!(&c[62..], &[0, 0, 0]);
        let black = realtime(0xFF, 0xFF, &[]);
        assert_eq!(&black[0][..6], &[0, 0, 0x09, 0xFF, 0xFF, 60]);
        assert!(black[0][6..].iter().all(|&x| x == 0));
    }

    // CMARGBGen2A1controller::ReadGen2Topology and RGBController_CMARGBGen2A1Controller::SetupZones
    #[test]
    fn gen2_targets() {
        let mut a = [0u8; 64];
        a[..15].copy_from_slice(&[
            0x80, 0x07, 0x03, 0x01, 3, 0x01, 8, 0x00, 0, 0x04, 70, 0, 0, 0, 0,
        ]);
        let t = port_targets(0x01, GEN2, &a);
        assert_eq!(
            t,
            vec![
                Target {
                    outer: 1,
                    sub: 1,
                    leds: 8
                },
                Target {
                    outer: 1,
                    sub: 4,
                    leds: 60
                }
            ]
        );
    }

    // CMARGBGen2A1controller::RealtimeWorker: one cycle per 40 ms period
    #[test]
    fn cycles_are_paced() {
        assert!(cycle_due(None));
        assert!(!cycle_due(Some(Duration::from_millis(39))));
        assert!(cycle_due(Some(Duration::from_millis(40))));
    }

    // CMARGBGen2A1controller::ReadGen1Geometry and DecodeGen1NominalLEDCount
    #[test]
    fn gen1_targets() {
        let mut a = [0u8; 64];
        a[3..6].copy_from_slice(&[0x02, 1, 0]);
        assert_eq!(
            port_targets(2, GEN1, &a),
            vec![Target {
                outer: 2,
                sub: 0,
                leds: 6
            }]
        );
        a[5] = 24;
        assert_eq!(port_targets(2, GEN1, &a)[0].leds, 24);
    }

    #[test]
    fn a_set_length_sizes_a_gen1_port() {
        let gen2 = Target {
            outer: 1,
            sub: 1,
            leds: 12,
        };
        let gen1 = Target {
            outer: 4,
            sub: 0,
            leds: 24,
        };
        crate::lengths::set_one("hid:cm-gen2-test", 0, 3);
        crate::lengths::set_one("hid:cm-gen2-test", 1, 2);
        let mut t = [gen2, gen1];
        sized("hid:cm-gen2-test", &mut t);
        assert_eq!((t[0].leds, t[1].leds), (12, 6));
        let names: Vec<String> = crate::lengths::resizable()
            .into_iter()
            .filter(|z| z.device == "hid:cm-gen2-test")
            .map(|z| z.name)
            .collect();
        assert_eq!(names, ["Port 3"]);
    }
}
