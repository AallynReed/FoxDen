//! Ported from OpenRGB Controllers/KlipschLuminaController (GPL-2.0-or-later).

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, HidIo, Io, any, at, backend, device, on_i, open_io};
use crate::registry::{Detector, Info};

const VID: u16 = 0x18B5;
const PID: u16 = 0x0070;
const VENDOR_IFACE: i32 = 4;
const LAMPARRAY_IFACE: i32 = 5;
const BUF: usize = 257;
const HEADER: [u8; 3] = [0xA5, 0x5A, 0x70];
const CMD_LIGHT_MODE: u8 = 0x0B;
const CMD_CUSTOM_LIGHTING: u8 = 0x11;
const MODE_DIRECT: u8 = 0x07;
const MODE_UNKNOWN: u8 = 0xFF;
const LEDS: usize = 12;
/// `KLIPSCH_LUMINA_STATE_TIMEOUT_MS` over `KLIPSCH_LUMINA_STATE_DELAY_MS`.
const STATE_TRIES: usize = 150;

/// `KlipschLuminaController::ReadState`: the light mode, or `MODE_UNKNOWN` when nothing answers.
fn read_state(io: &mut dyn Io) -> u8 {
    for _ in 0..STATE_TRIES {
        let mut b = vec![0u8; BUF];
        b[1..4].copy_from_slice(&HEADER);
        let _ = io.write(&b);
        io.sleep(Duration::from_millis(100));
        let mut r = vec![0u8; BUF];
        if io.get_input(&mut r).is_ok() && r[1..4] == HEADER {
            return r[13];
        }
    }
    MODE_UNKNOWN
}

/// `KlipschLuminaController::SendCommand`: framed, with a two's-complement checksum.
fn command(cmd: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len() + 7;
    let mut b = vec![0u8; BUF];
    b[1..7].copy_from_slice(&[HEADER[0], HEADER[1], HEADER[2], cmd, len as u8, 0x00]);
    b[7..7 + data.len()].copy_from_slice(data);
    let sum = b[1..len].iter().fold(0u8, |s, x| s.wrapping_add(*x));
    b[len] = sum.wrapping_neg();
    b
}

struct Lumina {
    lamparray: Option<Box<dyn Io>>,
    unlocked: bool,
}

impl Driver for Lumina {
    // RGBController_KlipschLumina::DeviceUpdateMode(Direct) -> SetLightMode(DIRECT).
    fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
        let changed = read_state(io) != MODE_DIRECT;
        if changed {
            io.write(&command(CMD_LIGHT_MODE, &[MODE_DIRECT]))?;
            io.sleep(Duration::from_millis(300));
        }
        if let Some(lamp) = self.lamparray.as_mut()
            && (changed || !self.unlocked)
        {
            let _ = lamp.feature(&[6, 0x01]);
            lamp.sleep(Duration::from_millis(200));
            self.unlocked = true;
        }
        Ok(())
    }

    // KlipschLuminaController::SetLEDColors: 12 LEDs, left satellite first.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let data: Vec<u8> = (0..LEDS)
            .flat_map(|i| {
                let c = at(colors, i);
                [c.0, c.1, c.2]
            })
            .collect();
        io.write(&command(CMD_CUSTOM_LIGHTING, &data))
    }

    // KLIPSCH_LUMINA_FRAME_INTERVAL_MS between frames.
    fn min_gap(&self) -> Duration {
        Duration::from_millis(50)
    }
}

/// Top left, top centre, top right, bottom left, bottom right, bottom centre; per satellite.
fn leds() -> Vec<[f32; 2]> {
    let cells = [[0, 0], [1, 0], [2, 0], [0, 1], [2, 1], [1, 1]];
    (0..2)
        .flat_map(|sat| {
            cells.iter().map(move |[x, y]| {
                [
                    0.05 + 0.7 * sat as f32 + 0.1 * *x as f32,
                    0.25 + 0.5 * *y as f32,
                ]
            })
        })
        .collect()
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if info.interface != VENDOR_IFACE {
        return Err("claimed by the vendor interface".into());
    }
    let mut io = open_io(api, info)?;
    let lamparray = crate::device_list(api).iter()
        .find(|x| {
            x.vendor_id() == VID && x.product_id() == PID && x.interface_number() == LAMPARRAY_IFACE
        })
        .and_then(|x| x.open_device(api).ok())
        .map(|h| Box::new(HidIo(h)) as Box<dyn Io>);
    if read_state(io.as_mut()) == MODE_UNKNOWN {
        return Err("no state report".into());
    }
    let dev = device(
        info.device_id("klipsch"),
        d.name,
        DeviceKind::Other,
        [6, 2],
        leds(),
    );
    let driver = Lumina {
        lamparray,
        unlocked: false,
    };
    Ok(backend(io, dev, driver, "klipsch lumina"))
}

