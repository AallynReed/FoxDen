//! Ported from OpenRGB Controllers/SinowealthController (GPL-2.0-or-later).

use std::time::Duration;

use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::common::{Driver, Hid, Io, KEYBOARD, MOUSE, NA, line, matrix, siblings};
use crate::registry::{Detector, Info};

const VID: u16 = 0x258A;

struct Probe {
    data: HidDevice,
    cmd: Option<HidDevice>,
    response: Vec<u8>,
}

/// DetectUsages for one expected report: of the collections on this usage page, the one that takes
/// `cmd` (if any) and the one that answers feature report `id` of `size` bytes; the last to answer wins.
fn probe(
    api: &HidApi,
    info: &Info,
    need: usize,
    id: u8,
    size: usize,
    cmd: Option<&[u8]>,
) -> Result<Probe, String> {
    let mut devs: Vec<Option<HidDevice>> = siblings(api, info, |s| s.usage_page == info.usage_page)
        .iter()
        .map(|s| s.open(api).ok())
        .collect();
    if devs.len() < need {
        return Err("too few collections".into());
    }
    let mut cmd_at = None;
    if let Some(cmd) = cmd {
        for (i, d) in devs.iter().enumerate() {
            if d.as_ref()
                .is_some_and(|d| d.send_feature_report(cmd).is_ok())
            {
                cmd_at = Some(i);
            }
        }
        if cmd_at.is_none() {
            return Err("no collection took the command".into());
        }
    }
    let mut data_at = None;
    let mut response = vec![0u8; size];
    for (i, d) in devs.iter().enumerate() {
        let mut buf = vec![0u8; size];
        buf[0] = id;
        if d.as_ref()
            .is_some_and(|d| d.get_feature_report(&mut buf).is_ok())
        {
            data_at = Some(i);
            response = buf;
        }
    }
    let data_at = data_at.ok_or("no collection answered")?;
    let data = devs[data_at].take().ok_or("closed")?;
    let cmd = cmd_at
        .filter(|&i| i != data_at)
        .and_then(|i| devs[i].take());
    Ok(Probe {
        data,
        cmd,
        response,
    })
}

/// SinowealthController::SetMode(Static) over the configuration GetProfile read: brightness 4, RBG or RGB.
pub fn mouse_config(read: &[u8], c: Color, rgb: bool) -> Vec<u8> {
    let mut b = vec![0u8; 520];
    let n = read.len().min(167);
    b[..n].copy_from_slice(&read[..n]);
    b[0x03] = 0x7B;
    b[0x06] = 0x00;
    b[0x35] = 0x02;
    b[0x38] = 0x04 << 4;
    let raw = if rgb {
        [c.0, c.1, c.2]
    } else {
        [c.0, c.2, c.1]
    };
    b[0x39..0x3C].copy_from_slice(&raw);
    b
}

pub const MOUSE_READ: [u8; 6] = [0x05, 0x11, 0x00, 0x00, 0x00, 0x00];
pub const FIRMWARE_READ: [u8; 6] = [0x05, 0x01, 0x00, 0x00, 0x00, 0x00];

/// Colour order by firmware: Some(true) RGB, Some(false) RBG, None for builds without LEDs.
pub fn firmware_order(fw: &[u8]) -> Option<bool> {
    match fw {
        b"V161" => None,
        b"3106" | b"3110" | b"2642" | b"V127" | b"V287" => Some(true),
        _ => Some(false),
    }
}

fn firmware(p: &Probe) -> Vec<u8> {
    let dev = p.cmd.as_ref().unwrap_or(&p.data);
    let mut buf = [0u8; 6];
    buf[0] = 0x05;
    if dev.send_feature_report(&FIRMWARE_READ).is_err() || dev.get_feature_report(&mut buf).is_err()
    {
        return Vec::new();
    }
    buf[2..6].to_vec()
}

struct GloriousMouse {
    cmd: Option<Box<dyn Io>>,
    rgb: bool,
}

impl Driver for GloriousMouse {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let asked = match self.cmd.as_mut() {
            Some(cmd) => cmd.send_feature(&MOUSE_READ),
            None => io.send_feature(&MOUSE_READ),
        };
        let mut read = vec![0u8; 520];
        read[0] = 0x04;
        // GetProfile below 131 bytes (or failed) skips the frame
        if asked.is_err() || io.get_feature(&mut read).unwrap_or(0) < 131 {
            return Ok(());
        }
        io.send_feature(&mouse_config(&read, colors[0], self.rgb))?;
        io.sleep(50);
        Ok(())
    }
}

