//! Cololight Hexa and Strip.

use super::net;
use foxden_core::{Backend, Color, Device, DeviceKind};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const DISCOVERY_PORT: u16 = 12345;
const SEARCH: &[u8] = b"Z-SEARCH * \r\n";
const CONTROL_PORT: u16 = 8900;
const MAGIC: [u8; 4] = *b"SZ00";
const SECURITY_LEN: usize = 16;
const SEQ_COMMAND: u8 = 0x21;
const SEQ_COLOR: u8 = 0x22;
const VERB_SET: u8 = 0x04;
const CTAG_STATE: u8 = 0x21;
const CTAG_COLOR: u8 = 0x24;
const BRIGHTNESS: [u8; 2] = [0x01, 0xCF];
const MODE: [u8; 2] = [0x02, 0xFF];
const DYNAMIC_MODE: [u8; 4] = [0x81, 0x00, 0x00, 0x00];
const BUFFER_SINGLE: u8 = 0x01;
/// Hardware brightness the add-on applies on start (0..=100).
const DEFAULT_BRIGHTNESS: u8 = 50;
const MODELS: [(&str, &str); 3] = [("C32", "Hexa"), ("HC32", "Hexa"), ("HKC32", "Strip")];
const RESEND: Duration = Duration::from_millis(1000);

/// "SZ00", buffer flag, BE32 length of everything after byte 10, 16 security bytes, sequence, body.
pub fn packet(buffer: bool, seq: u8, body: &[u8]) -> Vec<u8> {
    let mut p = MAGIC.to_vec();
    p.extend_from_slice(&[0x00, buffer as u8]);
    let len = (SECURITY_LEN + 1 + body.len()) as u32;
    p.extend_from_slice(&len.to_be_bytes());
    p.extend_from_slice(&[0; SECURITY_LEN]);
    p.push(seq);
    p.extend_from_slice(body);
    p
}

/// Command body: destination and source id, security, verb, tag, payload length, payload.
pub fn command(seq: u8, ctag: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![0; 9];
    body.extend_from_slice(&[VERB_SET, ctag, payload.len() as u8]);
    body.extend_from_slice(payload);
    packet(false, seq, &body)
}

pub fn brightness(level: u8) -> Vec<u8> {
    command(
        SEQ_COMMAND,
        CTAG_STATE,
        &[BRIGHTNESS[0], BRIGHTNESS[1], level.min(100)],
    )
}

pub fn dynamic_mode() -> Vec<u8> {
    command(
        SEQ_COMMAND,
        CTAG_STATE,
        &[
            MODE[0],
            MODE[1],
            DYNAMIC_MODE[0],
            DYNAMIC_MODE[1],
            DYNAMIC_MODE[2],
            DYNAMIC_MODE[3],
        ],
    )
}

pub fn solid(c: Color) -> Vec<u8> {
    command(
        SEQ_COLOR,
        CTAG_COLOR,
        &[MODE[0], MODE[1], 0x00, c.0, c.1, c.2],
    )
}

pub fn stream_color(c: Color) -> Vec<u8> {
    packet(true, SEQ_COMMAND, &[BUFFER_SINGLE, c.0, c.1, c.2])
}

pub struct Cololight {
    sock: UdpSocket,
    device: Device,
    started: bool,
    sent: Option<Color>,
    at: Instant,
}

impl Backend for Cololight {
    fn name(&self) -> &str {
        "cololight"
    }
    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Ok(());
        }
        let send = |p: Vec<u8>| self.sock.send(&p).map(|_| ()).map_err(|e| e.to_string());
        if !self.started {
            send(brightness(DEFAULT_BRIGHTNESS))?;
            send(dynamic_mode())?;
        }
        self.started = true;
        let c = colors.first().copied().unwrap_or_default();
        if self.sent == Some(c) && self.at.elapsed() < RESEND {
            return Ok(());
        }
        self.sock
            .send(&stream_color(c))
            .map_err(|e| e.to_string())?;
        self.sent = Some(c);
        self.at = Instant::now();
        Ok(())
    }
}

impl Drop for Cololight {
    fn drop(&mut self) {
        if self.started {
            let _ = self.sock.send(&solid(Color::BLACK));
        }
    }
}

pub fn parse_reply(b: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(b)
        .split("\r\n")
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect()
}

pub fn probe(tx: Sender<Box<dyn Backend>>, deadline: Instant, t: super::Targets) {
    let Some(sock) = net::udp_listen(DISCOVERY_PORT) else {
        return;
    };
    for ip in &t.ips {
        let _ = sock.send_to(SEARCH, (*ip, DISCOVERY_PORT));
    }
    if t.discover {
        let _ = sock.set_broadcast(true);
        let _ = sock.send_to(SEARCH, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    }
    let mut seen = Vec::new();
    net::gather(&sock, deadline - Duration::from_millis(300), |buf, from| {
        let Some(ip) = net::local_v4(from) else {
            return;
        };
        let r = parse_reply(buf);
        let Some(model) = r
            .get("subkey")
            .and_then(|k| MODELS.iter().find(|m| m.0 == k))
        else {
            return;
        };
        if seen.contains(&ip) {
            return;
        }
        seen.push(ip);
        let Ok(sock) = net::udp_to(ip, CONTROL_PORT) else {
            return;
        };
        let key = r
            .get("sn")
            .or(r.get("mac"))
            .cloned()
            .unwrap_or_else(|| ip.to_string());
        let tail: String = key
            .chars()
            .skip(key.chars().count().saturating_sub(8))
            .collect();
        let name = format!("Cololight {} {tail}", model.1);
        let kind = if model.1 == "Strip" {
            DeviceKind::Strip
        } else {
            DeviceKind::Other
        };
        let device = Device::grid(
            &format!("cololight:{key}"),
            &name,
            kind,
            [0.45, 0.45, 0.1, 0.1],
            1,
            1,
        );
        let _ = tx.send(Box::new(Cololight {
            sock,
            device,
            started: false,
            sent: None,
            at: Instant::now(),
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_bytes() {
        let b = brightness(50);
        assert_eq!(b.len(), 42);
        assert_eq!(&b[..10], &[0x53, 0x5A, 0x30, 0x30, 0, 0, 0, 0, 0, 0x20]);
        assert_eq!(
            &b[26..],
            &[
                0x21, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x04, 0x21, 0x03, 0x01, 0xCF, 50
            ]
        );
        let s = solid(Color(1, 2, 3));
        assert_eq!((s[9], s[26], s.len()), (0x23, 0x22, 45));
        assert_eq!(&s[36..], &[0x04, 0x24, 0x06, 0x02, 0xFF, 0x00, 1, 2, 3]);
        let d = dynamic_mode();
        assert_eq!(&d[39..], &[0x02, 0xFF, 0x81, 0, 0, 0]);
        let c = stream_color(Color(7, 8, 9));
        assert_eq!(&c[..10], &[0x53, 0x5A, 0x30, 0x30, 0, 1, 0, 0, 0, 21]);
        assert_eq!(&c[26..], &[0x21, 0x01, 7, 8, 9]);
    }

    #[test]
    fn reply() {
        let r = parse_reply(b"subkey=HKC32\r\nsn=ABCDEF0123456789\r\n");
        assert_eq!(r["subkey"], "HKC32");
        assert!(!parse_reply(SEARCH).contains_key("subkey"));
    }
}
