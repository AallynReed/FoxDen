//! WLED, LeetDesk and SRGBmods Wifi LC (protocol learned from SignalRGB's WLED.js, leetdesk.js and SRGBmods-WLC.js).

use super::net::{self, Hit};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use serde_json::{Value, json};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub const SERVICES: [&str; 3] = [
    "_wled._tcp.local.",
    "_aura._tcp.local.",
    "_srgbmods-wlc._tcp.local.",
];
/// DNRGB: protocol 4, 2 s timeout, 16-bit start index, then RGB.
const DNRGB: u8 = 0x04;
const DNRGB_TIMEOUT_S: u8 = 2;
const WLED_PER_PACKET: usize = 485;
const WLC_PER_PACKET: usize = 480;
const WLC_MARK: u8 = 0xAA;
const LEETDESK_DEFAULT_LEDS: usize = 512;
const RESEND: Duration = Duration::from_millis(900);
const HTTP_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flavor {
    Wled,
    LeetDesk,
    Wlc,
}

impl Flavor {
    fn of(service: &str) -> Self {
        if service.starts_with("_aura") {
            Flavor::LeetDesk
        } else if service.starts_with("_srgbmods-wlc") {
            Flavor::Wlc
        } else {
            Flavor::Wled
        }
    }
    fn json_root(self) -> &'static str {
        if self == Flavor::LeetDesk {
            "/jsonaura"
        } else {
            "/json"
        }
    }
    fn brand(self) -> &'static str {
        match self {
            Flavor::Wled => "wled",
            Flavor::LeetDesk => "leetdesk",
            Flavor::Wlc => "srgbmods-wlc",
        }
    }
}

pub fn dnrgb_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    colors
        .chunks(WLED_PER_PACKET)
        .enumerate()
        .map(|(i, c)| {
            let start = (i * WLED_PER_PACKET) as u16;
            let mut p = vec![DNRGB, DNRGB_TIMEOUT_S];
            p.extend_from_slice(&start.to_be_bytes());
            p.extend(c.iter().flat_map(|c| [c.0, c.1, c.2]));
            p
        })
        .collect()
}

pub fn wlc_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let total = colors.len().div_ceil(WLC_PER_PACKET).max(1);
    let mut out: Vec<Vec<u8>> = colors
        .chunks(WLC_PER_PACKET)
        .enumerate()
        .map(|(i, c)| {
            let mut p = vec![(i + 1) as u8, total as u8, 0x00, WLC_MARK];
            p.extend(c.iter().flat_map(|c| [c.0, c.1, c.2]));
            p
        })
        .collect();
    if out.is_empty() {
        out.push(vec![1, 1, 0x00, WLC_MARK]);
    }
    out
}

pub struct Wled {
    flavor: Flavor,
    ip: Ipv4Addr,
    port: u16,
    sock: UdpSocket,
    device: Device,
    nano_connect: bool,
    default_on: bool,
    default_bri: u64,
    started: bool,
    sent: Option<Vec<Color>>,
    at: Instant,
}

