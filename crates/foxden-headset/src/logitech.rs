//! Logitech headsets over HID++ 2.0: sidetone (feature 0x8300) and auto-off (feature 0x1F20).
//! Feature indexes are looked up by id on every use; nothing is written to an index taken on trust.

use std::time::Duration;

use foxden_battery::hid::{Coll, Dev};

use crate::proto::{Control, Family, Model, Shape, Status};
use crate::{Found, Reading};

pub const VID: u16 = 0x046D;
const LONG: u8 = 0x11;
const SHORT: u8 = 0x10;
const DEVICE: u8 = 0xFF;
const SW: u8 = 0x0C;
const SIDETONE: u16 = 0x8300;
const ADC: u16 = 0x1F20;
const WAIT: Duration = Duration::from_millis(500);
/// Auto-off minutes the headsets take, 0 for never.
const OFF_MINUTES: [i32; 7] = [0, 1, 2, 5, 10, 15, 30];

struct Row {
    pid: u16,
    name: &'static str,
    sidetone: bool,
    auto_off: bool,
    /// The G535 answers HID++ on its consumer collection instead of the vendor one.
    consumer: bool,
}

const fn r(pid: u16, name: &'static str, sidetone: bool, auto_off: bool) -> Row {
    Row {
        pid,
        name,
        sidetone,
        auto_off,
        consumer: false,
    }
}

const ROWS: &[Row] = &[
    r(0x0A5C, "G633", true, false),
    r(0x0A89, "G635", true, false),
    r(0x0A5B, "G933", true, false),
    r(0x0A87, "G935", true, false),
    r(0x0AB5, "G733", true, false),
    r(0x0AFE, "G733", true, false),
    r(0x0B1F, "G733", true, false),
    r(0x0A66, "G533", false, true),
    Row {
        pid: 0x0AC4,
        name: "G535",
        sidetone: true,
        auto_off: true,
        consumer: true,
    },
    r(0x0AA7, "G PRO", true, true),
    r(0x0AAA, "PRO X Wireless", true, true),
    r(0x0ABA, "PRO X Wireless", true, true),
    r(0x0AFB, "PRO X 2 LIGHTSPEED", true, true),
    r(0x0AFC, "PRO X 2 LIGHTSPEED", true, true),
];

fn row(pid: u16) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.pid == pid)
}

pub fn model(pid: u16) -> Option<Model> {
    row(pid).map(|r| Model {
        vid: VID,
        pid,
        name: r.name,
        family: Family::Logitech,
        sidetone: r.sidetone,
        sidetone_max: 100,
        mic: false,
        anc: false,
        wireless: true,
    })
}

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let Some(r) = row(m.pid) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if r.sidetone {
        out.push((Control::Sidetone, Shape::Steps { min: 0, max: 100 }));
    }
    if r.auto_off {
        out.push((
            Control::AutoOff,
            Shape::Minutes {
                max: 30,
                presets: OFF_MINUTES.to_vec(),
            },
        ));
    }
    out
}

/// The collection that carries long HID++ reports.
pub fn fits(c: &Coll, m: &Model) -> bool {
    let consumer = row(m.pid).is_some_and(|r| r.consumer);
    c.out_len >= 20
        && if consumer {
            c.iface == 3 && c.page == 0x000C && c.usage == 0x0001
        } else {
            c.page == 0xFF43 && c.usage == 0x0202
        }
}

fn request(feature: u8, function: u8, params: &[u8]) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[..4].copy_from_slice(&[LONG, DEVICE, feature, function << 4 | SW]);
    let n = params.len().min(16);
    f[4..4 + n].copy_from_slice(&params[..n]);
    f
}

/// Root getFeature.
fn lookup(id: u16) -> [u8; 20] {
    request(0x00, 0, &id.to_be_bytes())
}

/// Root ping: only the headset itself answers it.
fn ping() -> [u8; 20] {
    request(0x00, 1, &[0, 0, 0x5A])
}

/// The frame that sets `c` to `v` on the feature at `index`.
fn write(c: Control, index: u8, v: i32) -> Option<[u8; 20]> {
    match c {
        Control::Sidetone => Some(request(index, 1, &[v as u8])),
        Control::AutoOff => Some(request(index, 2, &[v as u8])),
        _ => None,
    }
}

/// Parameters of the reply to `req`, or the HID++ error code.
fn answer(req: &[u8; 20], r: &[u8]) -> Option<Result<Vec<u8>, u8>> {
    if r.len() < 7 || !matches!(r[0], SHORT | LONG) || r[1] != req[1] {
        return None;
    }
    if r[2] == req[2] && r[3] == req[3] {
        return Some(Ok(r[4..].to_vec()));
    }
    if matches!(r[2], 0xFF | 0x8F) && r[3] == req[2] && r[4] == req[3] {
        return Some(Err(r[5]));
    }
    None
}

