//! Ported from OpenRGB Controllers/EVGAUSBController/EVGAX12Controller (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "evga-x12";
const SIZE: usize = 40;
const REPORT_ID: u8 = 0x07;
const HEADER: u8 = 0xEA;
const GET: u8 = 0x01;
const SET: u8 = 0x02;
const CMD_MODE: u8 = 0x01;
const CMD_STATIC: u8 = 0x02;
const MODE_STATIC: u8 = 1;
const READ_ATTEMPTS: usize = 5;
const DELAY_MS: u64 = 5;
const WRITE_ATTEMPTS: usize = 3;
/// Side, wheel and logo; the right head slot stays unused.
const SLOTS: [usize; 3] = [0, 2, 3];

fn head(direction: u8, command: u8) -> [u8; SIZE] {
    let mut p = [0u8; SIZE];
    p[..4].copy_from_slice(&[REPORT_ID, HEADER, direction, command]);
    p
}

pub fn mode_report(mode: u8) -> [u8; SIZE] {
    let mut p = head(SET, CMD_MODE);
    for slot in SLOTS {
        p[4 + slot] = mode;
    }
    p
}

pub fn static_report(colors: &[Color], brightness: u8) -> [u8; SIZE] {
    let mut p = head(SET, CMD_STATIC);
    p[20..24].fill(1);
    for (c, slot) in colors.iter().zip(SLOTS) {
        let at = 4 + slot * 4;
        p[at..at + 4].copy_from_slice(&[brightness, c.0, c.1, c.2]);
    }
    p
}

/// GetReport: asks, then polls the feature report until it answers this command.
fn get_report(io: &mut dyn Io, command: u8) -> Result<Option<[u8; SIZE]>, String> {
    io.feature(&head(GET, command))?;
    for _ in 0..READ_ATTEMPTS {
        io.sleep(DELAY_MS);
        let mut p = [0u8; SIZE];
        p[0] = REPORT_ID;
        let n = io.get_feature(&mut p)?;
        if n == SIZE && p[1] == HEADER && p[3] == command {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// SetStaticReport: writes until the device reads the colours back.
fn set_static(io: &mut dyn Io, colors: &[Color]) -> Result<bool, String> {
    let p = static_report(colors, 255);
    for _ in 0..WRITE_ATTEMPTS {
        io.feature(&p)?;
        io.sleep(DELAY_MS);
        if let Some(reply) = get_report(io, CMD_STATIC)?
            && reply[4..20] == p[4..20]
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Default)]
pub struct X12 {
    active: bool,
}

impl Proto for X12 {
    // SetStaticAndActivate until it takes, then SetStatic
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if set_static(io, colors)? && !self.active {
            io.sleep(DELAY_MS);
            io.feature(&mode_report(MODE_STATIC))?;
            self.active = true;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let dev = rows(hid_id(info, FAMILY), d.name, DeviceKind::Mouse, &[1, 1, 1]);
    Ok(boxed(
        Box::new(info.open(api)?),
        dev,
        FAMILY,
        X12::default(),
    ))
}

pub static DETECTORS: &[Detector] = &[det(
    "EVGA X12 Gaming Mouse",
    0x3842,
    0x2422,
    1,
    0x0008,
    0x004B,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn static_verified_then_activated() {
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let mut echo = static_report(&a, 255).to_vec();
        echo[2] = GET;
        let wrong = pad(&[REPORT_ID, HEADER, GET, CMD_MODE], SIZE);
        let [first, _, next] = pushes(X12::default(), 3, vec![wrong, echo], &a, &a[..1]);
        let want_static = pad(
            &[
                7, 0xEA, 2, 2, 255, 1, 2, 3, 0, 0, 0, 0, 255, 4, 5, 6, 255, 7, 8, 9, 1, 1, 1, 1,
            ],
            SIZE,
        );
        assert_eq!(
            first,
            vec![
                Op::F(want_static),
                Op::Sleep(5),
                Op::F(pad(&[7, 0xEA, 1, 2], SIZE)),
                Op::Sleep(5),
                Op::GetF(pad(&[7], SIZE)),
                Op::Sleep(5),
                Op::GetF(pad(&[7], SIZE)),
                Op::Sleep(5),
                Op::F(pad(&[7, 0xEA, 2, 1, 1, 0, 1, 1], SIZE)),
            ]
        );
        // Unanswered reads: three writes, five polls each, and no activation again
        assert_eq!(next.len(), 3 * (3 + 2 * READ_ATTEMPTS));
    }
}
