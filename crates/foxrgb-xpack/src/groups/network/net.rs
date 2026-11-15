//! Shared LAN helpers: local-address guard, HTTP/1.1 client, mDNS browse, base64.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Private, link-local and CGNAT-free IPv4 ranges; nothing else is ever contacted.
pub fn is_local(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10
        || (o[0] == 172 && (16..32).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254)
}

pub fn local_v4(addr: SocketAddr) -> Option<Ipv4Addr> {
    match addr.ip() {
        IpAddr::V4(ip) if is_local(ip) => Some(ip),
        _ => None,
    }
}

pub fn left(deadline: Instant, cap: Duration) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .min(cap)
        .max(Duration::from_millis(20))
}

/// Cheap non-cryptographic random bytes (Twinkly challenge, ids).
pub fn noise(n: usize) -> Vec<u8> {
    let mut s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9)
        ^ (&n as *const usize as u64);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 24) as u8
        })
        .collect()
}

// ---------------------------------------------------------------- HTTP

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub fn build_request(
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut s = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    for (k, v) in headers {
        s.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        s.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    s.push_str("\r\n");
    let mut out = s.into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(b);
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

enum Framing {
    Length(usize),
    Chunked,
    None,
    Eof,
}

/// Parses a buffered response; `None` while more bytes are needed (unless `eof`).
pub fn parse_response(buf: &[u8], eof: bool) -> Option<Reply> {
    let end = find(buf, b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&buf[..end]);
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut framing = Framing::Eof;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if k == "content-length" {
            framing = Framing::Length(v.parse().ok()?);
        } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
            framing = Framing::Chunked;
        }
    }
    if status == 204 || status == 304 || (100..200).contains(&status) {
        framing = Framing::None;
    }
    let rest = &buf[end + 4..];
    let body = match framing {
        Framing::None => Vec::new(),
        Framing::Length(n) if rest.len() >= n => rest[..n].to_vec(),
        Framing::Length(_) if eof => rest.to_vec(),
        Framing::Length(_) => return None,
        Framing::Chunked => match dechunk(rest) {
            Some(b) => b,
            None if eof => rest.to_vec(),
            None => return None,
        },
        Framing::Eof if eof => rest.to_vec(),
        Framing::Eof => return None,
    };
    Some(Reply { status, body })
}

/// Chunked transfer decoding; `None` until the final zero chunk arrives.
pub fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = find(b, b"\r\n")?;
        let size_txt = String::from_utf8_lossy(&b[..eol]);
        let size = usize::from_str_radix(size_txt.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if b.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

pub fn http(
    ip: Ipv4Addr,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<Reply, String> {
    if !is_local(ip) {
        return Err(format!("{ip} is not on the local network"));
    }
    let start = Instant::now();
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    let _ = s.set_nodelay(true);
    s.set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let host = if port == 80 {
        ip.to_string()
    } else {
        format!("{ip}:{port}")
    };
    s.write_all(&build_request(method, &host, path, headers, body))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let rem = timeout.saturating_sub(start.elapsed());
        if rem.is_zero() {
            return parse_response(&buf, true).ok_or_else(|| "timed out".into());
        }
        s.set_read_timeout(Some(rem)).map_err(|e| e.to_string())?;
        match s.read(&mut chunk) {
            Ok(0) => return parse_response(&buf, true).ok_or_else(|| "bad reply".into()),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(r) = parse_response(&buf, false) {
                    return Ok(r);
                }
            }
            Err(e) => {
                return parse_response(&buf, true).ok_or_else(|| e.to_string());
            }
        }
    }
}

pub fn get_json(
    ip: Ipv4Addr,
    port: u16,
    path: &str,
    timeout: Duration,
) -> Option<serde_json::Value> {
    let r = http(ip, port, "GET", path, &[], None, timeout).ok()?;
    if r.ok() { r.json() } else { None }
}

