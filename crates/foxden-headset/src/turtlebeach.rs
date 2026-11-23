//! Turtle Beach Stealth Pro II and Atlas Air: settings are text key/value pairs carried in vendor
//! commands on report 6, with replies read back as input report 7.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use foxden_battery::hid::{Coll, Dev};

use crate::proto::{EQ_BANDS, EQ_CUSTOM, Family, Model, Shape};
use crate::{Control, Found, Reading, Status};

pub const VID: u16 = 0x10F5;
const OUT_ID: u8 = 0x06;
const IN_ID: u8 = 0x07;
const PAGE: u16 = 0xFF13;
const REPORT_LEN: usize = 62;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ProII,
    AtlasAir,
}

/// (pid, name, kind, on its own cable).
const MODELS: &[(u16, &str, Kind, bool)] = &[
    (0x2283, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x2284, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x2285, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x229F, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x2286, "Stealth Pro II Xbox", Kind::ProII, true),
    (0x229B, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x229C, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x229D, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x22A0, "Stealth Pro II Xbox", Kind::ProII, false),
    (0x229E, "Stealth Pro II Xbox", Kind::ProII, true),
    (0x2287, "Stealth Pro II", Kind::ProII, false),
    (0x2288, "Stealth Pro II", Kind::ProII, false),
    (0x2289, "Stealth Pro II", Kind::ProII, true),
    (0x225E, "Atlas Air", Kind::AtlasAir, false),
    (0x2260, "Atlas Air", Kind::AtlasAir, true),
];

fn kind(m: &Model) -> Kind {
    MODELS
        .iter()
        .find(|r| r.0 == m.pid)
        .map_or(Kind::ProII, |r| r.2)
}

pub fn model(pid: u16) -> Option<Model> {
    let &(pid, name, kind, cable) = MODELS.iter().find(|r| r.0 == pid)?;
    Some(Model {
        vid: VID,
        pid,
        name,
        family: Family::TurtleBeach,
        sidetone: true,
        sidetone_max: 0,
        mic: false,
        anc: kind == Kind::ProII,
        wireless: !cable,
    })
}

pub fn fits(c: &Coll) -> bool {
    c.page == PAGE && c.in_len > 0
}

const AUTO_OFF: [(i32, &str); 5] = [
    (0, "Never"),
    (5, "5 min"),
    (10, "10 min"),
    (20, "20 min"),
    (30, "30 min"),
];
const EQ_PRESETS: [(i32, &str); 4] = [
    (0, "Signature Sound"),
    (1, "Bass Boost"),
    (2, "Bass and Treble Boost"),
    (3, "Vocal Boost"),
];
const HEARING_MODES: [(i32, &str); 3] = [(0, "Legacy"), (1, "Footsteps"), (2, "Gunshots")];
/// Band gains in half-dB steps; the headset holds ±9 dB.
const BAND_MAX: i32 = 18;

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let pct = || Shape::Steps { min: 0, max: 100 };
    let choice = |o: &[(i32, &'static str)]| Shape::Choice {
        options: o.to_vec(),
    };
    match kind(m) {
        Kind::ProII => vec![
            (Control::Sidetone, pct()),
            (Control::AutoOff, choice(&AUTO_OFF)),
            (Control::Anc, Shape::Switch),
            (Control::AncLevel, pct()),
            (Control::NoiseGate, Shape::Switch),
            (Control::NoiseGateLevel, pct()),
            (Control::AiNoiseReduction, Shape::Switch),
            (Control::Superhuman, Shape::Switch),
            (Control::SuperhumanMode, choice(&HEARING_MODES)),
            (Control::SuperhumanLevel, pct()),
            (Control::VoicePrompts, pct()),
            (Control::WakeOnMotion, Shape::Switch),
            (Control::EqPreset, choice(&EQ_PRESETS)),
        ],
        Kind::AtlasAir => vec![
            (Control::Sidetone, pct()),
            (Control::AutoOff, choice(&AUTO_OFF)),
            (Control::Superhuman, Shape::Switch),
            (Control::SuperhumanMode, choice(&HEARING_MODES)),
            (Control::SuperhumanLevel, pct()),
            (Control::VoicePrompts, Shape::Steps { min: 0, max: 15 }),
        ],
    }
}

