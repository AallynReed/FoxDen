// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use super::{Board, Ctx, Driver, GIGABYTE_SUB_VEN, dmi, mobo_bus};
use crate::i2c::{SharedBus, lock};

const ADDR: u8 = 0x68;
const VENDOR_NAME: &str = "Gigabyte Technology Co., Ltd.";
const IDX_MODE: usize = 0x01;
const IDX_BRIGHTNESS: usize = 0x02;
const IDX_BLUE: usize = 0x04;
const IDX_GREEN: usize = 0x05;
const IDX_RED: usize = 0x06;
const APPLY_ADDR: u8 = 0x17;
const ACTION_APPLY: u16 = 0x01FF;
const LED_START_ADDR: u8 = 0x20;
const MODE_STATIC: u8 = 0x04;
const BRIGHTNESS_MAX: u8 = 0x64;
const SLOTS: usize = 10;

/// rgb_fusion_2_smbus_motherboards.
const BOARDS: [&str; 21] = [
    "B450 AORUS ELITE",
    "B450 AORUS ELITE V2",
    "B450 AORUS M",
    "B450 AORUS PRO WIFI-CF",
    "B450 AORUS PRO-CF",
    "B450 AORUS PRO-CF4",
    "B450 I AORUS PRO WIFI-CF",
    "B450M DS3H-CF",
    "X299 DESIGNARE EX-CF",
    "X399 AORUS XTREME-CF",
    "X399 DESIGNARE EX-CF",
    "X470 AORUS GAMING 5 WIFI",
    "X470 AORUS GAMING 5 WIFI-CF",
    "X470 AORUS GAMING 7 WIFI-CF",
    "X470 AORUS GAMING 7 WIFI-50-CF",
    "X470 AORUS ULTRA GAMING",
    "X470 AORUS ULTRA GAMING-CF",
    "B360M AORUS Gaming 3-CF",
    "B360 AORUS GAMING 3 WIFI-CF",
    "Z370 AORUS Gaming 5-CF",
    "Z370 AORUS Ultra Gaming-CF",
];

/// GetGigabyteFusion2SMBusLayout: the IT8295 slot behind each zone.
fn divisions(board: &str) -> Vec<u8> {
    match board {
        "Z370 AORUS Ultra Gaming-CF" => vec![0, 1, 2, 3, 4, 7, 8],
        _ => (0..SLOTS as u8).collect(),
    }
}

struct Fusion2 {
    ctx: Ctx,
    divisions: Vec<u8>,
    led_data: [[u8; 16]; SLOTS],
}

impl Fusion2 {
    fn new(ctx: Ctx, divisions: Vec<u8>) -> Self {
        let mut led_data = [[0u8; 16]; SLOTS];
        for slot in &mut led_data {
            slot[IDX_MODE] = MODE_STATIC;
            slot[IDX_BRIGHTNESS] = BRIGHTNESS_MAX;
        }
        Fusion2 {
            ctx,
            divisions,
            led_data,
        }
    }

    /// SetLEDEffect in Direct (static) mode at full brightness, then WriteLED.
    fn set_led(&mut self, led: usize, c: Color) {
        let d = &mut self.led_data[led];
        d[IDX_MODE] = MODE_STATIC;
        d[IDX_RED] = c.0;
        d[IDX_GREEN] = c.1;
        d[IDX_BLUE] = c.2;
        d[IDX_BRIGHTNESS] = BRIGHTNESS_MAX;
        let even = led - led % 2;
        let register = LED_START_ADDR + 2 * (led / 2) as u8;
        let block = [self.led_data[even], self.led_data[even + 1]].concat();
        let _ = lock(&self.ctx.bus).write_block_data(ADDR, register, &block);
    }
}

impl Driver for Fusion2 {
    fn write(&mut self, colors: &[Color], _first: bool) -> Result<(), String> {
        for (i, &c) in colors.iter().enumerate() {
            let led = self.divisions[i] as usize;
            self.set_led(led, c);
        }
        let _ = lock(&self.ctx.bus).write_word_data(ADDR, APPLY_ADDR, ACTION_APPLY);
        Ok(())
    }
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_with(&Ctx::real(bus), &dmi::read())
}

