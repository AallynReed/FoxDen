//! Ported from OpenRGB Controllers/RoccatController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::common::{Driver, HEADSET, Hid, Io, KEYBOARD, MOUSE, PAD, line, sum16};
use crate::registry::{Detector, Info};

pub const VID: u16 = 0x1E7D;

fn rgb(c: &Color) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// SwitchControl(true) of the Burst, Kone Pro and Kone Aimo; EnableDirect(true) of the Kone XP.
pub fn switch_control() -> [u8; 6] {
    [0x0E, 0x06, 0x01, 0x01, 0x00, 0xFF]
}

/// SendDirect of the Burst and Kone Pro.
pub fn burst_direct(colors: &[Color]) -> [u8; 11] {
    let mut p = [0u8; 11];
    p[0] = 0x0D;
    p[1] = 0x0B;
    for (i, c) in colors.iter().take(3).enumerate() {
        p[2 + 3 * i..5 + 3 * i].copy_from_slice(&rgb(c));
    }
    p
}

struct Burst;

impl Driver for Burst {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&switch_control())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&burst_direct(colors))
    }
}

fn open_burst(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    leds: usize,
) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        leds,
    );
    Ok(Hid::boxed("roccat burst", info.open(api)?, Burst, dev))
}

fn open_burst_core(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_burst(api, info, d, 1)
}

fn open_burst_pro(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_burst(api, info, d, 2)
}

/// SetModeValues(direct, speed 0, brightness 0xFF) of the Burst Pro Air.
pub fn burst_pro_air_mode() -> [u8; 30] {
    let mut p = [0u8; 30];
    p[..9].copy_from_slice(&[0x06, 0x01, 0x4C, 0x06, 0x06, 0x01, 0x00, 0xFF, 0x0F]);
    p
}

/// SetColors of the Burst Pro Air.
pub fn burst_pro_air_colors(colors: &[Color]) -> [u8; 30] {
    let mut p = [0u8; 30];
    p[..5].copy_from_slice(&[0x06, 0x01, 0x4C, 0x06, 0x14]);
    for (i, c) in colors.iter().take(4).enumerate() {
        p[5 + 5 * i] = i as u8 + 1;
        p[6 + 5 * i] = 0xFF;
        p[7 + 5 * i..10 + 5 * i].copy_from_slice(&rgb(c));
    }
    p
}

struct BurstProAir;

impl Driver for BurstProAir {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&burst_pro_air_mode())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&burst_pro_air_colors(colors))
    }
}

fn open_burst_pro_air(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        4,
    );
    Ok(Hid::boxed(
        "roccat burst pro air",
        info.open(api)?,
        BurstProAir,
        dev,
    ))
}

/// SendDirect of the Elo 7.1.
pub fn elo_direct(c: Color) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[0] = 0xFF;
    p[1] = 0x04;
    p[4..7].copy_from_slice(&rgb(&c));
    p
}

/// SendInit of the Elo 7.1, in order.
pub fn elo_init() -> Vec<[u8; 16]> {
    let mut a = [0u8; 16];
    a[0] = 0xFF;
    a[1] = 0x01;
    let mut b = a;
    b[1] = 0x02;
    let mut c = b;
    c[1] = 0x03;
    c[3] = 0x01;
    vec![a, b, c, elo_direct(Color::BLACK), a]
}

struct Elo;

impl Driver for Elo {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        elo_init().iter().try_for_each(|p| io.write(p))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&elo_direct(colors[0]))
    }
}

fn open_elo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Other,
        HEADSET,
        1,
    );
    Ok(Hid::boxed("roccat elo", info.open(api)?, Elo, dev))
}

/// InitialPacket of the Horde Aimo.
pub fn horde_init() -> [u8; 8] {
    [0x13, 0x08, 0x01, 0, 0, 0, 0, 0]
}

