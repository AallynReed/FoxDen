//! Sony INZONE dongles: vendor commands in HID report 02, answered by events with the same
//! transaction id. Every write reads the setting first and changes only its own byte.

use std::time::Duration;

use foxden_battery::hid::{Coll, Dev};

use crate::Reading;
use crate::proto::{Control, Family, Model, Shape, Status};

pub const VID: u16 = 0x054C;
const WAIT: Duration = Duration::from_millis(1500);
const TO_HEADSET: u8 = 0x41;

const BATTERY: u8 = 0x04;
const BALANCE: u8 = 0x22;
const SIDETONE: u8 = 0x23;
const AMBIENT: u8 = 0x41;
const BT_BOOT: u8 = 0x63;
const GUIDANCE: u8 = 0x84;

const GET: u8 = 0x01;
const SET: u8 = 0x02;
const RET: u8 = 0x10;
const NTFY: u8 = 0x20;
const NTFY_ACTIVE: u8 = 0xA0;

struct Kit {
    pid: u16,
    name: &'static str,
    anc: bool,
    voice: bool,
    bt_boot: bool,
}

const KITS: &[Kit] = &[
    Kit {
        pid: 0x0FA8,
        name: "INZONE H9 II",
        anc: true,
        voice: true,
        bt_boot: true,
    },
    Kit {
        pid: 0x0EC2,
        name: "INZONE Buds",
        anc: true,
        voice: true,
        bt_boot: false,
    },
    Kit {
        pid: 0x0EBF,
        name: "INZONE H5",
        anc: false,
        voice: false,
        bt_boot: false,
    },
];

fn kit(pid: u16) -> Option<&'static Kit> {
    KITS.iter().find(|k| k.pid == pid)
}

pub fn model(pid: u16) -> Option<Model> {
    kit(pid).map(|k| Model {
        vid: VID,
        pid,
        name: k.name,
        family: Family::Inzone,
        sidetone: true,
        sidetone_max: 10,
        mic: false,
        anc: k.anc,
        wireless: true,
    })
}

pub fn fits(c: &Coll) -> bool {
    c.page == 0xFF04 && c.out_len >= 64
}

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let Some(k) = kit(m.pid) else {
        return Vec::new();
    };
    let mut out = vec![(Control::Sidetone, Shape::Steps { min: 0, max: 10 })];
    if k.anc {
        out.extend([
            (
                Control::Anc,
                Shape::Choice {
                    options: vec![(0, "Off"), (1, "Noise cancelling"), (2, "Ambient sound")],
                },
            ),
            (Control::AmbientLevel, Shape::Steps { min: 1, max: 20 }),
            (Control::VoiceFocus, Shape::Switch),
        ]);
    }
    if k.voice {
        out.push((Control::VoicePrompts, Shape::Switch));
    }
    if k.bt_boot {
        out.push((Control::BtPowerOn, Shape::Switch));
    }
    out
}

/// Event id, byte index and parameter length that carry `c`.
fn slot(c: Control) -> Option<(u8, usize, usize)> {
    Some(match c {
        Control::Sidetone => (SIDETONE, 0, 2),
        Control::Anc => (AMBIENT, 0, 4),
        Control::AmbientLevel => (AMBIENT, 1, 4),
        Control::VoiceFocus => (AMBIENT, 3, 4),
        Control::VoicePrompts => (GUIDANCE, 0, 1),
        Control::BtPowerOn => (BT_BOOT, 0, 1),
        _ => return None,
    })
}

fn shape(m: &Model, c: Control) -> Option<Shape> {
    controls(m)
        .into_iter()
        .find(|(k, _)| *k == c)
        .map(|(_, s)| s)
}

