//! Ported from OpenRGB Controllers/EVGASMBusController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};

use super::{Board, Ctx, Driver, EVGA_SUB_VEN, dmi, mobo_bus};
use crate::i2c::{SharedBus, SmBus, lock};

const ADDR: u8 = 0x28;
const REG_01: u8 = 0x01;
const REG_RED: u8 = 0x09;
const REG_GREEN: u8 = 0x0A;
const REG_BLUE: u8 = 0x0B;
const REG_MODE: u8 = 0x0C;
const REG_CONTROL: u8 = 0x0E;
const REG_SPEED: u8 = 0x19;
const REG_21: u8 = 0x21;
const REG_22: u8 = 0x22;
const MODE_STATIC: u8 = 0x01;

/// ACX 30 has no direct mode; Static is its per-LED colour mode and saves with each update.
struct Acx30 {
    ctx: Ctx,
}

fn wbd(bus: &dyn SmBus, reg: u8, val: u8) {
    let _ = bus.write_byte_data(ADDR, reg, val);
}

fn unlock(bus: &dyn SmBus) {
    wbd(bus, REG_CONTROL, 0xE5);
    wbd(bus, REG_CONTROL, 0xE9);
    let _ = bus.read_byte_data(ADDR, REG_CONTROL);
}

fn relock(bus: &dyn SmBus) {
    wbd(bus, REG_CONTROL, 0xE0);
    let _ = bus.read_byte_data(ADDR, REG_CONTROL);
}

impl Driver for Acx30 {
    fn write(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        let bus = lock(&self.ctx.bus);
        if first {
            // DeviceUpdateMode with Static: SetMode, SetSpeed(0).
            unlock(&**bus);
            wbd(&**bus, REG_21, 0xE5);
            wbd(&**bus, REG_22, 0xE7);
            wbd(&**bus, REG_MODE, MODE_STATIC);
            wbd(&**bus, REG_SPEED, 0);
            relock(&**bus);
        }
        if let Some(c) = colors.first() {
            unlock(&**bus);
            wbd(&**bus, REG_RED, c.0);
            wbd(&**bus, REG_GREEN, c.1);
            wbd(&**bus, REG_BLUE, c.2);
            relock(&**bus);
        }
        Ok(())
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_with(&Ctx::real(bus), &dmi::read())
}

/// DetectAcx30SMBusControllers.
fn detect_with(ctx: &Ctx, dmi: &dmi::Dmi) -> Vec<Box<dyn Backend>> {
    let info = ctx.info();
    if !mobo_bus(&info) || info.pci_subsystem_vendor != EVGA_SUB_VEN {
        return Vec::new();
    }
    {
        let bus = lock(&ctx.bus);
        if bus.write_quick(ADDR).is_err() {
            return Vec::new();
        }
        match bus.read_byte_data(ADDR, REG_01) {
            Ok(r) if r > 0 && r & 1 == 1 => {}
            _ => return Vec::new(),
        }
    }
    let device = Device::grid(
        &format!("smbus:evga-{ADDR:02x}"),
        &format!("EVGA {}", dmi.mainboard),
        DeviceKind::Motherboard,
        [0.06, 0.12, 0.52, 0.05],
        1,
        1,
    );
    vec![Board::boxed(
        "evga acx30",
        device,
        Acx30 { ctx: ctx.clone() },
    )]
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, State};
    use super::*;

    fn found(reg01: u8, subvendor: u16) -> (fake::Handle, Vec<Box<dyn Backend>>) {
        let mut st = State::default();
        st.present.insert(ADDR);
        st.bytes.insert((ADDR, REG_01), reg01);
        let h = fake::bus(fake::info(0x8086, 0xA123, subvendor, 0), st);
        let dmi = dmi::Dmi {
            manufacturer: "EVGA".into(),
            mainboard: "Z370 FTW".into(),
        };
        let f = detect_with(&h.ctx, &dmi);
        (h, f)
    }

    // TestForAcx30SMBusController: quick write, then reg 0x01 must be odd.
    #[test]
    fn detection() {
        let (h, f) = found(0x03, 0x3842);
        assert_eq!(f[0].devices()[0].name, "EVGA Z370 FTW");
        assert_eq!(h.take(), ["quick 28", "rbd 28 01"]);
        let (_, f) = found(0x02, 0x3842);
        assert!(f.is_empty());
        let (h, f) = found(0x03, 0x1458);
        assert!(f.is_empty());
        assert!(h.take().is_empty());
    }

    // DeviceUpdateMode (Static) then DeviceUpdateSingleLED: Unlock, SetColors, Lock.
    #[test]
    fn static_frame() {
        let (h, mut f) = found(0x01, 0x3842);
        h.take();
        let id = f[0].devices()[0].id.clone();
        f[0].push(&id, &[Color(1, 2, 3)]).unwrap();
        let colors = [
            "wbd 28 0e e5",
            "wbd 28 0e e9",
            "rbd 28 0e",
            "wbd 28 09 01",
            "wbd 28 0a 02",
            "wbd 28 0b 03",
            "wbd 28 0e e0",
            "rbd 28 0e",
        ];
        let mut want = vec![
            "wbd 28 0e e5",
            "wbd 28 0e e9",
            "rbd 28 0e",
            "wbd 28 21 e5",
            "wbd 28 22 e7",
            "wbd 28 0c 01",
            "wbd 28 19 00",
            "wbd 28 0e e0",
            "rbd 28 0e",
        ];
        want.extend(colors);
        assert_eq!(h.take(), want);
        f[0].push(&id, &[Color(1, 2, 3)]).unwrap();
        assert!(h.take().is_empty());
        f[0].push(&id, &[Color(1, 2, 4)]).unwrap();
        assert_eq!(h.take().len(), colors.len());
    }
}