/// SetColors of the Horde Aimo.
pub fn horde_colors(colors: &[Color]) -> [u8; 24] {
    let mut p = [0u8; 24];
    p[..4].copy_from_slice(&[0x18, 24, 0xFF, 0xFF]);
    for (i, c) in colors.iter().take(6).enumerate() {
        p[4 + 3 * i..7 + 3 * i].copy_from_slice(&rgb(c));
    }
    let crc = sum16(&p[..22]);
    p[22..].copy_from_slice(&crc.to_le_bytes());
    p
}

struct Horde;

impl Driver for Horde {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&horde_init())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&horde_colors(colors))?;
        // OpenRGB reads the 3-byte answer into an uninitialised buffer; report 0x18 stands in.
        let _ = io.get_feature(&mut [0x18, 0, 0]);
        Ok(())
    }
}

fn open_horde(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Keyboard,
        KEYBOARD,
        6,
    );
    Ok(Hid::boxed("roccat horde aimo", info.open(api)?, Horde, dev))
}

/// SetChannelColors over all 11 LEDs (wheel, left strip, right strip, lower left, lower right).
pub fn kone_aimo_colors(colors: &[Color]) -> [u8; 46] {
    let mut p = [0u8; 46];
    p[0] = 0x0D;
    p[1] = 0x2E;
    for (i, c) in colors.iter().take(11).enumerate() {
        p[2 + 4 * i..5 + 4 * i].copy_from_slice(&rgb(c));
    }
    p
}

struct KoneAimo;

impl Driver for KoneAimo {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        let _ = io.get_feature(&mut [0u8; 3]);
        io.send_feature(&switch_control())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&kone_aimo_colors(colors))?;
        // OpenRGB reads the answer into its colour buffer; a scratch buffer keeps byte 1 intact.
        let _ = io.get_feature(&mut [0x0D, 0, 0]);
        Ok(())
    }
}

fn open_kone_aimo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        11,
    );
    Ok(Hid::boxed(
        "roccat kone aimo",
        info.open(api)?,
        KoneAimo,
        dev,
    ))
}

fn open_kone_pro(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        2,
    );
    Ok(Hid::boxed("roccat kone pro", info.open(api)?, Burst, dev))
}

/// SendRGB(direct) of the Kone Pro Air: mode 0x09, speed 0x0B, brightness 0x64.
pub fn kone_pro_air_direct(colors: &[Color]) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[..10].copy_from_slice(&[0x00, 0x10, 0x10, 0x0B, 0x00, 0x09, 0x64, 0x64, 0x64, 0x0B]);
    for (i, c) in colors.iter().take(2).enumerate() {
        p[10 + 3 * i..13 + 3 * i].copy_from_slice(&rgb(c));
    }
    p
}

struct KoneProAir;

impl Driver for KoneProAir {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&kone_pro_air_direct(colors))
    }
}

fn open_kone_pro_air(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        2,
    );
    Ok(Hid::boxed(
        "roccat kone pro air",
        info.open(api)?,
        KoneProAir,
        dev,
    ))
}

pub const XP_MODE_LEN: usize = 0xAE;
const XP_DIRECT: u8 = 0x0B;
const XP_MODES: [u8; 9] = [0x0B, 0x00, 0x01, 0x0A, 0x02, 0x03, 0x04, 0x05, 0x09];

/// roccat_kone_xp_mode_struct's defaults laid out as GetMode reads them.
pub fn kone_xp_default_mode() -> [u8; XP_MODE_LEN] {
    let mut b = [0u8; XP_MODE_LEN];
    b[0] = 0x06;
    b[3..7].copy_from_slice(&[0x06, 0x06, 0x1F, 0x01]);
    for (i, d) in [8u16, 16, 24, 32, 64, 8, 16, 24, 32, 64].iter().enumerate() {
        b[7 + 2 * i..9 + 2 * i].copy_from_slice(&d.to_le_bytes());
    }
    b[29] = 0x03;
    b[30] = 0x01;
    b[32] = 0xFF;
    b[33] = 0x0F;
    for i in 0..20 {
        b[37 + 6 * i] = 0xFF;
    }
    b[156..164].copy_from_slice(&[0x01, 0x64, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x80]);
    b
}

