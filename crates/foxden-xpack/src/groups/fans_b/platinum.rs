//! Corsair Hydro Platinum, Pro XT and RGB Elite coolers.

use super::util::{Cached, ask, device_key, due, fan_id, u16le};
use crate::fan::{Channel, FanHub, backend_on, byte};
use crate::hid::{Driver, Io, Proto, open_io};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_fan::FanBackend;
use foxden_hid::port::Device as Dev;
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

pub const VID: u16 = 0x1B1C;
/// `PIDLibrary` with each model's fan count.
pub const IDS: &[(u16, &str, usize)] = &[
    (0x0C18, "Corsair H100i Platinum", 2),
    (0x0C19, "Corsair H100i Platinum SE", 2),
    (0x0C17, "Corsair H115i Platinum", 2),
    (0x0C29, "Corsair H60i Pro XT", 1),
    (0x0C20, "Corsair H100i Pro XT", 2),
    (0x0C2D, "Corsair H100i Pro XT", 2),
    (0x0C21, "Corsair H115i Pro XT", 2),
    (0x0C2E, "Corsair H115i Pro XT", 2),
    (0x0C2F, "Corsair H150i Pro XT", 3),
    (0x0C22, "Corsair H150i Pro XT", 3),
    (0x0C35, "Corsair H100i RGB Elite", 2),
    (0x0C40, "Corsair H100i RGB Elite", 2),
    (0x0C36, "Corsair H115i RGB Elite", 2),
    (0x0C37, "Corsair H150i RGB Elite", 3),
    (0x0C41, "Corsair H150i RGB Elite", 3),
];
const LEN: usize = 65;
const MAGIC: u8 = 0x3F;
const STATUS: u8 = 0xFF;
const COOLING: u8 = 0b000;
const SOFTWARE_1: u8 = 0b001;
const SOFTWARE_2: u8 = 0b010;
const SOFTWARE_3: u8 = 0b011;
const THIRD_FAN: u8 = 0b011;
const COLORS: [u8; 3] = [0b100, 0b101, 0b110];
/// Fan modes from `deviceFanModes`.
const FAN_CURVE: u8 = 0x00;
const FAN_FIXED: u8 = 0x03;
/// Pump modes from `coolingModes`.
const QUIET: u8 = 0x00;
const BALANCED: u8 = 0x01;
const EXTREME: u8 = 0x02;
/// `coolingProfiles.Balanced`: seven (temperature, duty) points the firmware follows.
const BALANCED_CURVE: [u8; 14] = [
    0x1D, 0x33, 0x1E, 0x40, 0x1F, 0x4D, 0x20, 0x73, 0x21, 0xAD, 0x22, 0xD9, 0x23, 0xFF,
];
const CURVE_POINTS: u8 = 0x07;
/// Status reply offsets (hidapi buffer, no report id).
const ERROR_AT: usize = 4;
const LIQUID_AT: usize = 7;
const FAN_RPM_AT: [usize; 3] = [15, 22, 43];
const PUMP_RPM_AT: usize = 29;
const SEQUENCE_ERROR: u8 = 0x10;
const PACE_MS: u64 = 10;
const POLL_MS: u64 = 1000;
const RESEND_MS: u64 = 3000;
const BLOCK_LEDS: usize = 16;
const FAN_LEDS: usize = 32;

#[rustfmt::skip]
const CRC_TABLE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut b = 0;
        while b < 8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
            b += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// One sequence for everything this crate sends to these coolers: 8, 16 .. 248, then 8.
static SEQ: AtomicU8 = AtomicU8::new(0);

fn next_seq() -> u8 {
    let mut s = SEQ.load(Ordering::Relaxed);
    loop {
        let n = if s >= 248 { 8 } else { s + 8 };
        match SEQ.compare_exchange(s, n, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return n,
            Err(cur) => s = cur,
        }
    }
}

/// The shared device's sequence, which the lighting driver steps too, else this crate's.
fn seq(device: &Option<Arc<Dev>>) -> u8 {
    device.as_ref().map_or_else(next_seq, |d| d.next_seq() << 3)
}