/// DetectGigabyteRGBFusion2SMBusControllers with the default SupportedDevices list.
fn detect_with(ctx: &Ctx, dmi: &dmi::Dmi) -> Vec<Box<dyn Backend>> {
    if dmi.manufacturer != VENDOR_NAME || !BOARDS.contains(&dmi.mainboard.as_str()) {
        return Vec::new();
    }
    let info = ctx.info();
    if !mobo_bus(&info)
        || info.pci_subsystem_vendor != GIGABYTE_SUB_VEN
        || info.name.contains("dmdc")
    {
        return Vec::new();
    }
    if lock(&ctx.bus).write_quick(ADDR).is_err() {
        return Vec::new();
    }
    let divisions = divisions(&dmi.mainboard);
    let device = Device::grid(
        &format!("smbus:fusion2-{ADDR:02x}"),
        &dmi.mainboard,
        DeviceKind::Motherboard,
        [0.06, 0.12, 0.52, 0.05],
        divisions.len() as u32,
        1,
    );
    let driver = Fusion2::new(ctx.clone(), divisions);
    vec![Board::boxed("gigabyte rgb fusion 2 smbus", device, driver)]
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, State};
    use super::*;

    fn dmi(board: &str) -> dmi::Dmi {
        dmi::Dmi {
            manufacturer: VENDOR_NAME.into(),
            mainboard: board.into(),
        }
    }

    fn bus(name: &str) -> fake::Handle {
        let mut st = State::default();
        st.present.insert(ADDR);
        let mut info = fake::info(0x1022, 0x790B, 0x1458, 0x1000);
        info.name = name.into();
        fake::bus(info, st)
    }

    // DetectGigabyteRGBFusion2SMBusControllers: DMI list, subvendor and "dmdc" gates, then a quick write.
    #[test]
    fn detection_gates() {
        let h = bus("PawnIO SMBus");
        assert_eq!(detect_with(&h.ctx, &dmi("B450 AORUS ELITE")).len(), 1);
        assert_eq!(h.take(), ["quick 68"]);

        assert!(detect_with(&h.ctx, &dmi("Z690 AORUS ELITE")).is_empty());
        let mut other = dmi("B450 AORUS ELITE");
        other.manufacturer = "ASUSTeK COMPUTER INC.".into();
        assert!(detect_with(&h.ctx, &other).is_empty());
        assert!(h.take().is_empty());

        let h = bus("amd dmdc bus");
        assert!(detect_with(&h.ctx, &dmi("B450 AORUS ELITE")).is_empty());
        assert!(h.take().is_empty());
    }

    // LED update: effect per zone (32-byte block of the slot pair), then apply.
    #[test]
    fn direct_frame_generic_layout() {
        let h = bus("PawnIO SMBus");
        let mut f = detect_with(&h.ctx, &dmi("B450 AORUS ELITE"));
        h.take();
        let id = f[0].devices()[0].id.clone();
        assert_eq!(f[0].devices()[0].leds.len(), 10);
        let mut colors = vec![Color::BLACK; 10];
        colors[0] = Color(1, 2, 3);
        colors[1] = Color(4, 5, 6);
        f[0].push(&id, &colors).unwrap();
        let log = h.take();
        assert_eq!(log.len(), 11);
        let slot = |r: u8, g: u8, b: u8| {
            let mut s = [0u8; 16];
            s[1] = 4;
            s[2] = 0x64;
            s[4] = b;
            s[5] = g;
            s[6] = r;
            s
        };
        let first = [
            slot(1, 2, 3),
            [0, 4, 0x64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(log[0], format!("wblk 68 20 {first:02x?}"));
        let second = [slot(1, 2, 3), slot(4, 5, 6)].concat();
        assert_eq!(log[1], format!("wblk 68 20 {second:02x?}"));
        assert_eq!(
            log[2],
            format!("wblk 68 22 {:02x?}", [slot(0, 0, 0); 2].concat())
        );
        assert_eq!(
            log[9],
            format!("wblk 68 28 {:02x?}", [slot(0, 0, 0); 2].concat())
        );
        assert_eq!(log[10], "wwd 68 17 01ff");
        f[0].push(&id, &colors).unwrap();
        assert!(h.take().is_empty());
    }

    // GetGigabyteFusion2SMBusLayout: the Z370 Ultra Gaming maps zones to slots 0-4, 7, 8.
    #[test]
    fn z370_layout() {
        let h = bus("PawnIO SMBus");
        let mut f = detect_with(&h.ctx, &dmi("Z370 AORUS Ultra Gaming-CF"));
        h.take();
        let id = f[0].devices()[0].id.clone();
        assert_eq!(f[0].devices()[0].leds.len(), 7);
        f[0].push(&id, &[Color(9, 9, 9); 7]).unwrap();
        let regs: Vec<String> = h.take().iter().map(|l| l[..10].to_string()).collect();
        let want = [
            "wblk 68 20",
            "wblk 68 20",
            "wblk 68 22",
            "wblk 68 22",
            "wblk 68 24",
            "wblk 68 26",
            "wblk 68 28",
            "wwd 68 17 ",
        ];
        assert_eq!(regs, want);
    }
}
