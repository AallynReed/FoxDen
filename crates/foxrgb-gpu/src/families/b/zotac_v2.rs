//! Ported from OpenRGB Controllers/ZotacV2GPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::ZOTAC_V2;
use super::common::{Driver, Gpu, Io, c_string, detect_each, device_id, row};

const REG_RGB: u8 = 0xA0;
const CONFIG_IDLE: u8 = 0x00;
const CONFIG_ACTIVE: u8 = 0x01;
const SYNC_SYNCHRONIZED: u8 = 0x01;
const MODE_STATIC: u8 = 0x00;
const DIR_RIGHT: u8 = 0x01;
const MODE_FLASH: u8 = 0x04;
/// Effect modes RGBController_ZotacV2GPU registers besides Static and Flash.
const EFFECTS: [u8; 10] = [0x01, 0x02, 0x03, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x17, 0x18];

/// ZOTAC_V2_GPU_CONFIG: controller version, zone count, external LED strip.
const CONFIGS: [(&str, u8, bool); 12] = [
    ("N653E-1013", 2, false),
    ("N653A-1013", 1, false),
    ("N612E-1011", 2, false),
    ("N612A-1012", 2, false),
    ("N617E-1011", 3, false),
    ("N618E-1013", 4, true),
    ("N618A-1015", 4, true),
    ("N696E-1040", 1, false),
    ("N675E-1019", 1, true),
    ("N675E-1062", 1, true),
    ("N675A-1019", 5, true),
    ("N675A-1062", 5, true),
];

/// ZotacV2GPUController::ReadVersion
fn version(io: &Io, addr: u8) -> String {
    if !io.raw_write(addr, &[REG_RGB, 0xF1, 0, 0, 0, 0, 0, 0]) {
        return String::new();
    }
    io.raw_read(addr, 32)
        .map(|d| c_string(&d))
        .unwrap_or_default()
}

/// SetupInitialValues: Static takes the card's speed, brightness and direction unless it is in an
/// effect the controller lists, since active_mode starts on Static.
fn initial(io: &Io, addr: u8, zone: u8, zones: u8) -> (u8, u8, u8) {
    let mut out = (0, 100, 0);
    if !io.raw_write(addr, &[REG_RGB, 0xF0, 0, 0, 0, CONFIG_IDLE, zone, 0]) {
        return out;
    }
    if io.raw_read(addr, 32).is_none() {
        return out;
    }
    let Some(d) = io.raw_read(addr, 32) else {
        return out;
    };
    let effect = EFFECTS.contains(&d[6]) || (d[6] == MODE_FLASH && zones > 1);
    if d[5] == zone && d[4] == CONFIG_IDLE && !effect {
        out = (d[10], d[11], d[12]);
    }
    out
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, ZOTAC_V2, |bus, card| {
        let io = Io::new(bus);
        io.raw_read(card.addr, 32)?;
        let v = version(&io, card.addr);
        let &(_, zones, external) = CONFIGS.iter().find(|c| c.0 == v)?;
        // FindSynchronizedZoneNum(ZOTAC_V2_GPU_SYNC_SYNCHRONIZED)
        let zone = zones + external as u8;
        let (speed, brightness, direction) = initial(&io, card.addr, zone, zones);
        let dev = row(device_id("zotac-v2", bus, card.addr), card.name, 1);
        let driver = V2 {
            addr: card.addr,
            zone,
            speed,
            brightness,
            direction,
        };
        Some(Gpu::boxed(bus, dev, driver))
    })
}

struct V2 {
    addr: u8,
    zone: u8,
    speed: u8,
    brightness: u8,
    direction: u8,
}

impl V2 {
    // ZotacV2GPUController::SendCommand
    fn send(&self, io: &Io, idle_active: u8, zone: u8, sync: u8, cfg: [u8; 7]) {
        let [mode, r, g, b, speed, brightness, direction] = cfg;
        let pkt = [
            REG_RGB,
            0x01,
            0x00,
            0x00,
            0x00,
            idle_active,
            zone,
            mode,
            r,
            g,
            b,
            speed,
            brightness,
            direction,
            0x00,
            sync,
            0x00,
            0,
            0,
            0,
            0,
        ];
        io.raw_write(self.addr, &pkt);
    }
}

