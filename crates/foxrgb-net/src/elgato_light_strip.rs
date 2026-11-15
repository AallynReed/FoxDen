//! Ported from OpenRGB Controllers/ElgatoLightStripController (GPL-2.0-or-later).

use std::thread;
use std::time::Duration;

use foxrgb_core::{Color, DeviceKind};

use crate::elgato_key_light::PORT;
use crate::net::tcp_once;
use crate::{Driver, Light, local, parse, rgb2hsv, text};

const ACCESSORY_INFO: &[u8] =
    b"GET /elgato/accessory-info HTTP/1.1\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\0";
/// `GetBrightness` falls back to 100, which is also Direct mode's brightness.
const BRIGHTNESS: u32 = 100;
const SETTLE: Duration = Duration::from_millis(150);

/// `SetColor`: hue capped at 360 and the 0..255 saturation capped at 100, as OpenRGB does.
fn request(c: Color) -> Vec<u8> {
    let hsv = rgb2hsv(c);
    let hue = hsv.hue.min(360);
    let saturation = hsv.saturation.min(100);
    let body = format!(
        "{{\"lights\":[{{\"brightness\":{BRIGHTNESS},\"hue\":{hue},\"on\":1,\"saturation\":{saturation}}}],\"numberOfLights\":1}}"
    );
    let mut out = format!(
        "PUT /elgato/lights HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.push(0);
    out
}

/// displayName and serialNumber from the accessory-info reply.
fn accessory(reply: &[u8]) -> (String, String) {
    let reply = String::from_utf8_lossy(reply);
    let body = reply
        .split_once("\r\n\r\n")
        .map_or(reply.as_ref(), |(_, b)| b);
    let v = parse(body);
    (text(&v["displayName"]), text(&v["serialNumber"]))
}

struct Strip {
    ip: String,
}

impl Driver for Strip {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        thread::sleep(SETTLE);
        tcp_once(&self.ip, PORT, &request(c), false).map(drop)
    }
}

pub fn open(ip: &str) -> Option<Light> {
    local(ip).ok()?;
    let reply = tcp_once(ip, PORT, ACCESSORY_INFO, true).unwrap_or_default();
    let (name, serial) = accessory(&reply);
    Some(Light::new(
        format!(
            "net:elgato-light-strip:{}",
            if serial.is_empty() { ip } else { &serial }
        ),
        if name.is_empty() {
            "Elgato Light Strip".into()
        } else {
            name
        },
        DeviceKind::Strip,
        1,
        Box::new(Strip { ip: ip.into() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_request_caps_hue_and_saturation() {
        // red: hue 0, saturation 255 capped to 100
        let body = "{\"lights\":[{\"brightness\":100,\"hue\":0,\"on\":1,\"saturation\":100}],\"numberOfLights\":1}";
        let want = format!(
            "PUT /elgato/lights HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\r\n\r\n\0",
            body.len()
        );
        assert_eq!(request(Color(255, 0, 0)), want.as_bytes());
        // grey's hue of -1 becomes 360
        assert!(
            String::from_utf8(request(Color(9, 9, 9)))
                .unwrap()
                .contains("\"hue\":360,\"on\":1,\"saturation\":0")
        );
    }

    #[test]
    fn accessory_info() {
        let r = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 70\r\n\r\n{\"displayName\":\"Desk strip\",\"serialNumber\":\"CW1\",\"firmwareVersion\":\"1.0\"}";
        assert_eq!(accessory(r), ("Desk strip".into(), "CW1".into()));
        assert_eq!(
            ACCESSORY_INFO,
            b"GET /elgato/accessory-info HTTP/1.1\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\0"
        );
    }
}
