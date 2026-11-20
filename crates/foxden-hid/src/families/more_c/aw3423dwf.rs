//! Ported from OpenRGB Controllers/AlienwareMonitorController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

const FAMILY: &str = "alienware-aw3423dwf";
/// Logo, number and power button; 0x0B sets all three.
const LEDS: [u8; 3] = [0x01, 0x02, 0x08];
const ALL: u8 = 0x0B;
const OEM_KEY: [u8; 12] = [
    0xF5, 0x3F, 0xC1, 0x39, 0x44, 0x3A, 0x31, 0x79, 0x0D, 0xB1, 0x82, 0x76,
];

/// The login challenge is an input report, so `read` fetches one here.
struct InputReports(HidDevice);

impl Io for InputReports {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        Io::write(&mut self.0, data)
    }
    fn feature(&mut self, data: &[u8]) -> Result<(), String> {
        Io::feature(&mut self.0, data)
    }
    fn read(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        self.0.get_input_report(buf).map_err(|e| e.to_string())
    }
    fn sleep(&mut self, ms: u64) {
        Io::sleep(&mut self.0, ms)
    }
}

fn syn_key() -> [u8; 8] {
    let mut key = [0u8; 8];
    let (mut sk, mut ok) = (0, 0);
    while ok < OEM_KEY.len() && sk < 8 {
        let len = (OEM_KEY[ok] & 1) + ((OEM_KEY[ok] & 0x10) >> 4);
        for _ in 0..len {
            if sk == 8 {
                break;
            }
            key[sk] = OEM_KEY[ok + 1] ^ OEM_KEY[ok];
            ok += 1;
            sk += 1;
        }
        ok += 1;
    }
    key
}

/// GenerateKey: picks half of the 17-byte challenge by parity and folds it into the OEM key.
fn login_key(r: &[u8; 17]) -> [u8; 8] {
    let v = r[15] as u16 | (r[0] as u16) << 8;
    let mut out = [0u8; 8];
    if v.count_ones() % 2 == 1 {
        out.copy_from_slice(&r[..8]);
        let i = (r[14] & 7) as usize;
        out[i] ^= r[i + 8];
    } else {
        out.copy_from_slice(&r[8..16]);
        let i = (r[6] & 7) as usize;
        out[i] ^= r[i];
    }
    let mut key = syn_key();
    for (k, o) in key.iter_mut().zip(out) {
        *k ^= o;
    }
    key
}

fn control(head: &[u8], key: Option<[u8; 8]>) -> [u8; 193] {
    let mut b = [0u8; 193];
    b[1..1 + head.len()].copy_from_slice(head);
    if let Some(k) = key {
        b[65..73].copy_from_slice(&k);
    }
    b
}

/// SendColor: a fresh login, then the colour, then 200 ms.
fn send_colour(io: &mut dyn Io, led: u8, c: Color) -> Result<(), String> {
    io.write(&control(&[0x40, 0xE1, 0x01], None))?;
    let mut reply = [0u8; 193];
    let _ = io.read(&mut reply, 0);
    let mut challenge = [0u8; 17];
    challenge.copy_from_slice(&reply[1..18]);
    io.write(&control(&[0x40, 0xE1, 0x02], Some(login_key(&challenge))))?;
    let led_2 = match led {
        0x01 => 0xF6,
        0x02 => 0xF5,
        0x08 => 0xFF,
        _ => 0xFC,
    };
    let mut p = control(&[0x40, 0xC6, 0, 0, 0, 0, 0x0A, 0, 0x6E, 0, 0x82], None);
    p[65..75].copy_from_slice(&[0x51, 0x87, 0xD0, 0x04, led, c.0, c.1, c.2, 0x64, led_2]);
    io.write(&p)?;
    io.sleep(200);
    Ok(())
}

struct Aw3423dwf;

impl Proto for Aw3423dwf {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        if colors[0] == colors[1] && colors[1] == colors[2] {
            return send_colour(io, ALL, colors[0]);
        }
        for (&led, &c) in LEDS.iter().zip(colors) {
            send_colour(io, led, c)?;
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = Box::new(InputReports(info.open(api)?));
    let dev = line(info, FAMILY, d, DeviceKind::Other, LEDS.len());
    Ok(Box::new(Driver::new(io, dev, FAMILY, Aw3423dwf)))
}

pub static DETECTORS: &[Detector] = &[det(
    "Alienware AW3423DWF",
    0x187C,
    0x100E,
    ANY,
    ANY,
    ANY,
    open,
)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // pairs of OEM key bytes xored: F5^3F, 3F^C1, 39^44, 44^3A, 31^79, 79^0D, B1^82, 82^76
    const SYN: [u8; 8] = [0xCA, 0xFE, 0x7D, 0x7E, 0x48, 0x74, 0x33, 0xF4];

    #[test]
    fn login_key_by_parity() {
        assert_eq!(syn_key(), SYN);
        assert_eq!(login_key(&[0; 17]), SYN);
        // one bit set in byte 0: odd, so the first half with byte 0 xored by byte 8
        let mut r = [0u8; 17];
        r[0] = 0x01;
        r[8] = 0x10;
        let mut want = SYN;
        want[0] ^= 0x11;
        assert_eq!(login_key(&r), want);
    }

    #[test]
    fn login_then_colour() {
        let a = vec![Color(1, 2, 3); 3];
        let b = vec![Color(1, 2, 3), Color::BLACK, Color::BLACK];
        let [first, _, next] = pushes(Aw3423dwf, 3, vec![], &a, &b);
        assert_eq!(first.len(), 5);
        assert_eq!(first[0], w(pad(&[0, 0x40, 0xE1, 0x01], 193)));
        assert_eq!(first[1], Op::Read);
        let mut login = pad(&[0, 0x40, 0xE1, 0x02], 193);
        login[65..73].copy_from_slice(&SYN);
        assert_eq!(first[2], w(login));
        let mut colour = pad(&[0, 0x40, 0xC6, 0, 0, 0, 0, 0x0A, 0, 0x6E, 0, 0x82], 193);
        colour[65..75].copy_from_slice(&[0x51, 0x87, 0xD0, 0x04, 0x0B, 1, 2, 3, 0x64, 0xFC]);
        assert_eq!(first[3], w(colour));
        assert_eq!(first[4], Op::Sleep(200));
        assert_eq!(next.len(), 15);
        let Op::W(power) = &next[13] else { panic!() };
        assert_eq!(power[69..75], [0x08, 0, 0, 0, 0x64, 0xFF]);
    }
}
