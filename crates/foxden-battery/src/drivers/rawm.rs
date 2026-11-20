use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1915;

const RECEIVERS: [u16; 9] = [
    0x232A, 0x232B, 0x2335, 0x2336, 0x233B, 0x233C, 0x2346, 0x2347, 0x234B,
];
const MICE: [u16; 25] = [
    0x2328, 0x2329, 0x232C, 0x232D, 0x232E, 0x232F, 0x2330, 0x2331, 0x2332, 0x2333, 0x2334, 0x2337,
    0x2338, 0x2339, 0x233A, 0x233E, 0x233F, 0x2340, 0x2341, 0x2343, 0x2344, 0x2348, 0x2349, 0x234A,
    0x2352,
];

const CMD_QUERY: u8 = 0x01;
const CMD_QUERY_RESULT: u8 = 0x02;
const CMD_NOTIFY: u8 = 0x0B;
const CMD_PING: u8 = 0x0E;
const NOTIFY_BATTERY: u8 = 0x17;
const OS_PC: u8 = 0x03;

/// Output report: id 0, optional `C0|channel` prefix, `80|size`, then the event.
fn packet(event: &[u8], channel: Option<u8>) -> Vec<u8> {
    let mut p = vec![0x00];
    if let Some(ch) = channel {
        p.push(0xC0 | (ch & 0x3F));
    }
    p.push(0x80 | (event.len() as u8 & 0x3F));
    p.extend_from_slice(event);
    p
}

fn query_event(unix_secs: u64) -> Vec<u8> {
    let mut e = vec![CMD_QUERY, 0, OS_PC, 0, 0];
    e.extend_from_slice(&unix_secs.to_le_bytes());
    e[1] = e.len() as u8;
    e
}

const PING_EVENT: [u8; 4] = [CMD_PING, 4, 0, 0];

/// Reassembled device-to-host events of one link: `FF FF FF FF`, `[cmd | len hi << 4][len lo]`, body.
#[derive(Default)]
struct Events {
    buf: Vec<u8>,
    info: Option<String>,
    percent: Option<u8>,
    charging: bool,
}

impl Events {
    fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        if self.buf.len() > 8192 {
            self.buf.drain(..self.buf.len() - 4096);
        }
        let mut at = 0;
        while at + 6 <= self.buf.len() {
            if self.buf[at..at + 4] != [0xFF; 4] {
                at += 1;
                continue;
            }
            let h = at + 4;
            let cmd = self.buf[h] & 0x0F;
            let len = (((self.buf[h] as usize) << 4) & 0xF00) | self.buf[h + 1] as usize;
            if len < 2 {
                at = h;
                continue;
            }
            if h + len > self.buf.len() {
                break;
            }
            let body = self.buf[h + 2..h + len].to_vec();
            self.event(cmd, &body);
            at = h + len;
        }
        self.buf.drain(..at);
    }

    fn event(&mut self, cmd: u8, body: &[u8]) {
        match cmd {
            CMD_QUERY_RESULT => {
                let text = body.split(|&b| b == 0).next().unwrap_or_default();
                let Ok(json) = std::str::from_utf8(text) else {
                    return;
                };
                if !json.starts_with('{') {
                    return;
                }
                if let Some(v) = json_int(json, "battery").filter(|v| (0..=100).contains(v)) {
                    self.percent = Some(v as u8);
                }
                if let Some(v) = json_int(json, "chr") {
                    self.charging = v == 1;
                }
                self.info = Some(json.to_string());
            }
            CMD_NOTIFY if body.len() >= 3 && body[0] == NOTIFY_BATTERY && body[1] <= 100 => {
                self.percent = Some(body[1]);
                self.charging = body[2] == 1;
            }
            _ => {}
        }
    }
}

/// The USB device itself, and the mouse behind a receiver (channel 0).
#[derive(Default)]
struct Link {
    direct: Events,
    mouse: Events,
}

impl Link {
    fn feed(&mut self, mut r: &[u8]) {
        let mut target = &mut self.direct;
        if let Some(&b) = r.first()
            && b & 0xC0 == 0xC0
        {
            if b & 0x3F != 0 {
                return;
            }
            target = &mut self.mouse;
            r = &r[1..];
        }
        let Some(&head) = r.first() else { return };
        if head & 0xC0 != 0x80 {
            return;
        }
        let n = ((head & 0x3F) as usize).min(r.len() - 1);
        target.push(&r[1..1 + n]);
    }

    fn events(&self, channel: Option<u8>) -> &Events {
        if channel.is_some() {
            &self.mouse
        } else {
            &self.direct
        }
    }
}

