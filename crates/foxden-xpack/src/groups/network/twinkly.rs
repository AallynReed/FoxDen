//! Twinkly lights (protocol learned from SignalRGB's Twinkly.js).

use super::net;
use foxden_core::{Backend, Color, Device, DeviceKind};
use serde_json::{Value, json};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const DISCOVER_PORT: u16 = 5555;
const LISTEN_PORT: u16 = 59136;
const DISCOVER: &[u8] = b"\x01discover";
const RT_PORT: u16 = 7777;
const GEN3: u8 = 0x03;
const TOKEN_LEN: usize = 8;
const CHUNK_BYTES: usize = 900;
const CODE_OK: u64 = 1000;
const CHALLENGE_LEN: usize = 32;
const MODE_CHECK: Duration = Duration::from_secs(60);
const RESEND: Duration = Duration::from_millis(1000);
const HTTP_TIMEOUT: Duration = Duration::from_millis(400);
const SQUARES_SKU: &str = "TWQ064STW";

/// Twinkly Squares 8x8 panel, from SignalRGB's Components folder (row-major).
pub fn squares_8x8() -> Vec<[f32; 2]> {
    (0..64)
        .map(|i| [((i % 8) as f32 + 0.5) / 8.0, ((i / 8) as f32 + 0.5) / 8.0])
        .collect()
}

/// Gen3 real-time datagrams: 03, 8-byte token, 00 00, chunk index, up to 900 colour bytes.
pub fn gen3_packets(token: &[u8], colors: &[Color], bytes_per_led: usize) -> Vec<Vec<u8>> {
    let data: Vec<u8> = colors
        .iter()
        .flat_map(|c| {
            if bytes_per_led == 4 {
                vec![0, c.0, c.1, c.2]
            } else {
                vec![c.0, c.1, c.2]
            }
        })
        .collect();
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(CHUNK_BYTES).collect()
    };
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut p = vec![GEN3];
            p.extend_from_slice(token);
            p.extend_from_slice(&[0, 0, i as u8]);
            p.extend_from_slice(c);
            p
        })
        .collect()
}

