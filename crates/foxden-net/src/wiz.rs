//! Ported from OpenRGB Controllers/PhilipsWizController (GPL-2.0-or-later).

use std::net::UdpSocket;
use std::time::Duration;

use foxden_core::{Color, DeviceKind};

use crate::net::{self, udp};
use crate::{Driver, Found, Light, WizLight, parse, text};

const PORT: u16 = 38899;
const BRIGHTNESS_MAX: u8 = 100;

/// `SetColor`: setPilot with the white channels the strategy asks for, NUL-terminated.
fn set_pilot(c: Color, brightness: u8, cfg: &WizLight) -> Vec<u8> {
    let (mut r, mut g, mut b) = (c.0, c.1, c.2);
    let white = match cfg.white_strategy.as_str() {
        "Average" => ((r as u32 + g as u32 + b as u32) / 3) as u8,
        "Minimum" => {
            let w = r.min(g).min(b);
            if cfg.use_cool_white || cfg.use_warm_white {
                r -= w;
                g -= w;
                b -= w;
            }
            w
        }
        _ => 0,
    };
    let cool = if cfg.use_cool_white { white } else { 0 };
    let warm = if cfg.use_warm_white { white } else { 0 };
    let state = !(r == 0 && g == 0 && b == 0 && white == 0);
    let mut out = format!(
        "{{\"method\":\"setPilot\",\"params\":{{\"b\":{b},\"c\":{cool},\"dimming\":{brightness},\"g\":{g},\"r\":{r},\"state\":{state},\"w\":{warm}}}}}"
    )
    .into_bytes();
    out.push(0);
    out
}

const SYSTEM_CONFIG: &[u8] = b"{\"method\":\"getSystemConfig\"}\0";

/// moduleName and mac from a getSystemConfig reply.
fn system_config(reply: &[u8]) -> Option<(String, String)> {
    let v = parse(&String::from_utf8_lossy(reply));
    if v["method"] != "getSystemConfig" {
        return None;
    }
    Some((text(&v["result"]["moduleName"]), text(&v["result"]["mac"])))
}

struct Wiz {
    socket: UdpSocket,
    cfg: WizLight,
}

impl Driver for Wiz {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        self.socket
            .send(&set_pilot(c, BRIGHTNESS_MAX, &self.cfg))
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

pub fn open(cfg: &WizLight) -> Option<Light> {
    let socket = udp(&cfg.ip, PORT).ok()?;
    // RequestSystemConfig waits a second for the reply and carries on without it.
    let _ = socket.send(SYSTEM_CONFIG);
    let _ = socket.set_read_timeout(Some(Duration::from_secs(1)));
    let mut buf = [0u8; 1024];
    let (_, mac) = socket
        .recv(&mut buf)
        .ok()
        .and_then(|n| system_config(&buf[..n]))
        .unwrap_or_default();
    let key = if mac.is_empty() { cfg.ip.clone() } else { mac };
    Some(Light::new(
        format!("net:wiz:{key}"),
        "Philips Wiz".into(),
        DeviceKind::Other,
        1,
        Box::new(Wiz {
            socket,
            cfg: cfg.clone(),
        }),
    ))
}

/// Not in OpenRGB: Wiz lights answer getSystemConfig sent to the broadcast address.
pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket
        .send_to(SYSTEM_CONFIG, ("255.255.255.255", PORT))
        .is_err()
    {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(2))
        .into_iter()
        .filter_map(|(ip, reply)| {
            let (module, _) = system_config(&reply)?;
            Some(Found {
                family: "wiz",
                ip: ip.to_string(),
                port: PORT,
                name: if module.is_empty() {
                    "Philips Wiz".into()
                } else {
                    format!("Wiz {module}")
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(strategy: &str, cool: bool, warm: bool) -> WizLight {
        WizLight {
            ip: String::new(),
            use_cool_white: cool,
            use_warm_white: warm,
            white_strategy: strategy.into(),
        }
    }

    #[test]
    fn set_pilot_is_dumped_with_sorted_keys_and_a_nul() {
        assert_eq!(
            set_pilot(Color(255, 30, 0), 100, &cfg("Average", false, false)),
            b"{\"method\":\"setPilot\",\"params\":{\"b\":0,\"c\":0,\"dimming\":100,\"g\":30,\"r\":255,\"state\":true,\"w\":0}}\0"
        );
        // Average: (90+60+30)/3 = 60 on both white channels, colour untouched
        assert_eq!(
            set_pilot(Color(90, 60, 30), 100, &cfg("Average", true, true)),
            b"{\"method\":\"setPilot\",\"params\":{\"b\":30,\"c\":60,\"dimming\":100,\"g\":60,\"r\":90,\"state\":true,\"w\":60}}\0"
        );
        // Minimum: 30 moves from the colour to the warm channel
        assert_eq!(
            set_pilot(Color(90, 60, 30), 100, &cfg("Minimum", false, true)),
            b"{\"method\":\"setPilot\",\"params\":{\"b\":0,\"c\":0,\"dimming\":100,\"g\":30,\"r\":60,\"state\":true,\"w\":30}}\0"
        );
        assert_eq!(
            set_pilot(Color::BLACK, 100, &cfg("None", true, true)),
            b"{\"method\":\"setPilot\",\"params\":{\"b\":0,\"c\":0,\"dimming\":100,\"g\":0,\"r\":0,\"state\":false,\"w\":0}}\0"
        );
    }

    #[test]
    fn system_config_reply() {
        let reply = br#"{"method":"getSystemConfig","env":"pro","result":{"mac":"a8bb50aabbcc","moduleName":"ESP01_SHRGB1C_31","fwVersion":"1.24.0"}}"#;
        assert_eq!(
            system_config(reply),
            Some(("ESP01_SHRGB1C_31".into(), "a8bb50aabbcc".into()))
        );
        assert_eq!(system_config(br#"{"method":"getPilot"}"#), None);
        assert_eq!(SYSTEM_CONFIG, b"{\"method\":\"getSystemConfig\"}\0");
    }
}
