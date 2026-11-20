//! Ported from OpenRGB Controllers/RealtekARGBController (GPL-2.0-or-later).

use super::common::*;
use super::realtek::{GET, SET, SYNC_OPENRGB, UNLOCK_ADDR, header, ioctl};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "realtek-argb";
const GROUPS: usize = 5;
const HDR_SIZE: usize = 80;
const DATA_SIZE: usize = 96;
const CTL_FW: u8 = 0;
const CTL_APP: u8 = 1;

/// What the constructor and RGBController_RealtekARGB read before any LED is set.
pub struct State {
    pub hdr: [u8; HDR_SIZE],
    pub supported: bool,
    /// LED count of each enabled group, in group order.
    pub groups: Vec<(u8, usize)>,
}

/// set_appctl.
fn appctl(io: &mut dyn Io, grp: u8, ctl: u8) -> Result<bool, String> {
    ioctl(
        io,
        header(SET, &[0x4C, 0x01, ctl, grp], None),
        &mut [ctl],
        0,
        false,
    )
}

/// The cached `custled` block, read again while its first byte is still zero (get_custled).
struct Custled([u8; 16]);

impl Custled {
    fn get(&mut self, io: &mut dyn Io) -> Result<Option<[u8; 16]>, String> {
        if self.0[0] != 0 {
            return Ok(Some(self.0));
        }
        let mut data = vec![0u8; 4096];
        let head = header(GET, &[0x92], Some(UNLOCK_ADDR));
        if !ioctl(io, head, &mut data, 0x800, true)? {
            return Ok(None);
        }
        self.0.copy_from_slice(&data[70..86]);
        Ok(Some(self.0))
    }
}

/// get_argb_num.
fn argb_num(hdr: &[u8; HDR_SIZE], grp: usize) -> usize {
    u16::from_le_bytes([hdr[32 + grp * 2], hdr[33 + grp * 2]]) as usize
}

/// device_init, get_support_openrgb, then the reads of the RGBController constructor.
pub fn detect(io: &mut dyn Io) -> Result<State, String> {
    ioctl(
        io,
        header(GET, &[0x92], Some(UNLOCK_ADDR)),
        &mut [0u8; 96],
        0,
        true,
    )?;
    let mut hdr = [0u8; HDR_SIZE];
    ioctl(io, header(GET, &[0xCC, 0x02], None), &mut hdr, 0, true)?;
    for i in 0..GROUPS {
        let at = Some((HDR_SIZE + DATA_SIZE * i) as u32);
        ioctl(
            io,
            header(GET, &[0xCC, 0x02], at),
            &mut [0u8; DATA_SIZE],
            0,
            true,
        )?;
    }
    for i in 0..GROUPS {
        appctl(io, i as u8, CTL_FW)?;
    }
    let mut support = [0u8];
    let ok = ioctl(
        io,
        header(GET, &[0xCC, 0x04, SYNC_OPENRGB], None),
        &mut support,
        0,
        true,
    )?;
    if !ok || support[0] == 0 {
        release(io);
        return Ok(State {
            hdr,
            supported: false,
            groups: Vec::new(),
        });
    }

    let mut cust = Custled([0; 16]);
    if cust.get(io)?.is_some_and(|c| c[6] == 0x1) {
        ioctl(io, header(GET, &[0xC3], None), &mut [0u8; 4], 0, true)?;
    }
    ioctl(io, header(GET, &[0xA5], None), &mut [0u8; 20], 0, true)?;
    cust.get(io)?;
    let mut groups = Vec::new();
    for g in 0..GROUPS {
        if cust.get(io)?.is_some_and(|c| c[5] & (1 << g) != 0) {
            groups.push((g as u8, argb_num(&hdr, g)));
        }
    }
    Ok(State {
        hdr,
        supported: true,
        groups,
    })
}

struct Argb {
    hdr: [u8; HDR_SIZE],
    groups: Vec<(u8, usize)>,
    brightness: u16,
    app: [bool; GROUPS],
    prev_bright: [Option<u16>; GROUPS],
}

