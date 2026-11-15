//! Ported from OpenRGB Controllers/PowerColorGPUController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_smbus::i2c::SharedBus;

use super::cards::{POWERCOLOR_V1, POWERCOLOR_V2};
use super::common::{
    Driver, Gpu, Io, RECT, detect_each, device_id, info, is_amd_gpu_i2c_bus, row, sleep,
};

const MAGIC_V1: [u8; 3] = [0x01, 0x05, 0x00];
const MAGIC_V2: [u8; 3] = [0x01, 0x32, 0x00];

const V1_REG_MODE: u8 = 0x01;
const V1_REG_LED_ALL: u8 = 0x0E;
const V1_REG_MB_SYNC: u8 = 0x12;
const V1_REG_MAGIC: u8 = 0x90;
const V1_READ_OFFSET: u8 = 0x80;
const V1_DELAY: Duration = Duration::from_millis(32);

const V2_WRITE_MODE: u8 = 0x01;
const V2_WRITE_SYNC: u8 = 0x04;
const V2_WRITE_RGBX: u8 = 0x10;
const V2_WRITE_RGB1: u8 = 0x30;
const V2_WRITE_RGB2: u8 = 0x31;
const V2_READ_MODE: u8 = 0x81;
const V2_READ_MAGIC: u8 = 0x82;
const V2_READ_SYNC: u8 = 0x84;
const V2_READ_RGBX: u8 = 0x90;
const V2_NUM_LEDS: usize = 24;
const V2_DELAY: Duration = Duration::from_millis(50);

const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS_MAX: u8 = 0xFF;
const AMD_NAVI10_DEV: u16 = 0x731F;

/// RegisterRead: a 3 byte block read and the controller's delay; a failed read leaves zeros.
fn reg_read(io: &Io, addr: u8, reg: u8, delay: Duration) -> Option<[u8; 3]> {
    let d = io.read_block(addr, reg, 3);
    sleep(delay);
    d.map(|d| [d[0], d[1], d[2]])
}

fn reg_write(io: &Io, addr: u8, reg: u8, data: [u8; 3], delay: Duration) {
    io.write_block(addr, reg, &data);
    sleep(delay);
}

fn rgb(c: Color) -> [u8; 3] {
    [c.0, c.1, c.2]
}

pub fn detect_v1(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, POWERCOLOR_V1, |bus, card| {
        let i = info(bus);
        if !is_amd_gpu_i2c_bus(&i) {
            return None;
        }
        let io = Io::new(bus);
        let addr = card.addr;
        if io.read_block(addr, V1_REG_MAGIC, 3)? != MAGIC_V1 {
            return None;
        }
        // GetMode: brightness and speed of a card already in Static carry over to Direct.
        let has_sync_mode = i.pci_device > AMD_NAVI10_DEV;
        let sync = reg_read(&io, addr, V1_REG_MB_SYNC + V1_READ_OFFSET, V1_DELAY).unwrap_or([0; 3]);
        let mut mode = (BRIGHTNESS_MAX, 0);
        if !(sync[0] != 0 && has_sync_mode) {
            let m = reg_read(&io, addr, V1_REG_MODE + V1_READ_OFFSET, V1_DELAY).unwrap_or([0; 3]);
            if m[0] == MODE_STATIC {
                mode = (m[1], m[2]);
            }
        }
        let dev = row(device_id("powercolor-v1", bus, addr), card.name, 1);
        Some(Gpu::boxed(
            bus,
            dev,
            V1 {
                addr,
                brightness: mode.0,
                speed: mode.1,
            },
        ))
    })
}

struct V1 {
    addr: u8,
    brightness: u8,
    speed: u8,
}

impl Driver for V1 {
    // SetMode{static, brightness, speed}: sync off, then the mode.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        reg_write(io, self.addr, V1_REG_MB_SYNC, [0; 3], V1_DELAY);
        let m = [MODE_STATIC, self.brightness, self.speed];
        reg_write(io, self.addr, V1_REG_MODE, m, V1_DELAY);
    }

    // SetLEDColorAll.
    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        reg_write(io, self.addr, V1_REG_LED_ALL, rgb(colors[0]), V1_DELAY);
    }
}

