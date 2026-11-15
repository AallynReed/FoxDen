//! Ported from OpenRGB Controllers/RobobloqLightStripController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "robobloq";
const SYNC_SCREEN: u8 = 0x80;
const READ_DEVICE_INFO: u8 = 0x82;
const SET_BRIGHTNESS: u8 = 0x87;
const SET_DYNAMIC_SPEED: u8 = 0x8A;
const SET_OPEN_URL: u8 = 0x93;
const SPEED_MAX: u8 = 0x64;
const TUPLES: usize = 34;

/// Packet framing state: every packet carries a rolling index.
pub struct Link {
    index: u8,
}

impl Link {
    pub fn new() -> Link {
        Link { index: 0x02 }
    }

    fn report(io: &mut dyn Io, data: &[u8]) -> Result<(), String> {
        let mut r = [0u8; 65];
        r[1..].copy_from_slice(data);
        io.write(&r)
    }

    /// SendPacket: "RB", length, index, command, checksum.
    pub fn send(&mut self, io: &mut dyn Io, command: &[u8], flush: bool) -> Result<(), String> {
        let mut p = vec![0x52, 0x42, command.len() as u8 + 5, self.index];
        p.extend_from_slice(command);
        let sum: u32 = p.iter().map(|&b| b as u32).sum();
        p.push(sum as u8);
        p.resize(64, 0);
        Self::report(io, &p)?;
        self.index = self.index.wrapping_add(1);
        if flush {
            let mut buf = [0u8; 64];
            while io.read(&mut buf, 0)? > 0 {}
        }
        Ok(())
    }

    /// SendMultiPacket: "SC", 16-bit length, index, command, payload, checksum, in 64-byte reports.
    pub fn send_multi(
        &mut self,
        io: &mut dyn Io,
        command: u8,
        payload: &[u8],
    ) -> Result<(), String> {
        let len = payload.len() + 7;
        let mut d = vec![0x53, 0x43, (len >> 8) as u8, len as u8, self.index, command];
        d.extend_from_slice(payload);
        let sum: u32 = d.iter().map(|&b| b as u32).sum();
        d.push(sum as u8);
        d.resize(d.len().div_ceil(64) * 64, 0);
        for chunk in d.chunks(64) {
            Self::report(io, chunk)?;
            io.sleep(1);
        }
        self.index = self.index.wrapping_add(1);
        Ok(())
    }

    /// SendPacketWithReply for the device info request.
    pub fn request_info(&mut self, io: &mut dyn Io) -> Result<Option<Vec<u8>>, String> {
        let expected = self.index;
        self.send(io, &[READ_DEVICE_INFO], false)?;
        for _ in 0..3 {
            loop {
                io.sleep(10);
                let mut buf = [0u8; 64];
                let n = io.read(&mut buf, 1000)?;
                if n > 0 && buf[3] == expected {
                    return Ok(Some(buf[..n].to_vec()));
                }
                if n == 0 {
                    break;
                }
            }
            self.send(io, &[SET_OPEN_URL, 0x00], false)?;
        }
        Ok(None)
    }
}

/// MergeRobobloqRanges: joins neighbouring LEDs with the least colour error until 34 ranges remain.
pub fn merge_ranges(colors: &[Color], tuples: usize) -> Vec<u8> {
    struct Range {
        start: usize,
        end: usize,
        n: usize,
        sum: [f64; 3],
        term: f64,
    }
    let mut ranges: Vec<Range> = colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let sum = [c.0 as f64, c.1 as f64, c.2 as f64];
            Range {
                start: i + 1,
                end: i + 1,
                n: 1,
                sum,
                term: sum.iter().map(|v| v * v).sum(),
            }
        })
        .collect();
    while ranges.len() > tuples {
        let mut best = (f64::MAX, usize::MAX, 0.0);
        for i in 0..ranges.len() - 1 {
            let (a, b) = (&ranges[i], &ranges[i + 1]);
            let s = [
                a.sum[0] + b.sum[0],
                a.sum[1] + b.sum[1],
                a.sum[2] + b.sum[2],
            ];
            let merged = s.iter().map(|v| v * v).sum::<f64>() / (a.n + b.n) as f64;
            let delta = a.term + b.term - merged;
            if delta < best.0 {
                best = (delta, i, merged);
            }
        }
        if best.1 == usize::MAX {
            break;
        }
        let right = ranges.remove(best.1 + 1);
        let left = &mut ranges[best.1];
        left.end = right.end;
        left.n += right.n;
        for k in 0..3 {
            left.sum[k] += right.sum[k];
        }
        left.term = best.2;
    }
    ranges
        .iter()
        .flat_map(|r| {
            let avg = |k: usize| (r.sum[k] / r.n as f64).round() as u8;
            [r.start as u8, avg(0), avg(1), avg(2), r.end as u8]
        })
        .collect()
}