impl Driver for V2 {
    fn setup(&mut self, _io: &Io, _colors: &[Color]) {}

    // RGBController_ZotacV2GPU::DeviceUpdateMode in Static: on, then idle and active configs
    fn update(&mut self, io: &Io, colors: &[Color], _prev: Option<&[Color]>) {
        let Color(r, g, b) = colors[0];
        self.send(io, 0, 0, 0, [0; 7]);
        let direction = (self.direction == DIR_RIGHT) as u8;
        let cfg = [MODE_STATIC, r, g, b, self.speed, self.brightness, direction];
        self.send(io, CONFIG_IDLE, self.zone, SYNC_SYNCHRONIZED, cfg);
        self.send(io, CONFIG_ACTIVE, self.zone, SYNC_SYNCHRONIZED, cfg);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const CARD: &str = "ZOTAC GAMING GeForce RTX 3090 AMP Extreme Holo";

    fn version_reply() -> Vec<u8> {
        let mut v = b"N618A-1015".to_vec();
        v.resize(32, 0);
        v
    }

    // DetectZotacV2GPUControllers, ReadVersion, then GetMode for the synchronized zone (4 + 1).
    #[test]
    fn detect_version_and_initial() {
        let mut state = vec![0u8; 32];
        state[4] = 0;
        state[5] = 5;
        state[6] = MODE_STATIC;
        state[10] = 3;
        state[11] = 50;
        state[12] = 1;
        let v = version_reply();
        let bus = Fake::new("NV", ids(ZOTAC_V2, CARD))
            .raw(0x49, &[&v, &v, &v, &state])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            take(),
            s(&[
                "rraw 49 32",
                "wraw 49 a0 f1 00 00 00 00 00 00",
                "rraw 49 32",
                "wraw 49 a0 f0 00 00 00 00 05 00",
                "rraw 49 32",
                "rraw 49 32",
            ])
        );
        let tail = "00 00 00 00";
        assert_eq!(
            push(&mut b, &[Color(0x11, 0x22, 0x33)]),
            vec![
                format!("wraw 49 a0 01{}", " 00".repeat(19)),
                format!("wraw 49 a0 01 00 00 00 00 05 00 11 22 33 03 32 01 00 01 00 {tail}"),
                format!("wraw 49 a0 01 00 00 00 01 05 00 11 22 33 03 32 01 00 01 00 {tail}"),
            ]
        );
    }

    // Shine is not registered, so Static takes the card's values as it does in static.
    #[test]
    fn unlisted_mode_values() {
        let v = version_reply();
        let mut state = vec![0u8; 32];
        (state[5], state[6], state[10], state[11]) = (5, 0x05, 2, 40);
        let bus = Fake::new("NV", ids(ZOTAC_V2, CARD))
            .raw(0x49, &[&v, &v, &v, &state])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)])[1],
            "wraw 49 a0 01 00 00 00 00 05 00 01 02 03 02 28 00 00 01 00 00 00 00 00"
        );
    }

    // Static defaults when the card is in a listed effect: speed 0, brightness 100, left.
    #[test]
    fn static_defaults() {
        let v = version_reply();
        let mut state = vec![0u8; 32];
        (state[5], state[6], state[10], state[11]) = (5, 0x08, 2, 40);
        let bus = Fake::new("NV", ids(ZOTAC_V2, CARD))
            .raw(0x49, &[&v, &v, &v, &state])
            .shared();
        let mut b = run(&[super::detect], &bus).remove(0);
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)])[1],
            "wraw 49 a0 01 00 00 00 00 05 00 01 02 03 00 64 00 00 01 00 00 00 00 00"
        );
    }

    // An unknown controller version is not registered.
    #[test]
    fn unknown_version() {
        let bus = Fake::new("NV", ids(ZOTAC_V2, CARD))
            .raw(0x49, &[b"N999X-0000\0"])
            .shared();
        assert!(run(&[super::detect], &bus).is_empty());
    }
}