/// The plain line only claims the other interfaces, so OpenRGB's LampArray driver leaves them.
pub(super) static DETECTORS: [Detector; 2] = [
    on_i("Klipsch ProMedia Lumina", VID, PID, VENDOR_IFACE, open),
    any("Klipsch ProMedia Lumina", VID, PID, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, buf, ms};
    use super::*;

    fn state(mode: u8) -> Vec<u8> {
        let mut r = buf(BUF, &[0, 0xA5, 0x5A, 0x70]);
        r[13] = mode;
        r
    }

    fn probe() -> Op {
        Op::Write(buf(BUF, &[0, 0xA5, 0x5A, 0x70]))
    }

    // KlipschLuminaController::ReadState: probe, 100 ms, input report until the header comes back
    #[test]
    fn read_state_retries_until_header() {
        let mut fake = Fake::with(vec![vec![0; BUF], state(3)]);
        assert_eq!(read_state(&mut fake), 3);
        assert_eq!(
            fake.ops(),
            vec![
                probe(),
                ms(100),
                Op::GetInput(BUF),
                probe(),
                ms(100),
                Op::GetInput(BUF)
            ]
        );
        let mut silent = Fake::default();
        assert_eq!(read_state(&mut silent), MODE_UNKNOWN);
        assert_eq!(silent.ops().len(), 3 * STATE_TRIES);
    }

    // SetLightMode(DIRECT) from Static: mode command, 300 ms, LampArray control on interface 5
    #[test]
    fn setup_switches_to_direct_and_unlocks() {
        let mut fake = Fake::with(vec![state(3)]);
        let lamp = Fake::default();
        let mut l = Lumina {
            lamparray: Some(Box::new(lamp.clone())),
            unlocked: false,
        };
        l.setup(&mut fake).unwrap();
        let mut mode = buf(BUF, &[0, 0xA5, 0x5A, 0x70, 0x0B, 8, 0, 7]);
        mode[8] = 0u8.wrapping_sub(0xA5u8.wrapping_add(0x5A).wrapping_add(0x70 + 0x0B + 8 + 7));
        assert_eq!(fake.ops()[3..], [Op::Write(mode), ms(300)]);
        assert_eq!(lamp.ops(), vec![Op::Feature(vec![6, 1]), ms(200)]);

        // Already Direct and unlocked: only the state read.
        let mut fake = Fake::with(vec![state(7)]);
        l.setup(&mut fake).unwrap();
        assert_eq!(fake.ops().len(), 3);
        assert_eq!(lamp.ops().len(), 2);
    }

    // KlipschLuminaController::SetLEDColors -> SendCommand(CUSTOM_LIGHTING, 36 bytes)
    #[test]
    fn frame() {
        let mut fake = Fake::default();
        let colors: Vec<Color> = (0..12u8).map(|i| Color(i, 0x10, 0x20)).collect();
        let mut l = Lumina {
            lamparray: None,
            unlocked: false,
        };
        l.frame(&mut fake, &colors).unwrap();
        let mut want = buf(BUF, &[0, 0xA5, 0x5A, 0x70, 0x11, 43, 0]);
        for i in 0..12 {
            want[7 + i * 3..10 + i * 3].copy_from_slice(&[i as u8, 0x10, 0x20]);
        }
        let sum = want[1..43].iter().fold(0u8, |s, x| s.wrapping_add(*x));
        want[43] = sum.wrapping_neg();
        assert_eq!(fake.ops(), vec![Op::Write(want.clone())]);
        assert_eq!(want[1..44].iter().fold(0u8, |s, x| s.wrapping_add(*x)), 0);
        assert_eq!(l.min_gap(), Duration::from_millis(50));
    }
}
