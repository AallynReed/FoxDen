// SPDX-License-Identifier: GPL-2.0-or-later

use std::io::Write;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use foxden_core::{Color, DeviceKind};

use crate::net::{self, tcp_once};
use crate::{Driver, Found, Light, YeelightLight, local, quote};

const PORT: u16 = 55443;
const MUSIC_PORT: u16 = 55444;
/// The constructor waits for the light to connect back without a limit; this one gives up.
const MUSIC_WAIT: Duration = Duration::from_secs(3);

fn command(method: &str, params: &str) -> String {
    format!("{{\"id\":1,\"method\":\"{method}\",\"params\":[{params}]}}\r\n")
}

/// Each command goes out with its NUL, except on the music-mode socket.
fn with_nul(s: String) -> Vec<u8> {
    let mut b = s.into_bytes();
    b.push(0);
    b
}

fn set_power() -> String {
    command("set_power", "\"on\",\"sudden\",0,2")
}

fn set_music(host: &str, port: u16) -> String {
    command("set_music", &format!("1,{},{port}", quote(host)))
}

/// `SetColor`: a one-step colour flow, brightness from the brightest channel.
fn start_cf(c: Color) -> String {
    let bright = c.0.max(c.1).max(c.2) as f32;
    let bright = (100.0f32 * (bright / 255.0f32)) as i32;
    let rgb = (c.0 as u32) << 16 | (c.1 as u32) << 8 | c.2 as u32;
    command("start_cf", &format!("1,1,\"50,1,{rgb},{bright}\""))
}

struct Yeelight {
    ip: String,
    music: Option<TcpStream>,
}

impl Driver for Yeelight {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        let cmd = start_cf(c);
        match &mut self.music {
            Some(sock) => sock.write_all(cmd.as_bytes()).map_err(|e| e.to_string()),
            None => tcp_once(&self.ip, PORT, &with_nul(cmd), false).map(drop),
        }
    }
}

/// Music mode: the light connects back to us and takes commands without a rate limit.
fn music(cfg: &YeelightLight) -> Option<TcpStream> {
    let host = if cfg.host_ip.is_empty() {
        net::own_address(&cfg.ip)?.to_string()
    } else {
        cfg.host_ip.clone()
    };
    local(&host).ok()?;
    let (server, port) = (MUSIC_PORT..=u16::MAX).find_map(|p| {
        TcpListener::bind((Ipv4Addr::UNSPECIFIED, p))
            .ok()
            .map(|s| (s, p))
    })?;
    tcp_once(&cfg.ip, PORT, &with_nul(set_music(&host, port)), false).ok()?;
    server.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + MUSIC_WAIT;
    while Instant::now() < deadline {
        if let Ok((sock, from)) = server.accept() {
            if from.ip().to_string() != cfg.ip {
                continue;
            }
            sock.set_nonblocking(false).ok()?;
            return Some(sock);
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

pub fn open(cfg: &YeelightLight) -> Option<Light> {
    local(&cfg.ip).ok()?;
    tcp_once(&cfg.ip, PORT, &with_nul(set_power()), false).ok()?;
    let music = if cfg.music_mode { music(cfg) } else { None };
    Some(Light::new(
        format!("net:yeelight:{}", cfg.ip),
        "Yeelight".into(),
        DeviceKind::Other,
        1,
        Box::new(Yeelight {
            ip: cfg.ip.clone(),
            music,
        }),
    ))
}

/// Yeelight's documented SSDP-style search on port 1982.
pub const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1982\r\nMAN: \"ssdp:discover\"\r\nST: wifi_bulb\r\n";

/// Address and name from a search reply's `Location: yeelight://ip:port` and `name:` lines.
fn search_reply(reply: &str) -> Option<(String, String)> {
    let mut ip = None;
    let mut name = String::new();
    for line in reply.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("location:") {
            let rest = line.split_once("yeelight://")?.1;
            ip = Some(rest.split(':').next()?.trim().to_string());
        } else if lower.starts_with("name:") {
            name = line[5..].trim().to_string();
        }
    }
    Some((ip?, name))
}

pub fn discover() -> Vec<Found> {
    let Some(socket) = net::search_socket(0) else {
        return Vec::new();
    };
    if socket
        .send_to(SEARCH.as_bytes(), "239.255.255.250:1982")
        .is_err()
    {
        return Vec::new();
    }
    net::collect(&socket, Duration::from_secs(2))
        .into_iter()
        .filter_map(|(_, r)| search_reply(&String::from_utf8_lossy(&r)))
        .filter(|(ip, _)| local(ip).is_ok())
        .map(|(ip, name)| Found {
            family: "yeelight",
            ip,
            port: PORT,
            name: if name.is_empty() {
                "Yeelight".into()
            } else {
                format!("Yeelight {name}")
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_match_the_dumped_json() {
        assert_eq!(
            with_nul(set_power()),
            b"{\"id\":1,\"method\":\"set_power\",\"params\":[\"on\",\"sudden\",0,2]}\r\n\0"
        );
        assert_eq!(
            set_music("192.168.1.5", 55444),
            "{\"id\":1,\"method\":\"set_music\",\"params\":[1,\"192.168.1.5\",55444]}\r\n"
        );
        // 0xFF8000 = 16744448, bright (int)(100 * 255/255) = 100
        assert_eq!(
            start_cf(Color(255, 128, 0)),
            "{\"id\":1,\"method\":\"start_cf\",\"params\":[1,1,\"50,1,16744448,100\"]}\r\n"
        );
        // (int)(100 * (128/255)) = 50
        assert_eq!(
            start_cf(Color(0, 0, 128)),
            "{\"id\":1,\"method\":\"start_cf\",\"params\":[1,1,\"50,1,128,50\"]}\r\n"
        );
    }

    #[test]
    fn search_replies() {
        let r = "HTTP/1.1 200 OK\r\nCache-Control: max-age=3600\r\nLocation: yeelight://192.168.1.239:55443\r\nid: 0x000000000015243f\r\nmodel: color\r\nname: desk\r\n";
        assert_eq!(
            search_reply(r),
            Some(("192.168.1.239".into(), "desk".into()))
        );
        assert_eq!(search_reply("HTTP/1.1 200 OK\r\n"), None);
    }
}
