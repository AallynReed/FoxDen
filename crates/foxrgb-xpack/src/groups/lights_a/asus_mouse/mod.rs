//! ASUS mice and the ASUS Omni receiver (protocol learned from SignalRGB's Asus_Mouse.js, ASUS_Omni_Device.js, ASUS_Keyboard_Protocol.js and ASUS_Mouse_Protocol.js).

mod models;
mod omni_devices;

use super::common::{KEYBOARD_RECT, MOUSE_RECT, asus_direct, full, keyed};
use crate::hid::{Driver, Io, Proto, drain, open_io, open_sibling, read_until};
use foxrgb_core::{Backend, Color, DeviceKind};
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

const VID: u16 = 0x0B05;
const LEN: usize = 65;
const OMNI_LEN: usize = 64;
const DIRECT_MODE: [u8; 7] = [0x51, 0x28, 0x03, 0x00, 0x02, 0x64, 0x02];
/// Omni keyboards take 14 LEDs per packet.
const OMNI_KB_PER_PACKET: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wire {
    /// One `0x51 0x28` packet per LED with a brightness byte.
    Legacy,
    /// Direct mode once, then every LED in one `0x51 0x29 0xFF` packet.
    Modern,
}

pub struct Model {
    pub size: [u8; 2],
    pub cells: &'static [[u8; 2]],
    pub wire: Wire,
    pub brightness: u8,
}

pub const fn mouse(
    name: &'static str,
    pid: u16,
    interface: i32,
    page: u16,
    usage: u16,
) -> Detector {
    full(name, VID, pid, interface, page, usage, open_mouse)
}

/// `[report, 0x51, 0x29, a, 0, b, r, g, b ...]` with every LED's colour.
pub fn bulk(report: u8, a: u8, b: u8, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![report, 0x51, 0x29, a, 0x00, b];
    p.extend(colors.iter().flat_map(|c| [c.0, c.1, c.2]));
    p
}

pub fn legacy(index: u8, brightness: u8, c: Color) -> Vec<u8> {
    vec![
        0x00, 0x51, 0x28, index, 0x00, 0x00, brightness, c.0, c.1, c.2,
    ]
}

fn with_report(report: u8, body: &[u8]) -> Vec<u8> {
    let mut p = vec![report];
    p.extend_from_slice(body);
    p
}

pub fn mouse_packets(wire: Wire, brightness: u8, colors: &[Color]) -> Vec<Vec<u8>> {
    match wire {
        Wire::Legacy => colors
            .iter()
            .enumerate()
            .map(|(i, c)| legacy(i as u8, brightness, *c))
            .collect(),
        Wire::Modern => vec![bulk(0, 0xFF, 0, colors)],
    }
}

struct Mouse {
    wire: Wire,
    brightness: u8,
}

impl Proto for Mouse {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        if self.wire == Wire::Modern {
            io.write(&with_report(0, &DIRECT_MODE), LEN)?;
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in mouse_packets(self.wire, self.brightness, colors) {
            io.write(&p, LEN)?;
            if self.wire == Wire::Legacy {
                io.sleep(1);
            }
        }
        Ok(())
    }
}

fn open_mouse(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let m = models::PIDS
        .iter()
        .find(|(pid, _)| *pid == info.pid)
        .map(|(_, m)| *m)
        .ok_or("unknown ASUS mouse")?;
    let dev = keyed(
        &info.device_id("asus-mouse"),
        d.name,
        DeviceKind::Mouse,
        MOUSE_RECT,
        [m.size[0] as u32, m.size[1] as u32],
        m.cells,
    );
    let p = Mouse {
        wire: m.wire,
        brightness: m.brightness,
    };
    Ok(Driver::new(open_io(api, info)?, dev, "asus-mouse", p).boxed())
}

pub struct Linked {
    pub name: &'static str,
    pub size: [u8; 2],
    pub leds: &'static [u8],
    pub cells: &'static [[u8; 2]],
}

/// Omni keyboard frame on report 2: up to 14 LEDs, byte 3 is this packet's LED count.
pub fn omni_keyboard_packets(leds: &[u8], colors: &[Color]) -> Vec<Vec<u8>> {
    let slots: Vec<_> = leds
        .iter()
        .zip(
            colors
                .iter()
                .copied()
                .chain(std::iter::repeat(Color::BLACK)),
        )
        .map(|(i, c)| (*i, c))
        .collect();
    slots
        .chunks(OMNI_KB_PER_PACKET)
        .map(|ch| {
            let mut p = asus_direct(0x02, ch.len() as u8, ch);
            p.truncate(OMNI_LEN);
            p
        })
        .collect()
}

/// Pid of the linked device from the `0x01 0xA0` reply: byte 6 high, byte 5 minus 2 low.
pub fn omni_pid(reply: &[u8]) -> Option<u16> {
    let lo = reply.get(5)?.wrapping_sub(2);
    Some(u16::from_be_bytes([*reply.get(6)?, lo]))
}