pub fn has_eq(m: &Model) -> bool {
    m.family == Family::TurtleBeach && kind(m) == Kind::ProII
}

/// The key each control is kept at.
fn key(k: Kind, c: Control) -> Option<u16> {
    Some(match (k, c) {
        (_, Control::AutoOff) => 0x280,
        (Kind::ProII, Control::Sidetone) => 0x620,
        (Kind::ProII, Control::Anc) => 0x750,
        (Kind::ProII, Control::AncLevel) => 0x760,
        (Kind::ProII, Control::NoiseGate) => 0x700,
        (Kind::ProII, Control::NoiseGateLevel) => 0x710,
        (Kind::ProII, Control::AiNoiseReduction) => 0x630,
        (Kind::ProII, Control::Superhuman) => 0x720,
        (Kind::ProII, Control::SuperhumanMode) => 0x730,
        (Kind::ProII, Control::SuperhumanLevel) => 0x740,
        (Kind::ProII, Control::VoicePrompts) => 0x2D0,
        (Kind::ProII, Control::WakeOnMotion) => 0x2E0,
        (Kind::ProII, Control::EqPreset) => 0x1210,
        (Kind::AtlasAir, Control::Sidetone) => 0x630,
        (Kind::AtlasAir, Control::Superhuman) => 0x800,
        (Kind::AtlasAir, Control::SuperhumanMode) => 0x810,
        (Kind::AtlasAir, Control::SuperhumanLevel) => 0x820,
        (Kind::AtlasAir, Control::VoicePrompts) => 0xC00,
        _ => return None,
    })
}

fn to_wire(c: Control, v: i32) -> i32 {
    match c {
        Control::AutoOff => AUTO_OFF.iter().position(|o| o.0 == v).unwrap_or(0) as i32,
        Control::EqPreset => v + 1,
        _ => v,
    }
}

fn from_wire(c: Control, w: i32) -> Option<i32> {
    match c {
        Control::AutoOff => AUTO_OFF.get(usize::try_from(w).ok()?).map(|o| o.0),
        Control::EqPreset => match w {
            1..=4 => Some(w - 1),
            0 | 16..=20 => Some(EQ_CUSTOM),
            _ => None,
        },
        _ => Some(w),
    }
}

static COUNTER: AtomicU16 = AtomicU16::new(0);

/// `[06][total][05 5A][inner][02 99][48][01|03][FB][counter][00 x7][tag][verb][FF json]`.
fn request(verb: &str, arg: Option<&str>, counter: u16) -> Vec<u8> {
    let mut body = vec![
        0x02,
        0x99,
        0x48,
        if arg.is_some() { 0x03 } else { 0x01 },
        0xFB,
    ];
    body.extend(counter.to_be_bytes());
    body.extend([0; 7]);
    body.extend_from_slice(if arg.is_some() {
        &[0xB7]
    } else {
        &[0x61, 0x00]
    });
    body.extend(verb.bytes());
    if let Some(a) = arg {
        body.push(0xFF);
        body.extend(a.bytes());
    }
    let inner = (body.len() as u16).to_le_bytes();
    let total = (body.len() as u16 + 4).to_le_bytes();
    let mut f = vec![OUT_ID, total[0], total[1], 0x05, 0x5A, inner[0], inner[1]];
    f.extend(body);
    f
}

fn next() -> u16 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn set_frame(key: u16, value: i32) -> Vec<u8> {
    request(
        "set_kvp",
        Some(&format!("{{\"0x{key:x}\":\"{value}\"}}")),
        next(),
    )
}

fn read_frame(group: &str) -> Vec<u8> {
    request(&format!("S{group}"), None, next())
}

