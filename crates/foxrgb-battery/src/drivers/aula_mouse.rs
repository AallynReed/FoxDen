use std::time::{Duration, Instant};

use crate::hid::Dev;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x320F;
const PID: u16 = 0x22A0;
const REPORT: u8 = 0x04;
const BEGIN: u8 = 0x01;
const BATTERY: u8 = 0x1A;
const FINISH: u8 = 0x02;

fn request(cmd: u8) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = REPORT;
    r[3] = cmd;
    r
}

enum Frame {
    /// Link heartbeat pushed by the receiver; `false` when the mouse dropped off.
    Link(bool),
    Reply(u8),
    Other,
}

fn classify(r: &[u8]) -> Frame {
    if r.len() >= 9
        && r[0] == REPORT
        && r[1] == 0xAA
        && r[2] == 0
        && r[3] == 0xAA
        && matches!(r[8], 0 | 0xFF)
    {
        return Frame::Link(r[8] != 0);
    }
    if r.len() >= 8 && r[0] == REPORT && r[1] == 0 && r[2] == 0 && r[4..8] == [0; 4] {
        return Frame::Reply(r[3]);
    }
    Frame::Other
}

/// Level and charging from a battery reply; 0 is the receiver's empty cache.
fn parse_battery(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 10 || !matches!(classify(r), Frame::Reply(BATTERY)) {
        return None;
    }
    let (pct, state) = (r[8], r[9]);
    ((1..=100).contains(&pct) && state <= 2).then_some((pct, state != 0))
}

struct Session<'a> {
    dev: &'a Dev,
    link_down: bool,
}

impl Session<'_> {
    fn ask(&mut self, cmd: u8, until: Instant) -> Option<Vec<u8>> {
        if !self
            .dev
            .write_timeout(&request(cmd), Duration::from_millis(150))
        {
            return None;
        }
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            let Some(r) = self.dev.read(left.min(Duration::from_millis(100))) else {
                continue;
            };
            match classify(&r) {
                Frame::Link(up) => self.link_down = !up,
                Frame::Reply(c) if c == cmd && (cmd != BATTERY || r.len() >= 10) => return Some(r),
                _ => {}
            }
        }
        None
    }
}

/// Begin, battery, finish; the finish is always sent so no transaction stays open. Bounded to 1 s.
fn read_battery(dev: &Dev) -> (Option<(u8, bool)>, bool) {
    dev.flush();
    let start = Instant::now();
    let mut s = Session {
        dev,
        link_down: false,
    };
    let mut out = None;
    if s.ask(BEGIN, start + Duration::from_millis(450)).is_some() {
        out = s
            .ask(BATTERY, start + Duration::from_millis(800))
            .as_deref()
            .and_then(parse_battery);
    }
    s.ask(FINISH, start + Duration::from_secs(1));
    (out, s.link_down)
}

pub fn poll_aula_mouse(ctx: &mut Ctx) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| {
        c.iface == 1 && c.page == 0xFF1C && c.usage == 0x0092 && c.in_len == 64 && c.out_len == 64
    });
    for coll in colls {
        // The id is shared with other dongles; only this name speaks the protocol.
        if coll.product != "AULA-8K Dongle" {
            continue;
        }
        let Some(dev) = Dev::open(&coll) else {
            continue;
        };
        let mut b = Battery {
            key: ctx.stable_key("aula-mouse-320F:22A0"),
            name: "AULA wireless mouse (X9 / V9)".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Mouse,
            grace_secs: 1800,
            ..Default::default()
        };
        match read_battery(&dev) {
            (Some((pct, charging)), false) => {
                b.percent = Some(pct);
                b.charging = charging;
            }
            _ => b.asleep = true,
        }
        ctx.add(b, VID, PID);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bytes: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    #[test]
    fn battery_reply() {
        assert_eq!(
            parse_battery(&frame(&[4, 0, 0, 0x1A, 0, 0, 0, 0, 63, 0])),
            Some((63, false))
        );
        assert_eq!(
            parse_battery(&frame(&[4, 0, 0, 0x1A, 0, 0, 0, 0, 63, 1])),
            Some((63, true))
        );
        assert_eq!(
            parse_battery(&frame(&[4, 0, 0, 0x1A, 0, 0, 0, 0, 0, 0])),
            None
        );
        assert_eq!(
            parse_battery(&frame(&[4, 0, 0, 0x1A, 0, 0, 0, 0, 50, 3])),
            None
        );
        assert_eq!(
            parse_battery(&frame(&[4, 0, 0, 0x01, 0, 0, 0, 0, 50, 0])),
            None
        );
    }

    #[test]
    fn heartbeat_marks_the_link() {
        assert!(matches!(
            classify(&frame(&[4, 0xAA, 0, 0xAA, 0, 0, 0, 0, 0])),
            Frame::Link(false)
        ));
        assert!(matches!(
            classify(&frame(&[4, 0xAA, 0, 0xAA, 0, 0, 0, 0, 0xFF])),
            Frame::Link(true)
        ));
    }
}
