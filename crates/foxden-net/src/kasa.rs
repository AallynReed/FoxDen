// SPDX-License-Identifier: GPL-2.0-or-later

use std::io::{Read, Write};
use std::time::Duration;

use foxden_core::{Color, DeviceKind};
use serde_json::Value;

use crate::net::{self, tcp};
use crate::{Driver, Found, KasaLight, Light, parse, rgb2hsv, text};

const PORT: u16 = 9999;
const IV: u8 = 0xAB;
const RECEIVE_BUFFER: usize = 4096;
const SYSTEM_INFO_QUERY: &str = "{\"system\": {\"get_sysinfo\": {}}}";
const LIGHT_OFF: &str = "{\"smartlife.iot.smartbulb.lightingservice\": {\"transition_light_state\": {\"transition_period\": 0, \"on_off\":0, \"mode\":\"normal\"}}}";
const LEDSTRIP_OFF: &str = "{\"smartlife.iot.lightStrip\": {\"set_light_state\": {\"transition\": 0, \"on_off\":0, \"mode\":\"normal\"}}}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Light,
    LedStrip,
}

/// `XorPayload`: autokey XOR starting from 0xAB.
fn xor(data: &[u8]) -> Vec<u8> {
    let mut key = IV;
    data.iter()
        .map(|b| {
            key ^= b;
            key
        })
        .collect()
}

/// `XorEncryptedPayload`.
fn unxor(data: &[u8]) -> Vec<u8> {
    let mut key = IV;
    data.iter()
        .map(|b| {
            let plain = key ^ b;
            key = *b;
            plain
        })
        .collect()
}

/// `Encrypt`: big-endian length, then the XORed request.
fn encrypt(request: &str) -> Vec<u8> {
    let mut out = (request.len() as u32).to_be_bytes().to_vec();
    out.extend(xor(request.as_bytes()));
    out
}

fn set_state(c: Color, kind: Kind) -> String {
    let hsv = rgb2hsv(c);
    let hue = if hsv.hue == u32::MAX { 0 } else { hsv.hue };
    let saturation = hsv.saturation as u32 * 100 / 255;
    let value = hsv.value as u32 * 100 / 255;
    if saturation == 0 && value == 0 {
        return match kind {
            Kind::Light => LIGHT_OFF,
            Kind::LedStrip => LEDSTRIP_OFF,
        }
        .into();
    }
    match kind {
        Kind::Light => format!(
            "{{\"smartlife.iot.smartbulb.lightingservice\": {{\"transition_light_state\": {{\"transition_period\": 0, \"on_off\":1, \"mode\":\"normal\", \"hue\": {hue}, \"saturation\": {saturation}, \"brightness\": {value}, \"color_temp\": 0}}}}}}"
        ),
        Kind::LedStrip => format!(
            "{{\"smartlife.iot.lightStrip\": {{\"set_light_state\": {{\"transition\": 0, \"on_off\":1, \"mode\":\"normal\", \"hue\": {hue}, \"saturation\": {saturation}, \"brightness\": {value}, \"color_temp\": 0}}}}}}"
        ),
    }
}