pub fn send_json(
    ip: Ipv4Addr,
    port: u16,
    method: &str,
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<Reply, String> {
    let b = body.to_string();
    http(ip, port, method, path, &[], Some(b.as_bytes()), timeout)
}

// ---------------------------------------------------------------- UDP

pub fn udp_to(ip: Ipv4Addr, port: u16) -> Result<UdpSocket, String> {
    if !is_local(ip) {
        return Err(format!("{ip} is not on the local network"));
    }
    let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    s.connect(SocketAddrV4::new(ip, port))
        .map_err(|e| e.to_string())?;
    Ok(s)
}

/// Binds the add-on's listen port when free, otherwise any port.
pub fn udp_listen(port: u16) -> Option<UdpSocket> {
    UdpSocket::bind(("0.0.0.0", port))
        .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
        .ok()
}

/// Collects datagrams until `until`.
pub fn gather(sock: &UdpSocket, until: Instant, mut each: impl FnMut(&[u8], SocketAddr)) {
    let mut buf = vec![0u8; 9000];
    while Instant::now() < until {
        let rem = until
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(5));
        if sock.set_read_timeout(Some(rem)).is_err() {
            return;
        }
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            each(&buf[..n], from);
        }
    }
}

// ---------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|&b| b == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- mDNS

pub const MDNS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);
const T_A: u16 = 1;
const T_PTR: u16 = 12;
const T_TXT: u16 = 16;
const T_SRV: u16 = 33;

