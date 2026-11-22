// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Color, DeviceKind};

use crate::net::tcp_once;
use crate::{Driver, Light, local, rgb2hsv};

pub const PORT: u16 = 9123;

/// `HSVToK`: the hue picks a colour temperature; grey's undefined hue reads as -1.
fn kelvin(hue: u32) -> u32 {
    match hue as i32 {
        0..=60 => 2900,
        61..=120 => 4000,
        121..=180 => 5000,
        181..=240 => 6000,
        _ => 7000,
    }
}

/// `GetRequest`, NUL-terminated as it is written.
fn request(c: Color) -> Vec<u8> {
    let hsv = rgb2hsv(c);
    let body = format!(
        "{{\"lights\":[{{\"brightness\":{},\"on\":1,\"temperature\":{}}}],\"numberOfLights\":1}}",
        hsv.value,
        kelvin(hsv.hue)
    );
    let mut out = format!(
        "PUT /elgato/lights HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.push(0);
    out
}

struct KeyLight {
    ip: String,
}

impl Driver for KeyLight {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        tcp_once(&self.ip, PORT, &request(c), false).map(drop)
    }
}

pub fn open(ip: &str) -> Option<Light> {
    local(ip).ok()?;
    Some(Light::new(
        format!("net:elgato-key-light:{ip}"),
        "Elgato KeyLight".into(),
        DeviceKind::Other,
        1,
        Box::new(KeyLight { ip: ip.into() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_request_bytes() {
        let body = "{\"lights\":[{\"brightness\":255,\"on\":1,\"temperature\":2900}],\"numberOfLights\":1}";
        let want = format!(
            "PUT /elgato/lights HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\r\n\r\n\0",
            body.len()
        );
        assert_eq!(request(Color(255, 0, 0)), want.as_bytes());
    }

    #[test]
    fn hue_to_temperature() {
        assert_eq!(kelvin(rgb2hsv(Color(255, 255, 255)).hue), 7000);
        assert_eq!(kelvin(120), 4000);
        assert_eq!(kelvin(200), 6000);
        assert_eq!(kelvin(300), 7000);
    }
}
