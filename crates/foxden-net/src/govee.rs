// SPDX-License-Identifier: GPL-2.0-or-later

use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use foxden_core::{Color, DeviceKind};

use crate::net::{self, udp};
use crate::{Driver, Found, Light, local, parse, text};

const SCAN_PORT: u16 = 4001;
const REPLY_PORT: u16 = 4002;
const CONTROL_PORT: u16 = 4003;
const BRIGHTNESS: u32 = 100;
/// The constructor waits five seconds for the scan reply.
const SCAN_WAIT: Duration = Duration::from_secs(2);
/// KeepaliveThread: a frame every 30 seconds.
pub const KEEPALIVE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Linear,
    Serpentine(u32),
    SplitBars,
}

/// `govee_device_info`: LED count and layout of the models with razer streaming.
fn info(sku: &str) -> Option<(usize, Layout)> {
    Some(match sku {
        "H6022" => (132, Layout::Serpentine(12)),
        "H6046" => (20, Layout::SplitBars),
        "H612F" => (12, Layout::Linear),
        "H619A" | "H70B1" | "H6069" => (20, Layout::Linear),
        "H607C" => (174, Layout::Linear),
        _ => return None,
    })
}

fn msg(cmd: &str, data: &str) -> Vec<u8> {
    let mut out = format!("{{\"msg\":{{\"cmd\":\"{cmd}\",\"data\":{data}}}}}").into_bytes();
    out.push(0);
    out
}

fn set_color(c: Color) -> Vec<u8> {
    msg(
        "colorwc",
        &format!(
            "{{\"color\":{{\"b\":{},\"g\":{},\"r\":{}}},\"colorTemInKelvin\":\"0\"}}",
            c.2, c.1, c.0
        ),
    )
}

fn set_brightness(b: u32) -> Vec<u8> {
    msg("brightness", &format!("{{\"value\":{}}}", b.min(100)))
}

fn set_power(on: bool) -> Vec<u8> {
    msg("turn", &format!("{{\"value\":{}}}", on as u8))
}

pub fn scan() -> Vec<u8> {
    msg("scan", "{\"account_topic\":\"reserve\"}")
}

fn razer(pt: &[u8]) -> Vec<u8> {
    msg("razer", &format!("{{\"pt\":\"{}\"}}", base64(pt)))
}

fn razer_enable() -> Vec<u8> {
    razer(&[0xBB, 0x00, 0x01, 0xB1, 0x01, 0x0A])
}