pub fn set(m: &Model, c: Control, v: i32) -> Result<Vec<Vec<u8>>, String> {
    let shape = controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if !shape.accepts(v) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    let key = key(kind(m), c).ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    Ok(vec![set_frame(key, to_wire(c, v))])
}

/// One write per band, in tenths of a dB.
pub fn eq(m: &Model, bands: &[i32]) -> Result<Vec<Vec<u8>>, String> {
    if !has_eq(m) {
        return Err(format!("{} has no custom EQ here", m.name));
    }
    if bands.len() != EQ_BANDS || bands.iter().any(|b| b.abs() > BAND_MAX) {
        return Err("the EQ takes 10 bands of -9 to +9 dB".into());
    }
    Ok(bands
        .iter()
        .enumerate()
        .map(|(i, b)| set_frame(0x1220 + 0x10 * i as u16, b * 5))
        .collect())
}

/// Groups read for settings, after the general one.
fn groups(k: Kind) -> &'static [&'static str] {
    match k {
        Kind::ProII => &["Mic", "SAF", "AQG"],
        Kind::AtlasAir => &["Mic", "SHH", "Prm"],
    }
}

fn number(v: &str) -> Option<i32> {
    v.trim().parse().ok()
}

/// Settings in the key/value pairs of a reply.
fn values(m: &Model, kvp: &[(u16, String)]) -> Vec<(Control, i32)> {
    let k = kind(m);
    let mut out = Vec::new();
    for (c, shape) in controls(m) {
        let Some(want) = key(k, c) else { continue };
        let Some(w) = kvp.iter().find(|p| p.0 == want).and_then(|p| number(&p.1)) else {
            continue;
        };
        // Its prompts take 0..=15 and hold anything above as 15.
        let w = if k == Kind::AtlasAir && c == Control::VoicePrompts {
            w.min(15)
        } else {
            w
        };
        match from_wire(c, w) {
            Some(v) if c == Control::EqPreset || shape.accepts(v) => out.push((c, v)),
            _ => {}
        }
    }
    out
}

fn status_of(kvp: &[(u16, String)]) -> Status {
    let get = |k: u16| kvp.iter().find(|p| p.0 == k).and_then(|p| number(&p.1));
    Status {
        connected: true,
        battery: get(0x240)
            .filter(|b| (0..=100).contains(b))
            .map(|b| b as u8),
        charging: get(0x250) == Some(1),
        chatmix: None,
        values: Vec::new(),
    }
}

/// Payload of a report 7: `[07][len lo][len hi][bytes]`.
fn payload(r: &[u8]) -> &[u8] {
    if r.len() < 3 || r[0] != IN_ID {
        return &[];
    }
    let n = (u16::from_le_bytes([r[1], r[2]]) & 0x7FFF) as usize;
    &r[3..3 + n.min(r.len() - 3)]
}

