// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::Io;

pub const SET: [u8; 31] = [
    0x56, 0x53, 0x42, 0x43, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0xE3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0,
];
pub const GET: [u8; 31] = [
    0x56, 0x53, 0x42, 0x43, 0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0, 0x80, 0, 0x10, 0xE2, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
pub const END: [u8; 13] = [0x58, 0x53, 0x42, 0x53, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub const SYNC_OPENRGB: u8 = 6;
pub const UNLOCK_ADDR: u32 = 0xAC00_4000;

/// usb_hid_get_report: the data report id and length that fit `len` bytes.
pub fn report(len: usize) -> (u8, usize) {
    match len {
        0..=64 => (0x59, 64),
        65..=512 => (0x5A, 512),
        513..=1200 => (0x5B, 1200),
        1201..=2048 => (0x5C, 2048),
        _ => (0x57, 4096),
    }
}

/// A command header: `base` with `cmd` from byte 0x13 and `arg` at 0x17.
pub fn header(base: [u8; 31], cmd: &[u8], arg: Option<u32>) -> [u8; 31] {
    let mut h = base;
    h[0x13..0x13 + cmd.len()].copy_from_slice(cmd);
    if let Some(a) = arg {
        h[0x17..0x1B].copy_from_slice(&a.to_le_bytes());
    }
    h
}

/// usb_hid_ioctl: command header, data report in or out, then the status report.
/// Reads replace `data` with the report from byte `offset`; true when the status read returned bytes.
pub fn ioctl(
    io: &mut dyn Io,
    mut head: [u8; 31],
    data: &mut [u8],
    offset: usize,
    is_in: bool,
) -> Result<bool, String> {
    let (id, len) = report(data.len());
    if !is_in && !data.is_empty() {
        head[4] = data[0];
    }
    let n = (data.len() as u32).to_le_bytes();
    head[8..12].copy_from_slice(&n);
    head[0x1B..0x1F].copy_from_slice(&n);
    io.feature(&head)?;

    let mut buf = vec![0u8; len + offset];
    let copy = data.len().min(len);
    buf[..copy].copy_from_slice(&data[..copy]);
    buf[0] = id;
    if is_in {
        io.get_feature(&mut buf[..len])?;
        let end = data.len();
        data.copy_from_slice(&buf[offset..offset + end]);
    } else {
        io.feature(&buf[..len])?;
    }

    let mut end = END;
    Ok(io.get_feature(&mut end)? > 0)
}

#[cfg(test)]
pub mod tests {
    use super::super::common::rec::*;
    use super::*;

    /// The ops of one ioctl writing `data` with header `head`.
    pub fn set_ops(head: [u8; 31], data: &[u8]) -> Vec<Op> {
        let (id, len) = report(data.len());
        let mut h = head;
        if !data.is_empty() {
            h[4] = data[0];
        }
        let n = (data.len() as u32).to_le_bytes();
        h[8..12].copy_from_slice(&n);
        h[0x1B..0x1F].copy_from_slice(&n);
        let mut body = vec![0u8; len];
        body[..data.len()].copy_from_slice(data);
        body[0] = id;
        vec![f(h.to_vec()), f(body), Op::GetF(0x58, 13)]
    }

    /// The ops of one ioctl reading `len` bytes with header `head`.
    pub fn get_ops(head: [u8; 31], len: usize) -> Vec<Op> {
        let (id, size) = report(len);
        let mut h = head;
        let n = (len as u32).to_le_bytes();
        h[8..12].copy_from_slice(&n);
        h[0x1B..0x1F].copy_from_slice(&n);
        vec![f(h.to_vec()), Op::GetF(id, size), Op::GetF(0x58, 13)]
    }

    // usb_hid_get_report thresholds
    #[test]
    fn report_sizes() {
        assert_eq!(report(1), (0x59, 64));
        assert_eq!(report(64), (0x59, 64));
        assert_eq!(report(80), (0x5A, 512));
        assert_eq!(report(1200), (0x5B, 1200));
        assert_eq!(report(2048), (0x5C, 2048));
        assert_eq!(report(3220), (0x57, 4096));
    }

    // usb_hid_ioctl writing: data[0] goes into the header, the report id over data[0]
    #[test]
    fn ioctl_out() {
        let mut rec = Rec::with(vec![vec![0x58, 1]]);
        let head = header(SET, &[0x4C, 0x03, 6], Some(2));
        let mut data = [9, 8, 7, 6, 5, 4];
        assert!(ioctl(&mut rec, head, &mut data, 0, false).unwrap());
        let ops = rec.take();
        assert_eq!(ops, set_ops(head, &data));
        let Op::F(h) = &ops[0] else { panic!() };
        assert_eq!(h[4], 9);
        assert_eq!(&h[8..12], &[6, 0, 0, 0]);
        assert_eq!(&h[0x17..0x1F], &[2, 0, 0, 0, 6, 0, 0, 0]);
        let Op::F(b) = &ops[1] else { panic!() };
        assert_eq!(&b[..6], &[0x59, 8, 7, 6, 5, 4]);
    }

    // usb_hid_ioctl reading: data comes back from `offset`, report id included
    #[test]
    fn ioctl_in() {
        let reply: Vec<u8> = (0..64).collect();
        let mut rec = Rec::with(vec![reply, vec![]]);
        let mut data = [0u8; 4];
        assert!(!ioctl(&mut rec, header(GET, &[0xA5], None), &mut data, 0, true).unwrap());
        assert_eq!(data, [0, 1, 2, 3]);
    }
}
