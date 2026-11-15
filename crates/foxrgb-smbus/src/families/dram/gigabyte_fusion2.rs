//! Ported from OpenRGB Controllers/GigabyteRGBFusion2DRAMController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color};

use super::common::{Driver, Io, Proto, is_dram_smbus, stick};
use crate::i2c::{SharedBus, lock};

const DEV: u8 = 0x67;
const LED_COUNT: usize = 6;
const IDX_MODE: usize = 0x09;
const IDX_BRIGHTNESS: usize = 0x0A;
const IDX_BLUE: usize = 0x0C;
const IDX_GREEN: usize = 0x0D;
const IDX_RED: usize = 0x0E;
const LED_START_ADDR: u8 = 0x20;
const APPLY_ADDR: u8 = 0x28;
const ACTION_APPLY: u8 = 0x0F;
const MODE_PULSE: u8 = 0x02;
/// The Direct mode's default brightness.
const BRIGHTNESS: u8 = 100;

/// TestForGigabyteRGBFusion2DRAMController
fn test(io: &Io, a: u8) -> bool {
    if io.quick(a) < 0 {
        return false;
    }
    io.write_byte_data(a, 0xE1, 0x01);
    io.read_word_data(a, 0xED) == 0x3282 && io.read_word_data(a, 0xEB) == 0x0800
}

/// DetectGigabyteRGBFusion2DRAMControllers for one bus.
pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    if !is_dram_smbus(lock(bus).info()) || !test(&Io::new(bus), DEV) {
        return Vec::new();
    }
    let device = stick(
        format!("smbus:fusion2-dram-{DEV:02x}"),
        "RGB Fusion 2 DRAM".into(),
        0,
        LED_COUNT,
    );
    let fusion = Fusion {
        led_data: [0; 32],
        direct_initialized: false,
    };
    vec![Box::new(Driver::new(
        bus,
        "rgb fusion 2 dram",
        vec![device],
        fusion,
    ))]
}

/// RGBFusion2DRAMController, whose LED buffer persists between calls.
pub struct Fusion {
    led_data: [u8; 32],
    direct_initialized: bool,
}

impl Fusion {
    /// SetLEDEffect in Direct mode: pulse mode set up never to pulse.
    fn set_led_direct(&mut self, io: &Io, led: usize, c: Color) {
        let d = &mut self.led_data;
        d[0] = 1 << led;
        d[16] = 1;
        d[22] = 2;
        d[29] = 1;
        d[30] = 1;
        let truncate = self.direct_initialized;
        self.direct_initialized = true;
        d[IDX_MODE] = MODE_PULSE;
        d[IDX_BRIGHTNESS] = BRIGHTNESS;
        d[IDX_RED] = c.0;
        d[IDX_GREEN] = c.1;
        d[IDX_BLUE] = c.2;
        let len = if truncate { 15 } else { 32 };
        io.write_block_data(DEV, LED_START_ADDR, &d[..len]);
        io.write_byte_data(DEV, APPLY_ADDR, ACTION_APPLY);
    }
}

impl Proto for Fusion {
    const START_SENDS_FRAME: bool = true;

    /// DeviceUpdateMode is DeviceUpdateLEDs.
    fn start(&mut self, io: &Io, zones: &[Vec<Color>]) {
        self.frame(io, zones, 0);
    }

    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        for (led, &c) in zones[0].iter().enumerate() {
            self.set_led_direct(io, led, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::*;

    fn sim() -> Sim {
        let mut sim = Sim::default();
        sim.present.insert(DEV);
        sim.words.insert((DEV, 0xED), 0x3282);
        sim.words.insert((DEV, 0xEB), 0x0800);
        sim
    }

    // DetectGigabyteRGBFusion2DRAMControllers: quick, E1=01, then words 0xED and 0xEB
    #[test]
    fn detection() {
        let found = detect(&bus(sim()));
        assert_eq!(
            take(),
            vec![
                Quick(0x67),
                Wbd(0x67, 0xE1, 0x01),
                Rw(0x67, 0xED),
                Rw(0x67, 0xEB)
            ]
        );
        assert_eq!(found[0].devices()[0].leds.len(), 6);
        let mut s = sim();
        s.words.insert((DEV, 0xED), 0x3283);
        assert!(detect(&bus(s)).is_empty());
        assert_eq!(take().len(), 3);
    }

    fn packet(led: usize, c: Color, len: usize) -> Vec<u8> {
        let mut d = [0u8; 32];
        d[0] = 1 << led;
        d[9] = 2;
        d[10] = 100;
        d[12] = c.2;
        d[13] = c.1;
        d[14] = c.0;
        d[16] = 1;
        d[22] = 2;
        d[29] = 1;
        d[30] = 1;
        d[..len].to_vec()
    }

    // SetLEDEffect(Direct) per LED: only the very first packet is 32 bytes
    #[test]
    fn direct_frames() {
        let mut found = detect(&bus(sim()));
        take();
        let colors: Vec<Color> = (0..6).map(|i| Color(i, 10 + i, 20 + i)).collect();
        found[0].push("smbus:fusion2-dram-67", &colors).unwrap();
        let mut want = Vec::new();
        for (led, &c) in colors.iter().enumerate() {
            let len = if led == 0 { 32 } else { 15 };
            want.extend([Blk(0x67, 0x20, packet(led, c, len)), Wbd(0x67, 0x28, 0x0F)]);
        }
        assert_eq!(take(), want);
        let next = vec![Color(9, 9, 9); 6];
        found[0].push("smbus:fusion2-dram-67", &next).unwrap();
        let ops = take();
        assert_eq!(ops.len(), 12);
        assert_eq!(ops[0], Blk(0x67, 0x20, packet(0, Color(9, 9, 9), 15)));
    }
}
