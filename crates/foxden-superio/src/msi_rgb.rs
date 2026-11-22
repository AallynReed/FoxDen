// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use crate::lights::{Dmi, Driver, Light, OpenSio, SIO_ID_MASK, SIO_REG_LOGDEV, Sio};

const SIO_NCT6795_ID: u16 = 0xD350;
const SIO_NCT6797_ID: u16 = 0xD450;
const MSI_SIO_LOGDEV_RGB: u8 = 0x12;
const MSI_SIO_RGB_REG_ENABLE: u8 = 0xE0;
const MSI_SIO_RGB_REG_CFG_1: u8 = 0xE4;
const MSI_SIO_RGB_REG_RED_1_0: u8 = 0xF0;
const MSI_SIO_RGB_REG_GREEN_1_0: u8 = 0xF4;
const MSI_SIO_RGB_REG_BLUE_1_0: u8 = 0xF8;
const MSI_SIO_RGB_REG_CFG_3: u8 = 0xFF;
const MSI_SIO_RGB_ENABLE_MASK: u8 = 0xE0;

/// Board ids from the DMI product name, and whether the board inverts colours.
const COMPATIBLE_DEVICES: [(&str, bool); 50] = [
    ("7A40", false),
    ("7A34", false),
    ("7A39", false),
    ("7A38", false),
    ("7B79", false),
    ("7B73", false),
    ("7B61", false),
    ("7B54", false),
    ("7B49", false),
    ("7B48", false),
    ("7B45", false),
    ("7B44", false),
    ("7A59", false),
    ("7A57", false),
    ("7A68", false),
    ("7B40", false),
    ("7A94", false),
    ("7B06", false),
    ("7B08", false),
    ("7B09", false),
    ("7A58", false),
    ("7A62", false),
    ("7A69", false),
    ("7A70", false),
    ("7A72", false),
    ("7A78", false),
    ("7A79", false),
    ("7A37", false),
    ("7B89", true),
    ("7B90", true),
    ("7B19", true),
    ("7C02", true),
    ("7B75", true),
    ("7B22", true),
    ("7B23", true),
    ("7B24", true),
    ("7B27", true),
    ("7B30", true),
    ("7B31", true),
    ("7B51", true),
    ("7C04", true),
    ("7C00", true),
    ("7B98", true),
    ("7C22", true),
    ("7C24", true),
    ("7C01", true),
    ("7C39", true),
    ("7B86", true),
    ("7B87", true),
    ("7D95", false),
];

struct MsiRgb {
    sio: Sio,
}

impl MsiRgb {
    /// Setup.
    fn init(sio: &Sio, invert: bool) {
        sio.outb(SIO_REG_LOGDEV, 0x09);
        let val_at_2c = (sio.inb(0x2C) & 0b1111_0111) | 0b0001_0000;
        sio.outb(0x2C, val_at_2c);
        sio.outb(SIO_REG_LOGDEV, MSI_SIO_LOGDEV_RGB);
        let enable = sio.inb(MSI_SIO_RGB_REG_ENABLE);
        if enable & MSI_SIO_RGB_ENABLE_MASK != MSI_SIO_RGB_ENABLE_MASK {
            sio.outb(MSI_SIO_RGB_REG_ENABLE, 0xE0);
        }
        sio.outb(MSI_SIO_RGB_REG_CFG_1, 0x00);
        let mut ff_val = 0b1110_0010;
        if invert {
            ff_val |= 0b0001_1100;
        }
        sio.outb(MSI_SIO_RGB_REG_CFG_3, ff_val);
        sio.outb(0xFD, 0x00);
    }

    /// SetColor: 4 bits per channel, the same nibble in all eight colour steps.
    fn set_color(&self, c: Color) {
        let sio = &self.sio;
        sio.outb(SIO_REG_LOGDEV, MSI_SIO_LOGDEV_RGB);
        for (base, v) in [
            (MSI_SIO_RGB_REG_RED_1_0, c.0 >> 4),
            (MSI_SIO_RGB_REG_GREEN_1_0, c.1 >> 4),
            (MSI_SIO_RGB_REG_BLUE_1_0, c.2 >> 4),
        ] {
            for step in 0..4 {
                sio.outb(base + step, v | (v << 4));
            }
        }
    }
}

impl Driver for MsiRgb {
    fn write(&mut self, colors: &[Color], _first: bool) {
        self.sio.locked(|| {
            self.sio.enter();
            self.set_color(colors[0]);
            self.sio.exit();
        });
    }
}