impl Proto for Argb {
    /// DeviceUpdateLEDs in Direct: set_argb_direct for each group with LEDs.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for &(grp, n) in &self.groups {
            let zone = &colors[at..at + n];
            at += n;
            if n == 0 {
                continue;
            }
            let g = grp as usize;
            if !self.app[g] {
                if !appctl(io, grp, CTL_APP)? {
                    return Err("set_appctl failed".into());
                }
                self.app[g] = true;
            }
            if self.prev_bright[g] != Some(self.brightness) {
                self.prev_bright[g] = Some(self.brightness);
                self.hdr[42 + g * 2..44 + g * 2]
                    .copy_from_slice(&(self.brightness << 8).to_le_bytes());
                let mut hdr = self.hdr;
                ioctl(io, header(SET, &[0x4C, 0x02], None), &mut hdr, 0, false)?;
            }
            let mut buf: Vec<u8> = zone.iter().flat_map(|c| rgb(*c)).collect();
            let head = header(SET, &[0x4C, 0x03, SYNC_OPENRGB, grp], Some(n as u32));
            ioctl(io, head, &mut buf, 0, false)?;
        }
        Ok(())
    }

    /// The destructor hands every group back to the firmware.
    fn stop(&mut self, io: &mut dyn Io) {
        release(io);
    }
}