/// One PTR question per service, class IN with the unicast-response bit.
pub fn mdns_query(services: &[&str]) -> Vec<u8> {
    let mut p = vec![0, 0, 0, 0, 0, services.len() as u8, 0, 0, 0, 0, 0, 0];
    for s in services {
        for label in s.trim_end_matches('.').split('.') {
            p.push(label.len() as u8);
            p.extend_from_slice(label.as_bytes());
        }
        p.push(0);
        p.extend_from_slice(&T_PTR.to_be_bytes());
        p.extend_from_slice(&0x8001u16.to_be_bytes());
    }
    p
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rdata {
    Ptr(String),
    Srv(u16, String),
    A(Ipv4Addr),
    Txt(Vec<String>),
    Other,
}

fn read_name(p: &[u8], mut i: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut after = None;
    for _ in 0..64 {
        let len = *p.get(i)? as usize;
        if len == 0 {
            return Some((labels.join("."), after.unwrap_or(i + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = (len & 0x3F) << 8 | *p.get(i + 1)? as usize;
            after.get_or_insert(i + 2);
            i = ptr;
            continue;
        }
        labels.push(String::from_utf8_lossy(p.get(i + 1..i + 1 + len)?).into_owned());
        i += 1 + len;
    }
    None
}

/// All answer, authority and additional records of a response.
pub fn mdns_parse(p: &[u8]) -> Vec<(String, Rdata)> {
    let mut out = Vec::new();
    if p.len() < 12 || p[2] & 0x80 == 0 {
        return out;
    }
    let cnt = |o: usize| u16::from_be_bytes([p[o], p[o + 1]]) as usize;
    let (qd, total) = (cnt(4), cnt(6) + cnt(8) + cnt(10));
    let mut i = 12;
    for _ in 0..qd {
        let Some((_, n)) = read_name(p, i) else {
            return out;
        };
        i = n + 4;
    }
    for _ in 0..total {
        let Some((name, n)) = read_name(p, i) else {
            return out;
        };
        let Some(h) = p.get(n..n + 10) else {
            return out;
        };
        let kind = u16::from_be_bytes([h[0], h[1]]);
        let len = u16::from_be_bytes([h[8], h[9]]) as usize;
        let start = n + 10;
        let Some(rd) = p.get(start..start + len) else {
            return out;
        };
        let data = match kind {
            T_PTR => read_name(p, start).map(|(s, _)| Rdata::Ptr(s)),
            T_SRV if len >= 6 => read_name(p, start + 6)
                .map(|(s, _)| Rdata::Srv(u16::from_be_bytes([rd[4], rd[5]]), s)),
            T_A if len == 4 => Some(Rdata::A(Ipv4Addr::new(rd[0], rd[1], rd[2], rd[3]))),
            T_TXT => {
                let (mut v, mut j) = (Vec::new(), 0);
                while j < rd.len() {
                    let l = rd[j] as usize;
                    v.push(
                        String::from_utf8_lossy(rd.get(j + 1..j + 1 + l).unwrap_or(&[]))
                            .into_owned(),
                    );
                    j += 1 + l;
                }
                Some(Rdata::Txt(v))
            }
            _ => Some(Rdata::Other),
        };
        out.push((name, data.unwrap_or(Rdata::Other)));
        i = start + len;
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct Hit {
    pub service: String,
    pub instance: String,
    pub ip: Option<Ipv4Addr>,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

/// Turns records (with the sender's address) into service instances.
pub fn mdns_hits(records: &[(String, Rdata, Ipv4Addr)], services: &[&str]) -> Vec<Hit> {
    let norm = |s: &str| s.trim_end_matches('.').to_ascii_lowercase();
    let wanted: Vec<String> = services.iter().map(|s| norm(s)).collect();
    let mut hits: Vec<Hit> = Vec::new();
    for (name, d, from) in records {
        if let Rdata::Ptr(inst) = d
            && wanted.contains(&norm(name))
            && !hits.iter().any(|h| norm(&h.instance) == norm(inst))
        {
            hits.push(Hit {
                service: norm(name),
                instance: inst.clone(),
                ip: Some(*from),
                ..Hit::default()
            });
        }
    }
    for h in &mut hits {
        let mut target = None;
        for (name, d, _) in records {
            if norm(name) != norm(&h.instance) {
                continue;
            }
            match d {
                Rdata::Srv(port, t) => {
                    h.port = *port;
                    target = Some(norm(t));
                }
                Rdata::Txt(kv) => {
                    for e in kv {
                        let (k, v) = e.split_once('=').unwrap_or((e, ""));
                        h.txt.insert(k.to_ascii_lowercase(), v.to_string());
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = target
            && let Some(ip) = records.iter().find_map(|(n, d, _)| match d {
                Rdata::A(ip) if norm(n) == t && is_local(*ip) => Some(*ip),
                _ => None,
            })
        {
            h.ip = Some(ip);
        }
    }
    hits
}

/// Asks for `services` twice and listens until `until`.
pub fn mdns_browse(services: &[&str], until: Instant) -> Vec<Hit> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_multicast_ttl_v4(1);
    let q = mdns_query(services);
    let _ = sock.send_to(&q, MDNS);
    let resend = Instant::now() + Duration::from_millis(250);
    let mut records = Vec::new();
    let mut sent_again = false;
    let mut buf = vec![0u8; 9000];
    while Instant::now() < until {
        if !sent_again && Instant::now() >= resend {
            let _ = sock.send_to(&q, MDNS);
            sent_again = true;
        }
        let rem = until
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        let _ = sock.set_read_timeout(Some(rem.max(Duration::from_millis(5))));
        if let Ok((n, from)) = sock.recv_from(&mut buf)
            && let Some(ip) = local_v4(from)
        {
            records.extend(mdns_parse(&buf[..n]).into_iter().map(|(a, b)| (a, b, ip)));
        }
    }
    mdns_hits(&records, services)
        .into_iter()
        .filter(|h| h.ip.is_some_and(is_local))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_guard() {
        assert!(is_local(Ipv4Addr::new(192, 168, 1, 5)));
        assert!(is_local(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_local(Ipv4Addr::new(172, 20, 0, 1)));
        assert!(!is_local(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(!is_local(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn base64_round_trip() {
        assert_eq!(
            b64_encode(&[0xBB, 0x00, 0x01, 0xB1, 0x01, 0x0A]),
            "uwABsQEK"
        );
        assert_eq!(
            b64_encode(&[0xBB, 0x00, 0x01, 0xB1, 0x00, 0x0B]),
            "uwABsQAL"
        );
        assert_eq!(b64_encode(b"ab"), "YWI=");
        assert_eq!(b64_decode("YWI=").unwrap(), b"ab");
        assert_eq!(
            b64_decode("uwABsQEK").unwrap(),
            vec![0xBB, 0, 1, 0xB1, 1, 0x0A]
        );
    }

    #[test]
    fn request_and_responses() {
        let r = build_request("PUT", "10.0.0.2:16021", "/api/v1/x/state", &[], Some(b"{}"));
        let t = String::from_utf8(r).unwrap();
        assert!(t.starts_with("PUT /api/v1/x/state HTTP/1.1\r\nHost: 10.0.0.2:16021\r\n"));
        assert!(t.ends_with("Content-Length: 2\r\n\r\n{}"));
        let fixed = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(parse_response(fixed, false).unwrap().body, b"{}");
        assert!(parse_response(&fixed[..fixed.len() - 1], false).is_none());
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;x\r\npedia\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked, false).unwrap().body, b"Wikipedia");
        let empty = b"HTTP/1.1 204 No Content\r\n\r\n";
        let r = parse_response(empty, false).unwrap();
        assert_eq!((r.status, r.body.len()), (204, 0));
        let eof = b"HTTP/1.0 403 Forbidden\r\n\r\nno";
        assert!(parse_response(eof, false).is_none());
        assert_eq!(parse_response(eof, true).unwrap().status, 403);
    }

    #[test]
    fn mdns_query_bytes() {
        let q = mdns_query(&["_hue._tcp.local."]);
        let mut want = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        want.extend_from_slice(b"\x04_hue\x04_tcp\x05local\x00\x00\x0c\x80\x01");
        assert_eq!(q, want);
    }

    fn name(p: &mut Vec<u8>, s: &str) {
        for l in s.split('.') {
            p.push(l.len() as u8);
            p.extend_from_slice(l.as_bytes());
        }
        p.push(0);
    }

    fn rr(p: &mut Vec<u8>, kind: u16, rd: &[u8]) {
        p.extend_from_slice(&kind.to_be_bytes());
        p.extend_from_slice(&[0x80, 1, 0, 0, 0, 120]);
        p.extend_from_slice(&(rd.len() as u16).to_be_bytes());
        p.extend_from_slice(rd);
    }

    #[test]
    fn mdns_response_with_pointers() {
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 3];
        let svc = p.len();
        name(&mut p, "_wled._tcp.local");
        let mut ptr = b"\x04desk".to_vec();
        ptr.extend_from_slice(&[0xC0, svc as u8]);
        let inst = p.len() + 10;
        rr(&mut p, T_PTR, &ptr);
        p.extend_from_slice(&[0xC0, inst as u8]);
        let mut srv = vec![0, 0, 0, 0, 0, 80, 4];
        srv.extend_from_slice(b"desk");
        srv.extend_from_slice(&[0xC0, (svc + 11) as u8]);
        rr(&mut p, T_SRV, &srv);
        p.extend_from_slice(&[0xC0, inst as u8]);
        rr(&mut p, T_TXT, b"\x06mac=ab\x02id");
        name(&mut p, "desk.local");
        rr(&mut p, T_A, &[192, 168, 1, 40]);
        let recs: Vec<_> = mdns_parse(&p)
            .into_iter()
            .map(|(a, b)| (a, b, Ipv4Addr::new(192, 168, 1, 99)))
            .collect();
        assert_eq!(recs.len(), 4);
        let hits = mdns_hits(&recs, &["_wled._tcp.local."]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].instance, "desk._wled._tcp.local");
        assert_eq!(hits[0].port, 80);
        assert_eq!(hits[0].ip, Some(Ipv4Addr::new(192, 168, 1, 40)));
        assert_eq!(hits[0].txt.get("mac").map(String::as_str), Some("ab"));
        assert!(hits[0].txt.contains_key("id"));
    }
}
