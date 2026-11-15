//! Ported from OpenRGB Controllers/EVGAUSBController/EVGAMouseController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "evga-mouse";
const VID: u16 = 0x3842;
const WIRELESS_PID: u16 = 0x2402;
const SIZE: usize = 65;
const MODE_STATIC: u8 = 1;
const BRIGHTNESS_MAX: u8 = 100;
/// The speed the controller starts every LED with.
const SPEED: u8 = 100;
const MAX_ATTEMPTS: usize = 100;

/// SetMode for one LED.
pub fn mode_report(index: u8, mode: u8) -> [u8; SIZE] {
    let mut b = [0u8; SIZE];
    b[..8].copy_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x1D, 0x02, 0x81, 0x01]);
    b[8] = index;
    b[9] = mode;
    b
}

/// SetLed with one colour and no activation.
pub fn led_report(connection: u8, index: u8, c: Color) -> [u8; SIZE] {
    let mut b = [0u8; SIZE];
    b[..8].copy_from_slice(&[0x00, 0x00, 0x00, connection, 0x1D, 0x02, 0x00, 0x02]);
    b[8] = index;
    b[9] = MODE_STATIC;
    b[10] = BRIGHTNESS_MAX;
    b[11] = SPEED;
    b[14] = 1;
    b[15..18].copy_from_slice(&[c.0, c.1, c.2]);
    b
}

fn not_ready(b: &[u8]) -> bool {
    b[..8] == [0x00, 0xA0, 0x00, 0x02, 0x1D, 0x02, 0x80, 0x02]
}

pub struct Mouse {
    /// 0 wired, 2 through the wireless receiver.
    pub connection: u8,
}

impl Mouse {
    fn wait(&self, io: &mut dyn Io) {
        if self.connection == 2 {
            io.sleep(10);
        }
    }

    /// ReadPacketOrWait: polls until the answer is no longer "not ready".
    fn answer(&self, io: &mut dyn Io, b: &mut [u8; SIZE]) -> Result<(), String> {
        self.wait(io);
        let mut n = io.get_feature(b)?;
        let mut attempts = 1;
        while n == SIZE && attempts < MAX_ATTEMPTS && not_ready(b) {
            self.wait(io);
            n = io.get_feature(b)?;
            attempts += 1;
        }
        Ok(())
    }
}

impl Proto for Mouse {
    // The Static mode stands in for Direct, set on every LED
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for index in 0..3 {
            let mut b = mode_report(index, MODE_STATIC);
            io.feature(&b)?;
            io.get_feature(&mut b)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (index, c) in colors.iter().enumerate().take(3) {
            let mut b = led_report(self.connection, index as u8, *c);
            io.feature(&b)?;
            self.answer(io, &mut b)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let connection = if info.pid == WIRELESS_PID { 2 } else { 0 };
    let dev = rows(hid_id(info, FAMILY), d.name, DeviceKind::Mouse, &[1, 1, 1]);
    Ok(boxed(
        Box::new(info.open(api)?),
        dev,
        FAMILY,
        Mouse { connection },
    ))
}

pub static DETECTORS: &[Detector] = &[
    det("EVGA X20 Gaming Mouse", VID, 0x2420, 2, 0xFFFF, 0, open),
    det(
        "EVGA X20 USB Receiver",
        VID,
        WIRELESS_PID,
        2,
        0xFFFF,
        0,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetMode(Static) per LED, then SetLed per LED with its read-back; wireless waits 10 ms and retries
    #[test]
    fn static_mode_then_leds() {
        let busy = pad(&[0x00, 0xA0, 0x00, 0x02, 0x1D, 0x02, 0x80, 0x02], SIZE);
        let replies = vec![
            vec![0; SIZE],
            vec![0; SIZE],
            vec![0; SIZE],
            busy,
            vec![0; SIZE],
            vec![0; SIZE],
            vec![0; SIZE],
        ];
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(Mouse { connection: 2 }, 3, replies, &a, &[]);
        let mode = |i| pad(&[0, 0, 0, 0, 0x1D, 0x02, 0x81, 0x01, i, 1], SIZE);
        assert_eq!(first[0], Op::F(mode(0)));
        assert_eq!(first[1], Op::GetF(mode(0)));
        assert_eq!(first[4], Op::F(mode(2)));
        let led = |i, c: [u8; 3]| {
            pad(
                &[
                    0, 0, 0, 2, 0x1D, 0x02, 0, 0x02, i, 1, 100, 100, 0, 0, 1, c[0], c[1], c[2],
                ],
                SIZE,
            )
        };
        assert_eq!(first[6], Op::F(led(0, [1, 2, 3])));
        assert_eq!(first[7], Op::Sleep(10));
        assert_eq!(first[8], Op::GetF(led(0, [1, 2, 3])));
        assert_eq!(first[9], Op::Sleep(10));
        assert!(matches!(first[10], Op::GetF(_)));
        assert_eq!(first[11], Op::F(led(1, [4, 5, 6])));
        assert_eq!(first.len(), 6 + 5 + 3 + 3);
    }
}
