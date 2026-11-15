//! Ported from OpenRGB Controllers/RealtekBridgeController (GPL-2.0-or-later).

use super::common::*;
use super::realtek::{GET, SET, SYNC_OPENRGB, UNLOCK_ADDR, header, ioctl};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "realtek-bridge";
const HDR_SIZE: usize = 64;
const DATA_SIZE: usize = 3220;

/// What the constructor and detector read from the bridge.
pub struct State {
    pub hdr: [u8; HDR_SIZE],
    pub supported: bool,
}

fn checked(ok: bool, what: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

/// set_appctl.
fn appctl(io: &mut dyn Io, on: u8) -> Result<bool, String> {
    ioctl(
        io,
        header(SET, &[0x4C, 0x01, on], None),
        &mut [on],
        0,
        false,
    )
}

/// RealtekBridgeController constructor, get_support_openrgb and get_fw_ver, in OpenRGB's order.
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
    let mut data = vec![0u8; DATA_SIZE];
    let at = Some(HDR_SIZE as u32);
    ioctl(io, header(GET, &[0xCC, 0x02], at), &mut data, 0, true)?;
    appctl(io, 1)?;
    let mut support = [0u8];
    let ok = ioctl(
        io,
        header(GET, &[0xCC, 0x04, SYNC_OPENRGB], None),
        &mut support,
        0,
        true,
    )?;
    let supported = ok && support[0] != 0;
    if supported {
        ioctl(io, header(GET, &[0xA5], None), &mut [0u8; 20], 0, true)?;
    } else {
        appctl(io, 0)?;
    }
    Ok(State { hdr, supported })
}

/// get_argb_num.
pub fn argb_num(hdr: &[u8; HDR_SIZE]) -> i32 {
    i32::from_le_bytes(hdr[24..28].try_into().unwrap())
}

struct Bridge {
    hdr: [u8; HDR_SIZE],
    brightness: u16,
    prev_bright: Option<u16>,
}

impl Proto for Bridge {
    /// set_argb_direct with Direct's brightness, read from the header at detection.
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        checked(appctl(io, 1)?, "set_appctl")?;
        if self.prev_bright != Some(self.brightness) {
            self.prev_bright = Some(self.brightness);
            self.hdr[28..30].copy_from_slice(&(self.brightness << 8).to_le_bytes());
            let mut hdr = self.hdr;
            ioctl(io, header(SET, &[0x4C, 0x02], None), &mut hdr, 0, false)?;
        }
        let mut buf: Vec<u8> = colors.iter().flat_map(|c| rgb(*c)).collect();
        let head = header(SET, &[0x4C, 0x03, SYNC_OPENRGB], Some(colors.len() as u32));
        checked(ioctl(io, head, &mut buf, 0, false)?, "set_direct")
    }

    /// The destructor's set_appctl(false).
    fn stop(&mut self, io: &mut dyn Io) {
        let _ = appctl(io, 0);
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let state = detect(io.as_mut())?;
    if !state.supported {
        return Err("bridge does not support OpenRGB sync".into());
    }
    let n = argb_num(&state.hdr);
    if n <= 0 || n as usize * 3 > 4096 {
        appctl(io.as_mut(), 0)?;
        return Err(format!("bridge reports {n} LEDs"));
    }
    let brightness = u16::from_le_bytes([state.hdr[28], state.hdr[29]]) >> 8;
    let dev = line(info, FAMILY, d, DeviceKind::Strip, n as usize);
    let proto = Bridge {
        hdr: state.hdr,
        brightness,
        prev_bright: None,
    };
    Ok(boxed(io, FAMILY, dev, proto))
}

pub static DETECTORS: [Detector; 3] = [
    det("RTL9220", 0x0BDA, 0x9220, ANY, 0xFF00, 0x0001, open),
    det("RTL9201", 0x0BDA, 0x9201, ANY, 0xFF00, 0x0001, open),
    det("RTL9210", 0x0BDA, 0x9210, ANY, 0xFF00, 0x0001, open),
];

#[cfg(test)]
mod tests {
    use super::super::realtek::tests::{get_ops, set_ops};
    use super::*;
    use rec::*;