/// The Direct mode speed OpenRGB holds after reading the device's mode.
pub fn kone_xp_direct_speed(read: &[u8; XP_MODE_LEN]) -> u8 {
    if read[30] == XP_DIRECT || !XP_MODES.contains(&read[30]) {
        read[31]
    } else {
        0
    }
}

/// DeviceUpdateMode for Direct: GetMode's struct with mode 0x0B, brightness 0xFF and the colours, sent by SetMode.
pub fn kone_xp_set_direct(read: &[u8; XP_MODE_LEN], colors: &[Color]) -> [u8; XP_MODE_LEN] {
    let mut b = *read;
    b[0] = 0x06;
    b[1] = 0xAE;
    b[27] = (read[27] != 0) as u8;
    b[30] = XP_DIRECT;
    b[31] = kone_xp_direct_speed(read);
    b[32] = 0xFF;
    for i in 0..20 {
        let c = colors.get(i).copied().unwrap_or(Color::BLACK);
        b[36 + 6 * i..42 + 6 * i].copy_from_slice(&[0x14, 0xFF, c.0, c.1, c.2, 0x64]);
    }
    b[0xA3] = read[0xA3] & !0x80;
    let total = sum16(&b[..XP_MODE_LEN - 2]);
    b[XP_MODE_LEN - 2..].copy_from_slice(&total.to_le_bytes());
    b
}

/// SendDirect of the Kone XP.
pub fn kone_xp_direct(colors: &[Color]) -> [u8; 0x7A] {
    let mut b = [0u8; 0x7A];
    b[0] = 0x0D;
    b[1] = 0x7A;
    for (i, c) in colors.iter().take(20).enumerate() {
        b[2 + 6 * i..8 + 6 * i].copy_from_slice(&[c.0, c.1, c.2, 0xFF, 0xFF, 0xFF]);
    }
    b
}

/// WaitUntilReady: report 4 until byte 1 reads 1, at most 100 tries.
fn wait_ready(io: &mut dyn Io, len: usize, pause_ms: u64) {
    let mut buf = vec![0u8; len];
    buf[0] = 0x04;
    for i in 0..100 {
        if buf[1] == 1 {
            break;
        }
        if i != 0 {
            io.sleep(pause_ms);
        }
        let _ = io.get_feature(&mut buf);
    }
}

struct KoneXp;

impl Driver for KoneXp {
    fn start(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut prof = [0x05, 0, 0, 0];
        let profile = io.get_feature(&mut prof).map(|_| prof[2]).unwrap_or(0);
        io.send_feature(&[0x04, profile, 0x80, 0xFF])?;
        wait_ready(io, 4, 200);
        let mut read = [0u8; XP_MODE_LEN];
        read[0] = 0x06;
        if io.get_feature(&mut read).is_err() {
            read = kone_xp_default_mode();
        }
        io.send_feature(&kone_xp_set_direct(&read, colors))?;
        wait_ready(io, 4, 200);
        io.send_feature(&switch_control())?;
        wait_ready(io, 4, 200);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&kone_xp_direct(colors))
    }
}

fn open_kone_xp(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    // Left 9, right 9, wheel, DPI button.
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        20,
    );
    Ok(Hid::boxed("roccat kone xp", info.open(api)?, KoneXp, dev))
}

/// SetColor in Static mode (speed 0, no colour flow) over the profile report GetFeature returned.
pub fn kova_profile(read: &[u8; 28], wheel: Color, stripe: Color) -> [u8; 28] {
    let mut b = *read;
    b[0] = 0x06;
    b[1] = 28;
    b[14] |= 0x30 | 0x03;
    b[22..26].copy_from_slice(&[0, wheel.0, wheel.1, wheel.2]);
    b[18..22].copy_from_slice(&[0, stripe.0, stripe.1, stripe.2]);
    b[15] = 0;
    b[16] = 0x01;
    b[17] = 0;
    let sum = sum16(&b[..26]);
    b[26..].copy_from_slice(&sum.to_le_bytes());
    b
}

struct Kova;

