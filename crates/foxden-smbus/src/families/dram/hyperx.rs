//! Ported from OpenRGB Controllers/HyperXDRAMController (GPL-2.0-or-later).

use foxden_core::{Backend, Color};

use super::common::{Driver, Io, Proto, delay, stick};
use super::spd::Slot;
use crate::i2c::SharedBus;

const DEV: u8 = 0x27;
const REG_APPLY: u8 = 0xE1;
const REG_MODE_INDEPENDENT: u8 = 0xE5;
const MODE3_DIRECT: u8 = 0x21;
const SLOT_BASE: [u8; 4] = [0x11, 0x41, 0x71, 0xA1];
const SLOT_MAP: [u8; 4] = [0, 2, 1, 3];

/// DetectHyperXDRAMControllers
pub fn detect(
    bus: &SharedBus,
    io: &Io,
    slots: &mut [&mut Slot],
    _name: &'static str,
) -> Vec<Box<dyn Backend>> {
    if io.read_byte(DEV) < 0 {
        return Vec::new();
    }
    let (mut valid, mut fury, mut pred) = (0u8, false, false);
    for slot in slots.iter_mut() {
        valid |= 1 << slot.index();
        if slot.manufacturer_data(io, 0x06) == 0x01 {
            fury = true;
        } else {
            pred = true;
        }
        delay(1);
    }
    if valid == 0 {
        return Vec::new();
    }
    let name = match (fury, pred) {
        (true, false) => "HyperX Fury RGB",
        (false, true) => "HyperX Predator RGB",
        _ => "HyperX DRAM",
    };
    let hx = HyperX { valid };
    let devices: Vec<_> = (0..hx.slot_count())
        .map(|z| {
            let slot = hx.slot_of(z);
            stick(
                format!("smbus:hyperx-{DEV:02x}-{slot}"),
                format!("{name}, HyperX Slot {}", z + 1),
                slot as usize,
                5,
            )
        })
        .collect();
    if devices.is_empty() {
        return Vec::new();
    }
    vec![Box::new(Driver::new(bus, "hyperx dram", devices, hx))]
}

pub struct HyperX {
    valid: u8,
}

impl HyperX {
    /// GetSlotCount: only the first four slots carry LEDs.
    fn slot_count(&self) -> usize {
        (0..4).filter(|s| self.valid & (1 << s) != 0).count()
    }

    /// The slot the C++ SetLEDColor(led) loop lands on for zone `led_slot`.
    fn slot_of(&self, led_slot: usize) -> u8 {
        let mut slot_id = -1i32;
        let mut slot = 0;
        for &s in &SLOT_MAP {
            slot = s;
            if self.valid & (1 << s) != 0 {
                slot_id += 1;
            }
            if slot_id == led_slot as i32 {
                break;
            }
        }
        slot
    }

    fn set_led_color(&self, io: &Io, slot: u8, led: u8, c: Color) {
        let base = SLOT_BASE[slot as usize] + 3 * led;
        io.write_byte_data(DEV, REG_APPLY, 0x01);
        io.write_byte_data(DEV, base, c.0);
        io.write_byte_data(DEV, base + 1, c.1);
        io.write_byte_data(DEV, base + 2, c.2);
        io.write_byte_data(DEV, base + 0x10, 0x64);
    }

    fn send_apply(&self, io: &Io) {
        io.write_byte_data(DEV, REG_APPLY, 0x02);
        io.write_byte_data(DEV, REG_APPLY, 0x03);
    }
}

impl Proto for HyperX {
    /// SetMode(HYPERX_MODE_DIRECT)
    fn start(&mut self, io: &Io, _zones: &[Vec<Color>]) {
        io.write_byte_data(DEV, REG_APPLY, 0x01);
        io.write_byte_data(DEV, REG_MODE_INDEPENDENT, MODE3_DIRECT);
        self.send_apply(io);
    }

    /// DeviceUpdateZoneLEDs
    fn frame(&mut self, io: &Io, zones: &[Vec<Color>], zone: usize) {
        let slot = self.slot_of(zone);
        for (led, &c) in zones[zone].iter().enumerate() {
            self.set_led_color(io, slot, led as u8, c);
        }
        self.send_apply(io);
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Op::*, *};
    use super::super::spd;
    use super::*;

    fn detect_on(used: &[(u8, u8)]) -> (Vec<Op>, Vec<Box<dyn Backend>>) {
        let mut sim = Sim::default();
        for &(a, kind) in used {
            let mut e = ddr4_spd([0x01, 0x98], &[]);
            e[0x67] = kind;
            sim.ddr4.insert(a, e);
        }
        sim.present.insert(DEV);
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (mut slots, _) = spd::scan(&io);
        take();
        let mut refs: Vec<&mut Slot> = slots.iter_mut().collect();
        let found = detect(&bus, &io, &mut refs, "HyperX DRAM");
        (take(), found)
    }

    // DetectHyperXDRAMControllers: read_byte 0x27, manufacturer byte 6 per slot, 1 ms apart
    #[test]
    fn detection() {
        let (ops, found) = detect_on(&[(0x50, 1), (0x51, 1)]);
        assert_eq!(
            ops,
            vec![
                Rb(0x27),
                Rbd(0x50, 0x67),
                Sleep(1),
                Sleep(1),
                Rbd(0x51, 0x67),
                Sleep(1),
                Sleep(1),
            ]
        );
        let devs = found[0].devices();
        assert_eq!(devs[0].name, "HyperX Fury RGB, HyperX Slot 1");
        assert_eq!(devs[1].id, "smbus:hyperx-27-1");
        let (_, found) = detect_on(&[(0x50, 1), (0x52, 0)]);
        assert!(found[0].devices()[0].name.starts_with("HyperX DRAM,"));
    }

    // SetMode(Direct), then DeviceUpdateZoneLEDs: SetLEDColor per LED, then SendApply
    #[test]
    fn direct_frame_uses_slot_map() {
        let (_, mut found) = detect_on(&[(0x50, 0), (0x51, 0), (0x52, 0)]);
        let devs = found[0].devices();
        assert_eq!(
            devs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            [
                "smbus:hyperx-27-0",
                "smbus:hyperx-27-2",
                "smbus:hyperx-27-1"
            ]
        );
        let c = vec![Color(1, 2, 3); 5];
        found[0].push("smbus:hyperx-27-2", &c).unwrap();
        let mut want = vec![
            Wbd(0x27, 0xE1, 0x01),
            Wbd(0x27, 0xE5, 0x21),
            Wbd(0x27, 0xE1, 0x02),
            Wbd(0x27, 0xE1, 0x03),
        ];
        for led in 0..5u8 {
            let b = 0x71 + 3 * led;
            want.extend([
                Wbd(0x27, 0xE1, 0x01),
                Wbd(0x27, b, 1),
                Wbd(0x27, b + 1, 2),
                Wbd(0x27, b + 2, 3),
                Wbd(0x27, b + 0x10, 0x64),
            ]);
        }
        want.extend([Wbd(0x27, 0xE1, 0x02), Wbd(0x27, 0xE1, 0x03)]);
        assert_eq!(take(), want);
        found[0].push("smbus:hyperx-27-2", &c).unwrap();
        assert!(take().is_empty());
        found[0].push("smbus:hyperx-27-0", &c).unwrap();
        assert_eq!(take()[1], Wbd(0x27, 0x11, 1));
    }
}
