use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::hid::{Coll, Dev};
use crate::util::sum8;
use crate::{Battery, Ctx, Kind};

/// A read that fails well before its timeout is an I/O error (device gone), not silence.
enum Read {
    Got(Vec<u8>),
    Silent,
    Gone,
}

fn timed_read(dev: &Dev, wait: Duration) -> Read {
    let t = Instant::now();
    match dev.read(wait) {
        Some(r) => Read::Got(r),
        None if t.elapsed() + Duration::from_millis(20) < wait => Read::Gone,
        None => Read::Silent,
    }
}

fn left(end: Instant) -> Duration {
    end.saturating_duration_since(Instant::now())
}

// --- "BY" keyboard receivers (AULA, MCHOSE, Redragon) ---

const BY_RECEIVERS: [(u16, u16); 4] = [
    (0x3554, 0xFA09),
    (0x3554, 0xFB40),
    (0x41E4, 0x2001),
    (0x05AC, 0x024F),
];
const BY_REPORT: u8 = 0x13;
const BY_BATTERY: u8 = 0x4A;
const BY_MODEL: u8 = 0x05;
const BY_RESEND: Duration = Duration::from_millis(1300);

const MODELS: &[([u8; 6], &str)] = &[
    ([0x03, 0, 0, 0, 0x02, 0x1C], "MCHOSE G87"),
    ([0x03, 0, 0, 0, 0x00, 0x77], "MCHOSE K99"),
    ([0x03, 0, 0, 0, 0x01, 0xDF], "MCHOSE K99 V2"),
    ([0x03, 0, 0, 0, 0x01, 0x32], "MCHOSE G98"),
    ([0x03, 0, 0, 0, 0x00, 0xD6], "MCHOSE X75"),
    ([0x0A, 0, 0, 0, 0x00, 0x03], "MCHOSE G98 Pro"),
    ([0x03, 0, 0, 0, 0x01, 0xC6], "MCHOSE X75 V2"),
    ([0x03, 0, 0, 0, 0x01, 0x15], "MCHOSE G75 Pro"),
    ([0x0A, 0, 0, 0, 0x00, 0x18], "MCHOSE G98 V2"),
    ([0x0A, 0, 0, 0, 0x00, 0x2B], "MCHOSE UT98"),
    ([0x0A, 0, 0, 0, 0x00, 0x01], "MCHOSE Z75"),
    ([0x0A, 0, 0, 0, 0x00, 0x09], "MCHOSE Z75S"),
    ([0x03, 0, 0, 0, 0x01, 0xC7], "MCHOSE KX75"),
    ([0x03, 0, 0, 0, 0x01, 0x74], "MCHOSE GX87"),
    ([0x03, 0, 0, 0, 0x01, 0xED], "MCHOSE GX87 V2"),
    ([0x03, 0, 0, 0, 0x01, 0x35], "MCHOSE K87"),
    ([0x03, 0, 0, 0, 0x03, 0x35], "MCHOSE G75 V2"),
    ([0x0A, 0, 0, 0, 0x00, 0x34], "MCHOSE G87 V2"),
    ([0x03, 0, 0, 0, 0x00, 0x6A], "Redragon Horus Pro"),
    ([0x03, 0, 0, 0, 0x03, 0xDA], "Redragon Horus Pro V2"),
];

fn is_apple_id(vid: u16, pid: u16) -> bool {
    (vid, pid) == (0x05AC, 0x024F)
}

fn is_by_collection(c: &Coll) -> bool {
    if is_apple_id(c.vid, c.pid) {
        return c.iface == 1
            && c.page == 0xFF02
            && c.usage == 2
            && c.in_len == 20
            && c.out_len == 20;
    }
    matches!(c.page, 0xFF02 | 0xFF04) && c.usage == 2 && c.out_len >= 20 && c.in_len >= 20
}

/// The MCHOSE dongle on the Apple id also has a feature-only FF04:0002 collection on interface 1.
fn has_mchose_layout(ctx: &Ctx, vid: u16, pid: u16) -> bool {
    !ctx.find(vid, pid, |c| {
        c.iface == 1
            && c.page == 0xFF04
            && c.usage == 2
            && c.feat_len > 0
            && c.in_len == 0
            && c.out_len == 0
    })
    .is_empty()
}

