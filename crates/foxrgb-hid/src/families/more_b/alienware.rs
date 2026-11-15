//! Ported from OpenRGB Controllers/AlienwareController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{Driver, Io, any, at, backend, device, open_io, rows};
use crate::registry::{Detector, Info};

const LEN: usize = 34;
const REPORT: u8 = 0x20;
const USER_ANIM: u8 = 0x21;
const SELECT_ZONES: u8 = 0x23;
const ADD_ACTION: u8 = 0x24;
const DIM: u8 = 0x26;
const REPORT_FIRMWARE: u8 = 0x00;
const REPORT_CONFIG: u8 = 0x02;
const ANIM_NEW: u16 = 0x0001;
const ANIM_FINISH_SAVE: u16 = 0x0002;
const ANIM_FINISH_PLAY: u16 = 0x0003;
const ANIM_KEYBOARD: u16 = 0xFFFF;
const MODE_COLOR: u8 = 0x00;
const TEMPO_MAX: u16 = 0x00FA;
const MAX_ZONES_PER_PACKET: usize = 25;

/// `zone_quirks_table` and `zone_ids_table` by platform id.
fn platform(id: u16) -> (Option<usize>, Option<&'static [u8]>) {
    match id {
        0x0104 => (
            Some(12),
            Some(&[
                0x00, 0x01, 0x02, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
            ]),
        ),
        0x0C01 | 0x0E03 => (Some(4), Some(&[0x00, 0x01, 0x02, 0x03])),
        0x0A01 => (Some(16), None),
        0x0E07 | 0x0E0A => (Some(4), Some(&[0x10, 0x11, 0x12, 0x13])),
        _ => (None, None),
    }
}

fn request(head: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; LEN];
    b[1] = 0x03;
    b[2..2 + head.len()].copy_from_slice(head);
    b
}

/// `SendHIDReport` followed by `GetResponse`. The 1 s pause tests byte 3, the high byte of a
/// user-animation subcommand, so every command gets 60 ms as in OpenRGB.
fn transact(io: &mut dyn Io, b: &[u8]) -> Result<Vec<u8>, String> {
    io.feature(b)?;
    let finish =
        b[2] == USER_ANIM && (b[3] as u16 == ANIM_FINISH_PLAY || b[3] as u16 == ANIM_FINISH_SAVE);
    io.sleep(Duration::from_millis(if finish { 1000 } else { 60 }));
    let mut r = vec![0u8; LEN];
    io.get_feature(&mut r)?;
    Ok(r)
}

/// The check OpenRGB makes on most replies: status 3 and not an echo of the request.
fn accepted(req: &[u8], resp: &[u8]) -> bool {
    resp[1] == 0x03 && req != resp
}

/// `AlienwareController` constructor: Report(CONFIG), then Report(FIRMWARE). Returns zone ids.
fn detect(io: &mut dyn Io) -> Result<Vec<u8>, String> {
    let config = transact(io, &request(&[REPORT, REPORT_CONFIG]))?;
    let id = (config[4] as u16) << 8 | config[5] as u16;
    transact(io, &request(&[REPORT, REPORT_FIRMWARE]))?;
    let (quirk, ids) = platform(id);
    let n = quirk.unwrap_or(config[6] as usize);
    if n == 0 {
        return Err("no zones".into());
    }
    Ok((0..n)
        .map(|i| ids.and_then(|t| t.get(i).copied()).unwrap_or(i as u8))
        .collect())
}

/// `AlienwareController::UserAnimation`.
fn user_animation(io: &mut dyn Io, sub: u16, anim: u16, duration: u16) -> Result<bool, String> {
    let [s0, s1] = sub.to_be_bytes();
    let [a0, a1] = anim.to_be_bytes();
    let [d0, d1] = duration.to_be_bytes();
    let resp = transact(io, &request(&[USER_ANIM, s0, s1, a0, a1, d0, d1]))?;
    Ok(resp[1] != 0
        && match sub {
            ANIM_FINISH_SAVE => resp[7] == 0,
            ANIM_FINISH_PLAY => resp[5] == 0,
            _ => true,
        })
}

/// `SelectZones` and `Dim`: zone lists of up to 25 per report.
fn zone_command(io: &mut dyn Io, cmd: u8, arg: u8, zones: &[u8]) -> Result<bool, String> {
    let mut ok = true;
    for chunk in zones.chunks(MAX_ZONES_PER_PACKET) {
        let n = chunk.len() as u16;
        let mut head = vec![cmd, arg, (n >> 8) as u8, n as u8];
        head.extend_from_slice(chunk);
        let req = request(&head);
        let resp = transact(io, &req)?;
        ok &= accepted(&req, &resp);
    }
    Ok(ok)
}

/// `ModeAction` with one action.
fn mode_action(
    io: &mut dyn Io,
    mode: u8,
    duration: u16,
    tempo: u16,
    c: Color,
) -> Result<bool, String> {
    let [d0, d1] = duration.to_be_bytes();
    let [t0, t1] = tempo.to_be_bytes();
    let req = request(&[ADD_ACTION, mode, d0, d1, t0, t1, c.0, c.1, c.2]);
    let resp = transact(io, &req)?;
    Ok(accepted(&req, &resp))
}

struct Alienware {
    ids: Vec<u8>,
    dim_dirty: bool,
}

impl Alienware {
    /// `AlienwareController::UpdateDim`: every zone at dim 0, the Color mode's brightness.
    fn update_dim(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.dim_dirty && zone_command(io, DIM, 0, &self.ids)? {
            self.dim_dirty = false;
        }
        Ok(())
    }
}

impl Driver for Alienware {
    // RGBController_Alienware constructor: controller->UpdateDim().
    fn setup(&mut self, io: &mut dyn Io) -> Result<(), String> {
        self.update_dim(io)
    }

