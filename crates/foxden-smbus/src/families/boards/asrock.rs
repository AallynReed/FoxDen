// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use super::{ASROCK_SUB_VEN, Board, Ctx, Driver, dmi, mobo_bus};
use crate::i2c::{SharedBus, lock};

const ADDR: u8 = 0x6A;
const REG_FIRMWARE_VER: u8 = 0x00;
const REG_MODE: u8 = 0x30;
const REG_SELECT: u8 = 0x31;
const REG_SET_ALL_OR_COUNT: u8 = 0x32;
const REG_ZONE_SIZE: u8 = 0x33;
const REG_V2_COLOR: u8 = 0x34;
const MODE_STATIC: u8 = 0x11;
const ZONE_ADDRESSABLE: usize = 5;
const V2_ADDRESSABLE_LED: u8 = 0x19;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Asr,
    V1,
    V2,
}

/// Static is the per-LED colour mode; these controllers have no direct mode and save each update.
struct Polychrome {
    ctx: Ctx,
    kind: Kind,
    /// Select byte per LED: the LED index for ASR and V2, the zone index for V1.
    select: Vec<u8>,
}

impl Polychrome {
    fn block(&self, command: u8, data: &[u8]) {
        let _ = lock(&self.ctx.bus).write_block_data(ADDR, command, data);
    }

    /// ASRockASRRGB / PolychromeV2 SetMode(0, Static, 0).
    fn set_mode(&self) {
        self.block(REG_MODE, &[MODE_STATIC]);
        self.ctx.ms(1);
        if self.kind == Kind::V2 {
            self.block(REG_SET_ALL_OR_COUNT, &[]);
            self.ctx.ms(1);
        }
    }

    /// PolychromeV1 SetMode(zone, Static, 0).
    fn set_mode_v1(&self, zone: u8) {
        self.block(REG_SET_ALL_OR_COUNT, &[0x00]);
        self.ctx.ms(1);
        self.block(REG_SELECT, &[zone]);
        self.ctx.ms(1);
        self.block(REG_MODE, &[MODE_STATIC]);
        self.ctx.ms(1);
    }

    /// SetColorsAndSpeed in Static mode.
    fn set_color(&self, select: u8, c: Color) {
        self.block(REG_SELECT, &[select]);
        self.ctx.ms(1);
        let command = if self.kind == Kind::V2 {
            REG_V2_COLOR
        } else {
            MODE_STATIC
        };
        self.block(command, &[c.0, c.1, c.2]);
        self.ctx.ms(1);
    }
}

impl Driver for Polychrome {
    fn write(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first && self.kind != Kind::V1 {
            self.set_mode();
        }
        for (&select, &c) in self.select.iter().zip(colors) {
            if first && self.kind == Kind::V1 {
                self.set_mode_v1(select);
            }
            self.set_color(select, c);
        }
        Ok(())
    }
}

/// ReadLEDConfiguration: six zone sizes, or all zero when the read is not six bytes.
fn zone_sizes(ctx: &Ctx) -> [u8; 6] {
    match lock(&ctx.bus).read_block_data(ADDR, REG_ZONE_SIZE) {
        Ok(b) if b.len() == 6 => [b[0], b[1], b[2], b[3], b[4], b[5]],
        _ => [0; 6],
    }
}

