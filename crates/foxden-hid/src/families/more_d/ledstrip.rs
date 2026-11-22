// SPDX-License-Identifier: GPL-2.0-or-later

use std::net::UdpSocket;

use super::common::*;
use super::serial::Serial;
use foxden_core::{Backend, Color, DeviceKind};

const FAMILY: &str = "ledstrip";

/// The wire format a strip's microcontroller speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedProtocol {
    KeyboardVisualizer,
    Adalight,
    Tpm2,
}

impl LedProtocol {
    /// The names OpenRGB's `LEDStripDevices` settings use; `basic_i2c` is not supported.
    pub fn from_name(name: &str) -> Option<LedProtocol> {
        match name {
            "keyboard_visualizer" => Some(LedProtocol::KeyboardVisualizer),
            "adalight" => Some(LedProtocol::Adalight),
            "tpm2" => Some(LedProtocol::Tpm2),
            _ => None,
        }
    }
}

/// One entry of OpenRGB's `LEDStripDevices` settings. `port` is `COMx`, or `udp:host` with
/// `baud` holding the UDP port (Keyboard Visualizer only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedStrip {
    pub name: String,
    pub port: String,
    pub baud: u32,
    pub leds: usize,
    pub protocol: LedProtocol,
}

pub fn packet(protocol: LedProtocol, colors: &[Color]) -> Vec<u8> {
    let rgb = colors.iter().flat_map(|c| [c.0, c.1, c.2]);
    let n = colors.len();
    match protocol {
        LedProtocol::KeyboardVisualizer => {
            let mut b = vec![0xAA];
            b.extend(rgb);
            let sum = b.iter().fold(0u16, |s, &v| s.wrapping_add(v as u16));
            b.extend(sum.to_be_bytes());
            b
        }
        LedProtocol::Adalight => {
            let (hi, lo) = ((n >> 8) as u8, n as u8);
            let mut b = vec![0x41, 0x64, 0x61, hi, lo, hi ^ lo ^ 0x55];
            b.extend(rgb);
            b
        }
        LedProtocol::Tpm2 => {
            let len = n * 3;
            let mut b = vec![0xC9, 0xDA, (len >> 8) as u8, len as u8];
            b.extend(rgb);
            b.push(0x36);
            b
        }
    }
}

pub struct Strip {
    protocol: LedProtocol,
}

impl Proto for Strip {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        io.write(&packet(self.protocol, colors)).map(|_| ())
    }
}

struct Udp(UdpSocket);

impl Io for Udp {
    fn write(&mut self, data: &[u8]) -> Result<usize, String> {
        self.0.send(data).map_err(|e| e.to_string())
    }
    fn read(&mut self, _buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
        Ok(0)
    }
}

fn open(strip: &LedStrip) -> Option<Box<dyn Backend>> {
    let io: Box<dyn Io> = match strip.port.strip_prefix("udp:") {
        Some(host) if strip.protocol == LedProtocol::KeyboardVisualizer => {
            let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
            socket.connect((host, strip.baud as u16)).ok()?;
            Box::new(Udp(socket))
        }
        Some(_) => return None,
        None => Box::new(Serial::open(strip.port.trim_end_matches('\r'), strip.baud, false).ok()?),
    };
    let dev = rows(
        named_id(FAMILY, &strip.port),
        &strip.name,
        DeviceKind::Strip,
        &[strip.leds],
    );
    let protocol = strip.protocol;
    Some(boxed(io, dev, FAMILY, Strip { protocol }))
}

/// DetectLEDStripControllers: stops at the first entry missing its port, baud or LED count.
pub fn led_strips(strips: &[LedStrip]) -> Vec<Box<dyn Backend>> {
    strips
        .iter()
        .take_while(|s| !s.port.is_empty() && s.baud > 0 && s.leds > 0)
        .filter_map(open)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn protocols() {
        let a = [Color(1, 2, 3), Color(0xFF, 0xFF, 0xFF)];
        let [first, _, _] = pushes(
            Strip {
                protocol: LedProtocol::KeyboardVisualizer,
            },
            2,
            vec![],
            &a,
            &[],
        );
        let sum: u16 = 0xAA + 1 + 2 + 3 + 3 * 0xFF;
        assert_eq!(
            first,
            vec![w(vec![
                0xAA,
                1,
                2,
                3,
                0xFF,
                0xFF,
                0xFF,
                (sum >> 8) as u8,
                sum as u8
            ])]
        );
        assert_eq!(
            packet(LedProtocol::Adalight, &a),
            vec![0x41, 0x64, 0x61, 0, 2, 0x57, 1, 2, 3, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            packet(LedProtocol::Tpm2, &a),
            vec![0xC9, 0xDA, 0, 6, 1, 2, 3, 0xFF, 0xFF, 0xFF, 0x36]
        );
        assert_eq!(LedProtocol::from_name("basic_i2c"), None);
    }

    #[test]
    fn incomplete_entries_stop_detection() {
        let bad = LedStrip {
            name: "LED Strip".into(),
            port: String::new(),
            baud: 115_200,
            leds: 10,
            protocol: LedProtocol::Adalight,
        };
        assert!(led_strips(&[bad]).is_empty());
    }
}
