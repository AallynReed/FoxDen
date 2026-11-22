// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};

use super::ene_gpus::ENE_GPUS;
use super::{ASUS_SUB_VEN, Board, Ctx, Driver, dmi, dram_bus, mobo_bus};
use crate::i2c::{SharedBus, lock};

const RAM_ADDRESSES: [u8; 16] = [
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x4F, 0x66, 0x67, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
];
/// Where other software also remaps sticks; probed after the list above, never remapped to.
const RAM_LEFT_BY_OTHERS: [u8; 2] = [0x3E, 0x3F];
const MOBO_ADDRESSES: [u8; 3] = [0x40, 0x4E, 0x4F];

const REG_DEVICE_NAME: u16 = 0x1000;
const REG_MICRON_CHECK: u16 = 0x1030;
const REG_CONFIG_TABLE: u16 = 0x1C00;
const REG_COLORS_DIRECT: u16 = 0x8000;
const REG_DIRECT: u16 = 0x8020;
const REG_APPLY: u16 = 0x80A0;
const REG_SLOT_INDEX: u16 = 0x80F8;
const REG_I2C_ADDRESS: u16 = 0x80F9;
const REG_COLORS_DIRECT_V2: u16 = 0x8100;
const APPLY_VAL: u8 = 0x01;
const CONFIG_LED_COUNT: usize = 0x02;
const CONFIG_LED_COUNT_0107: usize = 0x03;
const NUM_ZONES: usize = 8;
const MAX_BLOCK: usize = 3;

/// ENESMBusInterface_i2c_smbus::ENERegisterRead.
fn reg_read(ctx: &Ctx, dev: u8, reg: u16) -> u8 {
    let bus = lock(&ctx.bus);
    let _ = bus.write_word_data(dev, 0x00, reg.swap_bytes());
    bus.read_byte_data(dev, 0x81).unwrap_or(0xFF)
}

/// ENESMBusInterface_i2c_smbus::ENERegisterWrite.
fn reg_write(ctx: &Ctx, dev: u8, reg: u16, val: u8) {
    let bus = lock(&ctx.bus);
    let _ = bus.write_word_data(dev, 0x00, reg.swap_bytes());
    let _ = bus.write_byte_data(dev, 0x01, val);
}

/// ENESMBusInterface_i2c_smbus::ENERegisterWriteBlock, falling back to byte writes.
fn reg_write_block(ctx: &Ctx, dev: u8, reg: u16, data: &[u8]) {
    let bus = lock(&ctx.bus);
    let _ = bus.write_word_data(dev, 0x00, reg.swap_bytes());
    if bus.write_block_data(dev, 0x03, data).is_err() {
        for &b in data {
            let _ = bus.write_byte_data(dev, 0x01, b);
        }
    }
}

fn c_str(buf: &[u8]) -> &[u8] {
    buf.split(|&b| b == 0).next().unwrap_or(&[])
}

/// Probe.
fn test(ctx: &Ctx, addr: u8) -> bool {
    let found = {
        let bus = lock(&ctx.bus);
        bus.read_byte(addr).is_ok() || bus.read_byte_data(addr, 0x00).is_ok()
    };
    if !found {
        return false;
    }
    for i in 0xA0..0xB0u8 {
        if lock(&ctx.bus).read_byte_data(addr, i) != Ok(i - 0xA0) {
            return false;
        }
    }
    let buf: Vec<u8> = (0..16)
        .map(|i| reg_read(ctx, addr, REG_MICRON_CHECK + i))
        .collect();
    c_str(&buf) != b"Micron"
}

