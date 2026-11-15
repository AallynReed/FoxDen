//! Royal Kludge keyboards (protocol learned from SignalRGB's RK_Keyboard_Controller.js).

mod layouts;

use super::common::positioned;
use crate::hid::{Driver, Io, Proto, rgb};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info, Open};
use hidapi::{HidApi, HidDevice};
use std::collections::HashMap;

const FAMILY: &str = "royal-kludge";
const VID: u16 = 0x258A;
const REPORT: u8 = 0x0A;
const REPORT_LEN: usize = 65;
const FIRST_DATA: usize = 61;
const ZONE_DATA: usize = 62;
const BUFFER: usize = FIRST_DATA + 6 * ZONE_DATA;

/// A key layout: wire slot and SignalRGB canvas cell per key, in plugin order.
pub struct Layout {
    pub w: u32,
    pub h: u32,
    pub slots: &'static [u16],
    pub pos: &'static [(u8, u8)],
}

/// Every collection of one interface that shares a usage, tried in order; the one that takes a
/// report id is remembered for it (SignalRGB picks these by collection number).
pub struct Collections {
    devs: Vec<HidDevice>,
    pick: HashMap<u8, usize>,
}

impl Collections {
    pub fn open(api: &HidApi, info: &Info) -> Result<Self, String> {
        let devs: Vec<HidDevice> = foxrgb_hid::device_list(api).iter()
            .map(Info::from_hidapi)
            .filter(|i| {
                i.vid == info.vid
                    && i.pid == info.pid
                    && i.serial == info.serial
                    && i.interface == info.interface
                    && i.usage_page == info.usage_page
                    && i.usage == info.usage
            })
            .filter_map(|i| i.open(api).ok())
            .collect();
        if devs.is_empty() {
            return Err("collection not found".into());
        }
        Ok(Collections {
            devs,
            pick: HashMap::new(),
        })
    }

    fn each<T>(
        &mut self,
        id: u8,
        mut f: impl FnMut(&mut HidDevice) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(&i) = self.pick.get(&id) {
            return f(&mut self.devs[i]);
        }
        let mut err = String::from("no collection took the report");
        for i in 0..self.devs.len() {
            match f(&mut self.devs[i]) {
                Ok(v) => {
                    self.pick.insert(id, i);
                    return Ok(v);
                }
                Err(e) => err = e,
            }
        }
        Err(err)
    }
}

impl Io for Collections {
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.each(data[0], |d| Io::write(d, data, len))
    }
    fn feature(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.each(data[0], |d| Io::feature(d, data, len))
    }
    fn get_feature(&mut self, first: u8, len: usize) -> Result<Vec<u8>, String> {
        self.each(first, |d| Io::get_feature(d, first, len))
    }
    fn read(&mut self, len: usize, timeout_ms: i32) -> Result<Vec<u8>, String> {
        Io::read(&mut self.devs[0], len, timeout_ms)
    }
}

/// The 7 feature reports of one frame: header `0A 07 <zone>` (zone 1 adds `06`), then key data.
pub fn packets(layout: &Layout, colors: &[Color]) -> Vec<Vec<u8>> {
    let mut buf = [0u8; BUFFER];
    for (&slot, c) in layout.slots.iter().zip(colors) {
        let s = slot as usize;
        buf[s..s + 3].copy_from_slice(&rgb(*c));
    }
    let mut out = Vec::with_capacity(7);
    let mut p = vec![REPORT, 0x07, 0x01, 0x06];
    p.extend_from_slice(&buf[..FIRST_DATA]);
    out.push(p);
    for (z, chunk) in buf[FIRST_DATA..].chunks(ZONE_DATA).enumerate() {
        let mut p = vec![REPORT, 0x07, z as u8 + 2];
        p.extend_from_slice(chunk);
        out.push(p);
    }
    out
}

struct Rk(&'static Layout);

impl Rk {
    fn send(&self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (i, p) in packets(self.0, colors).iter().enumerate() {
            if i > 0 {
                io.sleep(3);
            }
            io.feature(p, REPORT_LEN)?;
            if i == 0 {
                io.sleep(2);
            }
        }
        io.sleep(3);
        Ok(())
    }
}

impl Proto for Rk {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.send(io, colors)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = self.send(io, &vec![Color(0, 0, 0); self.0.slots.len()]);
    }
}

fn model(pid: u16) -> (&'static str, &'static Layout) {
    match pid {
        0x005D | 0x0059 | 0x00C8 | 0x010D | 0x00F4 | 0x00F3 => {
            ("Royal Kludge RK84", &layouts::RK84)
        }
        0x00C1 | 0x0173 | 0x01AF => ("Royal Kludge RK98", &layouts::RK98),
        _ => ("Royal Kludge RK61", &layouts::RK61),
    }
}