/// Index and version from a getFeature reply; `None` when the headset lacks the feature.
fn found(p: &[u8]) -> Option<(u8, u8)> {
    match *p {
        [index @ 1..=255, _, version, ..] => Some((index, version)),
        _ => None,
    }
}

/// Auto-off lives in the measurement feature only from its version 2.
fn usable(c: Control, version: u8) -> bool {
    c != Control::AutoOff || version >= 2
}

fn read_value(c: Control, p: &[u8]) -> Option<i32> {
    let v = *p.first()? as i32;
    match c {
        Control::Sidetone => (v <= 100).then_some(v),
        Control::AutoOff => Some(v),
        _ => None,
    }
}

struct Link(Dev);

impl Link {
    fn ask(&self, req: &[u8; 20]) -> Option<Result<Vec<u8>, u8>> {
        self.0.flush();
        if !self.0.write(req) {
            return None;
        }
        self.0.read_until(WAIT, |r| answer(req, r))
    }

    fn feature(&self, c: Control) -> Option<u8> {
        let id = match c {
            Control::Sidetone => SIDETONE,
            Control::AutoOff => ADC,
            _ => return None,
        };
        let (index, version) = found(&self.ask(&lookup(id))?.ok()?)?;
        usable(c, version).then_some(index)
    }
}

fn open(f: &Found) -> Option<Link> {
    Dev::open(&f.coll).map(Link)
}

pub fn read(f: &Found, settings: bool) -> Option<Reading> {
    let link = open(f)?;
    let _turn = foxden_core::hid_turn::take();
    let connected = match link.ask(&ping()) {
        None => return Some(Reading::default()),
        Some(r) => r.is_ok(),
    };
    let mut values = Vec::new();
    if connected && settings {
        for (c, _) in controls(&f.model) {
            let function = if c == Control::AutoOff { 1 } else { 0 };
            if let Some(index) = link.feature(c)
                && let Some(Ok(p)) = link.ask(&request(index, function, &[]))
                && let Some(v) = read_value(c, &p)
            {
                values.push((c, v));
            }
        }
    }
    Some(Reading {
        status: Some(Status {
            connected,
            ..Status::default()
        }),
        values,
    })
}

/// Checks `v` against the control's shape; auto-off takes only the listed minutes.
fn check(m: &Model, c: Control, v: i32) -> Result<(), String> {
    let shape = controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    let listed = c != Control::AutoOff || OFF_MINUTES.contains(&v);
    if shape.accepts(v) && listed {
        Ok(())
    } else {
        Err(format!("{v} is out of range for {}", c.id()))
    }
}

