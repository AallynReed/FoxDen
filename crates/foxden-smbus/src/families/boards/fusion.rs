// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use super::{Board, Ctx, Driver, GIGABYTE_SUB_VEN, mobo_bus};
use crate::i2c::{SharedBus, SmBus, lock};

const ADDR: u8 = 0x28;
const REG_BANK: u8 = 0xF0;
const REG_CH_MODE: [u8; 2] = [0x03, 0x13];
const REG_CH_TIMERS: [u8; 2] = [0x06, 0x16];
const REG_CH_RGB: [u8; 2] = [0x00, 0x08];
const WRITE_MODE_OFST: u8 = 0x10;
const MODE_STATIC: u8 = 0x00;
/// speed_table[.][0]: Direct leaves mode::speed at 0.
const TIMERS: [u16; 2] = [0x01E0, 0x4000];

struct Fusion {
    ctx: Ctx,
}

fn wbd(bus: &dyn SmBus, reg: u8, val: u8) {
    let _ = bus.write_byte_data(ADDR, reg, val);
}

impl Fusion {
    /// SetMode(RGB_FUSION_MODE_STATIC, 0).
    fn set_mode(&self) {
        let bus = lock(&self.ctx.bus);
        wbd(&**bus, REG_BANK, 0);
        for ch in 0..2 {
            wbd(&**bus, REG_CH_MODE[ch], MODE_STATIC + WRITE_MODE_OFST);
            let t = REG_CH_TIMERS[ch];
            wbd(&**bus, t, (TIMERS[0] >> 8) as u8);
            wbd(&**bus, t + 1, TIMERS[0] as u8);
            wbd(&**bus, t + 2, (TIMERS[1] >> 8) as u8);
            wbd(&**bus, t + 3, TIMERS[1] as u8);
        }
    }

    /// SetLEDColor: colour in bank 1, then the channel mode read back and rewritten in bank 0.
    fn set_color(&self, ch: usize, c: Color) {
        let bus = lock(&self.ctx.bus);
        let rgb = REG_CH_RGB[ch];
        wbd(&**bus, REG_BANK, 1);
        wbd(&**bus, rgb, c.0);
        wbd(&**bus, rgb + 1, c.1);
        wbd(&**bus, rgb + 2, c.2);
        wbd(&**bus, rgb + 3, 0x01);
        wbd(&**bus, REG_BANK, 0);
        let mode = bus.read_byte_data(ADDR, REG_CH_MODE[ch]).unwrap_or(0xFF);
        wbd(&**bus, REG_CH_MODE[ch], mode.wrapping_add(WRITE_MODE_OFST));
    }
}

impl Driver for Fusion {
    fn write(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            self.set_mode();
        }
        for (ch, &c) in colors.iter().enumerate().take(2) {
            self.set_color(ch, c);
        }
        Ok(())
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_with(&Ctx::real(bus))
}

/// Detection and setup.
fn detect_with(ctx: &Ctx) -> Vec<Box<dyn Backend>> {
    let info = ctx.info();
    if !mobo_bus(&info) || info.pci_subsystem_vendor != GIGABYTE_SUB_VEN {
        return Vec::new();
    }
    {
        let bus = lock(&ctx.bus);
        if bus.write_quick(ADDR).is_err() || bus.read_byte_data(ADDR, 0xF2) != Ok(0xC4) {
            return Vec::new();
        }
        wbd(&**bus, REG_BANK, 0);
        wbd(&**bus, 0x02, 0x09);
    }
    let device = Device::grid(
        &format!("smbus:fusion-{ADDR:02x}"),
        "Gigabyte Motherboard",
        DeviceKind::Motherboard,
        [0.06, 0.12, 0.52, 0.05],
        2,
        1,
    );
    vec![Board::boxed(
        "gigabyte rgb fusion",
        device,
        Fusion { ctx: ctx.clone() },
    )]
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, State};
    use super::*;

    fn found(c4: u8, subvendor: u16) -> (fake::Handle, Vec<Box<dyn Backend>>) {
        let mut st = State::default();
        st.present.insert(ADDR);
        st.bytes.insert((ADDR, 0xF2), c4);
        st.bytes.insert((ADDR, 0x03), 0x10);
        st.bytes.insert((ADDR, 0x13), 0x11);
        let h = fake::bus(fake::info(0x8086, 0xA2A3, subvendor, 0x5000), st);
        let found = detect_with(&h.ctx);
        (h, found)
    }

    // Probe plus setup's "enable control" writes.
    #[test]
    fn detection() {
        let (h, f) = found(0xC4, 0x1458);
        assert_eq!(f.len(), 1);
        assert_eq!(
            h.take(),
            ["quick 28", "rbd 28 f2", "wbd 28 f0 00", "wbd 28 02 09"]
        );

        let (h, f) = found(0xC3, 0x1458);
        assert!(f.is_empty());
        assert_eq!(h.take(), ["quick 28", "rbd 28 f2"]);

        let (h, f) = found(0xC4, 0x1043);
        assert!(f.is_empty());
        assert!(h.take().is_empty());
    }

    // Direct mode then each channel's LED colour.
    #[test]
    fn direct_frame() {
        let (h, mut f) = found(0xC4, 0x1458);
        h.take();
        let id = f[0].devices()[0].id.clone();
        f[0].push(&id, &[Color(1, 2, 3), Color(4, 5, 6)]).unwrap();
        let want = [
            "wbd 28 f0 00",
            "wbd 28 03 10",
            "wbd 28 06 01",
            "wbd 28 07 e0",
            "wbd 28 08 40",
            "wbd 28 09 00",
            "wbd 28 13 10",
            "wbd 28 16 01",
            "wbd 28 17 e0",
            "wbd 28 18 40",
            "wbd 28 19 00",
            "wbd 28 f0 01",
            "wbd 28 00 01",
            "wbd 28 01 02",
            "wbd 28 02 03",
            "wbd 28 03 01",
            "wbd 28 f0 00",
            "rbd 28 03",
            "wbd 28 03 20",
            "wbd 28 f0 01",
            "wbd 28 08 04",
            "wbd 28 09 05",
            "wbd 28 0a 06",
            "wbd 28 0b 01",
            "wbd 28 f0 00",
            "rbd 28 13",
            "wbd 28 13 21",
        ];
        assert_eq!(h.take(), want);
        f[0].push(&id, &[Color(1, 2, 3), Color(4, 5, 6)]).unwrap();
        assert!(h.take().is_empty());
        f[0].push(&id, &[Color(1, 2, 3), Color(4, 5, 7)]).unwrap();
        assert_eq!(h.take().len(), 16);
    }
}
