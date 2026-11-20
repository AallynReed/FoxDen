//! Ported from OpenRGB Controllers/LIFXController (GPL-2.0-or-later).

use std::net::UdpSocket;
use std::time::Duration;

use foxden_core::{Color, DeviceKind};

use crate::net::{self, udp};
use crate::{Driver, Found, LifxLight, Light, rgb2hsv};

const PORT: u16 = 56700;
const PROTOCOL: u16 = 1024;
const HEADER: usize = 36;
const DEFAULT_KELVIN: u16 = 3500;
const SET_COLOR: u16 = 102;
const SET_COLOR_ZONES: u16 = 501;
const GET_COLOR_ZONES: u16 = 502;
const STATE_ZONE: u16 = 503;
const SET_EXTENDED_COLOR_ZONES: u16 = 510;
const APPLY: u8 = 1;
const EXTENDED_COLORS: usize = 82;
/// FetchZoneCount waits five seconds; a missing light should not hold up the rest that long.
const ZONE_WAIT: Duration = Duration::from_secs(1);

/// `RGBColorToHSBK`: hue * (65535/360), saturation and brightness * (65535/256), wrapped to 16 bits.
fn hsbk(c: Color) -> [u16; 4] {
    let hsv = rgb2hsv(c);
    [
        hsv.hue.wrapping_mul(65535 / 360) as u16,
        hsv.saturation as u16 * (65535 / 256),
        hsv.value as u16 * (65535 / 256),
        DEFAULT_KELVIN,
    ]
}

/// `HeaderPacketSetDefaults`: size, protocol 1024, addressable, untagged, source 2, no target.
fn packet(kind: u16, payload: usize, sequence: u8) -> Vec<u8> {
    let size = HEADER + payload;
    let mut d = vec![0u8; size];
    d[0..2].copy_from_slice(&(size as u16).to_le_bytes());
    d[2] = PROTOCOL as u8;
    d[3] = ((PROTOCOL >> 8) & 0x0F) as u8 | 0x10;
    d[4..8].copy_from_slice(&2u32.to_le_bytes());
    d[23] = sequence;
    d[32..34].copy_from_slice(&kind.to_le_bytes());
    d
}

fn put_hsbk(d: &mut [u8], at: usize, c: Color) {
    for (i, v) in hsbk(c).iter().enumerate() {
        d[at + 2 * i..at + 2 * i + 2].copy_from_slice(&v.to_le_bytes());
    }
}

fn set_color(c: Color, sequence: u8) -> Vec<u8> {
    let mut d = packet(SET_COLOR, 13, sequence);
    put_hsbk(&mut d, HEADER + 1, c);
    d
}

fn set_color_zone(c: Color, zone: u8, sequence: u8) -> Vec<u8> {
    let mut d = packet(SET_COLOR_ZONES, 15, sequence);
    d[HEADER] = zone;
    d[HEADER + 1] = zone;
    put_hsbk(&mut d, HEADER + 2, c);
    d[HEADER + 14] = APPLY;
    d
}

/// At most 82 colours fit; OpenRGB would write past the packet beyond that.
fn set_extended(colors: &[Color], sequence: u8) -> Vec<u8> {
    let mut d = packet(SET_EXTENDED_COLOR_ZONES, 664, sequence);
    let colors = &colors[..colors.len().min(EXTENDED_COLORS)];
    d[HEADER + 4] = APPLY;
    d[HEADER + 7] = colors.len() as u8;
    for (i, c) in colors.iter().enumerate() {
        put_hsbk(&mut d, HEADER + 8 + i * 8, *c);
    }
    d
}

fn get_color_zones(sequence: u8) -> Vec<u8> {
    packet(GET_COLOR_ZONES, 2, sequence)
}

/// `FetchZoneCount`'s checks on the StateZone reply, including reading the size as one byte.
fn zone_count(d: &[u8]) -> Option<u8> {
    if d.len() < HEADER + 10 {
        return None;
    }
    let protocol = u16::from_le_bytes([d[2], d[3]]) & 0x0FFF;
    let kind = u16::from_le_bytes([d[32], d[33]]);
    (d[0] as usize == HEADER + 10 && protocol == PROTOCOL && kind == STATE_ZONE)
        .then_some(d[HEADER])
}

struct Lifx {
    socket: UdpSocket,
    multizone: bool,
    extended: bool,
    zones: usize,
    sequence: u8,
    cached: Vec<Option<Color>>,
}

impl Lifx {
    fn next(&mut self) -> u8 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn write(&self, d: &[u8]) -> Result<(), String> {
        self.socket.send(d).map(drop).map_err(|e| e.to_string())
    }
}

impl Driver for Lifx {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if !self.multizone {
            let seq = self.next();
            let c = colors.first().copied().unwrap_or_default();
            return self.write(&set_color(c, seq));
        }
        if self.extended {
            let seq = self.next();
            return self.write(&set_extended(colors, seq));
        }
        for (i, c) in colors.iter().enumerate().take(self.zones) {
            if self.cached[i] == Some(*c) {
                continue;
            }
            let seq = self.next();
            self.write(&set_color_zone(*c, i as u8, seq))?;
            self.cached[i] = Some(*c);
        }
        Ok(())
    }
}