pub fn detect_v2(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    detect_each(bus, POWERCOLOR_V2, |bus, card| {
        if !is_amd_gpu_i2c_bus(&info(bus)) {
            return None;
        }
        let io = Io::new(bus);
        let addr = card.addr;
        let magic = io.read_block(addr, V2_READ_MAGIC, 3)?;
        if magic != MAGIC_V1 && magic != MAGIC_V2 {
            return None;
        }
        // ReadConfig: mode, sync, then every LED, which seeds colors_copy.
        let m = reg_read(&io, addr, V2_READ_MODE, V2_DELAY).unwrap_or([0; 3]);
        let sync = reg_read(&io, addr, V2_READ_SYNC, V2_DELAY).unwrap_or([0; 3]);
        let copy = (0..V2_NUM_LEDS)
            .map(|i| {
                reg_read(&io, addr, V2_READ_RGBX + i as u8, V2_DELAY)
                    .map(|d| Color(d[0], d[1], d[2]))
            })
            .collect();
        let synced = sync.iter().all(|&b| b != 0);
        let (brightness, speed) = if !synced && m[0] == MODE_STATIC {
            (m[1], m[2])
        } else {
            (BRIGHTNESS_MAX, 0)
        };
        Some(Gpu::boxed(
            bus,
            v2_device(device_id("powercolor-v2", bus, addr), card.name),
            V2 {
                addr,
                brightness,
                speed,
                copy,
            },
        ))
    })
}

/// Stripe 1 (3), Stripe 2 (3), Hellstone (2 x 7, second row reversed), Devil (4).
fn v2_device(id: String, name: &str) -> Device {
    let mut leds = Vec::new();
    for i in 0..3 {
        leds.push([0.04, 0.2 + i as f32 * 0.3]);
    }
    for i in 0..3 {
        leds.push([0.12, 0.2 + i as f32 * 0.3]);
    }
    for k in 0..14 {
        let (col, r) = if k < 7 { (k, 0.25) } else { (13 - k, 0.75) };
        leds.push([0.22 + col as f32 * 0.08, r]);
    }
    for i in 0..4 {
        leds.push([0.82 + i as f32 * 0.05, 0.5]);
    }
    Device {
        id,
        name: name.into(),
        kind: DeviceKind::Gpu,
        rect: RECT,
        grid: [12, 3],
        leds,
    }
}

struct V2 {
    addr: u8,
    brightness: u8,
    speed: u8,
    copy: Vec<Option<Color>>,
}

impl Driver for V2 {
    // SetMode{static, brightness, speed}: SetSync(false), then the mode.
    fn setup(&mut self, io: &Io, _: &[Color]) {
        reg_write(io, self.addr, V2_WRITE_SYNC, [0; 3], V2_DELAY);
        let m = [MODE_STATIC, self.brightness, self.speed];
        reg_write(io, self.addr, V2_WRITE_MODE, m, V2_DELAY);
    }

