use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::{crc16_ccitt, xor8};
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x1532;
const LEVEL: u8 = 0x21;
const CHARGING: u8 = 0x2A;
/// Polling the V3 family often costs it its wireless link.
const MIN_INTERVAL: Duration = Duration::from_secs(300);

static CACHE: Mutex<Option<HashMap<u16, (Instant, Battery)>>> = Mutex::new(None);

fn cached(pid: u16) -> Option<Battery> {
    let g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()?
        .get(&pid)
        .filter(|(at, _)| at.elapsed() < MIN_INTERVAL)
        .map(|(_, b)| b.clone())
}

/// The last reading of a V3-family headset, while it is fresh.
pub fn last_reading(pid: u16) -> Option<Battery> {
    cached(pid)
}

fn store(pid: u16, b: &Battery) {
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new)
        .insert(pid, (Instant::now(), b.clone()));
}

fn headset(pid: u16, name: &str, via: &str, percent: Option<u8>, charging: bool) -> Battery {
    Battery {
        key: format!("razer-1532:{pid:04X}"),
        name: name.into(),
        via: via.into(),
        kind: Kind::Headset,
        percent,
        charging,
        ..Battery::default()
    }
}

fn add(ctx: &mut Ctx, pid: u16, mut b: Battery) {
    b.key = ctx.stable_key(&b.key);
    ctx.add(b, VID, pid);
}

fn read_for<T>(dev: &Dev, wait: Duration, mut pick: impl FnMut(&[u8]) -> Option<T>) -> Option<T> {
    let end = Instant::now() + wait;
    loop {
        let r = dev.read(end.checked_duration_since(Instant::now())?)?;
        if let Some(v) = pick(&r) {
            return Some(v);
        }
    }
}

// BlackShark V3 / V3 Pro, interface 5: `02 00 60 00 00 00 size 00 00 dir class ..`, xor of 0..62 at 62.

/// `dir` 0x80 reaches the headset, 0x00 the dongle.
fn v3_packet(dir: u8, class: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x02;
    b[2] = 0x60;
    b[6] = 0x04;
    b[9] = dir;
    b[10] = class;
    b[62] = xor8(&b[..62]);
    b
}

fn v3_reply(class: u8, r: &[u8]) -> Option<(u8, u8)> {
    (r.len() >= 15 && r[0] == 0x02 && r[2] == 0x60 && r[10] == class).then(|| (r[13], r[14]))
}

fn v3_name(pid: u16) -> &'static str {
    match pid {
        0x0A4E | 0x0A55 => "Razer BlackShark V3 Pro for Xbox",
        0x0576 | 0x0577 => "Razer BlackShark V3 Pro",
        _ => "Razer BlackShark V3",
    }
}

fn poll_v3(ctx: &mut Ctx) {
    for pid in [0x0576, 0x0577, 0x0579, 0x057A, 0x0A4E, 0x0A55] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.iface == 5 && c.out_len >= 64 && c.in_len >= 15
        });
        let Some(c) = colls.first() else { continue };
        if let Some(b) = cached(pid) {
            add(ctx, pid, b);
            continue;
        }
        let Some(dev) = Dev::open(c) else { continue };
        for (dir, class) in [(0x00, 0x02), (0x00, CHARGING), (0x80, CHARGING)] {
            dev.write(&v3_packet(dir, class));
        }
        sleep(Duration::from_millis(250));
        dev.flush();
        let ask = |class| {
            if !dev.write(&v3_packet(0x80, class)) {
                return None;
            }
            read_for(&dev, Duration::from_millis(600), |r| v3_reply(class, r))
        };
        let Some((level @ 0..=100, ch)) = ask(LEVEL) else {
            continue;
        };
        let cable_charging = ask(CHARGING).is_some_and(|(v, _)| v == 1);
        let cable = matches!(pid, 0x0576 | 0x0579 | 0x0A4E);
        let via = if cable {
            "USB cable"
        } else {
            "2.4 GHz receiver"
        };
        let b = headset(
            pid,
            v3_name(pid),
            via,
            Some(level),
            ch != 0 || cable_charging,
        );
        store(pid, &b);
        add(ctx, pid, b);
    }
}