fn exchange(io: &mut dyn Io, q: &[u8]) -> Option<Vec<u8>> {
    drain(io, OMNI_LEN);
    io.write(q, OMNI_LEN).ok()?;
    io.sleep(10);
    read_until(io, OMNI_LEN, 200, |r| r.first() == q.first()).ok()
}

/// The receiver reports a connected device with byte 5 = 1; the plugin tries four times.
fn connected(io: &mut dyn Io, q: &[u8]) -> bool {
    (0..4).any(|_| {
        let ok = exchange(io, q).is_some_and(|r| r.get(5) == Some(&1));
        if !ok {
            io.sleep(50);
        }
        ok
    })
}

struct OmniKeyboard {
    leds: &'static [u8],
}

impl Proto for OmniKeyboard {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in omni_keyboard_packets(self.leds, colors) {
            io.write(&p, OMNI_LEN)?;
        }
        Ok(())
    }
}

struct OmniMouse;

impl Proto for OmniMouse {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&with_report(0x03, &DIRECT_MODE), OMNI_LEN)
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut p = bulk(0x03, 0xFF, 0, colors);
        p.truncate(OMNI_LEN);
        io.write(&p, OMNI_LEN)
    }
}

fn open_omni(api: &HidApi, info: &Info, _d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut id = open_io(api, info)?;
    let reply = exchange(id.as_mut(), &[0x01, 0xA0]).ok_or("Omni receiver did not answer")?;
    let pid = omni_pid(&reply).ok_or("short Omni reply")?;
    let find = |t: &[(u16, &'static Linked)]| t.iter().find(|(p, _)| *p == pid).map(|(_, l)| *l);
    let id_of = info.device_id("asus-omni");
    if let Some(l) = find(omni_devices::KEYBOARDS) {
        let mut io: Box<dyn Io> =
            Box::new(open_sibling(api, info, Some(2), Some(0xFF00), Some(1))?);
        if !connected(io.as_mut(), &[0x02, 0x12, 0x03]) {
            return Err("no keyboard connected to the Omni receiver".into());
        }
        let size = [l.size[0] as u32, l.size[1] as u32];
        let dev = keyed(
            &id_of,
            l.name,
            DeviceKind::Keyboard,
            KEYBOARD_RECT,
            size,
            l.cells,
        );
        return Ok(Driver::new(io, dev, "asus-omni", OmniKeyboard { leds: l.leds }).boxed());
    }
    if let Some(l) = find(omni_devices::MICE) {
        let mut io: Box<dyn Io> =
            Box::new(open_sibling(api, info, Some(2), Some(0xFF01), Some(1))?);
        if !connected(io.as_mut(), &[0x03, 0x12, 0x00, 0x02]) {
            return Err("no mouse connected to the Omni receiver".into());
        }
        let size = [l.size[0] as u32, l.size[1] as u32];
        let dev = keyed(&id_of, l.name, DeviceKind::Mouse, MOUSE_RECT, size, l.cells);
        return Ok(Driver::new(io, dev, "asus-omni", OmniMouse).boxed());
    }
    Err(format!(
        "device {pid:04x} behind the Omni receiver is not in the library"
    ))
}

static OMNI: &[Detector] = &[full(
    "ASUS Omni Receiver",
    VID,
    0x1ACE,
    2,
    0xFF02,
    1,
    open_omni,
)];

pub fn detectors() -> Vec<&'static Detector> {
    models::DETECTORS.iter().chain(OMNI.iter()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_is_one_packet_per_led() {
        let p = mouse_packets(Wire::Legacy, 4, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(
            p,
            vec![legacy(0, 4, Color(1, 2, 3)), legacy(1, 4, Color(4, 5, 6))]
        );
        assert_eq!(p[1], vec![0, 0x51, 0x28, 1, 0, 0, 4, 4, 5, 6]);
    }

    #[test]
    fn modern() {
        let c = [Color(1, 2, 3), Color(4, 5, 6)];
        assert_eq!(
            mouse_packets(Wire::Modern, 4, &c),
            vec![vec![0, 0x51, 0x29, 0xFF, 0, 0, 1, 2, 3, 4, 5, 6]]
        );
    }

    #[test]
    fn omni_keyboard_and_pid() {
        let leds: Vec<u8> = (0..20).collect();
        let p = omni_keyboard_packets(&leds, &[Color(9, 9, 9); 20]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..5], [2, 0xC0, 0x81, 14, 0]);
        assert_eq!(p[1][..9], [2, 0xC0, 0x81, 6, 0, 14, 9, 9, 9]);
        assert_eq!(p[0].len(), 64);
        assert_eq!(omni_pid(&[1, 0xA0, 0, 0, 0, 0x85, 0x1A]), Some(0x1A83));
    }

    #[test]
    fn tables_match() {
        assert_eq!(models::PIDS.len(), models::DETECTORS.len());
        for (_, l) in omni_devices::KEYBOARDS {
            assert_eq!(l.leds.len(), l.cells.len());
        }
    }
}