/// `02 [len] 01 00 FC [len-4] 96 C3 41 [id] [type] [tid lo hi] [param] [sum from 96 on]`.
fn command(id: u8, kind: u8, tid: u16, param: &[u8]) -> Vec<u8> {
    let n = param.len() as u8;
    let [lo, hi] = tid.to_le_bytes();
    let mut r = vec![
        0x02,
        12 + n,
        0x01,
        0x00,
        0xFC,
        8 + n,
        0x96,
        0xC3,
        TO_HEADSET,
        id,
        kind,
        lo,
        hi,
    ];
    r.extend_from_slice(param);
    let sum = r[6..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    r.push(sum);
    r
}

struct Event {
    id: u8,
    kind: u8,
    tid: u16,
    param: Vec<u8>,
}

/// `02 [len] 04 FF [len-3] 00 96 C3 [1x] [id] [type] [tid lo hi] [param] [sum of 5..=len]`.
fn event(b: &[u8]) -> Option<Event> {
    if b.len() < 14 || b[0] != 0x02 {
        return None;
    }
    let len = b[1] as usize;
    if !(12..=62).contains(&len) || b.len() < len + 2 {
        return None;
    }
    if b[2] != 0x04 || b[3] != 0xFF || b[5] != 0x00 || b[6..8] != [0x96, 0xC3] || b[8] >> 4 != 0x1 {
        return None;
    }
    let sum = b[5..=len].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    (sum == b[len + 1]).then(|| Event {
        id: b[9],
        kind: b[10],
        tid: u16::from_le_bytes([b[11], b[12]]),
        param: b[13..=len].to_vec(),
    })
}

/// The parameter of the answer to a command `id` sent as `kind` with `tid`.
fn answer(b: &[u8], id: u8, kind: u8, tid: u16) -> Option<Vec<u8>> {
    let want = if kind == SET { NTFY } else { RET };
    let e = event(b)?;
    (e.id == id && e.tid == tid && (e.kind == want || e.kind == NTFY_ACTIVE)).then_some(e.param)
}

/// Value of `c` in the parameter of its event, when it is in range.
fn value(m: &Model, c: Control, param: &[u8]) -> Option<i32> {
    let (_, at, len) = slot(c)?;
    let v = *param.get(at).filter(|_| param.len() >= len)? as i32;
    shape(m, c)?.accepts(v).then_some(v)
}

/// The parameter that sets `c` to `v`, from the one just read. Refuses a reading that does not
/// look like this setting, so nothing is written to a model that lacks it.
fn patch(m: &Model, c: Control, v: i32, current: &[u8]) -> Result<Vec<u8>, String> {
    let s = shape(m, c).ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if !s.accepts(v) {
        return Err(format!("{v} is out of range for {}", c.id()));
    }
    let (_, at, len) = slot(c).ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    if value(m, c, current).is_none() {
        return Err("the headset's reply was not understood; nothing was changed".into());
    }
    let mut out = current[..len].to_vec();
    out[at] = v as u8;
    Ok(out)
}

/// Game and chat as the mixer takes them: both full at the centre.
fn mix(balance: u8) -> Option<(u8, u8)> {
    (balance <= 100).then(|| (((100 - balance) * 2).min(100), (balance * 2).min(100)))
}

/// `None` when the dongle did not take the command, `Some(None)` when nothing answered it.
fn ask(d: &Dev, id: u8, kind: u8, param: &[u8]) -> Option<Option<Vec<u8>>> {
    let tid = foxden_battery::inzone_tid();
    d.flush();
    if !d.write(&command(id, kind, tid, param)) {
        return None;
    }
    Some(d.read_until(WAIT, |r| answer(r, id, kind, tid)))
}

fn link(d: &Dev) -> Option<Status> {
    let Some(p) = ask(d, BALANCE, GET, &[])? else {
        return Some(Status::default());
    };
    match p.first() {
        Some(&b) if b != 0xFF => Some(Status {
            connected: true,
            chatmix: mix(b),
            ..Status::default()
        }),
        _ => Some(Status::default()),
    }
}

pub fn status(f: &Coll) -> Option<Status> {
    link(&Dev::open(f)?)
}

pub fn read(m: &Model, f: &Coll) -> Option<Reading> {
    let d = Dev::open(f)?;
    let mut status = link(&d)?;
    let mut values = Vec::new();
    if status.connected {
        if let Some(Some(p)) = ask(&d, BATTERY, GET, &[])
            && p.len() >= 2
            && p[1] <= 100
        {
            status.battery = Some(p[1]);
            status.charging = p[0] != 0;
        }
        let mut ids: Vec<u8> = Vec::new();
        for (c, _) in controls(m) {
            if let Some((id, ..)) = slot(c)
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        for id in ids {
            if let Some(Some(p)) = ask(&d, id, GET, &[]) {
                for (c, _) in controls(m) {
                    if slot(c).is_some_and(|s| s.0 == id)
                        && let Some(v) = value(m, c, &p)
                    {
                        values.push((c, v));
                    }
                }
            }
        }
    }
    Some(Reading {
        status: Some(status),
        values,
    })
}

pub fn set(m: &Model, f: &Coll, c: Control, v: i32) -> Result<(), String> {
    let (id, ..) = slot(c)
        .filter(|_| shape(m, c).is_some())
        .ok_or_else(|| format!("{} has no {} setting", m.name, c.id()))?;
    let d = Dev::open(f).ok_or("the headset's receiver could not be opened")?;
    let current = ask(&d, id, GET, &[])
        .ok_or("the headset's receiver did not take the request")?
        .ok_or("the headset did not answer; it may be switched off")?;
    let param = patch(m, c, v, &current)?;
    match ask(&d, id, SET, &param) {
        Some(Some(_)) => Ok(()),
        Some(None) => Err("the headset did not confirm the setting".into()),
        None => Err("the headset's receiver did not take the setting".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h9() -> Model {
        model(0x0FA8).unwrap()
    }

    fn report(hci: &[u8]) -> Vec<u8> {
        let mut r = vec![0x02, hci.len() as u8];
        r.extend_from_slice(hci);
        r.resize(64, 0);
        r
    }

    #[test]
    fn commands_match_known_frames() {
        assert_eq!(
            command(BALANCE, GET, 1, &[]),
            [
                0x02, 0x0C, 0x01, 0x00, 0xFC, 0x08, 0x96, 0xC3, 0x41, 0x22, 0x01, 0x01, 0x00, 0xBE
            ]
        );
        assert_eq!(
            command(BALANCE, SET, 0x65, &[30]),
            [
                0x02, 0x0D, 0x01, 0x00, 0xFC, 0x09, 0x96, 0xC3, 0x41, 0x22, 0x02, 0x65, 0x00, 0x1E,
                0x41
            ]
        );
        // Each is a known constant plus the transaction id and the value.
        let side = command(SIDETONE, SET, 7, &[4, 0xFF]);
        assert_eq!(side[..6], [0x02, 0x0E, 0x01, 0x00, 0xFC, 0x0A]);
        assert_eq!(side[15], 0xBEu8.wrapping_add(7 + 4));
        let anc = command(AMBIENT, SET, 9, &[1, 0x14, 0xFF, 0]);
        assert_eq!(anc[17], 0xF0u8.wrapping_add(9 + 1));
        assert_eq!(anc.len(), 18);
    }

    #[test]
    fn events_parse() {
        let r = report(&[
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x14, 0x22, 0x10, 0x01, 0x00, 0x32, 0xD2,
        ]);
        assert_eq!(answer(&r, BALANCE, GET, 1), Some(vec![50]));
        assert_eq!(answer(&r, BALANCE, GET, 2), None);
        assert_eq!(answer(&r, SIDETONE, GET, 1), None);
        assert_eq!(answer(&r, BALANCE, SET, 1), None);
        let mut bad = r.clone();
        bad[14] ^= 1;
        assert_eq!(answer(&bad, BALANCE, GET, 1), None);
        let set = report(&[
            0x04, 0xFF, 0x0A, 0x00, 0x96, 0xC3, 0x14, 0x22, 0x20, 0x65, 0x00, 0x1E, 0x32,
        ]);
        assert_eq!(answer(&set, BALANCE, SET, 0x65), Some(vec![30]));
    }

    #[test]
    fn writes_change_one_byte() {
        let m = h9();
        let amb = [0x02, 0x14, 0xFF, 0x01];
        assert_eq!(
            patch(&m, Control::Anc, 1, &amb).unwrap(),
            [1, 0x14, 0xFF, 1]
        );
        assert_eq!(
            patch(&m, Control::AmbientLevel, 3, &amb).unwrap(),
            [2, 3, 0xFF, 1]
        );
        assert_eq!(
            patch(&m, Control::VoiceFocus, 0, &amb).unwrap(),
            [2, 0x14, 0xFF, 0]
        );
        assert_eq!(
            patch(&m, Control::Sidetone, 10, &[5, 0x32]).unwrap(),
            [10, 0x32]
        );
        assert_eq!(patch(&m, Control::VoicePrompts, 0, &[1]).unwrap(), [0]);
        assert_eq!(patch(&m, Control::BtPowerOn, 1, &[0]).unwrap(), [1]);
        assert!(patch(&m, Control::Sidetone, 11, &[5, 0xFF]).is_err());
        assert!(patch(&m, Control::AmbientLevel, 0, &amb).is_err());
        assert!(patch(&m, Control::Anc, 3, &amb).is_err());
        assert!(patch(&m, Control::Sidetone, 2, &[0xFF, 0xFF]).is_err());
        assert!(patch(&m, Control::Sidetone, 2, &[5]).is_err());
        assert!(patch(&m, Control::Anc, 1, &[1, 0x14]).is_err());
        assert!(patch(&m, Control::MicVolume, 1, &[0, 1, 0xFF]).is_err());
    }

    #[test]
    fn models_and_values() {
        let buds = model(0x0EC2).unwrap();
        assert!(!controls(&buds).iter().any(|c| c.0 == Control::BtPowerOn));
        let h5 = model(0x0EBF).unwrap();
        assert_eq!(
            controls(&h5),
            vec![(Control::Sidetone, Shape::Steps { min: 0, max: 10 })]
        );
        assert!(patch(&h5, Control::Anc, 1, &[0, 0x14, 0xFF, 0]).is_err());
        assert!(model(0x0E53).is_none());
        assert_eq!(
            value(&h9(), Control::AmbientLevel, &[2, 20, 0xFF, 0]),
            Some(20)
        );
        assert_eq!(value(&h9(), Control::VoiceFocus, &[2, 20, 0xFF, 7]), None);
        assert_eq!(mix(50), Some((100, 100)));
        assert_eq!(mix(0), Some((100, 0)));
        assert_eq!(mix(100), Some((0, 100)));
        assert_eq!(mix(80), Some((40, 100)));
        assert_eq!(mix(0xFF), None);
    }
}