/// LED positions from `led/layout/full` (2d uses x,y; 3d uses x,z).
pub fn layout(v: &Value) -> Option<Vec<[f32; 2]>> {
    let pts = v["coordinates"].as_array()?;
    let second = if v["source"].as_str() == Some("3d") {
        "z"
    } else {
        "y"
    };
    let raw: Vec<(f64, f64)> = pts
        .iter()
        .map(|p| {
            (
                p["x"].as_f64().unwrap_or(0.0),
                p[second].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    if raw.is_empty() {
        return None;
    }
    let range = |f: fn(&(f64, f64)) -> f64| {
        let lo = raw.iter().map(f).fold(f64::INFINITY, f64::min);
        let hi = raw.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
        (lo, (hi - lo).max(1e-6))
    };
    let ((x0, xs), (y0, ys)) = (range(|p| p.0), range(|p| p.1));
    Some(
        raw.iter()
            .map(|&(x, y)| {
                [
                    ((x - x0) / xs * 0.96 + 0.02) as f32,
                    ((y - y0) / ys * 0.96 + 0.02) as f32,
                ]
            })
            .collect(),
    )
}

struct Session {
    token: String,
    raw: Vec<u8>,
}

fn login(ip: Ipv4Addr, timeout: Duration) -> Option<Session> {
    let challenge = net::b64_encode(&net::noise(CHALLENGE_LEN));
    let r = net::send_json(
        ip,
        80,
        "POST",
        "/xled/v1/login",
        &json!({"challenge": challenge}),
        timeout,
    )
    .ok()?;
    let v = r.json()?;
    let token = v["authentication_token"].as_str()?.to_string();
    let answer = v["challenge-response"].as_str()?.to_string();
    let body = json!({"challenge-response": answer}).to_string();
    let hdr = [("X-Auth-Token", token.as_str())];
    let ok = net::http(
        ip,
        80,
        "POST",
        "/xled/v1/verify",
        &hdr,
        Some(body.as_bytes()),
        timeout,
    )
    .ok()?;
    if ok.json().and_then(|v| v["code"].as_u64()) != Some(CODE_OK) {
        return None;
    }
    let raw = net::b64_decode(&token)?;
    (raw.len() == TOKEN_LEN).then_some(Session { token, raw })
}

fn authed(
    ip: Ipv4Addr,
    s: &Session,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Option<Value> {
    let b = body.map(|b| b.to_string());
    let hdr = [("X-Auth-Token", s.token.as_str())];
    net::http(
        ip,
        80,
        method,
        path,
        &hdr,
        b.as_deref().map(str::as_bytes),
        HTTP_TIMEOUT,
    )
    .ok()?
    .json()
}

pub struct Twinkly {
    ip: Ipv4Addr,
    sock: UdpSocket,
    session: Session,
    device: Device,
    bytes_per_led: usize,
    started: bool,
    prev_mode: Option<String>,
    checked: Instant,
    sent: Option<Vec<Color>>,
    at: Instant,
}

impl Twinkly {
    fn go_rt(&mut self) {
        let _ = authed(
            self.ip,
            &self.session,
            "POST",
            "/xled/v1/led/out/brightness",
            Some(json!({"mode": "enabled", "type": "A", "value": 100})),
        );
        let _ = authed(
            self.ip,
            &self.session,
            "POST",
            "/xled/v1/led/mode",
            Some(json!({"mode": "rt"})),
        );
    }
}

impl Backend for Twinkly {
    fn name(&self) -> &str {
        "twinkly"
    }
    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Ok(());
        }
        if !self.started {
            let mut mode = authed(self.ip, &self.session, "GET", "/xled/v1/led/mode", None);
            if mode.is_none()
                && let Some(s) = login(self.ip, HTTP_TIMEOUT)
            {
                self.session = s;
                mode = authed(self.ip, &self.session, "GET", "/xled/v1/led/mode", None);
            }
            self.prev_mode = mode
                .and_then(|v| v["mode"].as_str().map(String::from))
                .filter(|m| m != "rt");
            self.go_rt();
            self.started = true;
            self.checked = Instant::now();
        } else if self.checked.elapsed() >= MODE_CHECK {
            self.checked = Instant::now();
            let mode = authed(self.ip, &self.session, "GET", "/xled/v1/led/mode", None);
            if mode.is_none_or(|v| v["mode"].as_str() != Some("rt")) {
                if let Some(s) = login(self.ip, HTTP_TIMEOUT) {
                    self.session = s;
                }
                self.go_rt();
            }
        }
        if self.sent.as_deref() == Some(colors) && self.at.elapsed() < RESEND {
            return Ok(());
        }
        let mut c = colors.to_vec();
        c.resize(self.device.leds.len(), Color::BLACK);
        for p in gen3_packets(&self.session.raw, &c, self.bytes_per_led) {
            self.sock.send(&p).map_err(|e| e.to_string())?;
        }
        self.sent = Some(colors.to_vec());
        self.at = Instant::now();
        Ok(())
    }
}

impl Drop for Twinkly {
    fn drop(&mut self) {
        if self.started {
            let mode = self.prev_mode.clone().unwrap_or_else(|| "movie".into());
            let _ = authed(
                self.ip,
                &self.session,
                "POST",
                "/xled/v1/led/mode",
                Some(json!({"mode": mode})),
            );
        }
    }
}

fn open(ip: Ipv4Addr, deadline: Instant) -> Option<Twinkly> {
    let t = net::left(deadline, HTTP_TIMEOUT);
    let session = login(ip, t)?;
    let g = net::get_json(
        ip,
        80,
        "/xled/v1/gestalt",
        net::left(deadline, HTTP_TIMEOUT),
    )?;
    let bpl = g["bytes_per_led"].as_u64()? as usize;
    let count = g["number_of_led"].as_u64()? as usize;
    if bpl < 3 || count == 0 {
        return None;
    }
    let lay = authed(ip, &session, "GET", "/xled/v1/led/layout/full", None);
    let leds = lay
        .as_ref()
        .and_then(layout)
        .filter(|l| l.len() == count)
        .or_else(|| {
            (g["product_code"].as_str() == Some(SQUARES_SKU) && count == 64).then(squares_8x8)
        });
    let name = g["device_name"].as_str().unwrap_or("Twinkly");
    let key = g["mac"]
        .as_str()
        .map(|m| m.replace(':', ""))
        .unwrap_or_else(|| ip.to_string());
    let id = format!("twinkly:{key}");
    let device = match leds {
        Some(l) => {
            let mut d = Device::grid(&id, name, DeviceKind::Other, [0.3, 0.3, 0.4, 0.4], 16, 16);
            d.leds = l;
            d
        }
        None => Device::grid(
            &id,
            name,
            DeviceKind::Strip,
            [0.05, 0.8, 0.9, 0.04],
            count as u32,
            1,
        ),
    };
    let sock = net::udp_to(ip, RT_PORT).ok()?;
    Some(Twinkly {
        ip,
        sock,
        session,
        device,
        bytes_per_led: bpl,
        started: false,
        prev_mode: None,
        checked: Instant::now(),
        sent: None,
        at: Instant::now(),
    })
}

pub fn is_reply(b: &[u8]) -> bool {
    let text = String::from_utf8_lossy(b);
    text.contains("OKTwinkly") || text.contains("WHEREAREYOU") || b.get(4..6) == Some(b"OK")
}

pub fn probe(tx: Sender<Box<dyn Backend>>, deadline: Instant, t: super::Targets) {
    let mut seen = Vec::new();
    for ip in t.ips {
        seen.push(ip);
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(b) = open(ip, deadline) {
                let _ = tx.send(Box::new(b));
            }
        });
    }
    if !t.discover {
        return;
    }
    let Some(sock) = net::udp_listen(LISTEN_PORT) else {
        return;
    };
    let _ = sock.set_broadcast(true);
    let _ = sock.send_to(DISCOVER, (Ipv4Addr::BROADCAST, DISCOVER_PORT));
    net::gather(&sock, deadline - Duration::from_millis(900), |buf, from| {
        let Some(ip) = net::local_v4(from) else {
            return;
        };
        if buf == DISCOVER || !is_reply(buf) || seen.contains(&ip) {
            return;
        }
        seen.push(ip);
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(t) = open(ip, deadline) {
                let _ = tx.send(Box::new(t));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen3_chunks() {
        let token = [1, 2, 3, 4, 5, 6, 7, 8];
        let p = gen3_packets(&token, &vec![Color(9, 8, 7); 301], 3);
        assert_eq!(p.len(), 2);
        assert_eq!(&p[0][..12], &[3, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0]);
        assert_eq!(p[0].len(), 12 + 900);
        assert_eq!(&p[1][9..], &[0, 0, 1, 9, 8, 7]);
        let p = gen3_packets(&token, &[Color(1, 2, 3)], 4);
        assert_eq!(&p[0][12..], &[0, 1, 2, 3]);
    }

    #[test]
    fn layouts() {
        let v = json!({"source":"2d","coordinates":[{"x":0.0,"y":0.0,"z":0.0},{"x":1.0,"y":0.5,"z":0.0}]});
        let l = layout(&v).unwrap();
        assert_eq!(l[0], [0.02, 0.02]);
        assert_eq!(l[1], [0.98, 0.98]);
        assert_eq!(squares_8x8()[9], [1.5 / 8.0, 1.5 / 8.0]);
    }

    #[test]
    fn discovery_reply() {
        assert!(is_reply(b"\x05\x01\xa8\xc0OKTwinkly_ABC\0"));
        assert!(!is_reply(b"\x01discover"));
    }
}