pub fn set(f: &Found, c: Control, v: i32) -> Result<(), String> {
    check(&f.model, c, v)?;
    let link = open(f).ok_or("the headset's receiver could not be opened")?;
    let _turn = foxden_core::hid_turn::take();
    if !matches!(link.ask(&ping()), Some(Ok(_))) {
        return Err("the headset is switched off".into());
    }
    let index = link
        .feature(c)
        .ok_or_else(|| format!("this {} does not offer {}", f.model.name, c.id()))?;
    let frame = write(c, index, v).ok_or("no such setting")?;
    match link.ask(&frame) {
        Some(Ok(_)) => Ok(()),
        Some(Err(code)) => Err(format!(
            "the headset refused the setting (error {code:#04x})"
        )),
        None => Err("the headset did not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(f: &[u8; 20], bytes: &[u8]) {
        assert_eq!(&f[..bytes.len()], bytes);
        assert!(f[bytes.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn models() {
        let g935 = model(0x0A87).unwrap();
        assert_eq!(
            (g935.family, g935.brand(), g935.name),
            (Family::Logitech, "Logitech", "G935")
        );
        let ids = |pid| -> Vec<Control> {
            controls(&model(pid).unwrap())
                .into_iter()
                .map(|c| c.0)
                .collect()
        };
        assert_eq!(ids(0x0A87), vec![Control::Sidetone]);
        assert_eq!(ids(0x0A66), vec![Control::AutoOff]);
        assert_eq!(ids(0x0ABA), vec![Control::Sidetone, Control::AutoOff]);
        assert!(model(0x0AF7).is_none());
        assert!(model(0xC539).is_none());
        assert_eq!(crate::proto::any_model(VID, 0x0AC4).unwrap().name, "G535");
        assert!(!g935.has_eq());
    }

    #[test]
    fn requests() {
        head(&lookup(SIDETONE), &[0x11, 0xFF, 0x00, 0x0C, 0x83, 0x00]);
        head(&lookup(ADC), &[0x11, 0xFF, 0x00, 0x0C, 0x1F, 0x20]);
        head(&ping(), &[0x11, 0xFF, 0x00, 0x1C, 0x00, 0x00, 0x5A]);
        head(&request(0x07, 0, &[]), &[0x11, 0xFF, 0x07, 0x0C]);
        head(&request(0x05, 1, &[]), &[0x11, 0xFF, 0x05, 0x1C]);
    }

    #[test]
    fn writes() {
        head(
            &write(Control::Sidetone, 0x07, 64).unwrap(),
            &[0x11, 0xFF, 0x07, 0x1C, 64],
        );
        head(
            &write(Control::AutoOff, 0x05, 30).unwrap(),
            &[0x11, 0xFF, 0x05, 0x2C, 30],
        );
        head(
            &write(Control::Sidetone, 0x04, 0).unwrap(),
            &[0x11, 0xFF, 0x04, 0x1C],
        );
        assert!(write(Control::MicVolume, 0x04, 1).is_none());
    }

    #[test]
    fn ranges_are_checked() {
        let pro = model(0x0AAA).unwrap();
        assert!(check(&pro, Control::Sidetone, 100).is_ok());
        assert!(check(&pro, Control::Sidetone, 101).is_err());
        assert!(check(&pro, Control::Sidetone, -1).is_err());
        assert!(check(&pro, Control::AutoOff, 15).is_ok());
        assert!(check(&pro, Control::AutoOff, 0).is_ok());
        assert!(check(&pro, Control::AutoOff, 7).is_err());
        assert!(check(&pro, Control::AutoOff, 60).is_err());
        assert!(check(&pro, Control::MicVolume, 1).is_err());
        assert!(check(&model(0x0A66).unwrap(), Control::Sidetone, 10).is_err());
        assert!(check(&model(0x0A87).unwrap(), Control::AutoOff, 5).is_err());
    }

    #[test]
    fn replies() {
        let req = lookup(SIDETONE);
        let ok = [0x11, 0xFF, 0x00, 0x0C, 0x07, 0x00, 0x00, 0, 0, 0];
        assert_eq!(
            answer(&req, &ok).unwrap().map(|p| found(&p)),
            Ok(Some((7, 0)))
        );
        let absent = [0x11, 0xFF, 0x00, 0x0C, 0x00, 0x00, 0x00];
        assert_eq!(answer(&req, &absent).unwrap().map(|p| found(&p)), Ok(None));
        let off = [0x11, 0xFF, 0xFF, 0x00, 0x0C, 0x05, 0x00];
        assert_eq!(answer(&req, &off), Some(Err(0x05)));
        let short_err = [0x10, 0xFF, 0x8F, 0x00, 0x0C, 0x09, 0x00];
        assert_eq!(answer(&req, &short_err), Some(Err(0x09)));
        let other_sw = [0x11, 0xFF, 0x00, 0x0B, 0x07, 0x00, 0x00];
        assert_eq!(answer(&req, &other_sw), None);
        let other_err = [0x11, 0xFF, 0xFF, 0x00, 0x0B, 0x05, 0x00];
        assert_eq!(answer(&req, &other_err), None);
        assert_eq!(answer(&req, &[0x11, 0xFF, 0x00]), None);
    }

    #[test]
    fn feature_versions_and_values() {
        assert!(usable(Control::AutoOff, 2));
        assert!(!usable(Control::AutoOff, 1));
        assert!(usable(Control::Sidetone, 0));
        assert_eq!(read_value(Control::Sidetone, &[0x40, 0x00]), Some(64));
        assert_eq!(read_value(Control::Sidetone, &[0xC8]), None);
        assert_eq!(read_value(Control::AutoOff, &[0x0A, 0x00]), Some(10));
        assert_eq!(read_value(Control::AutoOff, &[]), None);
    }

    #[test]
    fn collections() {
        let c = |iface, page, usage, out_len| Coll {
            iface,
            page,
            usage,
            out_len,
            ..Coll::default()
        };
        let g935 = model(0x0A87).unwrap();
        assert!(fits(&c(3, 0xFF43, 0x0202, 20), &g935));
        assert!(!fits(&c(3, 0xFF43, 0x0201, 7), &g935));
        assert!(!fits(&c(3, 0x000C, 0x0001, 20), &g935));
        let g535 = model(0x0AC4).unwrap();
        assert!(fits(&c(3, 0x000C, 0x0001, 20), &g535));
        assert!(!fits(&c(0, 0x000C, 0x0001, 20), &g535));
    }
}