/// Direct register and LED count offset for a version string, from the constructor's table.
fn version_regs(version: &[u8]) -> (u16, Option<usize>) {
    match version {
        b"LED-0116" | b"DIMM_LED-0102" | b"DIMM_LED-0103" | b"AUMA0-E8K4-0101" => {
            (REG_COLORS_DIRECT, Some(CONFIG_LED_COUNT))
        }
        b"AUDA0-E6K5-0101" | b"AUMA0-E6K5-0106" | b"AUMA0-E6K5-0105" | b"AUMA0-E6K5-0104" => {
            (REG_COLORS_DIRECT_V2, Some(CONFIG_LED_COUNT))
        }
        b"AUMA0-E6K5-0107" | b"AUMA0-E6K5-1110" | b"AUMA0-E6K5-1111" | b"AUMA0-E6K5-1107"
        | b"AUMA0-E6K5-0008" | b"AUMA0-E6K5-1113" | b"AUMA0-E6K5-1114" => {
            (REG_COLORS_DIRECT_V2, Some(CONFIG_LED_COUNT_0107))
        }
        // Unlisted versions: the newer colour registers for the E6K5 family, the first ones otherwise.
        v if v.windows(4).any(|w| w == b"E6K5") => (REG_COLORS_DIRECT_V2, None),
        _ => (REG_COLORS_DIRECT, None),
    }
}

/// The chip's version string and the start of its config table, for device reports.
pub fn describe(bus: &SharedBus, addr: u8) -> String {
    let ctx = Ctx::real(bus);
    let name: Vec<u8> = (0..16)
        .map(|i| reg_read(&ctx, addr, REG_DEVICE_NAME + i))
        .collect();
    let config: Vec<String> = (0..16)
        .map(|i| format!("{:02x}", reg_read(&ctx, addr, REG_CONFIG_TABLE + i)))
        .collect();
    format!(
        "version \"{}\", config {}",
        String::from_utf8_lossy(c_str(&name)),
        config.join(" ")
    )
}

struct Chip {
    ctx: Ctx,
    dev: u8,
    direct_reg: u16,
    led_count: usize,
    /// LEDs the zones lay out: the per-zone counts summed.
    leds: usize,
}

/// Setup: version string and config table.
fn chip(ctx: &Ctx, dev: u8) -> Chip {
    let mut name: Vec<u8> = (0..16)
        .map(|i| reg_read(ctx, dev, REG_DEVICE_NAME + i))
        .collect();
    name[15] = 0;
    let config: Vec<u8> = (0..64)
        .map(|i| reg_read(ctx, dev, REG_CONFIG_TABLE + i))
        .collect();
    let (direct_reg, count_at) = version_regs(c_str(&name));
    Chip {
        ctx: ctx.clone(),
        dev,
        direct_reg,
        // An unlisted version takes the larger of the two count bytes.
        led_count: count_at.map_or_else(
            || config[CONFIG_LED_COUNT].max(config[CONFIG_LED_COUNT_0107]) as usize,
            |i| config[i] as usize,
        ),
        leds: (0..NUM_ZONES).map(|z| config[3 + z] as usize).sum(),
    }
}

impl Driver for Chip {
    fn write(&mut self, colors: &[Color], first: bool) -> Result<(), String> {
        if first {
            // SetDirect(true)
            reg_write(&self.ctx, self.dev, REG_DIRECT, 1);
            reg_write(&self.ctx, self.dev, REG_APPLY, APPLY_VAL);
        }
        // SetAllColorsDirect
        let buf: Vec<u8> = (0..self.led_count)
            .flat_map(|i| {
                let c = colors.get(i).copied().unwrap_or(Color::BLACK);
                [c.0, c.2, c.1]
            })
            .collect();
        for (n, chunk) in buf.chunks(MAX_BLOCK).enumerate() {
            let reg = self.direct_reg + (n * MAX_BLOCK) as u16;
            reg_write_block(&self.ctx, self.dev, reg, chunk);
        }
        Ok(())
    }
}

