// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::scsi::{self, Scsi};
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "seagate";
const LEDS: usize = 6;
const MODE_STATIC: u8 = 0x01;

/// SendPacket's vendor CDB, "SetLed", followed by the packet.
pub fn command(packet: &[u8]) -> Vec<u8> {
    let mut b = vec![
        0xD2,
        0x53,
        0x65,
        0x74,
        0x4C,
        0x65,
        0x64,
        0x00,
        0x00,
        0x30,
        packet.len() as u8,
        0x00,
    ];
    b.extend(packet);
    b
}

/// SetLEDStatic without saving.
pub fn static_packet(led: u8, c: Color) -> [u8; 14] {
    [
        0x0E,
        0x00,
        0x01,
        0x09,
        0x01,
        0x06,
        led,
        MODE_STATIC,
        0x00,
        c.0,
        c.1,
        c.2,
        0xFF,
        0xFF,
    ]
}

pub struct Seagate;

impl Proto for Seagate {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (led, c) in colors.iter().enumerate() {
            io.write(&command(&static_packet(led as u8, *c)))?;
        }
        Ok(())
    }
}

pub fn probe() -> Vec<Box<dyn Backend>> {
    scsi::enumerate()
        .into_iter()
        .filter(|d| d.vendor.starts_with("Seagate") && d.product.starts_with("FireCuda HDD"))
        .filter_map(|d| {
            let io = Scsi::open(&d.path).ok()?;
            let dev = rows(
                named_id(FAMILY, &d.path),
                "Seagate Device",
                DeviceKind::Strip,
                &[LEDS],
            );
            Some(boxed(Box::new(io), dev, FAMILY, Seagate))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn one_command_per_led() {
        let mut a = [Color::BLACK; LEDS];
        a[5] = Color(1, 2, 3);
        let [first, _, _] = pushes(Seagate, LEDS, vec![], &a, &[]);
        assert_eq!(first.len(), LEDS);
        assert_eq!(
            first[5],
            w(vec![
                0xD2, 0x53, 0x65, 0x74, 0x4C, 0x65, 0x64, 0, 0, 0x30, 0x0E, 0, 0x0E, 0, 1, 9, 1, 6,
                5, 1, 0, 1, 2, 3, 0xFF, 0xFF
            ])
        );
    }
}
