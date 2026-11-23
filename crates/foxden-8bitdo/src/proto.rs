//! The vendor channel: 64-byte output report 0x81 out, input report 0x02 back.
//!
//! Every request carries a 16-byte header after `04`: command, argument, length, total and
//! offset. Newer pads (`Wire::long`) drop the length byte after the report id, and pads with
//! `Wire::crc` carry a CRC-16 of the payload in the top half of the length field.

pub const OUT: u8 = 0x81;
pub const IN: u8 = 0x02;
pub const REPORT: usize = 64;
/// Most config bytes one request carries.
pub const CHUNK: usize = 45;
const HEADER: usize = 16;
/// The vendor library sends uninitialised stack (MSVC fills it with 0xCC) as a read's payload.
const FILL: u8 = 0xCC;

pub mod cmd {
    pub const WRITE: u16 = 0x0001;
    pub const READ: u16 = 0x0002;
    pub const COMMIT: u16 = 0x0006;
    pub const REPORTS: u16 = 0x0007;
    pub const SET_THEME: u16 = 0x0040;
    pub const THEME: u16 = 0x0041;
    pub const INFO: u16 = 0x0100;
    pub const LINK: u16 = 0x0120;
    pub const LEVEL: u16 = 0x0121;
}

/// The argument of `cmd::COMMIT`.
pub const COMMIT_ARG: u16 = 0x0123;
/// The status word of an answer to a config request.
pub const ACK: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Wire {
    /// `81 04 <header>` instead of `81 <n> 04 <header>`.
    pub long: bool,
    /// CRC-16 of the payload in the length field's high half.
    pub crc: bool,
}

/// CRC-16/MODBUS: reflected 0x8005, starting at 0xFFFF, as the vendor library computes it.
pub fn crc16(data: &[u8]) -> u16 {
    let mut c = 0xFFFFu16;
    for &b in data {
        let mut b = b;
        for _ in 0..8 {
            c = (c >> 1) ^ if (c ^ b as u16) & 1 != 0 { 0xA001 } else { 0 };
            b >>= 1;
        }
    }
    c
}

/// One request. `data` is the payload (cut to `CHUNK`); `None` sends the vendor's read filler.
pub fn request(
    w: Wire,
    cmd: u16,
    arg: u16,
    data: Option<&[u8]>,
    len: usize,
    total: u32,
    offset: u32,
) -> [u8; REPORT] {
    let len = len.min(CHUNK);
    let mut body = [FILL; HEADER + CHUNK];
    body[0..2].copy_from_slice(&cmd.to_le_bytes());
    body[2..4].copy_from_slice(&arg.to_le_bytes());
    let mut field = len as u32;
    match data {
        Some(d) => {
            let d = &d[..d.len().min(len)];
            body[HEADER..HEADER + d.len()].copy_from_slice(d);
            body[HEADER + d.len()..].fill(0);
        }
        None if len == 0 => body[HEADER..].fill(0),
        None => {}
    }
    if w.crc {
        field |= (crc16(&body[HEADER..HEADER + len]) as u32) << 16;
    }
    body[4..8].copy_from_slice(&field.to_le_bytes());
    body[8..12].copy_from_slice(&total.to_le_bytes());
    body[12..16].copy_from_slice(&offset.to_le_bytes());
    frame(w, &body[..HEADER + len])
}

/// A request with only a header: no payload, no filler.
pub fn short(w: Wire, cmd: u16, arg: u16) -> [u8; REPORT] {
    let mut body = [0u8; HEADER];
    body[0..2].copy_from_slice(&cmd.to_le_bytes());
    body[2..4].copy_from_slice(&arg.to_le_bytes());
    frame(w, &body)
}

/// A request the way `set_theme` sends it: a CRC of no payload on pads that check one.
pub fn bare(w: Wire, cmd: u16, arg: u16) -> [u8; REPORT] {
    request(w, cmd, arg, Some(&[]), 0, 0, 0)
}