    // RGBController_Alienware::DeviceUpdateMode in the per-LED Color mode -> UpdateController.
    // SetDirection always marks the controller dirty, so UpdateMode runs on every update.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let rejected = || Err::<(), String>("controller rejected the animation".into());
        if !user_animation(io, ANIM_NEW, ANIM_KEYBOARD, 0)? {
            return rejected();
        }
        for (i, &id) in self.ids.iter().enumerate() {
            if !zone_command(io, SELECT_ZONES, 1, &[id])? {
                return rejected();
            }
            if !mode_action(io, MODE_COLOR, 2000, TEMPO_MAX, at(colors, i))? {
                return rejected();
            }
        }
        if !user_animation(io, ANIM_FINISH_PLAY, ANIM_KEYBOARD, 0)? {
            return rejected();
        }
        self.update_dim(io)
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let ids = detect(io.as_mut())?;
    let (leds, grid) = rows(&[ids.len()]);
    let dev = device(
        info.device_id("alienware"),
        d.name,
        DeviceKind::Keyboard,
        grid,
        leds,
    );
    let driver = Alienware {
        ids,
        dim_dirty: true,
    };
    Ok(backend(io, dev, driver, "alienware"))
}

pub(super) static DETECTORS: [Detector; 2] = [
    any("Dell G Series LED Controller", 0x187C, 0x0550, open),
    any("Dell G Series LED Controller", 0x187C, 0x0551, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{Fake, Op, buf, ms};
    use super::*;

    fn get() -> Op {
        Op::GetFeature(vec![0; LEN])
    }

    fn ok() -> Vec<u8> {
        buf(LEN, &[0, 0x03, 0xEE])
    }

    // AlienwareController constructor: Report(CONFIG) and Report(FIRMWARE), G15 5520 zone ids
    #[test]
    fn detect_reads_platform() {
        let mut fake = Fake::with(vec![buf(LEN, &[0, 3, 0x20, 2, 0x0E, 0x07, 9]), ok()]);
        assert_eq!(detect(&mut fake).unwrap(), vec![0x10, 0x11, 0x12, 0x13]);
        assert_eq!(
            fake.ops(),
            vec![
                Op::Feature(buf(LEN, &[0, 3, 0x20, 0x02])),
                ms(60),
                get(),
                Op::Feature(buf(LEN, &[0, 3, 0x20, 0x00])),
                ms(60),
                get(),
            ]
        );
        // Unknown platform: zone count from byte 6, ids 0..n.
        let mut fake = Fake::with(vec![buf(LEN, &[0, 3, 0, 0, 0x12, 0x34, 3]), ok()]);
        assert_eq!(detect(&mut fake).unwrap(), vec![0, 1, 2]);
        let mut fake = Fake::with(vec![buf(LEN, &[0, 3, 0, 0, 0x12, 0x34, 0]), ok()]);
        assert!(detect(&mut fake).is_err());
    }

    // RGBController_Alienware constructor -> UpdateDim: Dim(all zones, 0)
    #[test]
    fn setup_dims_all_zones() {
        let mut fake = Fake::with(vec![ok()]);
        let mut a = Alienware {
            ids: vec![0x10, 0x11, 0x12, 0x13],
            dim_dirty: true,
        };
        a.setup(&mut fake).unwrap();
        assert_eq!(
            fake.ops(),
            vec![
                Op::Feature(buf(LEN, &[0, 3, 0x26, 0, 0, 4, 0x10, 0x11, 0x12, 0x13])),
                ms(60),
                get(),
            ]
        );
        assert!(!a.dim_dirty);
    }

    // AlienwareController::UpdateMode for the Color mode over two zones
    #[test]
    fn frame_plays_a_user_animation() {
        let mut fake = Fake::with(vec![ok(); 6]);
        let mut a = Alienware {
            ids: vec![0x00, 0x01],
            dim_dirty: false,
        };
        a.frame(&mut fake, &[Color(1, 2, 3), Color(4, 5, 6)])
            .unwrap();
        let sent: Vec<Op> = fake
            .ops()
            .into_iter()
            .filter(|o| matches!(o, Op::Feature(_)))
            .collect();
        assert_eq!(
            sent,
            vec![
                Op::Feature(buf(LEN, &[0, 3, 0x21, 0x00, 0x01, 0xFF, 0xFF, 0, 0])),
                Op::Feature(buf(LEN, &[0, 3, 0x23, 1, 0, 1, 0x00])),
                Op::Feature(buf(LEN, &[0, 3, 0x24, 0, 0x07, 0xD0, 0x00, 0xFA, 1, 2, 3])),
                Op::Feature(buf(LEN, &[0, 3, 0x23, 1, 0, 1, 0x01])),
                Op::Feature(buf(LEN, &[0, 3, 0x24, 0, 0x07, 0xD0, 0x00, 0xFA, 4, 5, 6])),
                Op::Feature(buf(LEN, &[0, 3, 0x21, 0x00, 0x03, 0xFF, 0xFF, 0, 0])),
            ]
        );
        assert_eq!(fake.ops().len(), 6 * 3);
        assert!(fake.ops().iter().all(|o| *o != ms(1000)));
    }

    // AlienwareController::SelectZones: an echoed request is a failure, which aborts UpdateMode
    #[test]
    fn echoed_reply_is_rejected() {
        let echo = buf(LEN, &[0, 3, 0x23, 1, 0, 1, 0x00]);
        let mut fake = Fake::with(vec![ok(), echo]);
        let mut a = Alienware {
            ids: vec![0x00],
            dim_dirty: false,
        };
        assert!(a.frame(&mut fake, &[Color(1, 2, 3)]).is_err());
        assert_eq!(fake.ops().len(), 6);
    }
}