fn backend(
    chip: Chip,
    id: String,
    name: &str,
    kind: DeviceKind,
    rect: [f32; 4],
) -> Option<Box<dyn Backend>> {
    // 0xFF everywhere is a chip that did not answer its config reads.
    if chip.leds == 0 || !(1..=120).contains(&chip.led_count) {
        return None;
    }
    let n = chip.leds as u32;
    let (cols, rows) = if kind == DeviceKind::Memory {
        (1, n)
    } else {
        (n, 1)
    };
    let device = Device::grid(&id, name, kind, rect, cols, rows);
    Some(Board::boxed("ene smbus", device, chip))
}

pub fn detect_dram(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    dram(&Ctx::real(bus))
}

/// Finds ENE DRAM.
fn dram(ctx: &Ctx) -> Vec<Box<dyn Backend>> {
    let mut found = Vec::new();
    if !dram_bus(&ctx.info()) {
        return found;
    }
    let mut idx: isize = -1;
    for slot in 0..8u8 {
        if lock(&ctx.bus).read_byte(0x77).is_err() {
            break;
        }
        loop {
            idx += 1;
            if idx >= RAM_ADDRESSES.len() as isize
                || lock(&ctx.bus)
                    .read_byte(RAM_ADDRESSES[idx as usize])
                    .is_err()
            {
                break;
            }
        }
        if let Some(&addr) = RAM_ADDRESSES.get(idx as usize) {
            reg_write(ctx, 0x77, REG_SLOT_INDEX, slot);
            reg_write(ctx, 0x77, REG_I2C_ADDRESS, addr << 1);
        }
    }
    for addr in RAM_ADDRESSES.into_iter().chain(RAM_LEFT_BY_OTHERS) {
        if test(ctx, addr) {
            let chip = chip(ctx, addr);
            if chip.led_count > 0 {
                let n = found.len() as f32;
                let rect = [0.62 + n * 0.035, 0.08, 0.025, 0.3];
                let id = format!("smbus:ene-dram-{addr:02x}");
                found.extend(backend(chip, id, "ENE DRAM", DeviceKind::Memory, rect));
            }
        }
        ctx.ms(1);
    }
    found
}

pub fn detect_board(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    board(&Ctx::real(bus), &dmi::read())
}

/// Finds ENE motherboard controllers.
fn board(ctx: &Ctx, dmi: &dmi::Dmi) -> Vec<Box<dyn Backend>> {
    let mut found = Vec::new();
    let info = ctx.info();
    if !mobo_bus(&info) || ![ASUS_SUB_VEN, 0, 0xFFFF].contains(&info.pci_subsystem_vendor) {
        return found;
    }
    let name = format!("ASUS {}", dmi.mainboard);
    for addr in MOBO_ADDRESSES {
        if test(ctx, addr) {
            let id = format!("smbus:ene-board-{addr:02x}");
            let rect = [0.06, 0.12, 0.52, 0.05];
            found.extend(backend(
                chip(ctx, addr),
                id,
                &name,
                DeviceKind::Motherboard,
                rect,
            ));
        }
        ctx.ms(1);
    }
    found
}

pub fn detect_gpu(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    gpu(&Ctx::real(bus))
}