/// `SendCommand`: one request and its length-prefixed reply.
fn exchange(ip: &str, port: u16, request: &str) -> Result<String, String> {
    let mut s = tcp(ip, port)?;
    s.write_all(&encrypt(request)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; RECEIVE_BUFFER];
    let mut got = s.read(&mut buf).map_err(|e| e.to_string())?;
    if got < 4 {
        return Err("no reply".into());
    }
    let full = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if full > RECEIVE_BUFFER {
        return Err("reply too long".into());
    }
    while got < full {
        let n = s.read(&mut buf[got..]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(String::from_utf8_lossy(&unxor(&buf[4..got])).to_string())
}

/// `Initialize`: colour smart bulbs only; KL4xx models are strips.
fn kind_of(sysinfo: &Value) -> Option<(Kind, String)> {
    let info = &sysinfo["system"]["get_sysinfo"];
    let kind = info["type"]
        .as_str()
        .or(info["mic_type"].as_str())?
        .to_lowercase();
    if !kind.contains("smartbulb") || info.get("is_color").is_some_and(|c| c != 1) {
        return None;
    }
    let model = info["model"].as_str()?;
    let kind = if model.contains("KL4") {
        Kind::LedStrip
    } else {
        Kind::Light
    };
    Some((kind, text(&info["deviceId"])))
}

struct Kasa {
    ip: String,
    kind: Kind,
}

impl Driver for Kasa {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        exchange(&self.ip, PORT, &set_state(c, self.kind)).map(drop)
    }
}

pub fn open(cfg: &KasaLight) -> Option<Light> {
    let reply = (0..3).find_map(|_| exchange(&cfg.ip, PORT, SYSTEM_INFO_QUERY).ok())?;
    let (kind, id) = kind_of(&parse(&reply))?;
    Some(Light::new(
        format!("net:kasa:{}", if id.is_empty() { &cfg.ip } else { &id }),
        if cfg.name.is_empty() {
            "Kasa Smart".into()
        } else {
            format!("Kasa Smart {}", cfg.name)
        },
        if kind == Kind::LedStrip {
            DeviceKind::Strip
        } else {
            DeviceKind::Other
        },
        1,
        Box::new(Kasa {
            ip: cfg.ip.clone(),
            kind,
        }),
    ))
}

/// The sysinfo query, XORed without its length, broadcast on UDP 9999.
pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket
        .send_to(
            &xor(SYSTEM_INFO_QUERY.as_bytes()),
            ("255.255.255.255", PORT),
        )
        .is_err()
    {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(2))
        .into_iter()
        .filter_map(|(ip, r)| {
            let info = parse(&String::from_utf8_lossy(&unxor(&r)));
            kind_of(&info)?;
            Some(Found {
                family: "kasa",
                ip: ip.to_string(),
                port: PORT,
                name: text(&info["system"]["get_sysinfo"]["alias"]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn xor_round_trips_and_matches_the_autokey() {
        // '{' = 0x7B: 0xAB ^ 0x7B = 0xD0, then 0xD0 ^ '"' (0x22) = 0xF2
        assert_eq!(&encrypt("{\"")[..], &[0, 0, 0, 2, 0xD0, 0xF2]);
        assert_eq!(
            unxor(&xor(SYSTEM_INFO_QUERY.as_bytes())),
            SYSTEM_INFO_QUERY.as_bytes()
        );
    }

    #[test]
    fn light_state_commands() {
        assert_eq!(
            set_state(Color(255, 0, 0), Kind::Light),
            "{\"smartlife.iot.smartbulb.lightingservice\": {\"transition_light_state\": {\"transition_period\": 0, \"on_off\":1, \"mode\":\"normal\", \"hue\": 0, \"saturation\": 100, \"brightness\": 100, \"color_temp\": 0}}}"
        );
        // white: undefined hue becomes 0, saturation 0 but brightness 100 stays on
        assert_eq!(
            set_state(Color(255, 255, 255), Kind::LedStrip),
            "{\"smartlife.iot.lightStrip\": {\"set_light_state\": {\"transition\": 0, \"on_off\":1, \"mode\":\"normal\", \"hue\": 0, \"saturation\": 0, \"brightness\": 100, \"color_temp\": 0}}}"
        );
        assert_eq!(set_state(Color::BLACK, Kind::Light), LIGHT_OFF);
        assert_eq!(set_state(Color::BLACK, Kind::LedStrip), LEDSTRIP_OFF);
    }

    #[test]
    fn sysinfo_picks_colour_bulbs() {
        let bulb = parse(
            r#"{"system":{"get_sysinfo":{"mic_type":"IOT.SMARTBULB","model":"KL130(US)","is_color":1,"deviceId":"80AB"}}}"#,
        );
        assert_eq!(kind_of(&bulb), Some((Kind::Light, "80AB".into())));
        let strip = parse(
            r#"{"system":{"get_sysinfo":{"type":"IOT.SMARTBULB","model":"KL430(US)","deviceId":"X"}}}"#,
        );
        assert_eq!(kind_of(&strip).unwrap().0, Kind::LedStrip);
        let mono = parse(
            r#"{"system":{"get_sysinfo":{"type":"IOT.SMARTBULB","model":"KL110","is_color":0}}}"#,
        );
        assert_eq!(kind_of(&mono), None);
        let plug =
            parse(r#"{"system":{"get_sysinfo":{"type":"IOT.SMARTPLUGSWITCH","model":"HS100"}}}"#);
        assert_eq!(kind_of(&plug), None);
    }

    #[test]
    fn exchange_frames_both_ways() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let t = thread::spawn(move || {
            let (mut s, _) = server.accept().unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).unwrap();
            s.write_all(&encrypt("{\"ok\":1}")).unwrap();
            buf[..n].to_vec()
        });
        assert_eq!(
            exchange("127.0.0.1", port, SYSTEM_INFO_QUERY).unwrap(),
            "{\"ok\":1}"
        );
        assert_eq!(t.join().unwrap(), encrypt(SYSTEM_INFO_QUERY));
    }
}