fn frame(w: Wire, body: &[u8]) -> [u8; REPORT] {
    let n = body.len().min(61);
    let mut r = [0u8; REPORT];
    r[0] = OUT;
    let at = if w.long {
        1
    } else {
        r[1] = n as u8 + 1;
        2
    };
    r[at] = 0x04;
    r[at + 1..at + 1 + n].copy_from_slice(&body[..n]);
    r
}

/// An answer: `02 04 <status> <cmd> <len> <total> <offset> <data>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub cmd: u16,
    /// Bytes carried or accepted.
    pub count: u16,
    pub data: Vec<u8>,
}

pub fn reply(r: &[u8]) -> Option<Reply> {
    if r.len() < 18 || r[0] != IN || r[1] != 0x04 {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]);
    Some(Reply {
        status: u16_at(2),
        cmd: u16_at(4),
        count: u16_at(6),
        data: r[18..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Wire = Wire {
        long: true,
        crc: true,
    };
    const OLD: Wire = Wire {
        long: false,
        crc: false,
    };

    #[test]
    fn crc_is_modbus() {
        assert_eq!(crc16(b"123456789"), 0x4B37);
        assert_eq!(crc16(&[]), 0xFFFF);
    }

    #[test]
    fn battery_queries_match_the_battery_driver() {
        let r = request(
            Wire {
                long: true,
                crc: false,
            },
            cmd::LEVEL,
            0,
            None,
            6,
            0,
            0,
        );
        assert_eq!(&r[..2], &[0x81, 0x04]);
        assert_eq!(&r[2..8], &[0x21, 0x01, 0x00, 0x00, 0x06, 0x00]);
    }

    #[test]
    fn write_chunk_long_with_crc() {
        let data: Vec<u8> = (0..60).collect();
        let r = request(LONG, cmd::WRITE, 0, Some(&data), 60, 0x638, 0x50C);
        let crc = crc16(&data[..45]);
        let mut want = vec![0x81, 0x04, 0x01, 0x00, 0x00, 0x00, 45, 0x00];
        want.extend(crc.to_le_bytes());
        want.extend([0x38, 0x06, 0, 0, 0x0C, 0x05, 0, 0]);
        want.extend(&data[..45]);
        want.resize(64, 0);
        assert_eq!(r.to_vec(), want);
    }

    #[test]
    fn read_short_without_crc() {
        let r = request(OLD, cmd::READ, 0, None, 100, 0x914, 0);
        assert_eq!(&r[..3], &[0x81, 16 + 45 + 1, 0x04]);
        assert_eq!(&r[3..7], &[0x02, 0x00, 0x00, 0x00]);
        assert_eq!(&r[7..11], &[45, 0, 0, 0]);
        assert_eq!(&r[11..15], &[0x14, 0x09, 0, 0]);
        assert!(r[19..64].iter().all(|&b| b == 0xCC));
    }

    #[test]
    fn theme_carries_its_argument_and_an_empty_crc() {
        let r = bare(LONG, cmd::SET_THEME, 4);
        assert_eq!(
            &r[..10],
            &[0x81, 0x04, 0x40, 0x00, 0x04, 0x00, 0, 0, 0xFF, 0xFF]
        );
        let r = short(LONG, cmd::COMMIT, COMMIT_ARG);
        assert_eq!(&r[..10], &[0x81, 0x04, 0x06, 0x00, 0x23, 0x01, 0, 0, 0, 0]);
        let r = short(OLD, cmd::REPORTS, 1);
        assert_eq!(&r[..8], &[0x81, 17, 0x04, 0x07, 0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn replies() {
        let mut r = vec![0u8; 64];
        r[..8].copy_from_slice(&[0x02, 0x04, 0x04, 0x00, 0x02, 0x00, 0x2D, 0x00]);
        r[18] = 7;
        let a = reply(&r).unwrap();
        assert_eq!(
            (a.status, a.cmd, a.count, a.data[0]),
            (ACK, cmd::READ, 45, 7)
        );
        r[0] = 0x01;
        assert!(reply(&r).is_none());
    }
}