/// Corsair's balanced liquid curve, as these coolers' firmware runs it: percent at `t` °C.
pub fn balanced_pct(t: f32) -> u8 {
    let pts: Vec<(f32, f32)> = BALANCED_CURVE
        .chunks(2)
        .map(|p| (f32::from(p[0]), f32::from(p[1]) * 100.0 / 255.0))
        .collect();
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    let pct = if t <= first.0 {
        first.1
    } else if t >= last.0 {
        last.1
    } else {
        let w = pts.windows(2).find(|w| t <= w[1].0).unwrap_or(&pts[..2]);
        w[0].1 + (t - w[0].0) / (w[1].0 - w[0].0) * (w[1].1 - w[0].1)
    };
    pct.round() as u8
}

/// The sequence of the device behind `io`, so a cooler's lighting and fans step one counter.
fn io_seq(io: &dyn Io) -> u8 {
    seq(&io.port().map(|p| p.device().clone()))
}

pub fn crc(data: &[u8]) -> u8 {
    data.iter().fold(0, |c, &b| CRC_TABLE[(c ^ b) as usize])
}

/// A 65-byte packet: `3F`, sequence with command bits, body, CRC of bytes 2..=63.
pub fn packet(seq: u8, command: u8, body: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; LEN];
    p[1] = MAGIC;
    p[2] = seq | command;
    let n = body.len().min(61);
    p[3..3 + n].copy_from_slice(&body[..n]);
    p[64] = crc(&p[2..64]);
    p
}

pub fn status_request(seq: u8) -> Vec<u8> {
    packet(seq, COOLING, &[STATUS, 0x00])
}

/// What one fan gets: a fixed duty byte, or the firmware's balanced curve.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fan {
    Fixed(u8),
    Curve,
}

impl Fan {
    fn mode(self) -> (u8, u8) {
        match self {
            Fan::Fixed(d) => (FAN_FIXED, d),
            Fan::Curve => (FAN_CURVE, 0x00),
        }
    }
}

