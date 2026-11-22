// SPDX-License-Identifier: GPL-2.0-or-later
//! : its HTTP calls and net_port.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use crate::local;

pub const CONNECT: Duration = Duration::from_millis(1500);
pub const READ: Duration = Duration::from_secs(3);

/// A UDP socket aimed at one light on this network.
pub fn udp(ip: &str, port: u16) -> Result<UdpSocket, String> {
    let addr = local(ip)?;
    let any: IpAddr = if addr.is_ipv4() {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        std::net::Ipv6Addr::UNSPECIFIED.into()
    };
    let socket = UdpSocket::bind((any, 0)).map_err(|e| e.to_string())?;
    socket.connect((addr, port)).map_err(|e| e.to_string())?;
    Ok(socket)
}

pub fn tcp(ip: &str, port: u16) -> Result<TcpStream, String> {
    let addr = SocketAddr::new(local(ip)?, port);
    let stream = TcpStream::connect_timeout(&addr, CONNECT).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(READ));
    let _ = stream.set_write_timeout(Some(READ));
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Connects, writes `bytes`, optionally reads until the light closes, and disconnects.
pub fn tcp_once(ip: &str, port: u16, bytes: &[u8], answer: bool) -> Result<Vec<u8>, String> {
    let mut stream = tcp(ip, port)?;
    stream.write_all(bytes).map_err(|e| e.to_string())?;
    let mut reply = Vec::new();
    if answer {
        read_until_closed(&mut stream, &mut reply);
    }
    Ok(reply)
}

fn read_until_closed(stream: &mut TcpStream, out: &mut Vec<u8>) {
    let deadline = Instant::now() + READ;
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
}

/// One plain HTTP/1.1 request to a light. No redirects, no name lookups.
pub fn http(
    method: &str,
    ip: &str,
    port: u16,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let mut stream = tcp(ip, port)?;
    stream
        .write_all(request(method, ip, port, path, body).as_bytes())
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    read_until_closed(&mut stream, &mut raw);
    response(&raw)
}

pub fn request(method: &str, ip: &str, port: u16, path: &str, body: Option<&str>) -> String {
    let host = match ip.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{ip}]:{port}"),
        _ => format!("{ip}:{port}"),
    };
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    match body {
        Some(body) => {
            out += &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        }
        None => out += "Content-Length: 0\r\n\r\n",
    }
    out
}

/// Status and body of a reply, plain or chunked.
pub fn response(raw: &[u8]) -> Result<(u16, String), String> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no reply")?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("bad reply")?;
    let chunked = head.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });
    if !chunked {
        return Ok((status, body.to_string()));
    }
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            break;
        }
        out += &after[..n];
        rest = after[n..].trim_start_matches("\r\n");
    }
    Ok((status, out))
}

/// A socket for a broadcast or multicast search, bound to `port` (0 for any).
pub fn search_socket(port: u16) -> Option<UdpSocket> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).ok()?;
    socket.set_broadcast(true).ok()?;
    let _ = socket.set_multicast_ttl_v4(1);
    socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    Some(socket)
}

/// Every reply that arrives within `wait`, with the address of the light that sent it.
pub fn collect(socket: &UdpSocket, wait: Duration) -> Vec<(IpAddr, Vec<u8>)> {
    let deadline = Instant::now() + wait;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        if let Ok((n, from)) = socket.recv_from(&mut buf)
            && local(&from.ip().to_string()).is_ok()
        {
            out.push((from.ip(), buf[..n].to_vec()));
        }
    }
    out
}

/// This PC's address on the way to `ip`. Connecting a UDP socket sends nothing.
pub fn own_address(ip: &str) -> Option<IpAddr> {
    let socket = udp(ip, 9).ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn requests_carry_the_body_length() {
        assert_eq!(
            request(
                "PUT",
                "192.168.1.2",
                80,
                "/api/u/lights/1/state",
                Some("{\"on\":true}")
            ),
            "PUT /api/u/lights/1/state HTTP/1.1\r\nHost: 192.168.1.2:80\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"on\":true}"
        );
        assert!(
            request("POST", "fe80::1", 16021, "/api/v1/new", None)
                .contains("Host: [fe80::1]:16021\r\n")
        );
    }

    #[test]
    fn plain_and_chunked_replies() {
        assert_eq!(
            response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]").unwrap(),
            (200, "[]".into())
        );
        assert_eq!(
            response(b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").unwrap(),
            (403, "abcde".into())
        );
        assert!(response(b"garbage").is_err());
    }

    #[test]
    fn talks_http_to_a_local_server() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let t = thread::spawn(move || {
            let (mut s, _) = server.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\npong")
                .unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        let (status, body) = http("GET", "127.0.0.1", port, "/ping", None).unwrap();
        assert_eq!((status, body.as_str()), (200, "pong"));
        assert!(t.join().unwrap().starts_with("GET /ping HTTP/1.1\r\n"));
    }

    #[test]
    fn refuses_addresses_off_the_network() {
        assert!(udp("8.8.8.8", 53).is_err());
        assert!(http("GET", "1.1.1.1", 80, "/", None).is_err());
    }
}