impl Driver for Kova {
    fn start(&mut self, io: &mut dyn Io, _: &[Color]) -> Result<(), String> {
        io.send_feature(&[0x04, 0x00, 0x80])
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut read = [0u8; 28];
        read[0] = 0x06;
        io.get_feature(&mut read)?;
        io.send_feature(&kova_profile(&read, colors[0], colors[1]))
    }
}

fn open_kova(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(
        info.device_id("roccat"),
        d.name,
        DeviceKind::Mouse,
        MOUSE,
        2,
    );
    Ok(Hid::boxed("roccat kova", info.open(api)?, Kova, dev))
}

const SENSE_DIRECT: u8 = 0x0B;
const SENSE_MODES: [u8; 6] = [0x0B, 0x01, 0x0A, 0x03, 0x04, 0x09];

/// SetMode(profile, direct) of the Sense Aimo, with the speed and brightness OpenRGB took from GetMode.
pub fn sense_set_direct(read: &[u8; 19], colors: &[Color]) -> [u8; 19] {
    let (speed, bright) = if read[2] == SENSE_DIRECT || !SENSE_MODES.contains(&read[2]) {
        (read[3], read[4])
    } else {
        (0, 0)
    };
    let mut b = [0u8; 19];
    b[0] = 0x02;
    b[1] = read[1];
    for (i, c) in colors.iter().take(2).enumerate() {
        b[2 + 8 * i..10 + 8 * i].copy_from_slice(&[
            SENSE_DIRECT,
            speed,
            bright,
            0x00,
            c.0,
            c.1,
            c.2,
            0xFF,
        ]);
    }
    b
}

/// SendDirect of the Sense Aimo.
pub fn sense_direct(colors: &[Color]) -> [u8; 9] {
    let mut b = [0u8; 9];
    b[0] = 0x03;
    for (i, c) in colors.iter().take(2).enumerate() {
        b[1 + 4 * i..5 + 4 * i].copy_from_slice(&[c.0, c.1, c.2, 0xFF]);
    }
    b
}

struct SenseAimo;

impl Driver for SenseAimo {
    fn start(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut read = [0u8; 19];
        read[0] = 0x02;
        if io.get_feature(&mut read).is_err() {
            read = [0u8; 19];
            read[2] = 0x01;
            read[4] = 0xFF;
        }
        io.send_feature(&sense_set_direct(&read, colors))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.send_feature(&sense_direct(colors))
    }
}

fn open_sense_aimo(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = line(info.device_id("roccat"), d.name, DeviceKind::Other, PAD, 2);
    Ok(Hid::boxed(
        "roccat sense aimo",
        info.open(api)?,
        SenseAimo,
        dev,
    ))
}

macro_rules! ipu {
    ($name:expr, $pid:expr, $i:expr, $p:expr, $u:expr, $open:expr) => {
        Detector {
            name: $name,
            vid: VID,
            pid: $pid,
            interface: Some($i),
            usage_page: Some($p),
            usage: Some($u),
            open: $open,
        }
    };
}