pub fn open(cfg: &LifxLight) -> Option<Light> {
    let socket = udp(&cfg.ip, PORT).ok()?;
    let mut lifx = Lifx {
        socket,
        multizone: cfg.multizone,
        extended: cfg.extended_multizone,
        zones: 1,
        sequence: 0,
        cached: Vec::new(),
    };
    if cfg.multizone {
        let seq = lifx.next();
        let _ = lifx.write(&get_color_zones(seq));
        let _ = lifx.socket.set_read_timeout(Some(ZONE_WAIT));
        let mut buf = [0u8; 128];
        if let Ok(n) = lifx.socket.recv(&mut buf)
            && let Some(count) = zone_count(&buf[..n])
        {
            lifx.zones = count as usize;
        }
    }
    lifx.cached = vec![None; lifx.zones];
    let zones = lifx.zones;
    let name = if cfg.name.is_empty() {
        "LIFX".to_string()
    } else {
        format!("LIFX {}", cfg.name)
    };
    Some(Light::new(
        format!("net:lifx:{}", cfg.ip),
        name,
        if zones > 1 {
            DeviceKind::Strip
        } else {
            DeviceKind::Other
        },
        zones,
        Box::new(lifx),
    ))
}

/// Not in OpenRGB: LIFX's documented GetService (2), tagged, sent to the broadcast address.
fn get_service() -> Vec<u8> {
    let mut d = packet(2, 0, 0);
    d[3] |= 0x20;
    d
}

pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket
        .send_to(&get_service(), ("255.255.255.255", PORT))
        .is_err()
    {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(2))
        .into_iter()
        .filter(|(_, d)| d.len() >= HEADER && u16::from_le_bytes([d[32], d[33]]) == 3)
        .map(|(ip, _)| Found {
            family: "lifx",
            ip: ip.to_string(),
            port: PORT,
            name: "LIFX".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(size: u16, kind: u16, seq: u8) -> Vec<u8> {
        let mut h = vec![0u8; 36];
        h[0] = size as u8;
        h[1] = (size >> 8) as u8;
        h[2] = 0x00;
        h[3] = 0x14;
        h[4] = 2;
        h[23] = seq;
        h[32] = kind as u8;
        h[33] = (kind >> 8) as u8;
        h
    }

    #[test]
    fn set_color_packet() {
        // red: hue 0, saturation 255*255 = 0xFE01, brightness 0xFE01, kelvin 3500 = 0x0DAC, duration 0
        let mut want = header(49, 102, 1);
        want.extend_from_slice(&[0, 0, 0, 0x01, 0xFE, 0x01, 0xFE, 0xAC, 0x0D, 0, 0, 0, 0]);
        assert_eq!(set_color(Color(255, 0, 0), 1), want);
    }

    #[test]
    fn grey_wraps_the_undefined_hue() {
        // hue -1 as unsigned * 182 truncated: 0xFFFFFFFF * 182 = ...FF4A, saturation 0
        assert_eq!(hsbk(Color(10, 10, 10)), [0xFF4A, 0, 10 * 255, 3500]);
        // green: hue 120 * 182 = 21840
        assert_eq!(hsbk(Color(0, 255, 0))[0], 21840);
    }

    #[test]
    fn set_color_zones_packet() {
        let mut want = header(51, 501, 7);
        want.extend_from_slice(&[3, 3, 0x4A, 0xFF, 0, 0, 0, 0, 0xAC, 0x0D, 0, 0, 0, 0, 1]);
        assert_eq!(set_color_zone(Color::BLACK, 3, 7), want);
    }

    #[test]
    fn extended_zones_packet() {
        let p = set_extended(&[Color(0, 0, 255), Color(255, 255, 255)], 2);
        assert_eq!(p.len(), 700);
        assert_eq!(&p[..36], header(700, 510, 2).as_slice());
        // duration 0, apply 1, zone index 0, count 2
        assert_eq!(&p[36..44], &[0, 0, 0, 0, 1, 0, 0, 2]);
        // blue: hue 240*182 = 43680 = 0xAAA0
        assert_eq!(
            &p[44..52],
            &[0xA0, 0xAA, 0x01, 0xFE, 0x01, 0xFE, 0xAC, 0x0D]
        );
        // white: grey hue 0xFF4A
        assert_eq!(&p[52..60], &[0x4A, 0xFF, 0, 0, 0x01, 0xFE, 0xAC, 0x0D]);
        assert!(p[60..].iter().all(|b| *b == 0));
        assert_eq!(set_extended(&[Color(1, 1, 1); 90], 1)[43], 82);
    }

    #[test]
    fn zone_count_reply() {
        let mut reply = header(46, 503, 9);
        reply.extend_from_slice(&[16, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(zone_count(&reply), Some(16));
        reply[32] = 0x02;
        assert_eq!(zone_count(&reply), None);
        assert_eq!(get_color_zones(1), {
            let mut g = header(38, 502, 1);
            g.extend_from_slice(&[0, 0]);
            g
        });
    }

    #[test]
    fn get_service_is_tagged() {
        assert_eq!(get_service(), {
            let mut g = header(36, 2, 0);
            g[3] = 0x34;
            g
        });
    }
}