/// Index just past the string starting at `at` (on its opening quote).
fn skip_string(s: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Index just past the object starting at `at` (on its `{`).
fn skip_object(s: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = at;
    while i < s.len() {
        match s[i] {
            b'"' => {
                i = skip_string(s, i)?;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find(s: &[u8], what: &[u8]) -> Option<usize> {
    s.windows(what.len()).position(|w| w == what)
}

/// The pairs of the complete reply to a read of `group`, once all of it has arrived. String values
/// only; nested objects are skipped.
fn reply(buf: &[u8], group: &str) -> Option<Vec<(u16, String)>> {
    let head = format!("{{\"OR\":\"{group}\"");
    let start = find(buf, head.as_bytes())?;
    let end = skip_object(buf, start)?;
    let obj = &buf[start..end];
    let mut i = find(obj, b"\"KVP\":{")? + 7;
    let mut out = Vec::new();
    loop {
        match obj.get(i)? {
            b'}' => return Some(out),
            b',' | b' ' => i += 1,
            b'"' => {
                let k_end = skip_string(obj, i)?;
                let k = std::str::from_utf8(&obj[i + 1..k_end - 1]).ok()?;
                i = k_end;
                if obj.get(i)? != &b':' {
                    return None;
                }
                i += 1;
                match obj.get(i)? {
                    b'"' => {
                        let v_end = skip_string(obj, i)?;
                        let v = String::from_utf8_lossy(&obj[i + 1..v_end - 1]).into_owned();
                        if let Ok(k) = u16::from_str_radix(k.trim_start_matches("0x"), 16) {
                            out.push((k, v));
                        }
                        i = v_end;
                    }
                    b'{' => i = skip_object(obj, i)?,
                    _ => {
                        while !matches!(obj.get(i)?, b',' | b'}') {
                            i += 1;
                        }
                    }
                }
            }
            _ => return None,
        }
    }
}

/// The headset drops a request sent right behind another.
const GAP: Duration = Duration::from_millis(100);
const WAIT: Duration = Duration::from_millis(1500);
const POLL: Duration = Duration::from_millis(15);

/// The shared turn is held per report, never across a wait.
fn turn<T>(f: impl FnOnce() -> T) -> T {
    let _turn = foxden_core::hid_turn::take();
    f()
}

struct Link {
    dev: Dev,
    last: Vec<u8>,
}

impl Link {
    fn open(f: &Found) -> Option<Self> {
        let mut l = Self {
            dev: Dev::open(&f.coll)?,
            last: Vec::new(),
        };
        l.drain();
        Some(l)
    }

    /// The next new report; the transmitter hands back its last one until it has another.
    fn next(&mut self) -> Option<Vec<u8>> {
        let r = turn(|| self.dev.get_input_report(IN_ID))?;
        if payload(&r).is_empty() || r == self.last {
            return None;
        }
        self.last = r.clone();
        Some(r)
    }

    fn drain(&mut self) {
        for _ in 0..50 {
            if self.next().is_none() {
                break;
            }
        }
    }

    fn ask(&mut self, group: &str) -> Option<Vec<(u16, String)>> {
        self.drain();
        if !turn(|| self.dev.write(&read_frame(group))) {
            return None;
        }
        let mut buf = Vec::new();
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            match self.next() {
                Some(r) => {
                    buf.extend_from_slice(payload(&r));
                    if let Some(kvp) = reply(&buf, group) {
                        sleep(GAP);
                        return Some(kvp);
                    }
                }
                None => sleep(POLL),
            }
        }
        None
    }
}

/// The last status per receiver: the dial loop asks every quarter second, a read takes ~100 ms.
static CACHE: Mutex<Vec<(String, Instant, Status)>> = Mutex::new(Vec::new());
const FRESH: Duration = Duration::from_secs(10);
const FRESH_OFF: Duration = Duration::from_secs(30);

fn remember(key: &str, s: &Status) {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|e| e.0 != key);
    c.push((key.to_string(), Instant::now(), s.clone()));
}

pub fn read(f: &Found) -> Option<Reading> {
    let mut l = Link::open(f)?;
    let Some(gsi) = l.ask("GSI") else {
        remember(&f.key, &Status::default());
        return Some(Reading {
            status: Some(Status::default()),
            values: Vec::new(),
        });
    };
    let status = status_of(&gsi);
    remember(&f.key, &status);
    let mut kvp = gsi;
    for g in groups(kind(&f.model)) {
        kvp.extend(l.ask(g).unwrap_or_default());
    }
    Some(Reading {
        status: Some(status),
        values: values(&f.model, &kvp),
    })
}

pub fn status(f: &Found) -> Option<Status> {
    {
        let c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = |s: &Status| if s.connected { FRESH } else { FRESH_OFF };
        if let Some(e) = c
            .iter()
            .find(|e| e.0 == f.key && e.1.elapsed() < fresh(&e.2))
        {
            return Some(e.2.clone());
        }
    }
    let mut l = Link::open(f)?;
    let s = l.ask("GSI").map(|k| status_of(&k)).unwrap_or_default();
    remember(&f.key, &s);
    Some(s)
}

/// Writes have no reply; each is given time before the next.
pub fn send(f: &Found, frames: Vec<Vec<u8>>) -> Result<(), String> {
    if frames.iter().any(|fr| fr.len() > REPORT_LEN) {
        return Err("the setting does not fit in one report".into());
    }
    let dev = Dev::open(&f.coll).ok_or("the headset's transmitter could not be opened")?;
    for frame in &frames {
        if !turn(|| dev.write(frame)) {
            return Err("the headset's transmitter did not take the setting".into());
        }
        sleep(GAP);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pro() -> Model {
        model(0x229D).unwrap()
    }

    fn atlas() -> Model {
        model(0x225E).unwrap()
    }

    /// The frame with its counter blanked.
    fn bare(mut f: Vec<u8>) -> Vec<u8> {
        f[12] = 0;
        f[13] = 0;
        f
    }

    fn json(f: &[u8]) -> &str {
        let at = f.iter().position(|b| *b == 0xFF).unwrap();
        std::str::from_utf8(&f[at + 1..]).unwrap()
    }

    #[test]
    fn captured_write_frame() {
        let f = request("set_kvp", Some(r#"{"0x760":"100"}"#), 0xDE44);
        let mut want = vec![
            0x06, 0x2A, 0x00, 0x05, 0x5A, 0x26, 0x00, 0x02, 0x99, 0x48, 0x03, 0xFB, 0xDE, 0x44,
        ];
        want.extend([0; 7]);
        want.push(0xB7);
        want.extend(b"set_kvp");
        want.push(0xFF);
        want.extend(br#"{"0x760":"100"}"#);
        assert_eq!(f, want);
        assert_eq!(
            bare(pro().set(Control::AncLevel, 100).unwrap().remove(0)),
            bare(want)
        );
    }

    #[test]
    fn read_frame_bytes() {
        let f = request("SGSI", None, 7);
        let mut want = vec![
            0x06, 0x18, 0x00, 0x05, 0x5A, 0x14, 0x00, 0x02, 0x99, 0x48, 0x01, 0xFB, 0x00, 0x07,
        ];
        want.extend([0; 7]);
        want.extend([0x61, 0x00]);
        want.extend(b"SGSI");
        assert_eq!(f, want);
    }

    #[test]
    fn models() {
        let p = pro();
        assert_eq!(
            (p.family, p.brand(), p.wireless, p.anc),
            (Family::TurtleBeach, "Turtle Beach", true, true)
        );
        assert!(!model(0x229E).unwrap().wireless);
        assert_eq!(
            crate::proto::any_model(VID, 0x2288).unwrap().name,
            "Stealth Pro II"
        );
        assert!(crate::proto::any_model(VID, 0x2251).is_none());
        assert!(!atlas().anc && !atlas().has_eq() && p.has_eq());
        assert!(atlas().controls().iter().all(|(c, _)| *c != Control::Anc));
    }

    #[test]
    fn pro_writes() {
        let p = pro();
        let one = |c, v| {
            let f = p.set(c, v).unwrap();
            assert_eq!(f.len(), 1);
            assert!(f[0].len() <= REPORT_LEN);
            json(&f[0]).to_string()
        };
        assert_eq!(one(Control::Sidetone, 55), r#"{"0x620":"55"}"#);
        assert_eq!(one(Control::AutoOff, 0), r#"{"0x280":"0"}"#);
        assert_eq!(one(Control::AutoOff, 20), r#"{"0x280":"3"}"#);
        assert_eq!(one(Control::Anc, 1), r#"{"0x750":"1"}"#);
        assert_eq!(one(Control::NoiseGate, 1), r#"{"0x700":"1"}"#);
        assert_eq!(one(Control::NoiseGateLevel, 40), r#"{"0x710":"40"}"#);
        assert_eq!(one(Control::AiNoiseReduction, 0), r#"{"0x630":"0"}"#);
        assert_eq!(one(Control::Superhuman, 1), r#"{"0x720":"1"}"#);
        assert_eq!(one(Control::SuperhumanMode, 2), r#"{"0x730":"2"}"#);
        assert_eq!(one(Control::SuperhumanLevel, 60), r#"{"0x740":"60"}"#);
        assert_eq!(one(Control::VoicePrompts, 30), r#"{"0x2d0":"30"}"#);
        assert_eq!(one(Control::WakeOnMotion, 1), r#"{"0x2e0":"1"}"#);
        assert_eq!(one(Control::EqPreset, 0), r#"{"0x1210":"1"}"#);
        assert_eq!(one(Control::EqPreset, 3), r#"{"0x1210":"4"}"#);
        assert!(p.set(Control::AutoOff, 15).is_err());
        assert!(p.set(Control::Sidetone, 101).is_err());
        assert!(p.set(Control::EqPreset, EQ_CUSTOM).is_err());
        assert!(p.set(Control::MicVolume, 5).is_err());
        assert_eq!(p.save(Some(Control::Sidetone)), crate::Save::None);
    }

    #[test]
    fn atlas_writes() {
        let a = atlas();
        let one = |c, v| json(&a.set(c, v).unwrap()[0]).to_string();
        assert_eq!(one(Control::Sidetone, 20), r#"{"0x630":"20"}"#);
        assert_eq!(one(Control::AutoOff, 30), r#"{"0x280":"4"}"#);
        assert_eq!(one(Control::Superhuman, 1), r#"{"0x800":"1"}"#);
        assert_eq!(one(Control::SuperhumanMode, 1), r#"{"0x810":"1"}"#);
        assert_eq!(one(Control::SuperhumanLevel, 60), r#"{"0x820":"60"}"#);
        assert_eq!(one(Control::VoicePrompts, 15), r#"{"0xc00":"15"}"#);
        assert!(a.set(Control::VoicePrompts, 16).is_err());
        for c in [
            Control::Anc,
            Control::NoiseGate,
            Control::EqPreset,
            Control::AiNoiseReduction,
        ] {
            assert!(a.set(c, 0).is_err());
        }
        assert!(a.eq(&[0; 10]).is_err());
    }

    #[test]
    fn eq_bands() {
        let mut bands = [0; 10];
        bands[0] = -18;
        bands[9] = 7;
        let f = pro().eq(&bands).unwrap();
        assert_eq!(f.len(), 10);
        assert_eq!(json(&f[0]), r#"{"0x1220":"-90"}"#);
        assert_eq!(json(&f[1]), r#"{"0x1230":"0"}"#);
        assert_eq!(json(&f[9]), r#"{"0x12b0":"35"}"#);
        bands[4] = 19;
        assert!(pro().eq(&bands).is_err());
        assert!(pro().eq(&[0; 9]).is_err());
    }

    fn report(bytes: &[u8]) -> Vec<u8> {
        let mut r = vec![IN_ID, bytes.len() as u8, 0];
        r.extend_from_slice(bytes);
        r.resize(62, 0xAA);
        r
    }

    #[test]
    fn split_reply() {
        let doc = br#"{"OR":"GSI","KVP":{"200":"1","220":"My Headset","240":"73","250":"1","280":"2","2d0":"40","2e0":"0"}}"#;
        let mut head = vec![0x05, 0x5B, 0x60, 0x00, 0x02, 0x99, 0x68, 0x45, 0xFB, 0, 1];
        head.extend_from_slice(&doc[..30]);
        let mut buf = Vec::new();
        buf.extend_from_slice(payload(&report(&head)));
        assert_eq!(reply(&buf, "GSI"), None);
        for part in doc[30..].chunks(59) {
            buf.extend_from_slice(payload(&report(part)));
        }
        let kvp = reply(&buf, "GSI").unwrap();
        assert_eq!(kvp[2], (0x240, "73".to_string()));
        let s = status_of(&kvp);
        assert_eq!((s.connected, s.battery, s.charging), (true, Some(73), true));
        assert_eq!(
            values(&pro(), &kvp),
            vec![
                (Control::AutoOff, 10),
                (Control::VoicePrompts, 40),
                (Control::WakeOnMotion, 0)
            ]
        );
        assert_eq!(reply(&buf, "Mic"), None);
        assert!(payload(&[IN_ID, 0, 0, 1, 2]).is_empty());
    }

    #[test]
    fn settings_replies() {
        let saf = br#"{"OR":"SAF","KVP":{"700":"1","710":"35","720":"0","730":"1","740":"80","750":"1","760":"0"}}"#;
        let mic = br#"{"OR":"Mic","KVP":{"600":"0","610":"48","620":"100","630":"1","640":"1","650":"0"}}"#;
        let aqg = br#"{"OR":"AQG","KVP":{"1200":"0","1210":"3","1220":"-30","12c0":"Bass & Treble Boost"}}"#;
        let mut kvp = reply(saf, "SAF").unwrap();
        kvp.extend(reply(mic, "Mic").unwrap());
        kvp.extend(reply(aqg, "AQG").unwrap());
        assert_eq!(
            values(&pro(), &kvp),
            vec![
                (Control::Sidetone, 100),
                (Control::Anc, 1),
                (Control::AncLevel, 0),
                (Control::NoiseGate, 1),
                (Control::NoiseGateLevel, 35),
                (Control::AiNoiseReduction, 1),
                (Control::Superhuman, 0),
                (Control::SuperhumanMode, 1),
                (Control::SuperhumanLevel, 80),
                (Control::EqPreset, 2),
            ]
        );
        let custom = reply(br#"{"OR":"AQG","KVP":{"1210":"0"}}"#, "AQG").unwrap();
        assert_eq!(
            values(&pro(), &custom),
            vec![(Control::EqPreset, EQ_CUSTOM)]
        );
        let slot = reply(br#"{"OR":"AQG","KVP":{"1210":"17"}}"#, "AQG").unwrap();
        assert_eq!(values(&pro(), &slot), vec![(Control::EqPreset, EQ_CUSTOM)]);
        let odd = reply(
            br#"{"OR":"SAF","KVP":{"700":"7","740":"x","730":"9"}}"#,
            "SAF",
        )
        .unwrap();
        assert!(values(&pro(), &odd).is_empty());
    }

    #[test]
    fn atlas_replies() {
        let mic = br#"{"OR":"Mic","KVP":{"600":"0","620":"1","630":"20"}}"#;
        let shh = br#"{"OR":"SHH","KVP":{"800":"1","810":"2","820":"60","830":"0"}}"#;
        let prm = br#"{"OR":"Prm","KVP":{"c00":"50","c10":"0"}}"#;
        let gsi = br#"{"OR":"GSI","KVP":{"240":"90","280":"4"}}"#;
        let mut kvp = Vec::new();
        for (doc, g) in [(&mic[..], "Mic"), (shh, "SHH"), (prm, "Prm"), (gsi, "GSI")] {
            kvp.extend(reply(doc, g).unwrap());
        }
        assert_eq!(
            values(&atlas(), &kvp),
            vec![
                (Control::Sidetone, 20),
                (Control::AutoOff, 30),
                (Control::Superhuman, 1),
                (Control::SuperhumanMode, 2),
                (Control::SuperhumanLevel, 60),
                (Control::VoicePrompts, 15),
            ]
        );
        let low = reply(br#"{"OR":"Prm","KVP":{"c00":"9"}}"#, "Prm").unwrap();
        assert_eq!(values(&atlas(), &low), vec![(Control::VoicePrompts, 9)]);
    }

    #[test]
    fn nested_and_pushed_values() {
        let doc = br#"{"UP":"SAF","KVP":{"750":"0"}}{"OR":"TX1","KVP":{"400":{"info":["2","1"]},"401":"80"}}"#;
        assert_eq!(reply(doc, "SAF"), None);
        assert_eq!(reply(doc, "TX1").unwrap(), vec![(0x401, "80".to_string())]);
        assert_eq!(
            reply(
                br#"{"OR":"GSI","KVP":{"220":"a \"b\" }","240":"5"}}"#,
                "GSI"
            )
            .unwrap()[1],
            (0x240, "5".into())
        );
    }
}