pub static DETECTORS: [Detector; 14] = [
    ipu!("Roccat Horde Aimo", 0x303E, 1, 0x0B, 0, open_horde),
    ipu!("Roccat Burst Core", 0x2DE6, 3, 0xFF01, 1, open_burst_core),
    ipu!("Roccat Burst Pro", 0x2DE1, 3, 0xFF01, 1, open_burst_pro),
    ipu!(
        "Roccat Burst Pro Air",
        0x2CA6,
        0,
        0x01,
        2,
        open_burst_pro_air
    ),
    ipu!("Roccat Kone Aimo", 0x2E27, 0, 0x0B, 0, open_kone_aimo),
    ipu!("Roccat Kone Aimo 16K", 0x2E2C, 0, 0x0B, 0, open_kone_aimo),
    ipu!("Roccat Kone Pro", 0x2C88, 3, 0xFF01, 1, open_kone_pro),
    ipu!(
        "Roccat Kone Pro Air",
        0x2C8E,
        2,
        0xFF00,
        1,
        open_kone_pro_air
    ),
    ipu!(
        "Roccat Kone Pro Air (Wired)",
        0x2C92,
        1,
        0xFF13,
        1,
        open_kone_pro_air
    ),
    ipu!("Roccat Kone XP", 0x2C8B, 3, 0xFF01, 1, open_kone_xp),
    ipu!("Roccat Kova", 0x2CEE, 0, 0x0B, 0, open_kova),
    ipu!(
        "Roccat Sense Aimo Mid",
        0x343A,
        0,
        0xFF01,
        1,
        open_sense_aimo
    ),
    ipu!(
        "Roccat Sense Aimo XXL",
        0x343B,
        0,
        0xFF01,
        1,
        open_sense_aimo
    ),
    ipu!("Roccat Elo 7.1", 0x3A34, 3, 0x0C, 1, open_elo),
];

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Mock, Op, pad};
    use super::*;

    fn backend<D: Driver + 'static>(drv: D, n: usize) -> (Mock, Box<dyn Backend>) {
        let m = Mock::default();
        let dev = line("d".into(), "D", DeviceKind::Mouse, MOUSE, n);
        (m.clone(), Box::new(Hid::new("t", Box::new(m), drv, dev)))
    }

    const C1: Color = Color(0x11, 0x22, 0x33);
    const C2: Color = Color(0x44, 0x55, 0x66);

    // RoccatBurstController::SwitchControl(true) then SendDirect.
    #[test]
    fn burst_pro_takes_control_then_sends_direct() {
        let (m, mut b) = backend(Burst, 2);
        b.push("d", &[C1, C2]).unwrap();
        b.push("d", &[C1, C2]).unwrap();
        assert_eq!(
            m.take(),
            vec![
                Op::Feature(vec![0x0E, 0x06, 0x01, 0x01, 0x00, 0xFF]),
                Op::Feature(vec![
                    0x0D, 0x0B, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0, 0, 0
                ]),
            ]
        );
    }

    // RoccatBurstProAirController::SetModeValues(0x01, 0, 0xFF) and SetColors.
    #[test]
    fn burst_pro_air_reports() {
        assert_eq!(
            burst_pro_air_mode().to_vec(),
            pad(&[0x06, 0x01, 0x4C, 0x06, 0x06, 0x01, 0x00, 0xFF, 0x0F], 30)
        );
        let p = burst_pro_air_colors(&[C1, C2, Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            p.to_vec(),
            pad(
                &[
                    0x06, 0x01, 0x4C, 0x06, 0x14, 1, 0xFF, 0x11, 0x22, 0x33, 2, 0xFF, 0x44, 0x55,
                    0x66, 3, 0xFF, 1, 2, 3, 4, 0xFF, 4, 5, 6
                ],
                30
            )
        );
    }

    // RoccatEloController::SendInit, which includes SendDirect(0), then SendDirect.
    #[test]
    fn elo_init_and_direct() {
        let (m, mut b) = backend(Elo, 1);
        b.push("d", &[C1]).unwrap();
        let w = |h: &[u8]| Op::Write(pad(h, 16));
        assert_eq!(
            m.take(),
            vec![
                w(&[0xFF, 0x01]),
                w(&[0xFF, 0x02]),
                w(&[0xFF, 0x03, 0x00, 0x01]),
                w(&[0xFF, 0x04]),
                w(&[0xFF, 0x01]),
                w(&[0xFF, 0x04, 0, 0, 0x11, 0x22, 0x33]),
            ]
        );
    }

    // RoccatHordeAimoController::InitialPacket and SetColors with its additive checksum.
    #[test]
    fn horde_reports() {
        assert_eq!(horde_init(), [0x13, 0x08, 0x01, 0, 0, 0, 0, 0]);
        let p = horde_colors(&[Color(0xFF, 0xFF, 0xFF); 6]);
        assert_eq!(p[..7], [0x18, 0x18, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        // 0x18 + 0x18 + 20 * 0xFF = 0x141C
        assert_eq!(p[22..], [0x1C, 0x14]);
        let (m, mut b) = backend(Horde, 6);
        b.push("d", &[C1; 6]).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::Feature(horde_init().to_vec()));
        assert_eq!(ops[1], Op::Feature(horde_colors(&[C1; 6]).to_vec()));
        assert_eq!(ops[2], Op::GetFeature(vec![0x18, 0, 0]));
    }

    // RoccatKoneAimoController::SendInit and SetChannelColors + SendUpdate.
    #[test]
    fn kone_aimo_init_and_frame() {
        let (m, mut b) = backend(KoneAimo, 11);
        let mut c = vec![Color::BLACK; 11];
        c[0] = C1;
        c[10] = C2;
        b.push("d", &c).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::GetFeature(vec![0, 0, 0]));
        assert_eq!(ops[1], Op::Feature(switch_control().to_vec()));
        let mut want = pad(&[0x0D, 0x2E, 0x11, 0x22, 0x33], 46);
        want[42..45].copy_from_slice(&[0x44, 0x55, 0x66]);
        assert_eq!(ops[2], Op::Feature(want));
        assert_eq!(ops[3], Op::GetFeature(vec![0x0D, 0, 0]));
    }

    // RoccatKoneProAirController::SendDirect -> SendRGB(true, ...).
    #[test]
    fn kone_pro_air_direct_frame() {
        assert_eq!(
            kone_pro_air_direct(&[C1, C2]),
            [
                0x00, 0x10, 0x10, 0x0B, 0x00, 0x09, 0x64, 0x64, 0x64, 0x0B, 0x11, 0x22, 0x33, 0x44,
                0x55, 0x66
            ]
        );
    }

    // RGBController_RoccatKoneXP::DeviceUpdateMode(Direct) via GetMode/SetMode, then SendDirect.
    #[test]
    fn kone_xp_direct_setup() {
        let mut read = kone_xp_default_mode();
        read[2] = 3;
        read[27] = 7;
        read[30] = 0x0B;
        read[31] = 5;
        read[0xA3] = 0x85;
        let mut colors = vec![Color::BLACK; 20];
        colors[0] = C1;
        let b = kone_xp_set_direct(&read, &colors);
        assert_eq!(b[..8], [0x06, 0xAE, 3, 0x06, 0x06, 0x1F, 0x01, 8]);
        assert_eq!(b[27..36], [1, 0, 3, 0x0B, 5, 0xFF, 0x0F, 0, 0]);
        assert_eq!(b[36..42], [0x14, 0xFF, 0x11, 0x22, 0x33, 0x64]);
        assert_eq!(b[42..48], [0x14, 0xFF, 0, 0, 0, 0x64]);
        assert_eq!(b[156..165], [0x01, 0x64, 0xFF, 0xFF, 0, 0, 0, 0x05, 0]);
        let sum = b[..172].iter().map(|&x| x as u32).sum::<u32>() as u16;
        assert_eq!(b[172..], sum.to_le_bytes());
        // a device in Static mode leaves Direct at speed 0
        read[30] = 0x01;
        assert_eq!(kone_xp_set_direct(&read, &colors)[31], 0);

        let d = kone_xp_direct(&colors);
        assert_eq!(d[..8], [0x0D, 0x7A, 0x11, 0x22, 0x33, 0xFF, 0xFF, 0xFF]);
        assert_eq!(d[8..14], [0, 0, 0, 0xFF, 0xFF, 0xFF]);
        assert_eq!(d[116..], [0, 0, 0, 0xFF, 0xFF, 0xFF]);
    }

    // RGBController_RoccatKoneXP constructor + DeviceUpdateMode: profile, wait, mode, wait, EnableDirect, wait.
    #[test]
    fn kone_xp_setup_sequence() {
        let (m, mut b) = backend(KoneXp, 20);
        m.reply(vec![0x05, 0x04, 0x02, 0]);
        m.reply(vec![0x04, 0x01, 0, 0]);
        m.reply(kone_xp_default_mode().to_vec());
        m.reply(vec![0x04, 0x00, 0, 0]);
        m.reply(vec![0x04, 0x01, 0, 0]);
        m.reply(vec![0x04, 0x01, 0, 0]);
        b.push("d", &[C1; 20]).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::GetFeature(vec![0x05, 0, 0, 0]));
        assert_eq!(ops[1], Op::Feature(vec![0x04, 0x02, 0x80, 0xFF]));
        assert_eq!(ops[2], Op::GetFeature(vec![0x04, 0, 0, 0]));
        assert_eq!(ops[3], Op::GetFeature(pad(&[0x06], XP_MODE_LEN)));
        assert_eq!(
            ops[4],
            Op::Feature(kone_xp_set_direct(&kone_xp_default_mode(), &[C1; 20]).to_vec())
        );
        assert_eq!(ops[5], Op::GetFeature(vec![0x04, 0, 0, 0]));
        assert_eq!(ops[6], Op::Sleep(200));
        assert_eq!(ops[7], Op::GetFeature(vec![0x04, 0, 0, 0]));
        assert_eq!(ops[8], Op::Feature(switch_control().to_vec()));
        assert_eq!(ops[9], Op::GetFeature(vec![0x04, 0, 0, 0]));
        assert_eq!(ops[10], Op::Feature(kone_xp_direct(&[C1; 20]).to_vec()));
        assert_eq!(ops.len(), 11);
    }

    // RoccatKovaController::SendInitialPacket and SetColor(Static) over FetchProfileData.
    #[test]
    fn kova_reports() {
        let (m, mut b) = backend(Kova, 2);
        let mut read = pad(&[0x06, 28], 28);
        read[14] = 0x40;
        read[16] = 0x03;
        read[17] = 2;
        m.reply(read.clone());
        b.push("d", &[C1, C2]).unwrap();
        let ops = m.take();
        assert_eq!(ops[0], Op::Feature(vec![0x04, 0x00, 0x80]));
        assert_eq!(ops[1], Op::GetFeature(pad(&[0x06], 28)));
        let mut want = read;
        want[14] = 0x73;
        want[15..26].copy_from_slice(&[0, 1, 0, 0, 0x44, 0x55, 0x66, 0, 0x11, 0x22, 0x33]);
        let sum = want[..26].iter().map(|&x| x as u16).sum::<u16>();
        want[26..].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(ops[2], Op::Feature(want));
    }

    // RGBController_RoccatSenseAimo::DeviceUpdateMode(Direct) and RoccatSenseAimoController::SendDirect.
    #[test]
    fn sense_aimo_reports() {
        let mut read = [0u8; 19];
        read[..5].copy_from_slice(&[0x02, 0x03, 0x0B, 0x07, 0x80]);
        assert_eq!(
            sense_set_direct(&read, &[C1, C2]).to_vec(),
            vec![
                0x02, 0x03, 0x0B, 0x07, 0x80, 0x00, 0x11, 0x22, 0x33, 0xFF, 0x0B, 0x07, 0x80, 0x00,
                0x44, 0x55, 0x66, 0xFF, 0x00
            ]
        );
        read[2] = 0x01;
        assert_eq!(sense_set_direct(&read, &[C1, C2])[2..5], [0x0B, 0, 0]);
        assert_eq!(
            sense_direct(&[C1, C2]),
            [0x03, 0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF]
        );
    }

    #[test]
    fn detectors_match_the_registered_collections() {
        let d = &DETECTORS[3];
        assert_eq!((d.vid, d.pid), (0x1E7D, 0x2CA6));
        assert_eq!(
            (d.interface, d.usage_page, d.usage),
            (Some(0), Some(1), Some(2))
        );
        assert_eq!(DETECTORS[13].usage_page, Some(0x0C));
    }

    // RoccatKoneAimoController::SendInit ignores the result of its first read.
    #[test]
    fn kone_aimo_failed_read_still_takes_control() {
        let (m, mut b) = backend(KoneAimo, 11);
        m.reply(Vec::new());
        m.reply(Vec::new());
        b.push("d", &[C1; 11]).unwrap();
        let ops = m.take();
        assert_eq!(ops[1], Op::Feature(switch_control().to_vec()));
        assert_eq!(ops.len(), 4);
    }
}