fn open(api: &HidApi, info: &Info, _: &Detector) -> Result<Box<dyn Backend>, String> {
    let (name, layout) = model(info.pid);
    let dev = positioned(
        &info.device_id(FAMILY),
        name,
        DeviceKind::Keyboard,
        layout.w,
        layout.h,
        layout.pos,
    );
    let io = Collections::open(api, info)?;
    Ok(Driver::new(Box::new(io), dev, FAMILY, Rk(layout)).boxed())
}

/// Interface 1, vendor page 0xFF00, usage 1.
pub const fn vendor(name: &'static str, vid: u16, pid: u16, open: Open) -> Detector {
    Detector {
        name,
        vid,
        pid,
        interface: Some(1),
        usage_page: Some(0xFF00),
        usage: Some(1),
        open,
    }
}

const fn d(name: &'static str, pid: u16) -> Detector {
    vendor(name, VID, pid, open)
}

pub static DETECTORS: &[Detector] = &[
    d("Royal Kludge RK61", 0x0055),
    d("Royal Kludge RK61", 0x007A),
    d("Royal Kludge RK61", 0x0060),
    d("Royal Kludge RK61", 0x004A),
    d("Royal Kludge RK61", 0x00EC),
    d("Royal Kludge RK61", 0x00E1),
    d("Royal Kludge RK61", 0x00F8),
    d("Royal Kludge RK61", 0x00C7),
    d("Royal Kludge RK61", 0x0065),
    d("Royal Kludge RK84", 0x005D),
    d("Royal Kludge RK84", 0x0059),
    d("Royal Kludge RK84", 0x00C8),
    d("Royal Kludge RK84", 0x010D),
    d("Royal Kludge RK84", 0x00F4),
    d("Royal Kludge RK84", 0x00F3),
    d("Royal Kludge RK98", 0x00C1),
    d("Royal Kludge RK98", 0x0173),
    d("Royal Kludge RK98", 0x01AF),
];

#[cfg(test)]
pub fn check(l: &Layout, buffer: usize, stride: usize) {
    assert_eq!(l.slots.len(), l.pos.len());
    assert!(
        l.pos
            .iter()
            .all(|&(x, y)| (x as u32) < l.w && (y as u32) < l.h)
    );
    assert!(
        l.slots
            .iter()
            .all(|&s| s as usize * stride + stride <= buffer)
    );
    let mut s = l.slots.to_vec();
    s.sort();
    s.dedup();
    assert_eq!(s.len(), l.slots.len());
}

#[cfg(test)]
mod tests {
    use super::super::common::mock::{Op, Rec};
    use super::*;

    #[test]
    fn layouts_fit() {
        for l in [&layouts::RK61, &layouts::RK84, &layouts::RK98] {
            check(l, BUFFER, 1);
            assert!(l.slots.iter().all(|&s| s as usize + 3 <= BUFFER));
        }
        assert_eq!(layouts::RK61.slots.len(), 61);
        assert_eq!(layouts::RK84.slots.len(), 84);
        assert_eq!(layouts::RK98.slots.len(), 102);
        assert_eq!(DETECTORS.len(), 18);
    }

    #[test]
    fn frame_splits_into_seven_zones() {
        let mut colors = vec![Color(0, 0, 0); 61];
        colors[0] = Color(1, 2, 3);
        colors[60] = Color(7, 8, 9);
        let p = packets(&layouts::RK61, &colors);
        assert_eq!(p.len(), 7);
        assert_eq!(p[0][..4], [0x0A, 0x07, 0x01, 0x06]);
        assert_eq!(p[0][4 + 4..4 + 7], [1, 2, 3]);
        assert_eq!(p[0].len(), 65);
        assert!(p[1..].iter().all(|x| x.len() == 65));
        assert_eq!(p[6][..3], [0x0A, 0x07, 0x07]);
        let off = 250 - FIRST_DATA - 3 * ZONE_DATA;
        assert_eq!(p[4][..3], [0x0A, 0x07, 0x05]);
        assert_eq!(p[4][3 + off..3 + off + 3], [7, 8, 9]);
    }

    #[test]
    fn stop_sends_black_feature_reports() {
        let mut io = Rec::default();
        Rk(&layouts::RK84).stop(&mut io);
        assert_eq!(io.ops.len(), 7);
        assert!(
            io.ops
                .iter()
                .all(|o| matches!(o, Op::Feature(b) if b.len() == 65 && b[0] == 0x0A))
        );
    }
}