struct Robobloq {
    link: Link,
}

impl Proto for Robobloq {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        // Initialize's open-URL, brightness and speed, then Direct's brightness 255
        self.link.send(io, &[SET_OPEN_URL, 0x00], true)?;
        self.link.send(io, &[SET_BRIGHTNESS, 0xF9], true)?;
        self.link
            .send(io, &[SET_DYNAMIC_SPEED, SPEED_MAX - 0x32], true)?;
        self.link.send(io, &[SET_BRIGHTNESS, 0xFF], true)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        self.link
            .send_multi(io, SYNC_SCREEN, &merge_ranges(colors, TUPLES))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let mut link = Link::new();
    // the constructor asks twice: once itself and once from Initialize
    link.request_info(io.as_mut())?;
    let reply = link
        .request_info(io.as_mut())?
        .filter(|r| r.len() >= 24)
        .ok_or("no device info")?;
    let count = reply[11] as usize;
    let dev = line(info, FAMILY, d, DeviceKind::Strip, count);
    Ok(Box::new(Driver::new(io, dev, FAMILY, Robobloq { link })))
}

pub static DETECTORS: &[Detector] = &[det(
    "Robobloq Monitor Light Strip",
    0x1A86,
    0xFE07,
    ANY,
    0xFF00,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn report(data: &[u8]) -> Op {
        let mut r = vec![0u8];
        r.extend(pad(data, 64));
        w(r)
    }

    // RequestDeviceInfo via SendPacketWithReply
    #[test]
    fn info_request() {
        let mut reply = vec![0u8; 24];
        reply[3] = 0x02;
        reply[11] = 60;
        let mut rec = Rec::with(vec![reply.clone()]);
        let mut link = Link::new();
        assert_eq!(link.request_info(&mut rec).unwrap(), Some(reply));
        // 0x52 + 0x42 + 6 + 2 + 0x82 = 0x11E
        assert_eq!(
            rec.take(),
            vec![
                report(&[0x52, 0x42, 6, 2, 0x82, 0x1E]),
                Op::Sleep(10),
                Op::Read
            ]
        );
    }

    #[test]
    fn merge_keeps_short_strips_and_joins_equal_neighbours() {
        let few = merge_ranges(&[Color(1, 2, 3), Color(5, 5, 5)], 34);
        assert_eq!(few, vec![1, 1, 2, 3, 1, 2, 5, 5, 5, 2]);
        let mut many = vec![Color(10, 10, 10); 35];
        many[0] = Color(255, 0, 0);
        let m = merge_ranges(&many, 34);
        assert_eq!(m.len(), 34 * 5);
        assert_eq!(m[..10], [1, 255, 0, 0, 1, 2, 10, 10, 10, 3]);
        let avg = merge_ranges(&[Color(0, 0, 0), Color(1, 1, 1)], 1);
        assert_eq!(avg, vec![1, 1, 1, 1, 2]);
    }

    // Initialize and Direct's brightness, then SetCustom -> SendSyncScreen
    #[test]
    fn start_then_sync_screen() {
        let a = vec![Color(1, 2, 3); 3];
        let [first, _, next] = pushes(Robobloq { link: Link::new() }, 3, vec![], &a, &[]);
        assert_eq!(first[0], report(&[0x52, 0x42, 7, 2, 0x93, 0, 0x30]));
        assert_eq!(first[1], Op::Read);
        assert_eq!(first[2], report(&[0x52, 0x42, 7, 3, 0x87, 0xF9, 0x1E]));
        assert_eq!(first[4], report(&[0x52, 0x42, 7, 4, 0x8A, 0x32, 0x5B]));
        assert_eq!(first[6], report(&[0x52, 0x42, 7, 5, 0x87, 0xFF, 0x26]));
        let payload = [1, 1, 2, 3, 1, 2, 1, 2, 3, 2, 3, 1, 2, 3, 3];
        let mut d = vec![0x53, 0x43, 0, 22, 6, 0x80];
        d.extend(payload);
        let sum: u32 = d.iter().map(|&b| b as u32).sum();
        d.push(sum as u8);
        assert_eq!(first[8..], [report(&d), Op::Sleep(1)]);
        let Op::W(n) = &next[0] else { panic!() };
        assert_eq!(n[1..6], [0x53, 0x43, 0, 22, 7]);
    }
}