/// `SendRazerData`: up to 255 colours with an XOR checksum.
fn razer_data(colors: &[Color]) -> Vec<u8> {
    let count = colors.len().min(255);
    let len = 2 + 3 * count;
    let mut pkt = vec![0xBB, (len >> 8) as u8, len as u8, 0xB0, 0x01, count as u8];
    for c in &colors[..count] {
        pkt.extend_from_slice(&[c.0, c.1, c.2]);
    }
    let sum = pkt.iter().fold(0, |a, b| a ^ b);
    pkt.push(sum);
    razer(&pkt)
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The scan reply's data, if it is one.
fn scan_reply(reply: &[u8]) -> Option<(String, String)> {
    let v = parse(&String::from_utf8_lossy(reply));
    let data = &v["msg"]["data"];
    (v["msg"]["cmd"] == "scan" && data["ip"].is_string())
        .then(|| (text(&data["ip"]), text(&data["sku"])))
}

/// LED positions for each layout, from `SetupZones`' matrix maps.
fn positions(count: usize, layout: Layout) -> ([u32; 2], Vec<[f32; 2]>) {
    match layout {
        Layout::Linear => (
            [count as u32, 1],
            (0..count)
                .map(|i| [(i as f32 + 0.5) / count as f32, 0.5])
                .collect(),
        ),
        Layout::Serpentine(width) => {
            let height = count as u32 / width;
            let mut leds = vec![[0.5, 0.5]; count];
            for y in 0..height {
                let led_y = height - 1 - y;
                for x in 0..width {
                    let led_x = if led_y & 1 == 1 { x } else { width - 1 - x };
                    leds[(led_y * width + led_x) as usize] = [
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    ];
                }
            }
            ([width, height], leds)
        }
        Layout::SplitBars => {
            let bar = count / 2;
            let width = 10 + 2;
            let mut leds = vec![[0.5, 0.5]; count];
            for y in 0..bar {
                let row = (y as f32 + 0.5) / bar as f32;
                leds[bar + y] = [0.5 / width as f32, row];
                leds[y] = [(width as f32 - 0.5) / width as f32, row];
            }
            ([width as u32, bar as u32], leds)
        }
    }
}

struct Govee {
    socket: UdpSocket,
    razer: bool,
    started: bool,
}

impl Govee {
    fn write(&self, d: &[u8]) -> Result<(), String> {
        self.socket.send(d).map(drop).map_err(|e| e.to_string())
    }
}

impl Driver for Govee {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        if colors.is_empty() {
            return Ok(());
        }
        if self.razer {
            self.write(&razer_data(colors))?;
            return self.write(&set_brightness(BRIGHTNESS));
        }
        if !self.started {
            self.write(&set_power(true))?;
        }
        self.write(&set_color(colors[0]))?;
        self.write(&set_brightness(BRIGHTNESS))?;
        self.started = true;
        Ok(())
    }

    fn refresh(&mut self, colors: &[Color]) -> Result<(), String> {
        if self.razer {
            self.write(&razer_enable())?;
        }
        self.send(colors)
    }
}

/// The scan the constructor sends to each /24 broadcast address. One socket, since replies share port 4002.
fn ask_skus(ips: &[String]) -> HashMap<String, String> {
    let mut skus = HashMap::new();
    let Some(socket) = net::search_socket(REPLY_PORT) else {
        return skus;
    };
    let mut nets: Vec<&str> = ips
        .iter()
        .filter_map(|ip| ip.rsplit_once('.').map(|(p, _)| p))
        .collect();
    nets.dedup();
    for prefix in nets {
        let _ = socket.send_to(&scan(), (format!("{prefix}.255").as_str(), SCAN_PORT));
    }
    let deadline = Instant::now() + SCAN_WAIT;
    while Instant::now() < deadline && skus.len() < ips.len() {
        for (_, reply) in net::collect(&socket, Duration::from_millis(200)) {
            if let Some((from, sku)) = scan_reply(&reply)
                && ips.contains(&from)
            {
                skus.insert(from, sku);
            }
        }
    }
    skus
}

/// Every Govee light, with its model when the search already knows it.
pub fn open_all(lights: &[(String, String)]) -> Vec<Light> {
    let unknown: Vec<String> = lights
        .iter()
        .filter(|(ip, sku)| sku.is_empty() && local(ip).is_ok())
        .map(|(ip, _)| ip.clone())
        .collect();
    let asked = if unknown.is_empty() {
        HashMap::new()
    } else {
        ask_skus(&unknown)
    };
    lights
        .iter()
        .filter_map(|(ip, sku)| {
            let sku = if sku.is_empty() {
                asked.get(ip).cloned().unwrap_or_default()
            } else {
                sku.clone()
            };
            open(ip, &sku)
        })
        .collect()
}

fn open(ip: &str, sku: &str) -> Option<Light> {
    let socket = udp(ip, CONTROL_PORT).ok()?;
    let found = info(sku);
    let (count, layout) = found.unwrap_or((1, Layout::Linear));
    let govee = Govee {
        socket,
        razer: found.is_some(),
        started: false,
    };
    // DeviceUpdateMode for Direct: switch razer streaming on.
    if govee.razer {
        govee.write(&razer_enable()).ok()?;
    }
    let (grid, leds) = positions(count, layout);
    let mut light = Light::new(
        format!("net:govee:{ip}"),
        if sku.is_empty() {
            "Govee".into()
        } else {
            format!("Govee {sku}")
        },
        if count > 1 {
            DeviceKind::Strip
        } else {
            DeviceKind::Other
        },
        count,
        Box::new(govee),
    )
    .keep(Some(KEEPALIVE));
    light.grid = grid;
    light.leds = leds;
    Some(light)
}