fn json_int(json: &str, key: &str) -> Option<i64> {
    let rest = &json[json.find(&format!("\"{key}\":"))? + key.len() + 3..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let rest = &json[json.find(&format!("\"{key}\":\""))? + key.len() + 4..];
    rest.find('"').map(|e| &rest[..e])
}

/// Asks for the device info and keeps pinging: the device only sends while the host does.
fn device_info(dev: &Dev, link: &mut Link, channel: Option<u8>, deadline: Instant) -> bool {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let send = |e: &[u8]| dev.write_timeout(&packet(e, channel), Duration::from_millis(200));
    if !send(&query_event(secs)) {
        return false;
    }
    while Instant::now() < deadline {
        if !send(&PING_EVENT) {
            return false;
        }
        let pump_end = (Instant::now() + Duration::from_millis(80)).min(deadline);
        while let Some(left) = pump_end.checked_duration_since(Instant::now()) {
            let Some(r) = dev.read(left) else { break };
            link.feed(&r);
        }
        if link.events(channel).info.is_some() {
            return true;
        }
    }
    false
}

fn display_name(dn: Option<&str>) -> String {
    let name = dn.filter(|n| !n.is_empty()).unwrap_or("wireless mouse");
    if name.contains("RAWM") || name.contains("SADES") {
        name.to_string()
    } else {
        format!("RAWM {name}")
    }
}

pub fn poll_rawm(ctx: &mut Ctx) {
    for (vid, pid) in ctx.products(VID) {
        let receiver = RECEIVERS.contains(&pid);
        if !receiver && !MICE.contains(&pid) {
            continue;
        }
        let colls = ctx.find(vid, pid, |c| {
            c.page == 0xFF00 && c.usage == 0x0001 && c.in_len == 65 && c.out_len == 65
        });
        for c in colls {
            let Some(dev) = Dev::open(&c) else { continue };
            dev.flush();
            // One deadline for the receiver hello and the mouse's info behind it.
            let deadline = Instant::now() + Duration::from_millis(1200);
            let mut link = Link::default();
            if !device_info(&dev, &mut link, None, deadline) {
                continue;
            }
            let channel = receiver.then_some(0);
            if receiver && !device_info(&dev, &mut link, channel, deadline) {
                // receiver there, mouse off or out of range
                ctx.claim(vid, pid);
                break;
            }
            let ev = link.events(channel);
            let Some(percent) = ev.percent else { continue };
            let json = ev.info.as_deref().unwrap_or_default();
            let mouse_pid = json_int(json, "pi")
                .filter(|p| (1..=0xFFFF).contains(p))
                .map_or(pid, |p| p as u16);
            let base = format!("rawm-1915:{mouse_pid:04X}");
            let b = Battery {
                key: base.clone(),
                name: display_name(json_str(json, "dn")),
                via: if receiver {
                    "2.4 GHz receiver"
                } else {
                    "USB cable"
                }
                .into(),
                kind: Kind::Mouse,
                percent: Some(percent),
                charging: ev.charging,
                grace_secs: 1800,
                ..Default::default()
            };
            // A charging mouse can answer on both its cable and the receiver: one entry, the cable's.
            match ctx.out.iter_mut().find(|o| o.key == base) {
                Some(existing) => {
                    if !receiver {
                        *existing = Battery { vid, ..b };
                    }
                    ctx.claim(vid, pid);
                }
                None => ctx.add(b, vid, pid),
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(payload: &[u8], channel: Option<u8>) -> Vec<Vec<u8>> {
        payload
            .chunks(60)
            .map(|c| {
                let mut f = Vec::new();
                if let Some(ch) = channel {
                    f.push(0xC0 | ch);
                }
                f.push(0x80 | c.len() as u8);
                f.extend_from_slice(c);
                f.resize(64, 0);
                f
            })
            .collect()
    }

    #[test]
    fn packets_and_query() {
        assert_eq!(
            packet(&PING_EVENT, Some(0)),
            [0x00, 0xC0, 0x84, 0x0E, 4, 0, 0]
        );
        let q = query_event(0x0102);
        assert_eq!(q.len(), 13);
        assert_eq!(&q[..7], &[CMD_QUERY, 13, OS_PC, 0, 0, 0x02, 0x01]);
    }

    #[test]
    fn info_split_over_reports_reaches_the_mouse_stream() {
        let json = br#"{"dn":"Leviathan V4","pi":9034,"battery":63,"chr":1}"#;
        let len = json.len() + 3;
        let mut ev = vec![
            0xFF,
            0xFF,
            0xFF,
            0xFF,
            CMD_QUERY_RESULT | ((len >> 4) as u8 & 0xF0),
            len as u8,
        ];
        ev.extend_from_slice(json);
        ev.push(0);
        let mut link = Link::default();
        for f in frames(&ev, Some(0)) {
            link.feed(&f);
        }
        let m = &link.mouse;
        assert_eq!((m.percent, m.charging), (Some(63), true));
        let info = m.info.as_deref().unwrap();
        assert_eq!(json_int(info, "pi"), Some(0x234A));
        assert_eq!(display_name(json_str(info, "dn")), "RAWM Leviathan V4");
        assert!(link.direct.info.is_none());
    }

    #[test]
    fn battery_notification() {
        let mut e = Events::default();
        e.push(&[0xFF, 0xFF, 0xFF, 0xFF, CMD_NOTIFY, 5, NOTIFY_BATTERY, 40, 0]);
        assert_eq!((e.percent, e.charging), (Some(40), false));
    }
}