fn by_request(cmd: u8, packets: u8) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[0] = BY_REPORT;
    f[1] = cmd;
    f[2] = packets;
    f[19] = sum8(&f[..19]);
    f
}

/// A reply to `cmd`: not an echo of a request, and either summed or MCHOSE's zero-trailer single packet.
fn by_is_reply(r: &[u8], cmd: u8) -> bool {
    if r.len() < 20 || r[0] != BY_REPORT || r[1] & 0x7F != cmd || r[4..19].iter().all(|&b| b == 0) {
        return false;
    }
    r[19] == sum8(&r[..19]) || (r[19] == 0 && r[2] == 1 && r[3] == 0)
}

/// Level and charging (high nibble 1 of the state byte).
fn by_battery(r: &[u8]) -> Option<(u8, bool)> {
    (1..=100).contains(&r[5]).then_some((r[5], r[6] >> 4 == 1))
}

/// The model code sits at bytes 5..11, on some firmware one byte earlier.
fn by_model(r: &[u8]) -> Option<&'static str> {
    [5, 4].iter().find_map(|&off| {
        let code = r.get(off..off + 6)?;
        MODELS
            .iter()
            .find(|(m, _)| m == code)
            .map(|&(_, name)| name)
    })
}

/// Sends the request, resending every 1.3 s (dozing keyboards miss the first) until `total` runs out.
fn by_exchange(dev: &Dev, cmd: u8, total: Duration) -> Option<Vec<u8>> {
    let start = Instant::now();
    let end = start + total;
    let mut sent = 0u32;
    dev.flush();
    loop {
        let now = Instant::now();
        if now >= end {
            return None;
        }
        if now >= start + BY_RESEND * sent {
            if !dev.write(&by_request(cmd, (sent & 1) as u8)) {
                return None;
            }
            sent += 1;
        }
        let next = (start + BY_RESEND * sent).min(end);
        let wait = next
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        match timed_read(dev, wait) {
            Read::Gone => return None,
            Read::Got(r) if by_is_reply(&r, cmd) => return Some(r),
            _ => {}
        }
    }
}

/// Backs off from receivers that never answered: another keyboard on the same id wakes on every request.
#[derive(Clone, Copy)]
struct Backoff {
    answered: bool,
    misses: u32,
    doublings: u32,
    first_miss: Option<Instant>,
    next_try: Option<Instant>,
}

impl Backoff {
    const NEW: Self = Self {
        answered: false,
        misses: 0,
        doublings: 0,
        first_miss: None,
        next_try: None,
    };

    fn waiting(&self, now: Instant) -> bool {
        !self.answered && self.next_try.is_some_and(|t| now < t)
    }

    fn missed(&mut self, now: Instant) {
        if self.answered {
            return;
        }
        let first = *self.first_miss.get_or_insert(now);
        self.misses += 1;
        if self.misses >= 3 && now - first >= Duration::from_secs(300) {
            let pause =
                Duration::from_secs(300 << self.doublings.min(4)).min(Duration::from_secs(3600));
            self.doublings += 1;
            self.next_try = Some(now + pause);
        }
    }
}

static BACKOFF: Mutex<[Backoff; 4]> = Mutex::new([Backoff::NEW; 4]);