fn open_mouse(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let p = probe(api, info, 3, 0x04, 520, Some(&MOUSE_READ))?;
    let rgb = firmware_order(&firmware(&p)).ok_or("this firmware has no LEDs")?;
    let dev = line(
        info.device_id("sinowealth"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        1,
    );
    let drv = GloriousMouse {
        cmd: p.cmd.map(|c| Box::new(c) as Box<dyn Io>),
        rgb,
    };
    Ok(Hid::boxed("sinowealth mouse", p.data, drv, dev))
}

/// SinowealthController1007::SendPacket in Custom mode (0x60 + default speed 2, direction right).
pub fn zet_packet(read: &[u8; 59], colors: &[Color]) -> [u8; 59] {
    let mut b = *read;
    b[23..44].fill(0);
    for (i, c) in colors.iter().take(7).enumerate() {
        b[23 + 3 * i..26 + 3 * i].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b[21] = 0x62;
    b[22] = 0x00;
    b
}

struct Zet;

impl Driver for Zet {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut read = [0u8; 59];
        read[0] = 0x04;
        if io.get_feature(&mut read).is_err() {
            return Ok(());
        }
        io.send_feature(&zet_packet(&read, colors))
    }
}

fn open_zet(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let p = probe(api, info, 5, 0x04, 59, None)?;
    let dev = line(
        info.device_id("sinowealth"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        7,
    );
    Ok(Hid::boxed("sinowealth zet fury pro", p.data, Zet, dev))
}

/// Genesis Thor 300 key index of each LED, in OpenRGB order.
#[rustfmt::skip]
const THOR_KEYS: [u8; 104] = [
    0x01, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0F, 0x10, 0x11, 0x12,
    0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3A, 0x3B, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D,
    0x4E, 0x4F, 0x50, 0x51, 0x52, 0x54, 0x58, 0x59, 0x5A, 0x44, 0x5D, 0x5F, 0x60, 0x61, 0x62, 0x63,
    0x64, 0x65, 0x66, 0x67, 0x68, 0x6B, 0x6D, 0x6F, 0x70, 0x71, 0x74, 0x75, 0x76, 0x79, 0x7C, 0x7D,
    0x7E, 0x80, 0x83, 0x84, 0x85, 0x86, 0x88, 0x72,
];

#[rustfmt::skip]
const THOR_MAP: [u32; 6 * 23] = [
    0, NA, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, NA, 13, 14, 15, NA, NA, NA, NA, NA,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, NA, 30, 31, 32, NA, 33, 34, 35, 36,
    37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, NA, NA, 50, 51, 52, NA, 53, 54, 55, NA,
    56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, NA, NA, NA, NA, NA, 70, 71, 72, 73,
    74, NA, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, NA, 85, NA, NA, 86, NA, NA, 87, 88, 89, NA,
    90, 91, 92, NA, NA, NA, 93, NA, NA, NA, 94, 95, 96, 97, NA, 98, 99, 100, NA, 101, NA, 102, 103,
];

/// SinowealthKeyboard90Controller::SendFeatureReport.
pub fn thor_report(cmd: u8, args: [u8; 5]) -> [u8; 8] {
    [0x0A, cmd, args[0], args[1], args[2], args[3], args[4], 0x00]
}

/// RGBController_SinowealthKeyboard90::DeviceUpdateLEDs: SendMode(Custom, full), every key, commit.
pub fn thor_frame(colors: &[Color]) -> Vec<[u8; 8]> {
    let mut out = vec![
        thor_report(0x03, [0x01, 0, 0, 0, 0]),
        thor_report(0x0A, [0x13, 0x04, 0x02, 0x07, 0]),
    ];
    out.extend(
        THOR_KEYS
            .iter()
            .zip(colors)
            .map(|(&k, c)| thor_report(0x0C, [0x01, k, c.0, c.1, c.2])),
    );
    out.push(thor_report(0x0C, [0x01, 0x89, 0, 0, 0]));
    out
}

struct Thor;

impl Driver for Thor {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        thor_frame(colors)
            .iter()
            .try_for_each(|r| io.send_feature(r))
    }
}

fn open_thor(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = matrix(
        info.device_id("sinowealth"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        (6, 23),
        &THOR_MAP,
        104,
    );
    Ok(Hid::boxed(
        "sinowealth genesis thor 300",
        info.open(api)?,
        Thor,
        dev,
    ))
}

/// Keyboards behind PID 010C by the model byte the probe returns: (name, LEDs up to the highest index).
pub fn kb10c_model(id: u8) -> Option<(&'static str, usize)> {
    match id {
        0xBE => Some(("Kreo Swarm", 96)),
        0xCD => Some(("AULA F75", 90)),
        0xA4 => Some(("AULA F99", 113)),
        0x0B => Some(("AULA F87 Pro", 102)),
        0xA3 => Some(("LEOBOG Hi75C Pro", 90)),
        0x20 | 0x05 => Some(("Redragon K686 Eisa Pro", 113)),
        _ => None,
    }
}

pub fn kb10c_probe_command() -> Vec<u8> {
    let mut c = vec![0u8; 520];
    c[..7].copy_from_slice(&[0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06]);
    c
}

/// SinowealthKeyboard10cController::SetLEDsDirect.
pub fn kb10c_direct(colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 520];
    b[..8].copy_from_slice(&[0x06, 0x08, 0x00, 0x00, 0x01, 0x00, 0x7A, 0x01]);
    for (i, c) in colors.iter().take(170).enumerate() {
        b[8 + 3 * i..11 + 3 * i].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

struct Kb10c;

impl Driver for Kb10c {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&kb10c_direct(colors))?;
        io.sleep(1);
        Ok(())
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
}

/// LED indices run down six-key columns, so index / 6 is the column and index % 6 the row.
fn kb10c_device(id: String, name: &str, count: usize) -> Device {
    let cols = count.div_ceil(6);
    let mut d = line(id, name, DeviceKind::Keyboard, KEYBOARD, 0);
    d.leds = (0..count)
        .map(|i| {
            [
                ((i / 6) as f32 + 0.5) / cols as f32,
                ((i % 6) as f32 + 0.5) / 6.0,
            ]
        })
        .collect();
    d.grid = [cols as u32, 6];
    d
}

fn open_kb10c(api: &HidApi, info: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
    let p = probe(api, info, 3, 0x06, 520, Some(&kb10c_probe_command()))?;
    let (name, count) = kb10c_model(p.response[13]).ok_or("unknown Sinowealth 010C model")?;
    let dev = kb10c_device(info.device_id("sinowealth"), name, count);
    Ok(Hid::boxed("sinowealth keyboard", p.data, Kb10c, dev))
}

pub static DETECTORS: [Detector; 9] = [
    Detector {
        name: "Glorious Model O / O-",
        vid: VID,
        pid: 0x0036,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "Glorious Model D / D-",
        vid: VID,
        pid: 0x0033,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "Glorious Model O Eternal",
        vid: 0x3794,
        pid: 0xA000,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "SinoWealth Mouse (0027)",
        vid: VID,
        pid: 0x0027,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "SinoWealth Mouse (0051)",
        vid: VID,
        pid: 0x0051,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "Everest GT-100 RGB",
        vid: VID,
        pid: 0x0029,
        interface: None,
        usage_page: Some(0xFF00),
        usage: None,
        open: open_mouse,
    },
    Detector {
        name: "ZET Fury Pro",
        vid: VID,
        pid: 0x1007,
        interface: Some(1),
        usage_page: Some(0xFF00),
        usage: Some(1),
        open: open_zet,
    },
    Detector {
        name: "Genesis Thor 300",
        vid: VID,
        pid: 0x0090,
        interface: Some(1),
        usage_page: Some(0xFF00),
        usage: Some(1),
        open: open_thor,
    },
    Detector {
        name: "Sinowealth Keyboard",
        vid: VID,
        pid: 0x010C,
        interface: Some(1),
        usage_page: Some(0xFF00),
        usage: Some(1),
        open: open_kb10c,
    },
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    const C1: Color = Color(0x11, 0x22, 0x33);

    // SinowealthController::GetProfile then SetMode(Static): 167 bytes copied, R B G colour.
    #[test]
    fn glorious_frame() {
        let m = Mock::default();
        let mut read = vec![0xAA; 520];
        read[0] = 0x04;
        m.reply(read.clone());
        let dev = line("m".into(), "M", DeviceKind::Mouse, MOUSE, 1);
        let mut b = Hid::new(
            "t",
            Box::new(m.clone()),
            GloriousMouse {
                cmd: None,
                rgb: false,
            },
            dev,
        );
        b.push("m", &[C1]).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::Feature(MOUSE_READ.to_vec()));
        assert_eq!(ops[1], Op::GetFeature(pad(&[0x04], 520)));
        let Op::Feature(cfg) = &ops[2] else { panic!() };
        assert_eq!(cfg[..8], [0x04, 0xAA, 0xAA, 0x7B, 0xAA, 0xAA, 0x00, 0xAA]);
        assert_eq!(cfg[0x35..0x3C], [0x02, 0xAA, 0xAA, 0x40, 0x11, 0x33, 0x22]);
        assert_eq!(cfg[166], 0xAA);
        assert!(cfg[167..].iter().all(|&x| x == 0));
        assert_eq!(ops[3], Op::Sleep(50));
        // a short configuration read aborts SetMode
        m.reply(vec![0x04; 100]);
        b.push("m", &[Color(1, 1, 1)]).unwrap();
        assert_eq!(m.take().len(), 2);
    }

    #[test]
    fn colour_order_by_firmware() {
        let read = vec![0u8; 520];
        assert_eq!(
            mouse_config(&read, C1, true)[0x39..0x3C],
            [0x11, 0x22, 0x33]
        );
        assert_eq!(
            mouse_config(&read, C1, false)[0x39..0x3C],
            [0x11, 0x33, 0x22]
        );
        assert_eq!(firmware_order(b"3106"), Some(true));
        assert_eq!(firmware_order(b"V102"), Some(false));
        assert_eq!(firmware_order(b""), Some(false));
        assert_eq!(firmware_order(b"V161"), None);
        assert_eq!(FIRMWARE_READ, [0x05, 0x01, 0, 0, 0, 0]);
        assert_eq!(DETECTORS.len(), 9);
    }

    // SinowealthController1007::SetLEDColors -> SendPacket.
    #[test]
    fn zet_packet_bytes() {
        let mut read = [0x55u8; 59];
        read[0] = 0x04;
        let p = zet_packet(&read, &[C1, Color(4, 5, 6)]);
        assert_eq!(p[..3], [0x04, 0x55, 0x55]);
        assert_eq!(p[21..32], [0x62, 0x00, 0x11, 0x22, 0x33, 4, 5, 6, 0, 0, 0]);
        assert!(p[29..44].iter().all(|&x| x == 0));
        assert_eq!(p[44..], [0x55; 15]);
    }

    // RGBController_SinowealthKeyboard90::DeviceUpdateLEDs.
    #[test]
    fn thor_frame_reports() {
        let mut colors = vec![Color::BLACK; 104];
        colors[103] = C1;
        let f = thor_frame(&colors);
        assert_eq!(f.len(), 107);
        assert_eq!(f[0], [0x0A, 0x03, 0x01, 0, 0, 0, 0, 0]);
        assert_eq!(f[1], [0x0A, 0x0A, 0x13, 0x04, 0x02, 0x07, 0, 0]);
        assert_eq!(f[2], [0x0A, 0x0C, 0x01, 0x01, 0, 0, 0, 0]);
        assert_eq!(f[105], [0x0A, 0x0C, 0x01, 0x72, 0x11, 0x22, 0x33, 0]);
        assert_eq!(f[106], [0x0A, 0x0C, 0x01, 0x89, 0, 0, 0, 0]);
    }

    // SinowealthKeyboard10cController::SetLEDsDirect and the keepalive thread's 1 s resend.
    #[test]
    fn kb10c_direct_and_keepalive() {
        let p = kb10c_direct(&[C1, Color(4, 5, 6)]);
        assert_eq!(
            p[..14],
            [
                0x06, 0x08, 0x00, 0x00, 0x01, 0x00, 0x7A, 0x01, 0x11, 0x22, 0x33, 4, 5, 6
            ]
        );
        assert_eq!(p.len(), 520);
        assert_eq!(
            kb10c_probe_command()[..8],
            [0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06, 0x00]
        );
        assert_eq!(kb10c_model(0x05), Some(("Redragon K686 Eisa Pro", 113)));
        assert_eq!(Kb10c.keepalive(), Some(Duration::from_secs(1)));
        let d = kb10c_device("k".into(), "K", 90);
        assert_eq!(d.leds.len(), 90);
        assert_eq!(d.leds[7], [1.5 / 15.0, 1.5 / 6.0]);
    }
}
