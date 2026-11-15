//! Ported from OpenRGB Controllers/NZXTHue1Controller (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Op, Protocol, any, device, open_with, rows, span};
use crate::registry::{Detector, Info};

/// The zone tops out at 40 LEDs; the C++ colour buffer overflows past that.
const MAX_LEDS: usize = 40;

/// `NZXTHue1Controller::Initialize`: LEDs from the 21-byte status reply.
fn leds_from_status(reply: &[u8]) -> usize {
    let count = reply.get(0x11).copied().unwrap_or(0) as usize;
    let strip = reply.get(0x10).copied().unwrap_or(0) >> 3 == 0;
    (count * if strip { 10 } else { 8 }).min(MAX_LEDS)
}

/// `NZXTHue1Controller::SendPacket` in fixed mode. The second report repeats the start of the
/// colour data rather than continuing at LED 20, exactly as OpenRGB sends it.
fn fixed(colors: &[Color]) -> [Vec<u8>; 2] {
    let data: Vec<u8> = colors.iter().flat_map(|c| [c.1, c.0, c.2]).collect();
    let first = colors.len().min(20);
    let mut a = vec![0u8; 65];
    a[0] = 0x02;
    a[1] = 0x4B;
    a[5..5 + first * 3].copy_from_slice(&data[..first * 3]);
    let rest = colors.len() - first;
    let mut b = vec![0u8; 65];
    b[0] = 0x03;
    b[1..1 + rest * 3].copy_from_slice(&data[..rest * 3]);
    [a, b]
}

struct Hue1 {
    leds: usize,
}

impl Protocol for Hue1 {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        fixed(&span(colors, 0, self.leds))
            .into_iter()
            .map(Op::Write)
            .collect()
    }
}

/// `NZXTHue1Controller::Initialize`'s two requests.
fn init_packets() -> [Vec<u8>; 2] {
    [0x5C, 0x5D].map(|command| {
        let mut b = vec![0u8; 65];
        b[0] = 0x01;
        b[1] = command;
        b
    })
}

fn initialize(hid: &HidDevice) -> Result<usize, String> {
    for b in init_packets() {
        hid.write(&b).map_err(|e| e.to_string())?;
    }
    for _ in 0..10 {
        let mut reply = [0u8; 21];
        if hid
            .read_timeout(&mut reply, 1000)
            .map_err(|e| e.to_string())?
            == 21
        {
            return Ok(leds_from_status(&reply));
        }
    }
    Err("no status reply".into())
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "nzxt hue+", |hid| {
        let leds = initialize(hid)?;
        if leds == 0 {
            return Err("no accessories".into());
        }
        let (pos, grid) = rows(&[leds]);
        let dev = device(
            info.device_id("nzxt-hue1"),
            d.name,
            DeviceKind::Strip,
            grid,
            pos,
        );
        Ok((dev, Hue1 { leds }))
    })
}

pub(super) static DETECTORS: [Detector; 1] = [any("NZXT Smart Device V1", 0x1E71, 0x1714, open)];

#[cfg(test)]
mod tests {
    use super::super::test_util::buf;
    use super::*;

    // NZXTHue1Controller::Initialize: 01 5C, then 01 5D
    #[test]
    fn init_requests() {
        assert_eq!(
            init_packets(),
            [buf(65, &[0x01, 0x5C]), buf(65, &[0x01, 0x5D])]
        );
    }

    // NZXTHue1Controller::Initialize
    #[test]
    fn status_gives_led_count() {
        let mut reply = [0u8; 21];
        reply[0x11] = 3;
        assert_eq!(leds_from_status(&reply), 30);
        reply[0x10] = 0x01 << 3;
        assert_eq!(leds_from_status(&reply), 24);
        reply[0x11] = 9;
        assert_eq!(leds_from_status(&reply), 40);
    }

    // NZXTHue1Controller::SetLEDs -> SendPacket(HUE_1_MODE_FIXED, false, 0, 0, ...)
    #[test]
    fn frame_matches_send_packet() {
        let colors: Vec<Color> = (0..24).map(|i| Color(i, 100 + i, 200)).collect();
        let ops = Hue1 { leds: 24 }.frame(&colors);
        let mut a = buf(65, &[0x02, 0x4B, 0x00, 0x00, 0x00]);
        let mut b = buf(65, &[0x03]);
        for i in 0..20u8 {
            let at = 5 + i as usize * 3;
            a[at..at + 3].copy_from_slice(&[100 + i, i, 200]);
        }
        // the second packet starts again at colour 0
        for i in 0..4u8 {
            let at = 1 + i as usize * 3;
            b[at..at + 3].copy_from_slice(&[100 + i, i, 200]);
        }
        assert_eq!(ops, vec![Op::Write(a), Op::Write(b)]);
    }

    #[test]
    fn short_frame_sends_empty_second_packet() {
        let ops = Hue1 { leds: 8 }.frame(&[Color(1, 2, 3); 8]);
        assert_eq!(ops[1], Op::Write(buf(65, &[0x03])));
    }
}