/// Finds ENE GPU controllers, for each table row whose ids match the bus.
fn gpu(ctx: &Ctx) -> Vec<Box<dyn Backend>> {
    let info = ctx.info();
    let mut found = Vec::new();
    for &(name, ven, dev, sub_ven, sub_dev, addr) in ENE_GPUS {
        let ids = (info.pci_vendor, info.pci_device);
        let sub = (info.pci_subsystem_vendor, info.pci_subsystem_device);
        if ids == (ven, dev) && sub == (sub_ven, sub_dev) && test(ctx, addr) {
            let id = format!("smbus:ene-gpu-{sub_dev:04x}-{addr:02x}");
            let rect = [0.06, 0.30, 0.40, 0.05];
            found.extend(backend(chip(ctx, addr), id, name, DeviceKind::Gpu, rect));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::super::fake::{self, Ene, State, ene_ctor, ene_r, ene_test, ene_w};
    use super::*;

    const INTEL_ADL: (u16, u16) = (0x8086, 0x7AA3);

    fn sticks(st: &mut State) {
        st.unmapped = vec![
            Ene::new("AUMA0-E6K5-0107", &[(2, 1), (3, 8)]),
            Ene::new("DIMM_LED-0102", &[(2, 5), (3, 5)]),
        ];
    }

    fn absent(addr: u8) -> Vec<String> {
        vec![format!("rb {addr:02x}"), format!("rbd {addr:02x} 00")]
    }

    // DRAM: remap two sticks off 0x77, then test all 16 addresses.
    #[test]
    fn dram_remaps_and_detects() {
        let mut st = State::default();
        sticks(&mut st);
        let h = fake::bus(fake::info(INTEL_ADL.0, INTEL_ADL.1, 0x1043, 0), st);
        let found = dram(&h.ctx);

        let mut want = vec!["rb 77".to_string(), "rb 70".into()];
        want.extend(ene_w(0x77, 0x80F8, 0));
        want.extend(ene_w(0x77, 0x80F9, 0xE0));
        want.extend(["rb 77".to_string(), "rb 71".into()]);
        want.extend(ene_w(0x77, 0x80F8, 1));
        want.extend(ene_w(0x77, 0x80F9, 0xE2));
        want.push("rb 77".into());
        for addr in RAM_ADDRESSES.into_iter().chain(RAM_LEFT_BY_OTHERS) {
            if addr == 0x70 || addr == 0x71 {
                want.extend(ene_test(addr));
                want.extend(ene_ctor(addr));
            } else {
                want.extend(absent(addr));
            }
            want.push("sleep 1".into());
        }
        assert_eq!(h.take(), want);

        let devices: Vec<_> = found.iter().flat_map(|b| b.devices()).collect();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "smbus:ene-dram-70");
        assert_eq!(devices[0].leds.len(), 8);
        assert_eq!(devices[1].leds.len(), 5);
    }

    // A stick another program left at 0x3E is found without being remapped.
    #[test]
    fn dram_finds_sticks_left_by_others() {
        let mut st = State::default();
        st.ene
            .insert(0x3E, Ene::new("DIMM_LED-0103", &[(2, 4), (3, 4)]));
        let h = fake::bus(fake::info(0x1022, 0x790B, 0, 0), st);
        let found = dram(&h.ctx);
        let devices: Vec<_> = found.iter().flat_map(|b| b.devices()).collect();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "smbus:ene-dram-3e");
        assert_eq!(devices[0].leds.len(), 4);
        assert!(!h.take().iter().any(|op| op.contains("f980")));
    }

    // DRAM: the remap skips addresses already in use.
    #[test]
    fn dram_remap_skips_taken_addresses() {
        let mut st = State::default();
        st.present.insert(0x70);
        st.unmapped = vec![Ene::new("DIMM_LED-0102", &[(2, 1), (3, 1)])];
        let h = fake::bus(fake::info(0x1022, 0x790B, 0, 0), st);
        dram(&h.ctx);
        let mut want = vec!["rb 77".to_string(), "rb 70".into(), "rb 71".into()];
        want.extend(ene_w(0x77, 0x80F8, 0));
        want.extend(ene_w(0x77, 0x80F9, 0xE2));
        want.push("rb 77".into());
        assert_eq!(h.take()[..want.len()], want[..]);
    }

    // IF_DRAM_SMBUS: any other bus is left alone.
    #[test]
    fn dram_ignores_other_buses() {
        let mut st = State::default();
        sticks(&mut st);
        let h = fake::bus(fake::info(0x10DE, 0x2507, 0x1043, 0x8872), st);
        assert!(dram(&h.ctx).is_empty());
        assert!(h.take().is_empty());
    }

    // Probe: a "Micron" string at 0x1030 rejects the chip.
    #[test]
    fn micron_chips_are_skipped() {
        let mut e = Ene::new("DIMM_LED-0102", &[(2, 1), (3, 1)]);
        for (i, b) in b"Micron".iter().enumerate() {
            e.mem.insert(0x1030 + i as u16, *b);
        }
        let mut st = State::default();
        st.ene.insert(0x70, e);
        let h = fake::bus(fake::info(0x1022, 0x790B, 0, 0), st);
        assert!(dram(&h.ctx).is_empty());
        let log = h.take();
        let mut want = vec!["rb 77".to_string()];
        want.extend(ene_test(0x70));
        want.push("sleep 1".into());
        assert_eq!(log[..want.len()], want[..]);
    }

    // An unlisted version takes its LED count from the config table.
    #[test]
    fn unknown_versions_count_from_the_config() {
        let mut st = State::default();
        st.ene
            .insert(0x70, Ene::new("SOMETHING-ELSE", &[(2, 3), (3, 4)]));
        let h = fake::bus(fake::info(0x1022, 0x790B, 0, 0), st);
        let found = dram(&h.ctx);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].leds.len(), 4);
    }

    // Setup: version string to direct register and LED count offset.
    #[test]
    fn version_table() {
        assert_eq!(version_regs(b"LED-0116"), (0x8000, Some(2)));
        assert_eq!(version_regs(b"AUDA0-E6K5-0101"), (0x8100, Some(2)));
        assert_eq!(version_regs(b"AUMA0-E6K5-1114"), (0x8100, Some(3)));
        assert_eq!(version_regs(b"AUMA0-E8K4-0101"), (0x8000, Some(2)));
        assert_eq!(version_regs(b"X"), (0x8000, None));
        assert_eq!(version_regs(b"AUMA0-E6K5-9999"), (0x8100, None));
    }

    fn one_chip(version: &str, block_writes_fail: bool) -> (fake::Handle, Box<dyn Backend>) {
        let mut st = State::default();
        st.ene.insert(0x70, Ene::new(version, &[(2, 2), (3, 2)]));
        st.block_writes_fail = block_writes_fail;
        let h = fake::bus(fake::info(0x1022, 0x790B, 0, 0), st);
        let mut found = dram(&h.ctx);
        h.take();
        (h, found.remove(0))
    }

    // SetDirect(true) then SetAllColorsDirect: R,B,G in 3-byte blocks at the v2 direct register.
    #[test]
    fn direct_frame() {
        let (h, mut b) = one_chip("AUMA0-E6K5-0104", false);
        let id = b.devices()[0].id.clone();
        let colors = [Color(1, 2, 3), Color(4, 5, 6)];
        b.push(&id, &colors).unwrap();
        let mut want = ene_w(0x70, 0x8020, 1);
        want.extend(ene_w(0x70, 0x80A0, 1));
        want.extend([
            "wwd 70 00 0081".to_string(),
            "wblk 70 03 [01, 03, 02]".into(),
        ]);
        want.extend([
            "wwd 70 00 0381".to_string(),
            "wblk 70 03 [04, 06, 05]".into(),
        ]);
        assert_eq!(h.take(), want);

        b.push(&id, &colors).unwrap();
        assert!(h.take().is_empty());

        b.push(&id, &[Color(9, 0, 0), Color(4, 5, 6)]).unwrap();
        let want = vec![
            "wwd 70 00 0081".to_string(),
            "wblk 70 03 [09, 00, 00]".into(),
            "wwd 70 00 0381".into(),
            "wblk 70 03 [04, 06, 05]".into(),
        ];
        assert_eq!(h.take(), want);
    }

    // ENERegisterWriteBlock: a failed block write falls back to byte writes at 0x01.
    #[test]
    fn block_write_fallback() {
        let (h, mut b) = one_chip("DIMM_LED-0103", true);
        let id = b.devices()[0].id.clone();
        b.push(&id, &[Color(1, 2, 3), Color(4, 5, 6)]).unwrap();
        let log = h.take();
        let want = [
            "wwd 70 00 0080",
            "wblk 70 03 [01, 03, 02]",
            "wbd 70 01 01",
            "wbd 70 01 03",
            "wbd 70 01 02",
        ];
        assert_eq!(log[4..9], want.map(String::from));
    }

    // Motherboard: three addresses with a 1 ms pause after each.
    #[test]
    fn board_detection() {
        let mut st = State::default();
        st.ene
            .insert(0x4E, Ene::new("AUMA0-E6K5-0104", &[(2, 3), (3, 2), (4, 1)]));
        let h = fake::bus(fake::info(0x8086, 0xA323, 0x1043, 0x8694), st);
        let dmi = dmi::Dmi {
            manufacturer: "ASUSTeK COMPUTER INC.".into(),
            mainboard: "ROG STRIX Z390-E GAMING".into(),
        };
        let found = board(&h.ctx, &dmi);
        let mut want = absent(0x40);
        want.push("sleep 1".into());
        want.extend(ene_test(0x4E));
        want.extend(ene_ctor(0x4E));
        want.push("sleep 1".into());
        want.extend(absent(0x4F));
        want.push("sleep 1".into());
        assert_eq!(h.take(), want);
        let d = &found[0].devices()[0];
        assert_eq!(d.name, "ASUS ROG STRIX Z390-E GAMING");
        assert_eq!(d.leds.len(), 3);
        assert_eq!(d.kind, DeviceKind::Motherboard);
    }

    // Motherboard: other subvendors are not probed.
    #[test]
    fn board_needs_asus_subvendor() {
        let mut st = State::default();
        st.ene.insert(0x40, Ene::new("LED-0116", &[(2, 1), (3, 1)]));
        let h = fake::bus(fake::info(0x8086, 0xA323, 0x1458, 0), st);
        assert!(board(&h.ctx, &dmi::Dmi::default()).is_empty());
        assert!(h.take().is_empty());
    }

    // 174 base rows, then 41 more.
    #[test]
    fn gpu_table() {
        assert_eq!(ENE_GPUS.len(), 174 + 41);
        assert!(ENE_GPUS.iter().all(|g| g.5 == 0x67 && g.3 == ASUS_SUB_VEN));
        assert_eq!(
            ENE_GPUS[0],
            (
                "ASUS ROG STRIX GeForce RTX 3050 Gaming",
                0x10DE,
                0x2507,
                0x1043,
                0x8872,
                0x67
            )
        );
        assert_eq!(
            ENE_GPUS[173],
            (
                "ASUS TUF Radeon RX 9070 XT Gaming OC",
                0x1002,
                0x7550,
                0x1043,
                0x0613,
                0x67
            )
        );
    }

    // GPU: only a bus with matching PCI ids is probed, at 0x67.
    #[test]
    fn gpu_detection() {
        let mut st = State::default();
        st.ene.insert(0x67, Ene::new("AUMA0-E6K5-0008", &[(3, 4)]));
        let h = fake::bus(fake::info(0x10DE, 0x2507, 0x1043, 0x8872), st);
        let found = gpu(&h.ctx);
        let mut want = ene_test(0x67);
        want.extend(ene_ctor(0x67));
        assert_eq!(h.take(), want);
        let d = &found[0].devices()[0];
        assert_eq!(d.name, "ASUS ROG STRIX GeForce RTX 3050 Gaming");
        assert_eq!(d.leds.len(), 4);

        let h = fake::bus(fake::info(0x10DE, 0x2507, 0x1043, 0x0001), State::default());
        assert!(gpu(&h.ctx).is_empty());
        assert!(h.take().is_empty());
    }

    // ENERegisterRead: the register word goes out byte-swapped.
    #[test]
    fn reads_swap_the_register_bytes() {
        assert_eq!(ene_r(0x70, 0x1C02)[0], "wwd 70 00 021c");
    }
}