// Kraken V4 Pro: V3 framing on page FF00; the hub (0568) relays 0x80 to the headset, the cable (0567) takes 0x00.

const KRAKEN_V4_PRO: &[(u16, i32, u8, &str)] = &[
    (0x0567, 5, 0x00, "USB cable"),
    (0x0568, 4, 0x80, "OLED hub"),
];

fn kraken_reply(cmd: u8, r: &[u8]) -> Option<u8> {
    (r.len() >= 14 && r[0] == 0x02 && r[1] == 0x02 && r[10] == cmd && r[11] == 0x01 && r[12] >= 1)
        .then(|| r[13])
}

fn kraken_get(dev: &Dev, dir: u8, cmd: u8) -> Option<u8> {
    let req = v3_packet(dir, cmd);
    let wait = Duration::from_secs(1);
    if dev.write(&req) {
        return read_for(dev, wait, |r| kraken_reply(cmd, r));
    }
    let first = dev.set_output_and_read(&req, wait)?;
    kraken_reply(cmd, &first).or_else(|| read_for(dev, wait, |r| kraken_reply(cmd, r)))
}

fn poll_kraken_v4_pro(ctx: &mut Ctx) {
    for &(pid, iface, dir, via) in KRAKEN_V4_PRO {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| {
            c.iface == iface && c.page == 0xFF00 && c.out_len >= 64 && c.in_len >= 14
        });
        let Some(dev) = colls.first().and_then(Dev::open) else {
            continue;
        };
        dev.flush();
        let level = kraken_get(&dev, dir, LEVEL).filter(|&l| l <= 100);
        let charging = level.is_some() && kraken_get(&dev, dir, CHARGING).is_some_and(|c| c != 0);
        let mut b = headset(pid, "Razer Kraken V4 Pro", via, level, charging);
        b.key = "razer-kraken-v4-pro".into();
        b.connected = level.is_some();
        b.grace_secs = 120;
        add(ctx, pid, b);
        for &(other, ..) in KRAKEN_V4_PRO {
            ctx.claim(VID, other);
        }
        return;
    }
}

// BlackShark V3 X: feature report 07, `07 status txid 00 00 00 argc class cmd args..`, xor of 3..=60 at 62.

static V3X_TXID: AtomicU8 = AtomicU8::new(0x40);

fn v3x_request(txid: u8, cmd: u8) -> [u8; 64] {
    let mut r = [0u8; 64];
    r[0] = 0x07;
    r[2] = txid;
    r[6] = 0x02;
    r[7] = 0x07;
    r[8] = cmd;
    r[62] = xor8(&r[3..=60]);
    r
}

#[derive(Debug, PartialEq)]
enum Feature {
    Value(u8),
    Failed,
}

fn v3x_reply(txid: u8, cmd: u8, r: &[u8]) -> Option<Feature> {
    if r.len() < 63 || r[2] != txid || r[7] != 0x07 || r[8] != cmd || r[6] < 2 {
        return None;
    }
    Some(if matches!(r[1], 0x01 | 0x02) {
        Feature::Value(r[10])
    } else {
        Feature::Failed
    })
}

fn v3x_get(dev: &Dev, cmd: u8) -> Option<u8> {
    let _turn = foxden_core::hid_turn::take();
    for _ in 0..4 {
        let txid = V3X_TXID.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if !dev.set_feature(&v3x_request(txid, cmd)) {
            return None;
        }
        sleep(Duration::from_millis(60));
        match dev.get_feature(0x07).and_then(|r| v3x_reply(txid, cmd, &r)) {
            Some(Feature::Value(v)) => return Some(v),
            Some(Feature::Failed) => return None,
            None => {}
        }
    }
    None
}

fn poll_v3x(ctx: &mut Ctx) {
    for pid in [0x057D, 0x057C] {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.iface == 0 && c.feat_len >= 64);
        let Some(c) = colls.first() else { continue };
        if let Some(b) = cached(pid) {
            add(ctx, pid, b);
            continue;
        }
        let Some(dev) = Dev::open_feature_only(c) else {
            continue;
        };
        let Some(level @ 0..=100) = v3x_get(&dev, 0x80) else {
            continue;
        };
        let charging = v3x_get(&dev, 0x84) == Some(1);
        let via = if pid == 0x057D {
            "2.4 GHz receiver"
        } else {
            "USB cable"
        };
        let b = headset(pid, "Razer BlackShark V3 X", via, Some(level), charging);
        store(pid, &b);
        add(ctx, pid, b);
    }
}