    const OK: [u8; 2] = [0x58, 0x53];

    fn hdr_reply() -> Vec<u8> {
        let mut h = vec![0u8; 64];
        h[0] = 0x59;
        h[24] = 3;
        h[29] = 0x80;
        h
    }

    // RealtekBridgeController constructor, get_support_openrgb, get_fw_ver
    #[test]
    fn detection_reads() {
        let replies = vec![
            vec![0x5A],
            OK.to_vec(),
            hdr_reply(),
            OK.to_vec(),
            vec![0x57],
            OK.to_vec(),
            OK.to_vec(),
            vec![0x59],
            OK.to_vec(),
            vec![0x59],
            OK.to_vec(),
        ];
        let mut rec = Rec::with(replies);
        let state = detect(&mut rec).unwrap();
        assert!(state.supported);
        assert_eq!(argb_num(&state.hdr), 3);
        assert_eq!(state.hdr[29], 0x80);
        let ops = rec.take();
        let want = [
            get_ops(header(GET, &[0x92], Some(0xAC00_4000)), 96),
            get_ops(header(GET, &[0xCC, 0x02], None), 64),
            get_ops(header(GET, &[0xCC, 0x02], Some(64)), 3220),
            set_ops(header(SET, &[0x4C, 0x01, 1], None), &[1]),
            get_ops(header(GET, &[0xCC, 0x04, 6], None), 1),
            get_ops(header(GET, &[0xA5], None), 20),
        ]
        .concat();
        assert_eq!(ops, want);
        assert_eq!(rec.left(), 0);
    }

    // get_support_openrgb answering 0: the detector deletes the controller, whose destructor releases
    #[test]
    fn unsupported_bridge() {
        let mut replies = vec![vec![1], OK.to_vec()];
        replies.extend([hdr_reply(), OK.to_vec(), vec![1], OK.to_vec(), OK.to_vec()]);
        replies.extend([vec![0], OK.to_vec(), OK.to_vec()]);
        let mut rec = Rec::with(replies);
        assert!(!detect(&mut rec).unwrap().supported);
        let ops = rec.take();
        assert_eq!(ops.len(), 18);
        assert_eq!(
            ops[15..],
            set_ops(header(SET, &[0x4C, 0x01, 0], None), &[0])
        );
    }

    // ~RealtekBridgeController: set_appctl(false), even when no frame was sent
    #[test]
    fn drop_releases_app_control() {
        let rec = Rec::with(vec![OK.to_vec()]);
        let proto = Bridge {
            hdr: [0; 64],
            brightness: 0,
            prev_bright: None,
        };
        drop(Driver::new(Box::new(rec.clone()), dev(1), "t", proto));
        assert_eq!(
            rec.take(),
            set_ops(header(SET, &[0x4C, 0x01, 0], None), &[0])
        );
    }

    // set_argb_direct: appctl, brightness header once, then set_direct
    #[test]
    fn direct_frames() {
        let mut hdr = [0u8; 64];
        hdr[0] = 0x59;
        hdr[24] = 2;
        hdr[29] = 0x80;
        let proto = Bridge {
            hdr,
            brightness: 0x80,
            prev_bright: None,
        };
        let a = [Color(1, 2, 3), Color(4, 5, 6)];
        let b = [Color(7, 8, 9), Color(4, 5, 6)];
        let replies = vec![OK.to_vec(); 6];
        let [first, again, next] = pushes(proto, 2, replies, &a, &b);
        assert!(again.is_empty());
        let appctl = set_ops(header(SET, &[0x4C, 0x01, 1], None), &[1]);
        let mut new_hdr = hdr;
        new_hdr[28..30].copy_from_slice(&[0x00, 0x80]);
        let direct = |c: &[u8]| set_ops(header(SET, &[0x4C, 0x03, 6], Some(2)), c);
        assert_eq!(
            first,
            [
                appctl.clone(),
                set_ops(header(SET, &[0x4C, 0x02], None), &new_hdr),
                direct(&[1, 2, 3, 4, 5, 6]),
            ]
            .concat()
        );
        assert_eq!(next, [appctl, direct(&[7, 8, 9, 4, 5, 6])].concat());
    }
}