fn poll_by_receivers(ctx: &mut Ctx) {
    let mut states = BACKOFF.lock().unwrap_or_else(|e| e.into_inner());
    for (i, &(vid, pid)) in BY_RECEIVERS.iter().enumerate() {
        let st = &mut states[i];
        if ctx.find(vid, pid, |_| true).is_empty() {
            *st = Backoff::NEW;
        }
        if ctx.is_claimed(vid, pid) || st.waiting(Instant::now()) {
            continue;
        }
        if is_apple_id(vid, pid) && !has_mchose_layout(ctx, vid, pid) {
            continue;
        }
        let (mut tried, mut answered) = (false, false);
        for coll in ctx.find(vid, pid, is_by_collection) {
            let Some(dev) = Dev::open(&coll) else {
                continue;
            };
            let generic = coll.product.contains("2.4G Wireless Receiver")
                || coll.product.contains("MCHOSE 2.4G Wireless");
            let model = if generic || is_apple_id(vid, pid) {
                by_exchange(&dev, BY_MODEL, Duration::from_millis(1300))
                    .as_deref()
                    .and_then(by_model)
            } else {
                None
            };
            tried = true;
            let Some((pct, charging)) = by_exchange(&dev, BY_BATTERY, Duration::from_secs(4))
                .as_deref()
                .and_then(by_battery)
            else {
                continue;
            };
            answered = true;
            let name = match model {
                Some(m) => m.to_string(),
                None if is_apple_id(vid, pid) && !generic && !coll.product.is_empty() => {
                    coll.product.clone()
                }
                None if vid == 0x41E4 || is_apple_id(vid, pid) => "MCHOSE wireless keyboard".into(),
                None => "AULA wireless keyboard".into(),
            };
            let b = Battery {
                key: ctx.stable_key(&format!("aula-{vid:04X}:{pid:04X}")),
                name,
                via: "2.4 GHz receiver".into(),
                kind: Kind::Keyboard,
                percent: Some(pct),
                charging,
                grace_secs: 1800,
                ..Default::default()
            };
            ctx.add(b, vid, pid);
            break;
        }
        if answered {
            st.answered = true;
        } else if tried {
            st.missed(Instant::now());
        }
    }
}

// --- HERO 68 Air on the "8K Wireless Receiver" (372E:106C) ---

const HERO_VID: u16 = 0x372E;
const HERO_PID: u16 = 0x106C;
const HERO_ID: u8 = 0x09;
const HERO_MAGIC: u8 = 0x66;
const T_DATA: u8 = 0x0E;
const T_ACK: u8 = 0x00;
const T_HOST_END: u8 = 0x08;
const T_DEV_END: u8 = 0x07;
const HERO_RESEND: Duration = Duration::from_millis(150);

/// Bytes 0..5 of a frame; the 3-bit sequence counter rides in the top bits of bytes 2, 3 and 4.
fn hero_header(counter: u8, index: u8, kind: u8) -> [u8; 20] {
    let bit = |mask: u8| if counter & mask != 0 { 0x80 } else { 0 };
    let mut f = [0u8; 20];
    f[..5].copy_from_slice(&[
        HERO_ID,
        HERO_MAGIC,
        0x05 | bit(4),
        index | bit(2),
        kind | bit(1),
    ]);
    f
}

fn hero_data(counter: u8, index: u8, msg: &[u8; 56]) -> [u8; 20] {
    let mut f = hero_header(counter, index, T_DATA);
    let at = (index as usize - 1) * 14;
    f[5..19].copy_from_slice(&msg[at..at + 14]);
    f[19] = sum8(&f[1..19]);
    f
}

fn hero_trailer(counter: u8, msg: &[u8; 56]) -> [u8; 20] {
    let mut f = hero_header(counter, 5, T_HOST_END);
    f[12] = 0xFF - sum8(msg);
    f[13] = sum8(&f[1..13]);
    f
}

fn hero_ack(r: &[u8]) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[..5].copy_from_slice(&[HERO_ID, HERO_MAGIC, r[2], r[3], r[4] & 0x80]);
    f[5] = sum8(&f[1..5]);
    f
}

fn hero_acks(r: &[u8], sent: &[u8]) -> bool {
    r[4] & 0x7F == T_ACK
        && r[2] == sent[2]
        && r[3] == sent[3]
        && r[4] & 0x80 == sent[4] & 0x80
        && r[5] == sum8(&r[1..5])
}

#[derive(Debug, PartialEq)]
enum HeroNews {
    /// The answer to our request: level and flags.
    Reply(u8, u8),
    /// Pushed by the keyboard on its own.
    Pushed(u8, u8),
    None,
}

/// First data frame of a keyboard message; the payload starts at byte 5.
fn hero_news(r: &[u8]) -> HeroNews {
    if r[5] == 0x87 && (1..=100).contains(&r[11]) {
        HeroNews::Reply(r[11], r[12])
    } else if r[5] == 0xFE && r[6] == 0x05 && (1..=100).contains(&r[7]) {
        HeroNews::Pushed(r[7], r[8])
    } else {
        HeroNews::None
    }
}