/// ~RealtekARGBController: set_appctl(i, FW) for all five groups.
fn release(io: &mut dyn Io) {
    for i in 0..GROUPS {
        let _ = appctl(io, i as u8, CTL_FW);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let state = detect(io.as_mut())?;
    if !state.supported {
        return Err("controller does not support OpenRGB sync".into());
    }
    let counts: Vec<usize> = state.groups.iter().map(|g| g.1).collect();
    if counts.iter().sum::<usize>() == 0 {
        release(io.as_mut());
        return Err("no ARGB LEDs configured".into());
    }
    let (leds, grid) = rows(&counts);
    let dev = placed(info, FAMILY, d, DeviceKind::Strip, grid, leds);
    let brightness = u16::from_le_bytes([state.hdr[42], state.hdr[43]]) >> 8;
    let proto = Argb {
        hdr: state.hdr,
        groups: state.groups,
        brightness,
        app: [false; GROUPS],
        prev_bright: [None; GROUPS],
    };
    Ok(boxed(io, FAMILY, dev, proto))
}

pub static DETECTORS: [Detector; 1] = [det("RTL9209", 0x0BDA, 0x9209, ANY, 0xFF00, 0x0001, open)];

#[cfg(test)]
mod tests {
    use super::super::realtek::tests::{get_ops, set_ops};
    use super::*;
    use rec::*;

    const OK: [u8; 2] = [0x58, 0x53];

    fn hdr_reply() -> Vec<u8> {
        let mut h = vec![0u8; 80];
        h[0] = 0x5A;
        h[32] = 2;
        h[36] = 3;
        h[43] = 0x40;
        h
    }

    fn custled_reply(byte5: u8, byte6: u8) -> Vec<u8> {
        let mut r = vec![0u8; 4096];
        r[0x800 + 70] = 0x52;
        r[0x800 + 75] = byte5;
        r[0x800 + 76] = byte6;
        r
    }

    fn init_ops() -> Vec<Op> {
        let mut ops = [
            get_ops(header(GET, &[0x92], Some(0xAC00_4000)), 96),
            get_ops(header(GET, &[0xCC, 0x02], None), 80),
        ]
        .concat();
        for i in 0..5u32 {
            ops.extend(get_ops(header(GET, &[0xCC, 0x02], Some(80 + 96 * i)), 96));
        }
        for i in 0..5u8 {
            ops.extend(set_ops(header(SET, &[0x4C, 0x01, 0, i], None), &[0]));
        }
        ops.extend(get_ops(header(GET, &[0xCC, 0x04, 6], None), 1));
        ops
    }

    fn init_replies() -> Vec<Vec<u8>> {
        let mut r = vec![vec![0x5A], OK.to_vec(), hdr_reply(), OK.to_vec()];
        for _ in 0..5 {
            r.extend([vec![0x5A], OK.to_vec()]);
        }
        r.extend(vec![OK.to_vec(); 5]);
        r.extend([vec![0x59], OK.to_vec()]);
        r
    }

    // device_init, get_support_openrgb, get_dev_name with the UUID name, get_fw_ver, SetupZones
    #[test]
    fn detection_reads() {
        let mut replies = init_replies();
        replies.extend([custled_reply(0b00101, 1), OK.to_vec()]);
        replies.extend([vec![0x59], OK.to_vec(), vec![0x59], OK.to_vec()]);
        let mut rec = Rec::with(replies);
        let state = detect(&mut rec).unwrap();
        assert!(state.supported);
        assert_eq!(state.groups, vec![(0, 2), (2, 3)]);
        let mut want = init_ops();
        want.extend(get_ops(header(GET, &[0x92], Some(0xAC00_4000)), 4096));
        want.extend(get_ops(header(GET, &[0xC3], None), 4));
        want.extend(get_ops(header(GET, &[0xA5], None), 20));
        assert_eq!(rec.take(), want);
        assert_eq!(rec.left(), 0);
    }

    // get_custled is read again on every call while custled[0] stays zero
    #[test]
    fn empty_custled_is_reread() {
        let mut replies = init_replies();
        replies.extend(vec![OK.to_vec(); 7 * 2 + 2]);
        let mut rec = Rec::with(replies);
        let state = detect(&mut rec).unwrap();
        assert!(state.groups.is_empty());
        let custled = get_ops(header(GET, &[0x92], Some(0xAC00_4000)), 4096);
        let mut want = init_ops();
        want.extend(custled.clone());
        want.extend(get_ops(header(GET, &[0xA5], None), 20));
        for _ in 0..6 {
            want.extend(custled.clone());
        }
        assert_eq!(rec.take(), want);
    }

    // set_argb_direct per enabled group: appctl APP and brightness once, then set_direct
    #[test]
    fn direct_frames() {
        let mut hdr = [0u8; 80];
        hdr[0] = 0x5A;
        hdr[43] = 0x40;
        let proto = Argb {
            hdr,
            groups: vec![(0, 1), (2, 2)],
            brightness: 0x40,
            app: [false; 5],
            prev_bright: [None; 5],
        };
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let mut b = a;
        b[2] = Color(0, 0, 1);
        let [first, again, next] = pushes(proto, 3, vec![OK.to_vec(); 12], &a, &b);
        assert!(again.is_empty());
        let mut hdr0 = hdr;
        hdr0[42..44].copy_from_slice(&[0, 0x40]);
        let mut hdr2 = hdr0;
        hdr2[46..48].copy_from_slice(&[0, 0x40]);
        let hdr_set = |h: &[u8]| set_ops(header(SET, &[0x4C, 0x02], None), h);
        let direct =
            |g: u8, n: u32, c: &[u8]| set_ops(header(SET, &[0x4C, 0x03, 6, g], Some(n)), c);
        assert_eq!(
            first,
            [
                set_ops(header(SET, &[0x4C, 0x01, 1, 0], None), &[1]),
                hdr_set(&hdr0),
                direct(0, 1, &[1, 2, 3]),
                set_ops(header(SET, &[0x4C, 0x01, 1, 2], None), &[1]),
                hdr_set(&hdr2),
                direct(2, 2, &[4, 5, 6, 7, 8, 9]),
            ]
            .concat()
        );
        assert_eq!(
            next,
            [direct(0, 1, &[1, 2, 3]), direct(2, 2, &[4, 5, 6, 0, 0, 1])].concat()
        );
    }

    // get_support_openrgb answering 0: the deleted controller hands every group back
    #[test]
    fn unsupported_releases() {
        let mut replies = init_replies();
        let n = replies.len();
        replies[n - 2] = vec![0];
        replies.extend(vec![OK.to_vec(); 5]);
        let mut rec = Rec::with(replies);
        assert!(!detect(&mut rec).unwrap().supported);
        let mut want = init_ops();
        for i in 0..5u8 {
            want.extend(set_ops(header(SET, &[0x4C, 0x01, 0, i], None), &[0]));
        }
        assert_eq!(rec.take(), want);
    }

    // ~RealtekARGBController: set_appctl(i, FW) for all five groups
    #[test]
    fn drop_returns_control() {
        let rec = Rec::with(vec![OK.to_vec(); 20]);
        let proto = Argb {
            hdr: [0; 80],
            groups: vec![(1, 1)],
            brightness: 0,
            app: [false; 5],
            prev_bright: [None; 5],
        };
        let mut d = Driver::new(Box::new(rec.clone()), dev(1), "t", proto);
        foxden_core::Backend::push(&mut d, "t", &[Color(1, 1, 1)]).unwrap();
        rec.take();
        drop(d);
        let mut want = Vec::new();
        for i in 0..5u8 {
            want.extend(set_ops(header(SET, &[0x4C, 0x01, 0, i], None), &[0]));
        }
        assert_eq!(rec.take(), want);
    }
}