/// `sendCoolingPacket` (and `sendCoolingProfile` for curve fans): fans 1 and 2 plus pump mode.
pub fn cooling(seq: u8, fan1: Fan, fan2: Fan, pump: u8) -> Vec<u8> {
    let mut b = [0xFFu8; 61];
    b[..9].copy_from_slice(&[0x14, 0x00, 0xFF, 0x05, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    for (i, f) in [fan1, fan2].into_iter().enumerate() {
        let (mode, duty) = f.mode();
        let at = 9 + i * 6;
        b[at..at + 6].copy_from_slice(&[mode, 0, 0, 0, 0, duty]);
        if f == Fan::Curve {
            let c = 28 + i * 14;
            b[c..c + 14].copy_from_slice(&BALANCED_CURVE);
        }
    }
    b[21..26].copy_from_slice(&[pump, 0xFF, 0xFF, 0x00, 0x00]);
    b[27] = CURVE_POINTS;
    packet(seq, COOLING, &b)
}

/// The third fan of the 360 mm models rides in its own packet, in fan 1's place.
pub fn cooling_third(seq: u8, fan3: Fan) -> Vec<u8> {
    let mut b = [0xFFu8; 61];
    b[..9].copy_from_slice(&[0x14, 0x00, 0xFF, 0x05, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let (mode, duty) = fan3.mode();
    b[9..15].copy_from_slice(&[mode, 0, 0, 0, 0, duty]);
    if fan3 == Fan::Curve {
        b[27] = CURVE_POINTS;
        b[28..42].copy_from_slice(&BALANCED_CURVE);
    }
    packet(seq, THIRD_FAN, &b)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub liquid: f32,
    pub fans: [u32; 3],
    pub pump: u32,
}

/// `getCoolingInfo`, read from a hidapi reply.
pub fn parse_status(r: &[u8]) -> Option<Status> {
    let raw = u16le(r, LIQUID_AT)?;
    Some(Status {
        liquid: f32::from(raw) / 256.0,
        fans: [
            u32::from(u16le(r, FAN_RPM_AT[0])?),
            u32::from(u16le(r, FAN_RPM_AT[1])?),
            u32::from(u16le(r, FAN_RPM_AT[2])?),
        ],
        pump: u32::from(u16le(r, PUMP_RPM_AT)?),
    })
}

/// Pump duty to the plugin's three pump modes.
pub fn pump_mode(duty: f32) -> u8 {
    if duty < 0.6 {
        QUIET
    } else if duty < 0.85 {
        BALANCED
    } else {
        EXTREME
    }
}

/// `Validate`: interface 0, or none.
pub fn wants(info: &Info) -> bool {
    info.vid == VID
        && IDS.iter().any(|i| i.0 == info.pid)
        && (info.interface == 0 || info.interface == -1)
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: String,
    fans: usize,
    duty: [Option<u8>; 3],
    pump: Option<u8>,
    status: Cached<Status>,
    sent: Option<(Vec<Fan>, u8, Instant)>,
    /// The device's epoch at the last push: lighting setup and release reuse command 3.
    epoch: u32,
}

impl Hub {
    fn device(&self) -> Option<Arc<Dev>> {
        self.io.port().map(|p| p.device().clone())
    }

    fn write(&mut self, p: &[u8]) -> Result<(), String> {
        self.io.write(p, LEN)?;
        self.io.sleep(PACE_MS);
        Ok(())
    }

    fn plan(&self) -> (Vec<Fan>, u8) {
        let fans = (0..self.fans)
            .map(|i| self.duty[i].map_or(Fan::Curve, Fan::Fixed))
            .collect();
        (fans, self.pump.unwrap_or(BALANCED))
    }

    fn push(&mut self, force: bool) -> Result<(), String> {
        let device = self.device();
        let epoch = device.as_ref().map_or(0, |d| d.epoch());
        let force = force || std::mem::replace(&mut self.epoch, epoch) != epoch;
        let (fans, pump) = self.plan();
        let fresh = self
            .sent
            .as_ref()
            .is_some_and(|(f, p, at)| *f == fans && *p == pump && !due(Some(*at), RESEND_MS));
        if fresh && !force {
            return Ok(());
        }
        if self.fans == 3 {
            self.write(&cooling_third(seq(&device), fans[2]))?;
        }
        let second = fans.get(1).copied().unwrap_or(Fan::Curve);
        self.write(&cooling(seq(&device), fans[0], second, pump))?;
        self.sent = Some((fans, pump, Instant::now()));
        Ok(())
    }

    fn controlling(&self) -> bool {
        self.duty.iter().any(Option::is_some) || self.pump.is_some()
    }

    fn read_status(&mut self) -> Option<Status> {
        let device = self.device();
        let io = self.io.as_mut();
        self.status.get(|| {
            for _ in 0..2 {
                let r = ask(io, &status_request(seq(&device)), LEN, LEN, 100, |r| {
                    r.len() > 44
                })
                .ok()?;
                if r.get(ERROR_AT).is_some_and(|e| e & SEQUENCE_ERROR != 0) {
                    continue;
                }
                return parse_status(&r);
            }
            None
        })
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn channels(&self) -> Vec<Channel> {
        let mut c = vec![Channel::pump("Pump")];
        c.extend((1..=self.fans).map(|i| Channel::fan(format!("Fan {i}"))));
        c
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Liquid Temperature".into()]
    }
    fn rpm(&mut self, channel: usize) -> Option<u32> {
        // Lighting's software setup rewrites command 3, the third fan's, so the curve goes again.
        let moved = self.device().is_some_and(|d| d.epoch() != self.epoch);
        if self.controlling() || moved {
            let _ = self.push(false);
        }
        let s = self.read_status()?;
        match channel {
            0 => Some(s.pump),
            i if i <= self.fans => Some(s.fans[i - 1]),
            _ => None,
        }
    }
    fn set_duty(&mut self, channel: usize, duty: f32) -> Result<(), String> {
        match channel {
            0 => self.pump = Some(pump_mode(duty)),
            i if i <= self.fans => self.duty[i - 1] = Some(byte(duty)),
            _ => return Err("no such channel".into()),
        }
        self.push(false)
    }
    fn release(&mut self, channel: usize) -> Result<(), String> {
        let was = self.controlling();
        match channel {
            0 => self.pump = None,
            i if i <= self.fans => self.duty[i - 1] = None,
            _ => return Ok(()),
        }
        if was { self.push(true) } else { Ok(()) }
    }
    fn temperature(&mut self, sensor: usize) -> Option<f32> {
        if sensor != 0 {
            return None;
        }
        self.read_status().map(|s| s.liquid).filter(|&t| t > 0.0)
    }
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let &(_, name, fans) = IDS.iter().find(|i| i.0 == info.pid)?;
    let io = open_io(api, info).ok()?;
    Some(backend_on(
        info,
        Hub {
            io,
            id: fan_id(info, "corsair-platinum"),
            name: name.into(),
            fans,
            duty: [None; 3],
            pump: None,
            status: Cached::new(POLL_MS),
            sent: None,
            epoch: 0,
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

/// `EnableSoftwareControl`'s three bodies.
fn software_bodies() -> [[u8; 61]; 3] {
    let mut m = [[0xFFu8; 61]; 3];
    m[0][0] = 0x01;
    m[0][1] = 0x01;
    m[0][15..19].fill(0x7F);
    for i in [20, 25, 30, 35, 40, 45] {
        m[0][i] = 0x00;
    }
    for i in 0..40u8 {
        m[1][i as usize] = i;
        m[2][i as usize] = 0x28 + i;
    }
    m
}

/// Pump-head LED index -> cell of the plugin's 5x5 layout.
const BLOCK_CELLS: [(u8, u8); BLOCK_LEDS] = [
    (2, 1),
    (3, 2),
    (2, 3),
    (1, 2),
    (3, 4),
    (2, 4),
    (1, 4),
    (0, 3),
    (0, 2),
    (0, 1),
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 1),
    (4, 2),
    (4, 3),
];

/// `sendColors`: BGR by hardware index, pump head then the fan channel, in 60-byte packets.
pub fn color_packets(colors: &[Color], mut seq: impl FnMut() -> u8) -> Vec<Vec<u8>> {
    let data: Vec<u8> = colors.iter().flat_map(|c| [c.2, c.1, c.0]).collect();
    data.chunks(60)
        .zip(COLORS)
        .map(|(chunk, cmd)| packet(seq(), cmd, chunk))
        .collect()
}

struct Lights;

impl Proto for Lights {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for (body, cmd) in software_bodies()
            .iter()
            .zip([SOFTWARE_1, SOFTWARE_2, SOFTWARE_3])
        {
            io.write(&packet(io_seq(&*io), cmd, body), LEN)?;
            io.sleep(PACE_MS);
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in color_packets(colors, || io_seq(&*io)) {
            io.write(&p, LEN)?;
            io.sleep(PACE_MS);
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(5000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.frame(io, &[Color(0, 0, 0); BLOCK_LEDS + FAN_LEDS]);
    }
}

fn open_lights(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    let mut dev = Device::grid(
        &info.device_id("corsair-platinum-xpack"),
        d.name,
        DeviceKind::Other,
        [0.34, 0.12, 0.24, 0.24],
        5,
        6,
    );
    dev.leds = BLOCK_CELLS
        .iter()
        .map(|&(x, y)| [(f32::from(x) + 0.5) / 5.0, (f32::from(y) + 0.5) / 6.0])
        .chain((0..FAN_LEDS).map(|i| [(i as f32 + 0.5) / FAN_LEDS as f32, 5.5 / 6.0]))
        .collect();
    Ok(Driver::new(io, dev, "corsair platinum", Lights).boxed())
}

pub static DETECTORS: [Detector; 4] = [
    crate::hid::on_i("Corsair H115i Pro XT", VID, 0x0C2E, 0, open_lights),
    crate::hid::on_i("Corsair H115i Pro XT", VID, 0x0C2E, -1, open_lights),
    crate::hid::on_i("Corsair H150i Pro XT", VID, 0x0C2F, 0, open_lights),
    crate::hid::on_i("Corsair H150i Pro XT", VID, 0x0C2F, -1, open_lights),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_table_start() {
        assert_eq!(&CRC_TABLE[..8], &[0, 7, 14, 9, 28, 27, 18, 21]);
        assert_eq!(CRC_TABLE[255], 243);
    }

    // sendCoolingPacket(fixed, d1, fixed, d2, balanced)
    #[test]
    fn cooling_fixed_layout() {
        let p = cooling(0x10, Fan::Fixed(0x80), Fan::Fixed(0x40), BALANCED);
        assert_eq!(p.len(), 65);
        assert_eq!(
            &p[..12],
            &[
                0, 0x3F, 0x10, 0x14, 0, 0xFF, 0x05, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF
            ]
        );
        assert_eq!(
            &p[12..30],
            &[
                3, 0, 0, 0, 0, 0x80, 3, 0, 0, 0, 0, 0x40, 1, 0xFF, 0xFF, 0, 0, 0xFF
            ]
        );
        assert_eq!(p[30], 0x07);
        assert!(p[31..64].iter().all(|&b| b == 0xFF));
        assert_eq!(p[64], crc(&p[2..64]));
    }

    // sendCoolingProfile(Balanced): both fans on the curve
    #[test]
    fn cooling_curve_layout() {
        let p = cooling(0x08, Fan::Curve, Fan::Curve, BALANCED);
        assert_eq!(p[12], 0x00);
        assert_eq!(p[18], 0x00);
        assert_eq!(&p[31..45], &BALANCED_CURVE);
        assert_eq!(&p[45..59], &BALANCED_CURVE);
        assert!(p[59..64].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn third_fan_packet() {
        let p = cooling_third(0x18, Fan::Fixed(0x99));
        assert_eq!(p[2], 0x1B);
        assert_eq!(&p[12..18], &[3, 0, 0, 0, 0, 0x99]);
        assert!(p[18..64].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn status_parse() {
        let mut r = vec![0u8; 64];
        r[7] = 0x80;
        r[8] = 0x1E;
        r[15..17].copy_from_slice(&1200u16.to_le_bytes());
        r[22..24].copy_from_slice(&1100u16.to_le_bytes());
        r[29..31].copy_from_slice(&2500u16.to_le_bytes());
        r[43..45].copy_from_slice(&900u16.to_le_bytes());
        let s = parse_status(&r).unwrap();
        assert_eq!(s.fans, [1200, 1100, 900]);
        assert_eq!(s.pump, 2500);
        assert!((s.liquid - 30.5).abs() < 0.01);
        assert_eq!(status_request(0x20)[..5], [0, 0x3F, 0x20, 0xFF, 0x00]);
    }

    #[test]
    fn colors_split_in_three() {
        let c = vec![Color(1, 2, 3); BLOCK_LEDS + FAN_LEDS];
        let p = color_packets(&c, next_seq);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0][2] & 0b111, 0b100);
        assert_eq!(&p[0][3..6], &[3, 2, 1]);
        assert_eq!(p[2][2] & 0b111, 0b110);
        assert_eq!(pump_mode(0.5), QUIET);
        assert_eq!(pump_mode(0.7), BALANCED);
        assert_eq!(pump_mode(1.0), EXTREME);
    }

    // One sequence for lights and fans; lighting setup rewrites command 3, so fans resend.
    #[test]
    fn lighting_and_fans_share_the_sequence_and_resend_after_setup() {
        use crate::hid::wire::Wire;
        let wire = Wire::default();
        let port = wire.port();
        let mut hub = Hub {
            io: Box::new(port.clone()),
            id: "fan:p".into(),
            name: "P".into(),
            fans: 3,
            duty: [None; 3],
            pump: None,
            status: Cached::new(POLL_MS),
            sent: None,
            epoch: 0,
        };
        hub.set_duty(1, 0.5).unwrap();
        let dev = Device::grid("p", "P", DeviceKind::Other, [0.0; 4], 48, 1);
        let mut lights = Driver::new(Box::new(port.clone()), dev, "p", Lights);
        lights.push("p", &[Color(1, 2, 3); 48]).unwrap();
        assert!(port.device().others_active(foxden_hid::port::FANS));
        hub.status = Cached::new(60_000);
        let _ = hub.status.get(|| None);
        hub.rpm(1);
        let sent = wire.take();
        let cmds: Vec<u8> = sent.iter().map(|p| p[2] & 0b111).collect();
        assert_eq!(cmds, [3, 0, 1, 2, 3, 4, 5, 6, 3, 0]);
        let seqs: Vec<u8> = sent.iter().map(|p| p[2] >> 3).collect();
        assert_eq!(seqs, (1..=10).collect::<Vec<u8>>());
        drop(lights);
        assert!(!port.device().others_active(foxden_hid::port::FANS));
    }

    #[test]
    fn sequence_wraps() {
        for _ in 0..40 {
            let s = next_seq();
            assert!((8..=248).contains(&s) && s.is_multiple_of(8));
        }
    }
}