struct Hero<'a> {
    dev: &'a Dev,
    end: Instant,
    level: Option<(u8, u8)>,
    in_reply: bool,
    reply_done: bool,
    gone: bool,
}

impl Hero<'_> {
    /// One frame of ours; keyboard data and trailers are acknowledged on the way. False on silence or I/O error.
    fn next_frame(&mut self, wait: Duration) -> Option<Vec<u8>> {
        let until = Instant::now() + wait;
        loop {
            let w = left(until);
            if w.is_zero() {
                return None;
            }
            let r = match timed_read(self.dev, w) {
                Read::Got(r) => r,
                Read::Gone => {
                    self.gone = true;
                    return None;
                }
                Read::Silent => return None,
            };
            if r.len() < 20 || r[0] != HERO_ID || r[1] != HERO_MAGIC {
                continue;
            }
            match r[4] & 0x7F {
                T_DATA => {
                    // A damaged frame gets no ACK, so the keyboard sends it again.
                    if r[19] != sum8(&r[1..19]) {
                        continue;
                    }
                    self.dev.write(&hero_ack(&r));
                    if r[3] & 0x7F == 1 {
                        self.in_reply = false;
                        match hero_news(&r) {
                            HeroNews::Reply(l, f) => {
                                self.level = Some((l, f));
                                self.in_reply = true;
                            }
                            HeroNews::Pushed(l, f) if self.level.is_none() => {
                                self.level = Some((l, f))
                            }
                            _ => {}
                        }
                    }
                }
                T_DEV_END => {
                    self.dev.write(&hero_ack(&r));
                    self.reply_done |= self.in_reply;
                    self.in_reply = false;
                }
                _ => {}
            }
            return Some(r);
        }
    }

    fn send(&mut self, f: &[u8; 20]) -> bool {
        while !left(self.end).is_zero() {
            if !self.dev.write(f) {
                return false;
            }
            let resend = Instant::now() + HERO_RESEND;
            loop {
                let w = left(resend).min(left(self.end));
                if w.is_zero() {
                    break;
                }
                match self.next_frame(w) {
                    Some(r) if hero_acks(&r, f) => return true,
                    Some(_) => {}
                    None if self.gone => return false,
                    None => break,
                }
            }
        }
        false
    }

    fn battery(&mut self) -> Option<(u8, bool)> {
        let mut msg = [0u8; 56];
        msg[..5].copy_from_slice(&[0x09, 0x87, 0x00, 0x00, 0x01]);
        self.dev.flush();
        let mut counter = 1u8;
        for i in 1..=4 {
            if !self.send(&hero_data(counter, i, &msg)) {
                return None;
            }
            counter = (counter + 1) & 7;
        }
        if !self.send(&hero_trailer(counter, &msg)) {
            return None;
        }
        while !(self.level.is_some() && self.reply_done) {
            if self.next_frame(left(self.end)).is_none() {
                break;
            }
        }
        self.level.map(|(l, flags)| (l, flags & 0x10 != 0))
    }
}

fn poll_hero(ctx: &mut Ctx) {
    if !ctx.products(HERO_VID).contains(&(HERO_VID, HERO_PID)) {
        return;
    }
    for coll in ctx.find(HERO_VID, HERO_PID, |c| {
        c.page == 0xFF60 && c.usage == 0x61 && c.out_len >= 20 && c.in_len >= 20
    }) {
        let Some(dev) = Dev::open(&coll) else {
            continue;
        };
        let mut hero = Hero {
            dev: &dev,
            end: Instant::now() + Duration::from_secs(1),
            level: None,
            in_reply: false,
            reply_done: false,
            gone: false,
        };
        let Some((pct, charging)) = hero.battery() else {
            continue;
        };
        let b = Battery {
            key: ctx.stable_key(&format!("aula-{HERO_VID:04X}:{HERO_PID:04X}")),
            name: "AULA wireless keyboard".into(),
            via: "2.4 GHz receiver".into(),
            kind: Kind::Keyboard,
            percent: Some(pct),
            charging,
            ..Default::default()
        };
        ctx.add(b, HERO_VID, HERO_PID);
        break;
    }
}

// --- SC580 / SC580X mouse (cable 088D:062E, receiver 089D:062F) ---