// BlackShark V2 HyperSpeed (0565): `02 00 seq 00 00 00 04 00 00 80 cmd ..`, xor of 0..62 at 62.

static HS_SEQ: AtomicU8 = AtomicU8::new(0);

fn hs_request(cmd: u8, seq: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = 0x02;
    b[2] = seq;
    b[6] = 0x04;
    b[9] = 0x80;
    b[10] = cmd;
    b[62] = xor8(&b[..62]);
    b
}

fn hs_reply(cmd: u8, seq: u8, r: &[u8]) -> Option<u8> {
    let ok = r.len() >= 64
        && r[0] == 0x02
        && r[2] == seq
        && r[10] == cmd
        && r[12] >= 1
        && xor8(&r[1..62]) == r[62];
    ok.then(|| r[13])
}

fn hs_query(dev: &Dev, cmd: u8) -> Option<u8> {
    let seq = 0x60 | (HS_SEQ.fetch_add(1, Ordering::Relaxed) & 0x1F);
    dev.flush();
    if !dev.write(&hs_request(cmd, seq)) {
        return None;
    }
    read_for(dev, Duration::from_millis(650), |r| hs_reply(cmd, seq, r))
}

fn poll_v2_hyperspeed(ctx: &mut Ctx) {
    const PID: u16 = 0x0565;
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let mut colls = ctx.find(VID, PID, |c| {
        matches!(c.page, 0xFF14 | 0xFF00) && c.usage == 0x01 && c.out_len >= 64 && c.in_len >= 64
    });
    colls.sort_by_key(|c| c.page != 0xFF14);
    for c in colls {
        let Some(dev) = Dev::open(&c) else { continue };
        let Some(level @ 0..=100) = hs_query(&dev, LEVEL) else {
            continue;
        };
        let charging = hs_query(&dev, CHARGING).is_some_and(|v| v > 0);
        add(
            ctx,
            PID,
            headset(
                PID,
                "Razer BlackShark V2 HyperSpeed",
                "2.4 GHz receiver",
                Some(level),
                charging,
            ),
        );
        return;
    }
}

// BlackShark V2 Pro (2020, 0528): status request on feature report FF, then the receiver's answer.

const V2_2020_REQUEST: [u8; 9] = [0xFF, 0x0A, 0x00, 0xFD, 0x04, 0x12, 0xF1, 0x02, 0x05];
const V2_2020_FRESH: [u8; 9] = [0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];
const V2_2020_STALE: [u8; 9] = [0xFF, 0x01, 0x00, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];

#[derive(Debug, PartialEq)]
struct V2Status {
    millivolts: u32,
    percent: Option<u8>,
    charging: bool,
}

/// `None` for a report that is neither fresh nor (when `last`) the stale idle answer.
fn v2_2020_status(r: &[u8], last: bool) -> Option<V2Status> {
    if r.len() != 64 {
        return None;
    }
    if r.starts_with(&V2_2020_FRESH) {
        let mv = u16::from_be_bytes([r[12], r[13]]) as u32;
        let charging = mv != 0 && r[11] == 0x09;
        let percent = (mv != 0 && !charging && r[14] <= 100).then_some(r[14]);
        return Some(V2Status {
            millivolts: mv,
            percent,
            charging,
        });
    }
    (last && r.starts_with(&V2_2020_STALE)).then_some(V2Status {
        millivolts: 0,
        percent: None,
        charging: false,
    })
}