    // DeviceUpdateLEDs: both banks when every LED matches, otherwise the LEDs that changed.
    fn update(&mut self, io: &Io, colors: &[Color], _: Option<&[Color]>) {
        if colors.windows(2).all(|w| w[0] == w[1]) {
            reg_write(io, self.addr, V2_WRITE_RGB1, rgb(colors[0]), V2_DELAY);
            reg_write(io, self.addr, V2_WRITE_RGB2, rgb(colors[0]), V2_DELAY);
        } else {
            for (i, &c) in colors.iter().enumerate() {
                if self.copy[i] != Some(c) {
                    reg_write(io, self.addr, V2_WRITE_RGBX + i as u8, rgb(c), V2_DELAY);
                }
            }
        }
        self.copy = colors.iter().map(|&c| Some(c)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{Fake, ids, push, run, s, take};
    use super::*;

    const V1_CARD: &str = "PowerColor Red Devil RX6800XT";
    const V2_CARD: &str = "PowerColor Red Devil RX7900XTX";

    // DetectPowerColorRedDevilGPUControllersV1: AMD ADL bus and the V1 magic.
    #[test]
    fn v1_detect() {
        let bus = Fake::new("NVIDIA", ids(POWERCOLOR_V1, V1_CARD))
            .block(0x22, 0x90, &MAGIC_V1)
            .shared();
        assert!(run(&[detect_v1, detect_v2], &bus).is_empty());
        assert!(take().is_empty());
        let bus = Fake::new("AMD ADL", ids(POWERCOLOR_V1, V1_CARD))
            .block(0x22, 0x90, &MAGIC_V2)
            .shared();
        assert!(run(&[detect_v1], &bus).is_empty());
        assert_eq!(take(), s(&["rblk 22 90 3"]));
    }

    // RGBController_PowerColorRedDevilV1: GetMode, DeviceUpdateMode, DeviceUpdateLEDs.
    #[test]
    fn v1_direct_and_frame() {
        let bus = Fake::new("AMD ADL", ids(POWERCOLOR_V1, V1_CARD))
            .block(0x22, 0x90, &MAGIC_V1)
            .block(0x22, 0x92, &[0, 0, 0])
            .block(0x22, 0x81, &[0x01, 0x80, 0x10])
            .shared();
        let mut b = run(&[detect_v1, detect_v2], &bus).remove(0);
        assert_eq!(
            take(),
            s(&[
                "rblk 22 90 3",
                "rblk 22 92 3",
                "sleep 32ms",
                "rblk 22 81 3",
                "sleep 32ms",
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(1, 2, 3)]),
            s(&[
                "wblk 22 12 00 00 00",
                "sleep 32ms",
                "wblk 22 01 01 80 10",
                "sleep 32ms",
                "wblk 22 0e 01 02 03",
                "sleep 32ms",
            ])
        );
    }

    // GetMode on a Navi 2 card in motherboard sync skips the mode read; Static defaults apply.
    #[test]
    fn v1_sync_defaults() {
        let bus = Fake::new("AMD ADL", ids(POWERCOLOR_V1, V1_CARD))
            .block(0x22, 0x90, &MAGIC_V1)
            .block(0x22, 0x92, &[1, 0, 1])
            .shared();
        let mut b = run(&[detect_v1], &bus).remove(0);
        assert_eq!(take(), s(&["rblk 22 90 3", "rblk 22 92 3", "sleep 32ms"]));
        assert_eq!(push(&mut b, &[Color(1, 2, 3)])[2], "wblk 22 01 01 ff 00");
    }

    fn v2_bus() -> Fake {
        (0..24u8).fold(
            Fake::new("AMD ADL", ids(POWERCOLOR_V2, V2_CARD))
                .block(0x22, 0x82, &MAGIC_V1)
                .block(0x22, 0x81, &[0x02, 0x40, 0x20])
                .block(0x22, 0x84, &[0, 0, 0]),
            |f, i| f.block(0x22, 0x90 + i, &[i, 0, 0]),
        )
    }

    // ReadConfig: mode, sync and 24 LED reads, each followed by 50 ms.
    #[test]
    fn v2_detect() {
        let bus = v2_bus().shared();
        assert_eq!(run(&[detect_v1, detect_v2], &bus).len(), 1);
        let log = take();
        assert_eq!(log.len(), 1 + 2 * 26);
        assert_eq!(
            log[..5],
            s(&[
                "rblk 22 82 3",
                "rblk 22 81 3",
                "sleep 50ms",
                "rblk 22 84 3",
                "sleep 50ms"
            ])
        );
        assert_eq!(log[log.len() - 2], "rblk 22 a7 3");
        let bad = Fake::new("AMD ADL", ids(POWERCOLOR_V2, V2_CARD)).block(0x22, 0x82, &[1, 2, 3]);
        assert!(run(&[detect_v2], &bad.shared()).is_empty());
    }

    // DeviceUpdateMode (Static: sync off, mode), then DeviceUpdateLEDs against colors_copy.
    #[test]
    fn v2_direct_and_frame() {
        let bus = v2_bus().shared();
        let mut b = run(&[detect_v2], &bus).remove(0);
        let mut colors: Vec<Color> = (0..24u8).map(|i| Color(i, 0, 0)).collect();
        colors[5] = Color(9, 9, 9);
        assert_eq!(
            push(&mut b, &colors),
            s(&[
                "wblk 22 04 00 00 00",
                "sleep 50ms",
                "wblk 22 01 01 ff 00",
                "sleep 50ms",
                "wblk 22 15 09 09 09",
                "sleep 50ms",
            ])
        );
        assert_eq!(
            push(&mut b, &[Color(7, 7, 7); 24]),
            s(&[
                "wblk 22 30 07 07 07",
                "sleep 50ms",
                "wblk 22 31 07 07 07",
                "sleep 50ms"
            ])
        );
        colors = vec![Color(7, 7, 7); 24];
        colors[23] = Color(1, 2, 3);
        assert_eq!(
            push(&mut b, &colors),
            s(&["wblk 22 27 01 02 03", "sleep 50ms"])
        );
    }
}
