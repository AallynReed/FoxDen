// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use crate::lights::{Dmi, Driver, Light, OpenSio, SIO_ID_MASK, Sio};

const SIO_ITE8620_ID: u16 = 0x8620;
const SIO_ITE8688_ID: u16 = 0x8688;
const EXTENDED_FUNCTION_ENTRY: [u8; 4] = [0x87, 0x01, 0x55, 0x55];
const CHIPSELECT_REGISTER_1: u8 = 0x07;
const CHIPSELECT_VALUE_1: u8 = 0x00;
const CHIPEXIT_REGISTER_1: u8 = 0x02;
const CHIPEXIT_VALUE_1: u8 = 0x01;
const RED_REGISTER: [u8; 2] = [0xB3, 0xC3];
const GREEN_REGISTER: [u8; 2] = [0xB4, 0xC4];
const BLUE_REGISTER: [u8; 2] = [0xB5, 0xC5];
/// GIGABYTE_MODE1_STATIC, which Direct's value 0 selects.
const STATIC: [(u8, u8); 6] = [
    (0xB0, 0x0F),
    (0xB1, 0x00),
    (0xB2, 0x20),
    (0xC0, 0x0F),
    (0xC1, 0x00),
    (0xC2, 0x10),
];

const COMPATIBLE_DEVICES: [&str; 2] = ["X570 UD", "Z390 DESIGNARE-CF"];

/// GigabyteSuperIOEnter: the ITE8620 Ambient LED extended function mode key.
fn enter(sio: &Sio) {
    sio.enter();
    for key in EXTENDED_FUNCTION_ENTRY {
        sio.inb(key);
    }
}

struct GigabyteSio {
    sio: Sio,
}

impl GigabyteSio {
    fn chip_entry(&self) {
        enter(&self.sio);
        self.sio.outb(CHIPSELECT_REGISTER_1, CHIPSELECT_VALUE_1);
    }

    fn chip_exit(&self) {
        self.sio.outb(CHIPEXIT_REGISTER_1, CHIPEXIT_VALUE_1);
    }

    fn set_mode_static(&self) {
        self.chip_entry();
        for (reg, val) in STATIC {
            self.sio.outb(reg, val);
        }
        self.chip_exit();
    }

    fn set_color(&self, c: Color) {
        self.chip_entry();
        for i in 0..2 {
            self.sio.outb(RED_REGISTER[i], c.0);
            self.sio.outb(GREEN_REGISTER[i], c.1);
            self.sio.outb(BLUE_REGISTER[i], c.2);
        }
        self.chip_exit();
    }
}

impl Driver for GigabyteSio {
    fn write(&mut self, colors: &[Color], first: bool) {
        self.sio.locked(|| {
            if first {
                self.set_mode_static();
            }
            self.set_color(colors[0]);
        });
    }
}

/// Detection. The board list is checked before PawnIO is loaded.
pub(crate) fn detect(dmi: &Dmi, open: OpenSio) -> Vec<Box<dyn Backend>> {
    if dmi.manufacturer != "Gigabyte Technology Co., Ltd."
        || !COMPATIBLE_DEVICES
            .iter()
            .any(|name| dmi.mainboard.contains(name))
    {
        return Vec::new();
    }
    let mut found = Vec::new();
    for addr in [0x2E, 0x4E] {
        let Some(sio) = open(addr) else { continue };
        let id = sio.locked(|| {
            enter(&sio);
            sio.devid()
        });
        if matches!(id & SIO_ID_MASK, SIO_ITE8620_ID | SIO_ITE8688_ID) {
            let device = Device::grid(
                &format!("superio:gigabyte-{addr:02x}"),
                &format!("Gigabyte {}", dmi.mainboard),
                DeviceKind::Motherboard,
                [0.06, 0.12, 0.52, 0.05],
                1,
                1,
            );
            found.push(Light::boxed(
                "gigabyte-superio",
                device,
                GigabyteSio { sio },
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lights::fake::{self, Log};

    const ENTER: [&str; 7] = [
        "select_slot 00",
        "superio_inb 87",
        "superio_inb 87",
        "superio_inb 87",
        "superio_inb 01",
        "superio_inb 55",
        "superio_inb 55",
    ];

    fn run(board: &str, id: u16) -> (Log, Vec<Box<dyn Backend>>) {
        let log = Log::default();
        let open = fake::chips(&log, &[(0x2E, id)], &[]);
        let dmi = fake::dmi("Gigabyte Technology Co., Ltd.", board, "");
        let found = detect(&dmi, &open);
        (log, found)
    }

    #[test]
    fn detection() {
        let (log, found) = run("X570 UD", 0x8688);
        assert_eq!(found.len(), 1);
        let mut want: Vec<&str> = ENTER.to_vec();
        want.extend(["superio_inb 20", "superio_inb 21"]);
        want.extend(["select_slot 01"]);
        want.extend(&ENTER[1..]);
        want.extend(["superio_inb 20", "superio_inb 21"]);
        assert_eq!(fake::take(&log), want);

        let (_, found) = run("X570 AORUS ELITE", 0x8688);
        assert!(found.is_empty());
        let (_, found) = run("Z390 DESIGNARE-CF", 0x8622);
        assert_eq!(found.len(), 1);
        let (_, found) = run("X570 UD", 0x8686);
        assert!(found.is_empty());
    }

    // Direct (static) mode then each LED colour.
    #[test]
    fn direct_frame() {
        let (log, mut found) = run("X570 UD", 0x8688);
        fake::take(&log);
        let id = found[0].devices()[0].id.clone();
        found[0].push(&id, &[Color(1, 2, 3)]).unwrap();
        let mut want: Vec<&str> = ENTER.to_vec();
        want.extend([
            "superio_outb 07 00",
            "superio_outb b0 0f",
            "superio_outb b1 00",
            "superio_outb b2 20",
            "superio_outb c0 0f",
            "superio_outb c1 00",
            "superio_outb c2 10",
            "superio_outb 02 01",
        ]);
        want.extend(ENTER);
        want.extend([
            "superio_outb 07 00",
            "superio_outb b3 01",
            "superio_outb b4 02",
            "superio_outb b5 03",
            "superio_outb c3 01",
            "superio_outb c4 02",
            "superio_outb c5 03",
            "superio_outb 02 01",
        ]);
        assert_eq!(fake::take(&log), want);
        found[0].push(&id, &[Color(4, 5, 6)]).unwrap();
        assert_eq!(fake::take(&log).len(), ENTER.len() + 8);
    }
}