fn poll_v2_pro_2020(ctx: &mut Ctx) {
    const PID: u16 = 0x0528;
    const READS: usize = 5;
    if ctx.is_claimed(VID, PID) {
        return;
    }
    for c in ctx.find(VID, PID, |c| {
        c.iface == 3 && c.page == 0xFF00 && c.usage == 0x0001 && c.feat_len == 64
    }) {
        let Some(dev) = Dev::open_feature_only(&c) else {
            continue;
        };
        if !dev.set_feature(&V2_2020_REQUEST) {
            continue;
        }
        for read in 0..READS {
            sleep(Duration::from_millis(100));
            let Some(s) = dev
                .get_feature(0xFF)
                .and_then(|r| v2_2020_status(&r, read == READS - 1))
            else {
                continue;
            };
            let mut b = headset(
                PID,
                "Razer BlackShark V2 Pro (2020)",
                "2.4 GHz receiver",
                s.percent,
                s.charging,
            );
            b.connected = s.millivolts != 0;
            add(ctx, PID, b);
            return;
        }
    }
}

// BlackShark V2 Pro (2023, 0555): `02 80 total 00 00 'P' 'A' inner 00 type cmd flag`.

fn pa_frame(total: u8, inner: u8, kind: u8, cmd: u8, flag: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..12].copy_from_slice(&[
        0x02, 0x80, total, 0, 0, b'P', b'A', inner, 0, kind, cmd, flag,
    ]);
    b
}

/// Routes queries from the dongle to the headset; switched off after each query, nothing is stored.
fn pa_remote(dev: &Dev, on: bool) {
    dev.write(&pa_frame(0x07, 0x0E, 0x02, 0xE1, on as u8));
    sleep(Duration::from_millis(35));
}

fn pa_reply(cmd: u8, r: &[u8]) -> Option<u8> {
    (r.len() >= 16 && r[0] == 0x02 && r[12] == cmd && r[13] == 0x01 && r[14] >= 1).then(|| r[15])
}

fn pa_query(dev: &Dev, cmd: u8) -> Option<u8> {
    let _turn = foxden_core::hid_turn::take();
    dev.flush();
    pa_remote(dev, true);
    let mut value = None;
    for _ in 0..3 {
        pa_remote(dev, true);
        if !dev.write(&pa_frame(0x08, 0x08, 0x03, cmd, 0)) {
            break;
        }
        value = read_for(dev, Duration::from_millis(150), |r| pa_reply(cmd, r));
        if value.is_some() {
            break;
        }
    }
    pa_remote(dev, false);
    value
}

fn poll_v2_pro_2023(ctx: &mut Ctx) {
    const PID: u16 = 0x0555;
    if ctx.is_claimed(VID, PID) {
        return;
    }
    for c in ctx.find(VID, PID, |c| {
        c.page == 0xFF00 && c.out_len >= 64 && c.in_len >= 16
    }) {
        let Some(dev) = Dev::open(&c) else { continue };
        let Some(level @ 0..=100) = pa_query(&dev, LEVEL) else {
            continue;
        };
        let charging = pa_query(&dev, CHARGING).is_some_and(|v| v > 0);
        add(
            ctx,
            PID,
            headset(
                PID,
                "Razer BlackShark V2 Pro (2023)",
                "2.4 GHz receiver",
                Some(level),
                charging,
            ),
        );
        return;
    }
}

// Barracuda X: Airoha RACE through a USB bridge. 0550 frames `02 80 len crc payload` out and `02 len stream` in;
// 0552 has a different bridge with `01 80 len payload` both ways and no CRC or acknowledgement.

fn bridge_report(payload: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[..3].copy_from_slice(&[0x02, 0x80, payload.len() as u8]);
    let n = payload.len().min(59);
    f[5..5 + n].copy_from_slice(&payload[..n]);
    let crc = crc16_ccitt(0, &f).to_le_bytes();
    f[3..5].copy_from_slice(&crc);
    f
}

fn plain_report(payload: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    let n = payload.len().min(61);
    f[..3].copy_from_slice(&[0x01, 0x80, n as u8]);
    f[3..3 + n].copy_from_slice(&payload[..n]);
    f
}

/// Collects input reports into a byte stream and cuts complete `50 49` frames out of it.
#[derive(Default)]
struct Stream(Vec<u8>);

impl Stream {
    fn push(&mut self, r: &[u8]) {
        let (start, len) = match r {
            [0x02, len, ..] => (2, *len as usize),
            [0x01, 0x80, len, ..] => (3, *len as usize),
            _ => return,
        };
        if let Some(bytes) = r.get(start..start + len) {
            self.0.extend_from_slice(bytes);
        }
    }