const SC580: [(u16, u16, bool); 2] = [(0x088D, 0x062E, true), (0x089D, 0x062F, false)];
const SC_INFO: u8 = 0x30;
const SC_KEY: &str = "aula-sc580";

#[derive(Debug, PartialEq)]
enum ScReply {
    /// Device info data, bytes 4..31.
    Info(Vec<u8>),
    Offline,
    Busy,
    /// A data error, a bad checksum or the answer to an earlier request: ask again.
    Again,
    Ignore,
}

fn sc_request() -> [u8; 33] {
    let mut f = [0u8; 33];
    f[1] = SC_INFO;
    f[3] = 0x01;
    f
}

fn sc_classify(r: &[u8]) -> ScReply {
    if r.len() < 32 || matches!(r[0], 0xD1 | 0xD2) {
        return ScReply::Ignore;
    }
    if r[0] != SC_INFO {
        return ScReply::Again;
    }
    match r[1] {
        0 if r[31] == sum8(&r[4..31]) => ScReply::Info(r[4..31].to_vec()),
        0 => ScReply::Ignore,
        1 => ScReply::Busy,
        2 => ScReply::Offline,
        _ => ScReply::Again,
    }
}

/// `None` when the device never gave a usable answer.
fn sc_info(dev: &Dev) -> Option<ScReply> {
    let end = Instant::now() + Duration::from_secs(1);
    dev.flush();
    while !left(end).is_zero() {
        if !dev.write(&sc_request()) {
            return None;
        }
        let resend = Instant::now() + Duration::from_millis(250);
        loop {
            let w = left(resend).min(left(end));
            if w.is_zero() {
                break;
            }
            let r = match timed_read(dev, w) {
                Read::Got(r) => r,
                Read::Gone => return None,
                Read::Silent => continue,
            };
            match sc_classify(&r) {
                ScReply::Ignore => {}
                ScReply::Again => break,
                ScReply::Busy => {
                    sleep(left(end).min(Duration::from_millis(300)));
                    break;
                }
                done => return Some(done),
            }
        }
    }
    None
}

fn poll_sc580(ctx: &mut Ctx) {
    for (vid, pid, cable) in SC580 {
        if ctx.is_claimed(vid, pid) {
            continue;
        }
        // The cable answered first while the mouse charges next to its receiver: one entry.
        if ctx.out.iter().any(|b| b.key == SC_KEY) {
            ctx.claim(vid, pid);
            continue;
        }
        for coll in ctx.find(vid, pid, |c| {
            c.page == 0xFF02 && c.usage == 0 && c.out_len >= 33 && c.in_len >= 33
        }) {
            let Some(dev) = Dev::open(&coll) else {
                continue;
            };
            let Some(reply) = sc_info(&dev) else { continue };
            ctx.claim(vid, pid);
            let mut b = Battery {
                key: ctx.stable_key(SC_KEY),
                name: "AULA SC580".into(),
                via: if cable {
                    "USB cable"
                } else {
                    "2.4 GHz receiver"
                }
                .into(),
                kind: Kind::Mouse,
                grace_secs: 1800,
                vid,
                ..Default::default()
            };
            match reply {
                ScReply::Info(d) if d[10] != 0 => {
                    if d[9] > 100 {
                        break;
                    }
                    // While charging the mouse reports a fixed 100, so the level is unknown.
                    match d[8] {
                        1 => b.charging = true,
                        2 => {
                            b.full = true;
                            b.percent = Some(d[9]);
                        }
                        _ => b.percent = Some(d[9]),
                    }
                }
                // Dozing or switched off: keep the last level.
                _ => b.asleep = true,
            }
            ctx.out.push(b);
            break;
        }
    }
}

