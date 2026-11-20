//! Ported from OpenRGB Controllers/NanoleafController (GPL-2.0-or-later).

use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use foxden_core::{Color, DeviceKind};

use crate::net::{self, http, udp};
use crate::{Driver, Found, Light, NanoleafPanel, local, parse, text};

pub const PORT: u16 = 16021;
const STREAM_PORT: u16 = 60222;
const LIGHT_PANELS: &str = "NL22";
const V2_MODELS: [&str; 4] = ["NL29", "NL42", "NL59", "NL69"];
const PAIR_FOR: Duration = Duration::from_secs(30);
const PAIR_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Version {
    V1,
    V2,
}

fn version(model: &str) -> Option<Version> {
    if model == LIGHT_PANELS {
        Some(Version::V1)
    } else if V2_MODELS.contains(&model) {
        Some(Version::V2)
    } else {
        None
    }
}

/// `StartExternalControl` request body.
fn ext_control(v: Version) -> String {
    let tag = match v {
        Version::V1 => "v1",
        Version::V2 => "v2",
    };
    format!(
        "{{\"write\":{{\"animType\":\"extControl\",\"command\":\"display\",\"extControlVersion\":\"{tag}\"}}}}"
    )
}

/// `UpdateLEDs`: one frame, every panel set at once.
fn frame(v: Version, panels: &[u16], colors: &[Color]) -> Vec<u8> {
    let colour = |i: usize| colors.get(i).copied().unwrap_or_default();
    match v {
        Version::V1 => {
            let mut m = vec![panels.len() as u8];
            for (i, id) in panels.iter().enumerate() {
                let c = colour(i);
                m.extend_from_slice(&[*id as u8, 1, c.0, c.1, c.2, 0, 0]);
            }
            m
        }
        Version::V2 => {
            let n = panels.len();
            let mut m = vec![(n >> 8) as u8, n as u8];
            for (i, id) in panels.iter().enumerate() {
                let c = colour(i);
                m.extend_from_slice(&[(id >> 8) as u8, *id as u8, c.0, c.1, c.2, 0, 0, 0]);
            }
            m
        }
    }
}

struct Panels {
    socket: UdpSocket,
    version: Version,
    ids: Vec<u16>,
}

impl Driver for Panels {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        self.socket
            .send(&frame(self.version, &self.ids, colors))
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// Reads the layout, then switches the panels to external control.
pub fn open(cfg: &NanoleafPanel) -> Option<Light> {
    if cfg.auth_token.is_empty() {
        return None;
    }
    let base = format!("/api/v1/{}", cfg.auth_token);
    let (status, body) = http("GET", &cfg.ip, cfg.port, &base, None).ok()?;
    if status != 200 {
        return None;
    }
    let data = parse(&body);
    let model = text(&data["model"]);
    let ids: Vec<u16> = data["panelLayout"]["layout"]["positionData"]
        .as_array()?
        .iter()
        .filter_map(|p| p["panelId"].as_u64().map(|id| id as u16))
        .collect();
    let v = version(&model)?;
    let (status, reply) = http(
        "PUT",
        &cfg.ip,
        cfg.port,
        &format!("{base}/effects"),
        Some(&ext_control(v)),
    )
    .ok()?;
    if status / 100 != 2 {
        return None;
    }
    let socket = match v {
        Version::V1 => {
            let reply = parse(&reply);
            let ip = text(&reply["streamControlIpAddr"]);
            let port = reply["streamControlPort"].as_u64()? as u16;
            udp(&ip, port).ok()?
        }
        Version::V2 => udp(&cfg.ip, STREAM_PORT).ok()?,
    };
    let serial = text(&data["serialNo"]);
    let name = text(&data["name"]);
    Some(Light::new(
        format!(
            "net:nanoleaf:{}",
            if serial.is_empty() { &cfg.ip } else { &serial }
        ),
        if name.is_empty() {
            "Nanoleaf".into()
        } else {
            name
        },
        DeviceKind::Strip,
        ids.len(),
        Box::new(Panels {
            socket,
            version: v,
            ids,
        }),
    ))
}

/// `NanoleafController::Pair`, retried while the user holds the power button.
pub fn pair(ip: &str, port: u16) -> Result<NanoleafPanel, String> {
    local(ip)?;
    let start = Instant::now();
    loop {
        if let Ok((200, body)) = http("POST", ip, port, "/api/v1/new", None) {
            let token = text(&parse(&body)["auth_token"]);
            if !token.is_empty() {
                return Ok(NanoleafPanel {
                    ip: ip.into(),
                    port,
                    auth_token: token,
                });
            }
        }
        if start.elapsed() >= PAIR_FOR {
            return Err("the panels were not in pairing mode".into());
        }
        thread::sleep(PAIR_EVERY);
    }
}

const SERVICE: &str = "_nanoleafapi._tcp.local.";

/// `mdns_query_send(sock, MDNS_RECORDTYPE_PTR, service, ...)` from an ephemeral port, query id 0.
fn mdns_query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in SERVICE.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    // PTR, class IN with the unicast-response bit
    q.extend_from_slice(&[0x00, 0x0C, 0x80, 0x01]);
    q
}

/// Whether an mDNS reply answers for the Nanoleaf service.
fn answers(reply: &[u8]) -> bool {
    let needle = b"\x0c_nanoleafapi";
    reply.len() > 12 && reply[2] & 0x80 != 0 && reply.windows(needle.len()).any(|w| w == needle)
}

/// NanoleafScanningThread: replies come from the panels themselves, on the default port.
pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket.send_to(&mdns_query(), "224.0.0.251:5353").is_err() {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(3))
        .into_iter()
        .filter(|(_, reply)| answers(reply))
        .map(|(ip, _)| Found {
            family: "nanoleaf",
            ip: ip.to_string(),
            port: PORT,
            name: "Nanoleaf".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_control_bodies() {
        assert_eq!(
            ext_control(Version::V1),
            r#"{"write":{"animType":"extControl","command":"display","extControlVersion":"v1"}}"#
        );
        assert_eq!(version("NL42"), Some(Version::V2));
        assert_eq!(version("NL22"), Some(Version::V1));
        assert_eq!(version("NL11"), None);
    }

    #[test]
    fn v1_frame_is_seven_bytes_a_panel() {
        let f = frame(
            Version::V1,
            &[0x12, 0x34],
            &[Color(1, 2, 3), Color(4, 5, 6)],
        );
        assert_eq!(f, vec![2, 0x12, 1, 1, 2, 3, 0, 0, 0x34, 1, 4, 5, 6, 0, 0]);
    }

    #[test]
    fn v2_frame_is_eight_bytes_a_panel() {
        let f = frame(
            Version::V2,
            &[0x1234, 0x00ff],
            &[Color(1, 2, 3), Color(4, 5, 6)],
        );
        assert_eq!(
            f,
            vec![
                0, 2, 0x12, 0x34, 1, 2, 3, 0, 0, 0, 0x00, 0xff, 4, 5, 6, 0, 0, 0
            ]
        );
    }

    #[test]
    fn mdns_ptr_query_bytes() {
        let mut want = vec![0u8, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 12];
        want.extend_from_slice(b"_nanoleafapi\x04_tcp\x05local\x00\x00\x0c\x80\x01");
        assert_eq!(mdns_query(), want);
        let mut reply = want.clone();
        reply[2] = 0x84;
        assert!(answers(&reply));
        assert!(!answers(&want));
    }
}