    fn next(&mut self) -> Option<Vec<u8>> {
        let start = self
            .0
            .windows(2)
            .position(|w| w == [0x50, 0x49])
            .unwrap_or(self.0.len().saturating_sub(1));
        self.0.drain(..start);
        if self.0.len() < 10 {
            return None;
        }
        let len = 10 + u16::from_le_bytes([self.0[8], self.0[9]]) as usize;
        (self.0.len() >= len).then(|| self.0.drain(..len).collect())
    }
}

fn remote_ack(seq: u8, f: &[u8]) -> Option<bool> {
    (f.len() >= 13 && f[10] == 0x0E && f[11] == 0x80 | seq).then(|| f[12] == 0)
}

fn get_reply(cmd: u8, f: &[u8]) -> Option<Option<u8>> {
    (f.len() >= 13 && f[2] == 0x08 && f[10] == cmd && f[11] == 0x01)
        .then(|| (f[12] >= 1 && f.len() >= 14).then(|| f[13]))
}

struct Bridge {
    dev: Dev,
    stream: Stream,
    seq: u8,
    plain: bool,
}

impl Bridge {
    /// Sends `50 41 group seq body`; the 0550 bridge answers a get-input-report 02 with 'O' (taken) or 'F' (busy).
    fn command(&mut self, group: u8, body: &[u8]) -> bool {
        self.seq = self.seq % 127 + 1;
        let mut payload = vec![0x50, 0x41, group, self.seq];
        payload.extend_from_slice(body);
        if self.plain {
            return self.dev.write(&plain_report(&payload));
        }
        let f = bridge_report(&payload);
        for attempt in 0..5 {
            if attempt > 0 {
                sleep(Duration::from_millis(50));
            }
            if !self.dev.write(&f) {
                return false;
            }
            match self
                .dev
                .get_input_report(0x02)
                .and_then(|s| s.get(1).copied())
            {
                Some(b'O') => return true,
                Some(b'F') => {}
                _ => return false,
            }
        }
        false
    }

    fn await_frame<T>(&mut self, wait: Duration, pick: impl Fn(&[u8]) -> Option<T>) -> Option<T> {
        let end = Instant::now() + wait;
        loop {
            while let Some(f) = self.stream.next() {
                if let Some(v) = pick(&f) {
                    return Some(v);
                }
            }
            let r = self.dev.read(end.checked_duration_since(Instant::now())?)?;
            self.stream.push(&r);
        }
    }

    /// Routes commands to the headset; switched off again after reading, nothing is stored.
    fn remote(&mut self, on: bool) -> bool {
        if !self.command(0x0E, &[0x02, 0xE1, on as u8]) {
            return false;
        }
        let seq = self.seq;
        // The 0552 bridge does not always acknowledge; the following read proves the switch.
        let wait = Duration::from_millis(if self.plain { 400 } else { 800 });
        self.await_frame(wait, |f| remote_ack(seq, f)) == Some(true) || self.plain
    }

    fn get(&mut self, cmd: u8) -> Option<u8> {
        if !self.command(0x08, &[0x03, cmd, 0x00, 0x00]) {
            return None;
        }
        self.await_frame(Duration::from_secs(1), |f| get_reply(cmd, f))
            .flatten()
    }
}

/// The 0552 bridge answers `01 40` with `01 40 01 01` once it accepts commands.
fn plain_handshake(dev: &Dev) -> bool {
    dev.flush();
    dev.write(&[0x01, 0x40])
        && dev
            .read_until(Duration::from_millis(500), |r| {
                r.starts_with(&[0x01, 0x40, 0x01, 0x01]).then_some(())
            })
            .is_some()
}

fn poll_barracuda_x(ctx: &mut Ctx) {
    for pid in [0x0550, 0x0552] {
        if !ctx.is_claimed(VID, pid) {
            barracuda_x(ctx, pid);
        }
    }
}

