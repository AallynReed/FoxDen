// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color};

use super::common::{Driver, Io, Proto, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const DEV: u8 = 0x77;
const REG_LED0_DIRECT_COLOR: u8 = 0x17;
const SIGNATURE: [u8; 8] = [0x50, 0x44, 0x41, 0x31, 0, 0, 0, 0];

/// DetectPatriotViperSteelControllers
pub fn detect(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    _name: &'static str,
) -> Vec<Box<dyn Backend>> {
    if io.quick(DEV) < 0 {
        return Vec::new();
    }
    let mut valid = 0u8;
    for slot in slots.iter_mut() {
        if (0..8).all(|i| slot.manufacturer_data(io, i) == SIGNATURE[i as usize]) {
            valid |= 1 << slot.index();
        }
    }
    if valid == 0 {
        return Vec::new();
    }
    let device = stick(
        format!("smbus:patriot-viper-steel-{DEV:02x}"),
        "Patriot Viper Steel RGB".into(),
        valid.trailing_zeros() as usize,
        5,
    );
    vec![Box::new(Driver::new(
        bus,
        "patriot viper steel",
        vec![device],
        Steel,
    ))]
}

pub struct Steel;

impl Proto for Steel {
    /// DeviceUpdateMode does nothing.
    fn start(&mut self, _io: &Io, _zones: &[Vec<Color>]) {}

    /// DeviceUpdateLEDs: ViperRegisterWrite(0x17 + led, r, b, g) per LED.
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], _zone: usize) {
        for (led, c) in zones[0].iter().enumerate() {
            io.write_byte_data(DEV, REG_LED0_DIRECT_COLOR + led as u8, c.0);
            io.write_byte_data(DEV, c.1, c.2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    // DetectPatriotViperSteelControllers on a blank-JEDEC (0xFF7E) module, then one frame
    #[test]
    fn detection_and_frame() {
        let mut sim = Sim::default();
        let mut e = ddr4_spd([0xFF, 0xFF], &SIGNATURE);
        e[0x61..0x69].copy_from_slice(&SIGNATURE);
        sim.ddr4.insert(0x52, e);
        sim.present.insert(DEV);
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        assert_eq!(slots[0].jedec, 0xFF7E);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let mut found = detect(&bus, &io, &mut refs, "Patriot Viper Steel");
        let mut want = vec![Quick(0x77)];
        for i in 0..8 {
            want.extend([Rbd(0x52, 0x61 + i), Sleep(1)]);
        }
        assert_eq!(take(), want);
        assert_eq!(found[0].devices()[0].rect[0], 0.62 + 2.0 * 0.035);

        let colors: Vec<Color> = (0..5).map(|i| Color(i, 10 + i, 20 + i)).collect();
        found[0]
            .push("smbus:patriot-viper-steel-77", &colors)
            .unwrap();
        let mut want = Vec::new();
        for i in 0..5u8 {
            want.extend([Wbd(0x77, 0x17 + i, i), Wbd(0x77, 10 + i, 20 + i)]);
        }
        assert_eq!(take(), want);
    }
}
