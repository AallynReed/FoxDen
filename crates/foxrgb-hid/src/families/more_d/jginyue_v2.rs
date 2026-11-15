//! Ported from OpenRGB Controllers/JGINYUEInternalUSBV2Controller (GPL-2.0-or-later).

use super::common::*;
use super::serial::{self, Serial};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "jginyue-v2";
const VID: u16 = 0x1A86;
const PID: u16 = 0xE30B;
const BAUD: u32 = 115_200;

/// Init_device and Init_Zone: each ARGB area and its LED count.
pub fn read_zones(hid: &mut dyn Io) -> Vec<(u8, usize)> {
    let mut b = [0u8; 64];
    b[..2].copy_from_slice(&[0x01, 0x0F]);
    if hid.write(&b).is_err() {
        return Vec::new();
    }
    hid.sleep(20);
    match hid.read(&mut b, 1000) {
        Ok(n) if n > 0 && b[1] == 0x0F => {}
        _ => return Vec::new(),
    }
    let mut zones = Vec::new();
    for i in 0..8 {
        if b[4] & (1 << i) != 0 {
            let area = 1u8 << i;
            zones.push((area, b[5 + i] as usize));
            let mut q = [0u8; 64];
            q[..3].copy_from_slice(&[0x01, 0x11, area]);
            if hid.write(&q).is_ok() {
                hid.sleep(20);
                let _ = hid.read(&mut q, 1000);
            }
        }
    }
    zones
}

/// DirectLEDControl's CDC packet for one area.
pub fn direct_packet(area: u8, colors: &[Color]) -> Vec<u8> {
    let n = colors.len();
    let mut len = 10 + n * 3;
    if len.is_multiple_of(64) {
        len += 2;
    }
    let mut b = vec![0u8; len];
    b[..5].copy_from_slice(&[0x10, 0x01, area, 0x20, n as u8]);
    for (i, c) in colors.iter().enumerate() {
        b[8 + i * 3..11 + i * 3].copy_from_slice(&[c.0, c.1, c.2]);
    }
    b
}

pub struct JginyueV2 {
    zones: Vec<(u8, usize)>,
}

impl Proto for JginyueV2 {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for &(area, n) in &self.zones {
            io.write(&direct_packet(area, &colors[at..at + n]))?;
            at += n;
        }
        Ok(())
    }
}

/// Only JGINYUE boards, and only with the CDC port Direct mode needs.
fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    if !serial::board_manufacturer()
        .to_uppercase()
        .contains("JGINYUE")
    {
        return Err("not a JGINYUE board".into());
    }
    let mut hid = info.open(api)?;
    let port = serial::usb_ports()
        .into_iter()
        .find(|p| (p.vid, p.pid) == (VID, PID))
        .ok_or("no CDC port for Direct mode")?;
    let cdc = Serial::open(&port.name, BAUD, false)?;
    let zones = read_zones(&mut hid);
    let counts: Vec<usize> = zones.iter().map(|z| z.1).collect();
    let dev = rows(
        hid_id(info, FAMILY),
        d.name,
        DeviceKind::Motherboard,
        &counts,
    );
    Ok(boxed(Box::new(cdc), dev, FAMILY, JginyueV2 { zones }))
}

pub static DETECTORS: &[Detector] = &[det(
    "JGINYUE Internal USB ControllerV2",
    VID,
    PID,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn zones_from_status() {
        let mut status = pad(&[0x01, 0x0F, 0, 0, 0b101], 64);
        status[5] = 30;
        status[7] = 120;
        let mut zones = Vec::new();
        let ops = record(vec![status], |io| zones = read_zones(io));
        assert_eq!(zones, vec![(1, 30), (4, 120)]);
        assert_eq!(ops[0], w(pad(&[0x01, 0x0F], 64)));
        assert_eq!(ops[3], w(pad(&[0x01, 0x11, 1], 64)));
        assert_eq!(ops[6], w(pad(&[0x01, 0x11, 4], 64)));
        assert_eq!(ops.len(), 9);
    }

    #[test]
    fn direct_packets() {
        let proto = JginyueV2 {
            zones: vec![(1, 1), (2, 18)],
        };
        let mut a = vec![Color::BLACK; 19];
        a[0] = Color(1, 2, 3);
        let [first, _, _] = pushes(proto, 19, vec![], &a, &[]);
        assert_eq!(
            first[0],
            w(vec![0x10, 1, 1, 0x20, 1, 0, 0, 0, 1, 2, 3, 0, 0])
        );
        // 10 + 18 * 3 = 64 bytes gets two more
        assert_eq!(first[1], w(pad(&[0x10, 1, 2, 0x20, 18], 66)));
    }
}