fn barracuda_x(ctx: &mut Ctx, pid: u16) {
    let plain = pid == 0x0552;
    let accept = |c: &Coll| {
        c.page == 0xFF00 && (plain || c.usage == 0x0001) && c.out_len >= 64 && c.in_len >= 64
    };
    for c in ctx.find(VID, pid, accept) {
        let Some(dev) = Dev::open(&c) else { continue };
        if plain && !plain_handshake(&dev) {
            continue;
        }
        let mut link = Bridge {
            dev,
            stream: Stream::default(),
            seq: 0,
            plain,
        };
        if !link.remote(true) {
            continue;
        }
        let level = link.get(LEVEL);
        let cable = level.and_then(|_| link.get(CHARGING));
        link.remote(false);
        let Some(level @ 0..=100) = level else {
            continue;
        };
        let name = if plain {
            "Razer Barracuda X"
        } else {
            "Razer Barracuda X (2022)"
        };
        add(
            ctx,
            pid,
            headset(pid, name, "2.4 GHz receiver", Some(level), cable == Some(1)),
        );
        return;
    }
}

pub fn poll_razer_headsets(ctx: &mut Ctx) {
    poll_v2_hyperspeed(ctx);
    poll_barracuda_x(ctx);
    poll_v2_pro_2020(ctx);
    poll_v2_pro_2023(ctx);
    poll_v3x(ctx);
    poll_v3(ctx);
    poll_kraken_v4_pro(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_frames() {
        let p = v3_packet(0x80, LEVEL);
        assert_eq!(p[..11], [0x02, 0, 0x60, 0, 0, 0, 0x04, 0, 0, 0x80, 0x21]);
        assert_eq!(p[62], 0x02 ^ 0x60 ^ 0x04 ^ 0x80 ^ 0x21);
        let mut r = [0u8; 64];
        r[..15].copy_from_slice(&[
            0x02, 0, 0x60, 0, 0, 0, 0x06, 0, 0, 0x80, 0x21, 0, 0x02, 0x4E, 0x01,
        ]);
        assert_eq!(v3_reply(LEVEL, &r), Some((78, 1)));
        assert_eq!(v3_reply(CHARGING, &r), None);
    }

    #[test]
    fn kraken_v4_pro_frames() {
        let p = v3_packet(0x80, LEVEL);
        assert_eq!(p[..11], [0x02, 0, 0x60, 0, 0, 0, 0x04, 0, 0, 0x80, 0x21]);
        assert_eq!(p[62], 0xC7);
        assert!(p[11..62].iter().all(|&b| b == 0) && p[63] == 0);
        assert_eq!(v3_packet(0x80, CHARGING)[62], 0xCC);
        assert_eq!(
            v3_packet(0x00, CHARGING)[..11],
            [0x02, 0, 0x60, 0, 0, 0, 0x04, 0, 0, 0x00, 0x2A]
        );
        let mut r = [0u8; 64];
        r[..14].copy_from_slice(&[
            0x02, 0x02, 0x60, 0, 0, 0, 0x05, 0, 0, 0x80, 0x21, 0x01, 0x01, 0x4B,
        ]);
        assert_eq!(kraken_reply(LEVEL, &r), Some(75));
        assert_eq!(kraken_reply(CHARGING, &r), None);
        r[1] = 0x03;
        assert_eq!(kraken_reply(LEVEL, &r), None);
        r[1] = 0x02;
        r[11] = 0x00;
        assert_eq!(kraken_reply(LEVEL, &r), None);
    }

    #[test]
    fn v3x_frames() {
        let q = v3x_request(0x41, 0x80);
        assert_eq!(q[..9], [0x07, 0, 0x41, 0, 0, 0, 0x02, 0x07, 0x80]);
        assert_eq!(q[62], 0x02 ^ 0x07 ^ 0x80);
        let mut r = q;
        r[1] = 0x02;
        r[10] = 0x3C;
        assert_eq!(v3x_reply(0x41, 0x80, &r), Some(Feature::Value(60)));
        assert_eq!(v3x_reply(0x42, 0x80, &r), None);
        r[1] = 0x04;
        assert_eq!(v3x_reply(0x41, 0x80, &r), Some(Feature::Failed));
    }

    #[test]
    fn hyperspeed_frames() {
        let q = hs_request(LEVEL, 0x63);
        assert_eq!(q[..11], [0x02, 0, 0x63, 0, 0, 0, 0x04, 0, 0, 0x80, 0x21]);
        let mut r = q;
        r[12] = 0x01;
        r[13] = 0x5A;
        r[62] = xor8(&r[1..62]);
        assert_eq!(hs_reply(LEVEL, 0x63, &r), Some(90));
        assert_eq!(hs_reply(LEVEL, 0x64, &r), None);
        r[62] ^= 0xFF;
        assert_eq!(hs_reply(LEVEL, 0x63, &r), None);
    }

    #[test]
    fn v2_pro_2020_answers() {
        let mut r = [0u8; 64];
        r[..9].copy_from_slice(&V2_2020_FRESH);
        r[11] = 0x01;
        r[12..15].copy_from_slice(&[0x0F, 0x3C, 0x41]);
        assert_eq!(
            v2_2020_status(&r, false),
            Some(V2Status {
                millivolts: 3900,
                percent: Some(65),
                charging: false
            })
        );
        r[11] = 0x09;
        assert_eq!(v2_2020_status(&r, false).unwrap().percent, None);
        let mut s = [0u8; 64];
        s[..9].copy_from_slice(&V2_2020_STALE);
        assert_eq!(v2_2020_status(&s, false), None);
        assert_eq!(v2_2020_status(&s, true).unwrap().millivolts, 0);
    }

    #[test]
    fn pa_frames() {
        assert_eq!(
            pa_frame(0x08, 0x08, 0x03, LEVEL, 0)[..12],
            [0x02, 0x80, 0x08, 0, 0, 0x50, 0x41, 0x08, 0, 0x03, 0x21, 0]
        );
        let mut r = [0u8; 64];
        r[0] = 0x02;
        r[12..16].copy_from_slice(&[0x21, 0x01, 0x01, 0x46]);
        assert_eq!(pa_reply(LEVEL, &r), Some(70));
        r[13] = 0x02;
        assert_eq!(pa_reply(LEVEL, &r), None);
    }

    #[test]
    fn bridge_framing_and_stream() {
        let f = bridge_report(&[0x50, 0x41, 0x08, 0x01, 0x03, 0x21, 0x00, 0x00]);
        assert_eq!(f[..3], [0x02, 0x80, 0x08]);
        let mut z = f;
        z[3] = 0;
        z[4] = 0;
        assert_eq!(u16::from_le_bytes([f[3], f[4]]), crc16_ccitt(0, &z));

        let frame = [
            0x50, 0x49, 0x08, 0x81, 0, 0, 0, 0, 0x04, 0x00, 0x21, 0x01, 0x01, 0x37,
        ];
        let mut s = Stream::default();
        let mut r1 = vec![0x02, 9, 0xAA, 0xBB];
        r1.extend_from_slice(&frame[..7]);
        let mut r2 = vec![0x02, 7];
        r2.extend_from_slice(&frame[7..]);
        s.push(&r1);
        assert_eq!(s.next(), None);
        s.push(&r2);
        let got = s.next().unwrap();
        assert_eq!(got, frame);
        assert_eq!(get_reply(LEVEL, &got), Some(Some(0x37)));
        let ack = [0x50, 0x49, 0x0E, 0, 0, 0, 0, 0, 0x03, 0, 0x0E, 0x85, 0x00];
        assert_eq!(remote_ack(5, &ack), Some(true));
        assert_eq!(remote_ack(6, &ack), None);
    }

    #[test]
    fn plain_bridge_framing() {
        let f = plain_report(&[0x50, 0x41, 0x08, 0x02, 0x03, 0x2A, 0x00, 0x00]);
        assert_eq!(
            f[..11],
            [0x01, 0x80, 0x08, 0x50, 0x41, 0x08, 0x02, 0x03, 0x2A, 0, 0]
        );
        let frame = [
            0x50, 0x49, 0x08, 0x81, 0, 0, 0, 0, 0x04, 0x00, 0x21, 0x01, 0x01, 0x25,
        ];
        let mut r = vec![0x01, 0x80, frame.len() as u8];
        r.extend_from_slice(&frame);
        r.resize(64, 0);
        let mut s = Stream::default();
        s.push(&r);
        assert_eq!(get_reply(LEVEL, &s.next().unwrap()), Some(Some(37)));
        s.push(&[0x01, 0x40, 0x01, 0x01]);
        assert_eq!(s.next(), None);
    }
}