/// Detection. The board list is checked before PawnIO is loaded.
pub(crate) fn detect(dmi: &Dmi, open: OpenSio) -> Vec<Box<dyn Backend>> {
    let maker = dmi.manufacturer.as_str();
    if !matches!(
        maker,
        "Micro-Star International Co., Ltd." | "Micro-Star International Co., Ltd" | "MSI"
    ) {
        return Vec::new();
    }
    let Some(&(_, invert)) = COMPATIBLE_DEVICES
        .iter()
        .find(|(name, _)| dmi.mainboard.contains(name))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for addr in [0x2E, 0x4E] {
        let Some(sio) = open(addr) else { continue };
        let hit = sio.locked(|| {
            sio.enter();
            let hit = matches!(sio.devid() & SIO_ID_MASK, SIO_NCT6795_ID | SIO_NCT6797_ID);
            if hit {
                MsiRgb::init(&sio, invert);
            }
            sio.exit();
            hit
        });
        if hit {
            let device = Device::grid(
                &format!("superio:msi-rgb-{addr:02x}"),
                &format!("MSI {}", dmi.mainboard),
                DeviceKind::Motherboard,
                [0.06, 0.12, 0.52, 0.05],
                1,
                1,
            );
            found.push(Light::boxed("msi-rgb", device, MsiRgb { sio }));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lights::fake::{self, Log};

    const ENTER: [&str; 3] = ["select_slot 00", "superio_inb 87", "superio_inb 87"];
    // added around every sequence, as nct6687 does
    const EXIT: &str = "pio_outb 2e aa";

    fn msi(board: &str) -> Dmi {
        fake::dmi("Micro-Star International Co., Ltd.", board, "")
    }

    fn run(dmi: &Dmi, id: u16, enable: u8) -> (Log, Vec<Box<dyn Backend>>) {
        let log = Log::default();
        let open = fake::chips(&log, &[(0x2E, id)], &[(0x2C, 0xFF), (0xE0, enable)]);
        let found = detect(dmi, &open);
        (log, found)
    }

    // Detection then setup, for an inverting board.
    #[test]
    fn detection_and_constructor() {
        let (log, found) = run(&msi("MS-7B89"), 0xD352, 0x00);
        assert_eq!(found.len(), 1);
        let mut want: Vec<&str> = ENTER.to_vec();
        want.extend([
            "superio_inb 20",
            "superio_inb 21",
            "superio_outb 07 09",
            "superio_inb 2c",
            "superio_outb 2c f7",
            "superio_outb 07 12",
            "superio_inb e0",
            "superio_outb e0 e0",
            "superio_outb e4 00",
            "superio_outb ff fe",
            "superio_outb fd 00",
            EXIT,
            "select_slot 01",
            "superio_inb 87",
            "superio_inb 87",
            "superio_inb 20",
            "superio_inb 21",
            "pio_outb 4e aa",
        ]);
        assert_eq!(fake::take(&log), want);
    }

    #[test]
    fn enabled_and_plain_board() {
        let (log, found) = run(&msi("B450 TOMAHAWK (MS-7C02)"), 0xD452, 0xE0);
        assert_eq!(found.len(), 1);
        let log = fake::take(&log);
        assert!(!log.contains(&"superio_outb e0 e0".to_string()));
        assert!(log.contains(&"superio_outb ff fe".to_string()));
        let (log, _) = run(&msi("MS-7A40"), 0xD350, 0xE0);
        assert!(fake::take(&log).contains(&"superio_outb ff e2".to_string()));
    }

    #[test]
    fn other_boards_and_chips_are_left_alone() {
        let (log, found) = run(&msi("MS-7C56"), 0xD350, 0);
        assert!(found.is_empty());
        assert!(fake::take(&log).is_empty());
        let (log, found) = run(&fake::dmi("ASUSTeK", "MS-7B89", ""), 0xD350, 0);
        assert!(found.is_empty());
        assert!(fake::take(&log).is_empty());
        let (_, found) = run(&msi("MS-7B89"), 0xD592, 0);
        assert!(found.is_empty());
    }

    // LED update.
    #[test]
    fn direct_frame() {
        let (log, mut found) = run(&msi("MS-7A40"), 0xD350, 0xE0);
        fake::take(&log);
        let id = found[0].devices()[0].id.clone();
        assert_eq!(id, "superio:msi-rgb-2e");
        found[0].push(&id, &[Color(0x12, 0xAB, 0xFF)]).unwrap();
        let mut want: Vec<&str> = ENTER.to_vec();
        want.extend([
            "superio_outb 07 12",
            "superio_outb f0 11",
            "superio_outb f1 11",
            "superio_outb f2 11",
            "superio_outb f3 11",
            "superio_outb f4 aa",
            "superio_outb f5 aa",
            "superio_outb f6 aa",
            "superio_outb f7 aa",
            "superio_outb f8 ff",
            "superio_outb f9 ff",
            "superio_outb fa ff",
            "superio_outb fb ff",
            EXIT,
        ]);
        assert_eq!(fake::take(&log), want);
        found[0].push(&id, &[Color(0x12, 0xAB, 0xFF)]).unwrap();
        assert!(fake::take(&log).is_empty());
    }
}