impl Wled {
    fn from_info(
        flavor: Flavor,
        ip: Ipv4Addr,
        port: u16,
        v: &Value,
        name_hint: &str,
    ) -> Option<Self> {
        let (udp, leds, mac, name, arch) = if flavor == Flavor::Wlc {
            if v["product"].as_str() != Some("SRGBmods-WLC") {
                return None;
            }
            (
                v["udpport"].as_u64()?,
                v["ledcount"].as_u64().unwrap_or(0) as usize,
                v["mac"].as_str().unwrap_or_default().to_string(),
                v["name"].as_str().unwrap_or(name_hint).to_string(),
                v["mcu"].as_str().unwrap_or_default().to_string(),
            )
        } else {
            let info = &v["info"];
            let leds = info["leds"]["count"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(if flavor == Flavor::LeetDesk {
                    LEETDESK_DEFAULT_LEDS
                } else {
                    0
                });
            (
                info["udpport"].as_u64()?,
                leds,
                info["mac"].as_str().unwrap_or_default().to_string(),
                info["name"].as_str().unwrap_or(name_hint).to_string(),
                info["arch"].as_str().unwrap_or_default().to_string(),
            )
        };
        if leds == 0 {
            return None;
        }
        let sock = net::udp_to(ip, udp as u16).ok()?;
        let key = if mac.is_empty() { ip.to_string() } else { mac };
        let id = format!("{}:{key}", flavor.brand());
        let device = Device::grid(
            &id,
            &name,
            DeviceKind::Strip,
            [0.05, 0.9, 0.9, 0.04],
            leds as u32,
            1,
        );
        Some(Wled {
            flavor,
            ip,
            port,
            sock,
            device,
            nano_connect: arch == "NanoConnect",
            default_on: v["state"]["on"].as_bool().unwrap_or(true),
            default_bri: v["state"]["bri"].as_u64().unwrap_or(128),
            started: false,
            sent: None,
            at: Instant::now(),
        })
    }

    fn state(&self, on: bool, bri: u64) {
        if self.flavor == Flavor::Wlc {
            return;
        }
        let path = format!("{}/state/", self.flavor.json_root());
        let body = json!({"on": on, "bri": bri, "live": false});
        let _ = net::send_json(self.ip, self.port, "POST", &path, &body, HTTP_TIMEOUT);
    }

    fn frame(&self, colors: &[Color]) -> Result<(), String> {
        if self.flavor == Flavor::Wlc {
            let packets = wlc_packets(colors);
            let gap = if self.nano_connect && packets.len() > 1 {
                10
            } else {
                1
            };
            for p in packets {
                self.sock.send(&p).map_err(|e| e.to_string())?;
                std::thread::sleep(Duration::from_millis(gap));
            }
        } else {
            for p in dnrgb_packets(colors) {
                self.sock.send(&p).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

impl Backend for Wled {
    fn name(&self) -> &str {
        self.flavor.brand()
    }
    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Ok(());
        }
        if !self.started {
            if self.flavor == Flavor::Wlc {
                let _ = net::http(
                    self.ip,
                    self.port,
                    "GET",
                    "/turnon",
                    &[],
                    None,
                    HTTP_TIMEOUT,
                );
            } else {
                self.state(true, 255);
            }
            self.started = true;
        }
        if self.sent.as_deref() == Some(colors) && self.at.elapsed() < RESEND {
            return Ok(());
        }
        let mut c = colors.to_vec();
        c.resize(self.device.leds.len(), Color::BLACK);
        self.frame(&c)?;
        self.sent = Some(colors.to_vec());
        self.at = Instant::now();
        Ok(())
    }
}

impl Drop for Wled {
    fn drop(&mut self) {
        if self.started {
            let _ = self.frame(&vec![Color::BLACK; self.device.leds.len()]);
            self.state(self.default_on, self.default_bri);
        }
    }
}

fn open(hit: &Hit, deadline: Instant) -> Option<Wled> {
    let ip = hit.ip?;
    let flavor = Flavor::of(&hit.service);
    let port = if hit.port == 0 { 80 } else { hit.port };
    let path = if flavor == Flavor::Wlc {
        "/".to_string()
    } else {
        format!("{}/", flavor.json_root())
    };
    let v = net::get_json(ip, port, &path, net::left(deadline, HTTP_TIMEOUT))?;
    let hint = hit
        .instance
        .split('.')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("WLED");
    Wled::from_info(flavor, ip, port, &v, hint)
}

pub fn probe(tx: Sender<Box<dyn Backend>>, deadline: Instant, t: super::Targets) {
    let mut seen = Vec::new();
    for ip in t.ips {
        seen.push(ip);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let found = SERVICES.iter().find_map(|service| {
                let hit = Hit {
                    service: service.to_string(),
                    instance: String::new(),
                    ip: Some(ip),
                    port: 80,
                    txt: Default::default(),
                };
                open(&hit, deadline)
            });
            if let Some(b) = found {
                let _ = tx.send(Box::new(b));
            }
        });
    }
    if !t.discover {
        return;
    }
    let listen = deadline - Duration::from_millis(600);
    let hits = net::mdns_browse(&SERVICES, listen);
    for hit in hits {
        let Some(ip) = hit.ip else { continue };
        if seen.contains(&ip) {
            continue;
        }
        seen.push(ip);
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(b) = open(&hit, deadline) {
                let _ = tx.send(Box::new(b));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dnrgb_chunks() {
        let c = vec![Color(1, 2, 3); 500];
        let p = dnrgb_packets(&c);
        assert_eq!(p.len(), 2);
        assert_eq!(&p[0][..7], &[4, 2, 0, 0, 1, 2, 3]);
        assert_eq!(p[0].len(), 4 + 485 * 3);
        assert_eq!(&p[1][..4], &[4, 2, 0x01, 0xE5]);
        assert_eq!(p[1].len(), 4 + 15 * 3);
    }

    #[test]
    fn wlc_chunks() {
        let c = vec![Color(9, 8, 7); 481];
        let p = wlc_packets(&c);
        assert_eq!(p.len(), 2);
        assert_eq!(&p[0][..7], &[1, 2, 0, 0xAA, 9, 8, 7]);
        assert_eq!(&p[1][..], &[2, 2, 0, 0xAA, 9, 8, 7]);
    }

    #[test]
    fn flavors() {
        assert_eq!(Flavor::of("_aura._tcp.local"), Flavor::LeetDesk);
        assert_eq!(Flavor::of("_srgbmods-wlc._tcp.local"), Flavor::Wlc);
        assert_eq!(Flavor::of("_wled._tcp.local"), Flavor::Wled);
    }

    #[test]
    fn info_parsing() {
        let v = json!({"state":{"on":false,"bri":40},"info":{"udpport":21324,"leds":{"count":30},"mac":"aabbcc","name":"Desk","arch":"esp32"}});
        let w = Wled::from_info(Flavor::Wled, Ipv4Addr::new(192, 168, 1, 9), 80, &v, "x").unwrap();
        assert_eq!(w.device.leds.len(), 30);
        assert_eq!(w.device.id, "wled:aabbcc");
        assert!(!w.default_on);
        let wlc = json!({"product":"SRGBmods-WLC","udpport":21325,"ledcount":12,"mac":"m","name":"W","mcu":"NanoConnect"});
        let w = Wled::from_info(Flavor::Wlc, Ipv4Addr::new(192, 168, 1, 9), 80, &wlc, "x").unwrap();
        assert!(w.nano_connect);
        assert_eq!(w.device.leds.len(), 12);
    }
}