pub fn poll_aula(ctx: &mut Ctx) {
    poll_sc580(ctx);
    poll_by_receivers(ctx);
    poll_hero(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_frames() {
        let req = by_request(BY_BATTERY, 0);
        assert_eq!(req[19], 0x13 + 0x4A);

        let mut r = [0u8; 20];
        r[..7].copy_from_slice(&[0x13, 0xCA, 0x00, 0x00, 0x00, 72, 0x10]);
        r[19] = sum8(&r[..19]);
        assert!(by_is_reply(&r, BY_BATTERY));
        assert_eq!(by_battery(&r), Some((72, true)));

        r[19] = 0;
        assert!(!by_is_reply(&r, BY_BATTERY));
        r[2] = 1;
        assert!(
            by_is_reply(&r, BY_BATTERY),
            "MCHOSE single packet with a zero trailer"
        );

        assert!(!by_is_reply(&req, BY_BATTERY), "a request echo");
        r[6] = 0x01;
        assert_eq!(by_battery(&r), Some((72, false)));
    }

    #[test]
    fn by_model_code_at_either_offset() {
        let mut r = [0u8; 20];
        r[5..11].copy_from_slice(&[0x03, 0, 0, 0, 0x01, 0x35]);
        assert_eq!(by_model(&r), Some("MCHOSE K87"));
        let mut r = [0u8; 20];
        r[4..10].copy_from_slice(&[0x0A, 0, 0, 0, 0x00, 0x01]);
        assert_eq!(by_model(&r), Some("MCHOSE Z75"));
    }

    #[test]
    fn backoff_waits_after_five_minutes_of_misses() {
        let t0 = Instant::now();
        let mut b = Backoff::NEW;
        for _ in 0..3 {
            b.missed(t0);
        }
        assert!(!b.waiting(t0));
        let later = t0 + Duration::from_secs(301);
        b.missed(later);
        assert!(b.waiting(later + Duration::from_secs(299)));
        assert!(!b.waiting(later + Duration::from_secs(301)));
    }

    #[test]
    fn hero_counter_bits_and_checksums() {
        let msg = {
            let mut m = [0u8; 56];
            m[..5].copy_from_slice(&[0x09, 0x87, 0, 0, 0x01]);
            m
        };
        let f = hero_data(1, 1, &msg);
        assert_eq!(
            &f[..10],
            &[0x09, 0x66, 0x05, 0x01, 0x8E, 0x09, 0x87, 0, 0, 0x01]
        );
        assert_eq!(f[19], sum8(&f[1..19]));
        let f = hero_data(6, 2, &msg);
        assert_eq!(&f[2..5], &[0x85, 0x82, 0x0E]);

        let t = hero_trailer(5, &msg);
        assert_eq!(&t[2..5], &[0x85, 0x05, 0x88]);
        assert_eq!(t[12], 0xFF - (0x09 + 0x87 + 0x01));
        assert_eq!(t[13], sum8(&t[1..13]));

        let ack = hero_ack(&t);
        assert_eq!(
            &ack[..6],
            &[
                0x09,
                0x66,
                0x85,
                0x05,
                0x80,
                sum8(&[0x66, 0x85, 0x05, 0x80])
            ]
        );
        assert!(hero_acks(&ack, &t));
        assert!(!hero_acks(&ack, &f));
    }

    #[test]
    fn hero_reply_and_push() {
        let mut r = [0u8; 20];
        r[5..13].copy_from_slice(&[0x87, 0, 0, 0x01, 0, 0x02, 58, 0x10]);
        assert_eq!(hero_news(&r), HeroNews::Reply(58, 0x10));
        let mut r = [0u8; 20];
        r[5..9].copy_from_slice(&[0xFE, 0x05, 40, 0x00]);
        assert_eq!(hero_news(&r), HeroNews::Pushed(40, 0));
    }

    #[test]
    fn sc580_replies() {
        let mut r = vec![0u8; 32];
        r[0] = SC_INFO;
        r[4..8].copy_from_slice(b"M137");
        r[12] = 0; // charge state
        r[13] = 66;
        r[14] = 1;
        r[31] = sum8(&r[4..31]);
        let ScReply::Info(d) = sc_classify(&r) else {
            panic!()
        };
        assert_eq!((d[8], d[9], d[10]), (0, 66, 1));
        r[31] ^= 1;
        assert_eq!(sc_classify(&r), ScReply::Ignore);
        r[1] = 2;
        assert_eq!(sc_classify(&r), ScReply::Offline);
        r[0] = 0x31;
        assert_eq!(sc_classify(&r), ScReply::Again);
        r[0] = 0xD1;
        assert_eq!(sc_classify(&r), ScReply::Ignore);
    }
}