/// Govee's LAN scan, sent to its multicast group; replies arrive on port 4002.
pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(REPLY_PORT) else {
        return Vec::new();
    };
    let group = Ipv4Addr::new(239, 255, 255, 250);
    let _ = socket.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED);
    if socket.send_to(&scan(), (group, SCAN_PORT)).is_err() {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(2))
        .into_iter()
        .filter_map(|(from, reply)| {
            let (ip, sku) = scan_reply(&reply)?;
            let ip = if local(&ip).is_ok() {
                ip
            } else {
                from.to_string()
            };
            Some(Found {
                family: "govee",
                ip,
                port: CONTROL_PORT,
                name: if sku.is_empty() {
                    "Govee".into()
                } else {
                    format!("Govee {sku}")
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: Vec<u8>) -> String {
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn commands_are_nlohmann_dumps_with_a_nul() {
        assert_eq!(
            s(set_color(Color(1, 2, 3))),
            "{\"msg\":{\"cmd\":\"colorwc\",\"data\":{\"color\":{\"b\":3,\"g\":2,\"r\":1},\"colorTemInKelvin\":\"0\"}}}\0"
        );
        assert_eq!(
            s(set_brightness(150)),
            "{\"msg\":{\"cmd\":\"brightness\",\"data\":{\"value\":100}}}\0"
        );
        assert_eq!(
            s(set_power(true)),
            "{\"msg\":{\"cmd\":\"turn\",\"data\":{\"value\":1}}}\0"
        );
        assert_eq!(
            s(scan()),
            "{\"msg\":{\"cmd\":\"scan\",\"data\":{\"account_topic\":\"reserve\"}}}\0"
        );
    }

    #[test]
    fn razer_packets() {
        // BB 00 01 B1 01 0A
        assert_eq!(
            s(razer_enable()),
            "{\"msg\":{\"cmd\":\"razer\",\"data\":{\"pt\":\"uwABsQEK\"}}}\0"
        );
        // BB 00 05 B0 01 01 FF 00 00, checksum BB^05^B0^01^01^FF = 0xF1
        assert_eq!(
            s(razer_data(&[Color(255, 0, 0)])),
            "{\"msg\":{\"cmd\":\"razer\",\"data\":{\"pt\":\"uwAFsAEB/wAA8Q==\"}}}\0"
        );
        assert_eq!(base64(&[0xBB, 0x00]), "uwA=");
    }

    #[test]
    fn scan_reply_names_the_light() {
        let r = br#"{"msg":{"cmd":"scan","data":{"ip":"192.168.1.23","device":"AA:BB","sku":"H619A","bleVersionHard":"3.01.01"}}}"#;
        assert_eq!(scan_reply(r), Some(("192.168.1.23".into(), "H619A".into())));
        assert_eq!(
            scan_reply(br#"{"msg":{"cmd":"devStatus","data":{}}}"#),
            None
        );
        assert_eq!(info("H619A"), Some((20, Layout::Linear)));
        assert_eq!(info("H6001"), None);
    }

    #[test]
    fn matrix_layouts_follow_the_maps() {
        // two rows of 2: bottom row (led_y 0) runs right to left
        let (grid, leds) = positions(4, Layout::Serpentine(2));
        assert_eq!(grid, [2, 2]);
        assert_eq!(
            leds,
            vec![[0.75, 0.75], [0.25, 0.75], [0.25, 0.25], [0.75, 0.25]]
        );
        let (grid, leds) = positions(20, Layout::SplitBars);
        assert_eq!(grid, [12, 10]);
        assert_eq!(leds[0], [11.5 / 12.0, 0.05]);
        assert_eq!(leds[10], [0.5 / 12.0, 0.05]);
    }
}