/// The LED list each board's zones build.
fn selects(kind: Kind, sizes: [u8; 6]) -> Vec<u8> {
    match kind {
        Kind::Asr => vec![0],
        Kind::V1 => (0..6u8).filter(|&z| sizes[z as usize] > 0).collect(),
        Kind::V2 => {
            let mut leds = Vec::new();
            for (zone, &n) in sizes.iter().enumerate() {
                if zone == ZONE_ADDRESSABLE {
                    if n > 0 {
                        leds.push(V2_ADDRESSABLE_LED);
                    }
                } else {
                    for _ in 0..n {
                        leds.push(leds.len() as u8);
                    }
                }
            }
            leds
        }
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_with(&Ctx::real(bus), &dmi::read())
}

/// Finds ASRock controllers.
fn detect_with(ctx: &Ctx, dmi: &dmi::Dmi) -> Vec<Box<dyn Backend>> {
    let info = ctx.info();
    if !mobo_bus(&info) || info.pci_subsystem_vendor != ASROCK_SUB_VEN {
        return Vec::new();
    }
    let version = {
        let bus = lock(&ctx.bus);
        if bus.write_quick(ADDR).is_err() {
            return Vec::new();
        }
        match bus.read_block_data(ADDR, REG_FIRMWARE_VER) {
            Ok(b) if b.len() == 2 => b[0],
            _ => 0,
        }
    };
    let kind = match version {
        1 => Kind::Asr,
        2 => Kind::V1,
        3 => Kind::V2,
        _ => return Vec::new(),
    };
    let sizes = if kind == Kind::Asr {
        [0; 6]
    } else {
        zone_sizes(ctx)
    };
    let select = selects(kind, sizes);
    if select.is_empty() {
        return Vec::new();
    }
    let device = Device::grid(
        &format!("smbus:asrock-{ADDR:02x}"),
        &format!("ASRock {}", dmi.mainboard),
        DeviceKind::Motherboard,
        [0.06, 0.12, 0.52, 0.05],
        select.len() as u32,
        1,
    );
    let driver = Polychrome {
        ctx: ctx.clone(),
        kind,
        select,
    };
    vec![Board::boxed("asrock polychrome", device, driver)]
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, State};
    use super::*;

    fn board(version: [u8; 2], sizes: Option<Vec<u8>>) -> (fake::Handle, Vec<Box<dyn Backend>>) {
        let mut st = State::default();
        st.present.insert(ADDR);
        st.blocks.insert((ADDR, 0x00), version.to_vec());
        if let Some(s) = sizes {
            st.blocks.insert((ADDR, 0x33), s);
        }
        let h = fake::bus(fake::info(0x1022, 0x790B, 0x1849, 0x7C56), st);
        let dmi = dmi::Dmi {
            manufacturer: "ASRock".into(),
            mainboard: "B450 Steel Legend".into(),
        };
        let found = detect_with(&h.ctx, &dmi);
        (h, found)
    }

    fn push(b: &mut Box<dyn Backend>, colors: &[Color]) {
        let id = b.devices()[0].id.clone();
        b.push(&id, colors).unwrap();
    }

    // Detection, then static mode and colours with speed.
    #[test]
    fn asr_rgb() {
        let (h, mut found) = board([1, 4], None);
        assert_eq!(h.take(), ["quick 6a", "rblk 6a 00"]);
        assert_eq!(found[0].devices()[0].name, "ASRock B450 Steel Legend");
        push(&mut found[0], &[Color(1, 2, 3)]);
        let want = [
            "wblk 6a 30 [11]",
            "sleep 1",
            "wblk 6a 31 [00]",
            "sleep 1",
            "wblk 6a 11 [01, 02, 03]",
            "sleep 1",
        ];
        assert_eq!(h.take(), want);
        push(&mut found[0], &[Color(1, 2, 3)]);
        assert!(h.take().is_empty());
        push(&mut found[0], &[Color(7, 2, 3)]);
        assert_eq!(
            h.take(),
            want[2..]
                .iter()
                .map(|s| s.replace("01, 02", "07, 02"))
                .collect::<Vec<_>>()
        );
    }

    // ASRockPolychromeV1: per zone SetMode(Static) then SetColorsAndSpeed; later frames colours only.
    #[test]
    fn polychrome_v1() {
        let (h, mut found) = board([2, 0], Some(vec![1, 0, 1, 0, 0, 30]));
        assert_eq!(h.take(), ["quick 6a", "rblk 6a 00", "rblk 6a 33"]);
        assert_eq!(found[0].devices()[0].leds.len(), 3);
        push(
            &mut found[0],
            &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)],
        );
        let mut want = Vec::new();
        for (zone, rgb) in [(0, "01, 02, 03"), (2, "04, 05, 06"), (5, "07, 08, 09")] {
            want.extend([
                "wblk 6a 32 [00]".to_string(),
                "sleep 1".into(),
                format!("wblk 6a 31 [{zone:02x}]"),
                "sleep 1".into(),
                "wblk 6a 30 [11]".into(),
                "sleep 1".into(),
                format!("wblk 6a 31 [{zone:02x}]"),
                "sleep 1".into(),
                format!("wblk 6a 11 [{rgb}]"),
                "sleep 1".into(),
            ]);
        }
        assert_eq!(h.take(), want);
        push(
            &mut found[0],
            &[Color(1, 2, 3), Color(4, 5, 6), Color(0, 0, 1)],
        );
        let log = h.take();
        assert_eq!(log.len(), 12);
        assert_eq!(
            log[8..],
            [
                "wblk 6a 31 [05]",
                "sleep 1",
                "wblk 6a 11 [00, 00, 01]",
                "sleep 1"
            ]
        );
    }

    // ASRockPolychromeV2: SetMode writes mode, an empty LED count block, then per-LED select + colour.
    #[test]
    fn polychrome_v2() {
        let (h, mut found) = board([3, 1], Some(vec![2, 0, 0, 0, 0, 12]));
        assert_eq!(h.take(), ["quick 6a", "rblk 6a 00", "rblk 6a 33"]);
        assert_eq!(found[0].devices()[0].leds.len(), 3);
        push(
            &mut found[0],
            &[Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)],
        );
        let want = [
            "wblk 6a 30 [11]",
            "sleep 1",
            "wblk 6a 32 []",
            "sleep 1",
            "wblk 6a 31 [00]",
            "sleep 1",
            "wblk 6a 34 [01, 02, 03]",
            "sleep 1",
            "wblk 6a 31 [01]",
            "sleep 1",
            "wblk 6a 34 [04, 05, 06]",
            "sleep 1",
            "wblk 6a 31 [19]",
            "sleep 1",
            "wblk 6a 34 [07, 08, 09]",
            "sleep 1",
        ];
        assert_eq!(h.take(), want);
    }

    // GetFirmwareVersion: a read that is not two bytes is an unknown type, so nothing is added.
    #[test]
    fn unknown_firmware() {
        let (h, found) = board([4, 0], None);
        assert!(found.is_empty());
        assert_eq!(h.take(), ["quick 6a", "rblk 6a 00"]);
    }

    // IF_MOBO_SMBUS + ASROCK_SUB_VEN gate.
    #[test]
    fn other_vendors_are_not_probed() {
        let mut st = State::default();
        st.present.insert(ADDR);
        let h = fake::bus(fake::info(0x1022, 0x790B, 0x1043, 0), st);
        assert!(detect_with(&h.ctx, &dmi::Dmi::default()).is_empty());
        assert!(h.take().is_empty());
    }
}
