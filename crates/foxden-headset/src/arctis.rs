//! Older SteelSeries Arctis headsets: Arctis 1 Wireless, Arctis 7, Arctis 7+ and Arctis Pro Wireless.

use crate::proto::{Control, EQ_PRESETS, Family, Model, Save, Shape, Status};

const fn a(pid: u16, name: &'static str, family: Family, sidetone: bool) -> Model {
    Model {
        vid: 0x1038,
        pid,
        name,
        family,
        sidetone,
        sidetone_max: 0,
        mic: false,
        anc: false,
        wireless: true,
    }
}

const MODELS: &[Model] = &[
    a(0x12B3, "Arctis 1 Wireless", Family::Arctis1, true),
    a(0x12B6, "Arctis 1 Wireless for Xbox", Family::Arctis1, true),
    a(
        0x12D0,
        "Arctis 1 Wireless Cyberpunk 2077",
        Family::Arctis1,
        true,
    ),
    a(
        0x12D2,
        "Arctis 1 Wireless for Xbox Cyberpunk 2077",
        Family::Arctis1,
        true,
    ),
    a(0x12D7, "Arctis 7X", Family::Arctis1, true),
    a(0x12D5, "Arctis 7P", Family::Arctis1, true),
    a(0x1260, "Arctis 7", Family::Arctis7, true),
    a(0x12AD, "Arctis 7 (2019)", Family::Arctis7, true),
    a(0x220E, "Arctis 7+", Family::Arctis7Plus, true),
    a(0x2236, "Arctis 7+ Destiny 2", Family::Arctis7Plus, true),
    a(0x2216, "Arctis 7X+", Family::Arctis7Plus, true),
    a(0x2212, "Arctis 7P+", Family::Arctis7Plus, false),
    a(0x1290, "Arctis Pro Wireless", Family::ArctisPro, true),
];

/// Only these 7+ bases have a ChatMix dial.
const DIAL: &[u16] = &[0x220E, 0x2236];

pub fn model(pid: u16) -> Option<Model> {
    MODELS.iter().copied().find(|m| m.pid == pid)
}

pub fn has(f: Family) -> bool {
    matches!(
        f,
        Family::Arctis1 | Family::Arctis7 | Family::Arctis7Plus | Family::ArctisPro
    )
}

pub fn id(m: &Model) -> u8 {
    match m.family {
        Family::Arctis1 | Family::Arctis7 => 0x06,
        _ => 0x00,
    }
}

pub fn collection(m: &Model) -> (i32, Option<(u16, u16)>) {
    match m.family {
        Family::ArctisPro => (0, None),
        Family::Arctis7Plus => (3, Some((0xFFC0, 0x0001))),
        _ => (3, Some((0xFF43, 0x0202))),
    }
}

const LEVELS: [(i32, &str); 4] = [(0, "Off"), (1, "Low"), (2, "Medium"), (3, "High")];
const MINUTES: [i32; 7] = [0, 5, 10, 15, 30, 60, 90];
/// Arctis Pro Wireless off time, in tens of minutes.
const PRO_OFF: [(i32, &str); 13] = [
    (0, "Never"),
    (1, "10 min"),
    (2, "20 min"),
    (3, "30 min"),
    (4, "40 min"),
    (5, "50 min"),
    (6, "60 min"),
    (7, "70 min"),
    (8, "80 min"),
    (9, "90 min"),
    (10, "100 min"),
    (11, "110 min"),
    (12, "120 min"),
];

pub fn controls(m: &Model) -> Vec<(Control, Shape)> {
    let minutes = || Shape::Minutes {
        max: 90,
        presets: MINUTES.to_vec(),
    };
    let levels = || Shape::Choice {
        options: LEVELS.to_vec(),
    };
    let mut out = Vec::new();
    match m.family {
        Family::Arctis1 | Family::Arctis7 => out.extend([
            (Control::Sidetone, Shape::Steps { min: 0, max: 18 }),
            (Control::AutoOff, minutes()),
        ]),
        Family::Arctis7Plus => {
            if m.sidetone {
                out.push((Control::Sidetone, levels()));
            }
            out.extend([
                (Control::MicVolume, Shape::Steps { min: 1, max: 7 }),
                (Control::AutoOff, minutes()),
                (Control::MicLed, levels()),
                (
                    Control::EqPreset,
                    Shape::Choice {
                        options: EQ_PRESETS.to_vec(),
                    },
                ),
            ]);
        }
        Family::ArctisPro => out.extend([
            (Control::Sidetone, Shape::Steps { min: 0, max: 9 }),
            (
                Control::AutoOff,
                Shape::Choice {
                    options: PRO_OFF.to_vec(),
                },
            ),
        ]),
        _ => {}
    }
    out
}

/// 7+ EQ band bytes: 0x18 is 0 dB, one step is half a dB.
const EQ_ZERO: i32 = 0x18;

/// 7+ preset curves, by the ids of [`EQ_PRESETS`].
fn curve(preset: i32) -> Option<[u8; 10]> {
    Some(match preset {
        0 => [0x18; 10],
        1 => [0x1F, 0x20, 0x1A, 0x15, 0x15, 0x16, 0x16, 0x16, 0x16, 0x23],
        2 => [0x0E, 0x16, 0x11, 0x13, 0x20, 0x24, 0x1F, 0x11, 0x18, 0x11],
        3 => [0x1E, 0x1B, 0x15, 0x10, 0x10, 0x13, 0x1B, 0x1E, 0x20, 0x1F],
        _ => return None,
    })
}

/// The frames that set `c` to `v`; `v` is already in range. `None` for other families.
pub fn set(m: &Model, c: Control, v: i32) -> Option<Vec<Vec<u8>>> {
    let b = v as u8;
    let frame = match (m.family, c) {
        (Family::Arctis1 | Family::Arctis7, Control::Sidetone) if v == 0 => vec![0x06, 0x35, 0x00],
        (Family::Arctis1 | Family::Arctis7, Control::Sidetone) => vec![0x06, 0x35, 0x01, 0x00, b],
        (Family::Arctis1, Control::AutoOff) => vec![0x06, 0x53, b],
        (Family::Arctis7, Control::AutoOff) => vec![0x06, 0x51, b],
        (Family::Arctis7Plus, Control::Sidetone) => vec![0x00, 0x39, b],
        (Family::Arctis7Plus, Control::MicVolume) => vec![0x00, 0x37, b],
        (Family::Arctis7Plus, Control::AutoOff) => vec![0x00, 0xA3, b],
        (Family::Arctis7Plus, Control::MicLed) => vec![0x00, 0xAE, b],
        (Family::Arctis7Plus, Control::EqPreset) => {
            let mut f = vec![0x00, 0x33];
            f.extend(curve(v)?);
            f
        }
        (Family::ArctisPro, Control::Sidetone) => vec![0x00, 0x39, 0xAA, b],
        (Family::ArctisPro, Control::AutoOff) => vec![0x00, 0x3C, 0xAA, b],
        _ => return None,
    };
    Some(vec![frame])
}

/// Custom 7+ EQ from half-dB gains already checked against the EQ range.
pub fn eq(bands: &[i32]) -> Vec<Vec<u8>> {
    let mut f = vec![0x00, 0x33];
    f.extend(bands.iter().map(|b| (EQ_ZERO + b) as u8));
    vec![f]
}

pub fn save(m: &Model) -> Save {
    match m.family {
        Family::Arctis1 | Family::Arctis7 => Save::Settled(vec![0x06, 0x09]),
        Family::Arctis7Plus => Save::Settled(vec![0x00, 0x09]),
        _ => Save::None,
    }
}

pub fn status_request(m: &Model) -> Option<Vec<u8>> {
    Some(match m.family {
        Family::Arctis1 => vec![0x06, 0x12],
        Family::Arctis7 => vec![0x06, 0x18],
        Family::Arctis7Plus => vec![0x00, 0xB0],
        Family::ArctisPro => vec![0x00, 0x41, 0xAA],
        _ => return None,
    })
}

/// The Pro Wireless base answers without echoing the command.
pub fn echoes(m: &Model) -> bool {
    m.family != Family::ArctisPro
}

fn on(battery: Option<u8>, charging: bool, chatmix: Option<(u8, u8)>) -> Status {
    Status {
        connected: true,
        battery,
        charging,
        chatmix,
        values: Vec::new(),
    }
}

pub fn status(m: &Model, r: &[u8]) -> Option<Status> {
    match m.family {
        Family::Arctis1 => {
            let [0x06, 0x12, link, pct, ..] = *r else {
                return None;
            };
            Some(if link == 0x01 {
                Status::default()
            } else {
                on(Some(pct.min(100)), false, None)
            })
        }
        Family::Arctis7 => {
            let [0x06, 0x18, pct, ..] = *r else {
                return None;
            };
            Some(if pct == 0 {
                Status::default()
            } else {
                on(Some(pct.min(100)), false, None)
            })
        }
        Family::Arctis7Plus => {
            let [0xB0, link, level, charge, ..] = *r else {
                return None;
            };
            if link != 0x03 {
                return Some(Status::default());
            }
            let chatmix = match *r {
                [_, _, _, _, game, chat, ..] if DIAL.contains(&m.pid) => {
                    Some((game.min(100), chat.min(100)))
                }
                _ => None,
            };
            Some(on(
                (level <= 4).then(|| level * 25),
                charge == 0x01,
                chatmix,
            ))
        }
        Family::ArctisPro => match r.first()? {
            0x02 => Some(Status::default()),
            _ => Some(on(None, false, None)),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{self, any_model};

    fn ids(m: &Model) -> Vec<Control> {
        m.controls().into_iter().map(|c| c.0).collect()
    }

    #[test]
    fn models() {
        let one = any_model(0x1038, 0x12B3).unwrap();
        assert_eq!((one.family, one.brand()), (Family::Arctis1, "SteelSeries"));
        assert_eq!(one.collection(), (3, Some((0xFF43, 0x0202))));
        assert_eq!(proto::model(0x12AD).unwrap().family, Family::Arctis7);
        assert_eq!(proto::model(0x1290).unwrap().collection(), (0, None));
        assert_eq!(
            proto::model(0x2216).unwrap().collection(),
            (3, Some((0xFFC0, 0x0001)))
        );
        assert!(proto::model(0x12C2).is_none() && proto::model(0x1252).is_none());
        let p = proto::model(0x2212).unwrap();
        assert!(!ids(&p).contains(&Control::Sidetone));
        assert!(ids(&p).contains(&Control::MicVolume) && p.has_eq());
        assert!(!one.has_eq() && one.eq(&[0; 10]).is_err());
    }

    #[test]
    fn arctis_1_and_7_frames() {
        let one = proto::model(0x12B3).unwrap();
        let seven = proto::model(0x1260).unwrap();
        for m in [one, seven] {
            assert_eq!(
                m.set(Control::Sidetone, 18).unwrap(),
                vec![vec![0x06, 0x35, 0x01, 0x00, 18]]
            );
            assert_eq!(
                m.set(Control::Sidetone, 0).unwrap(),
                vec![vec![0x06, 0x35, 0x00]]
            );
            assert!(m.set(Control::Sidetone, 19).is_err());
            assert!(m.set(Control::AutoOff, 91).is_err());
            assert!(m.set(Control::MicVolume, 3).is_err());
            assert_eq!(
                m.save(Some(Control::Sidetone)),
                Save::Settled(vec![0x06, 0x09])
            );
            assert_eq!(m.settled_save(), Some(vec![0x06, 0x09]));
        }
        assert_eq!(
            one.set(Control::AutoOff, 90).unwrap(),
            vec![vec![0x06, 0x53, 90]]
        );
        assert_eq!(
            seven.set(Control::AutoOff, 15).unwrap(),
            vec![vec![0x06, 0x51, 15]]
        );
        assert_eq!(one.status_request(), vec![0x06, 0x12]);
        assert_eq!(seven.status_request(), vec![0x06, 0x18]);
        assert_eq!(seven.reply_head(&[0x06, 0x18]), &[0x06, 0x18]);
    }

    #[test]
    fn arctis_7_plus_frames() {
        let m = proto::model(0x220E).unwrap();
        assert_eq!(m.set(Control::Sidetone, 3).unwrap(), vec![vec![0, 0x39, 3]]);
        assert_eq!(
            m.set(Control::MicVolume, 7).unwrap(),
            vec![vec![0, 0x37, 7]]
        );
        assert!(m.set(Control::MicVolume, 0).is_err());
        assert_eq!(
            m.set(Control::AutoOff, 30).unwrap(),
            vec![vec![0, 0xA3, 30]]
        );
        assert_eq!(m.set(Control::MicLed, 2).unwrap(), vec![vec![0, 0xAE, 2]]);
        assert!(m.set(Control::VolumeLimiter, 1).is_err());
        assert_eq!(
            m.set(Control::EqPreset, 0).unwrap(),
            vec![vec![0, 0x33, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24]]
        );
        assert_eq!(
            m.set(Control::EqPreset, 1).unwrap(),
            vec![vec![
                0, 0x33, 0x1F, 0x20, 0x1A, 0x15, 0x15, 0x16, 0x16, 0x16, 0x16, 0x23
            ]]
        );
        assert_eq!(
            m.set(Control::EqPreset, 3).unwrap()[0][2..],
            [0x1E, 0x1B, 0x15, 0x10, 0x10, 0x13, 0x1B, 0x1E, 0x20, 0x1F]
        );
        assert!(m.set(Control::EqPreset, 4).is_err());
        let mut bands = [0; 10];
        bands[0] = -20;
        bands[9] = 20;
        assert_eq!(
            m.eq(&bands).unwrap(),
            vec![vec![0, 0x33, 4, 24, 24, 24, 24, 24, 24, 24, 24, 44]]
        );
        bands[1] = 21;
        assert!(m.eq(&bands).is_err());
        assert_eq!(m.save(None), Save::Settled(vec![0, 0x09]));
        assert!(
            proto::model(0x2212)
                .unwrap()
                .set(Control::Sidetone, 1)
                .is_err()
        );
        assert_eq!(m.reply_head(&[0x00, 0xB0]), &[0xB0]);
    }

    #[test]
    fn arctis_pro_wireless_frames() {
        let m = proto::model(0x1290).unwrap();
        assert_eq!(
            m.set(Control::Sidetone, 9).unwrap(),
            vec![vec![0, 0x39, 0xAA, 9]]
        );
        assert!(m.set(Control::Sidetone, 10).is_err());
        assert_eq!(
            m.set(Control::AutoOff, 12).unwrap(),
            vec![vec![0, 0x3C, 0xAA, 12]]
        );
        assert!(m.set(Control::AutoOff, 13).is_err());
        assert_eq!(m.save(Some(Control::Sidetone)), Save::None);
        assert_eq!(m.status_request(), vec![0, 0x41, 0xAA]);
        assert!(m.reply_head(&[0, 0x41, 0xAA]).is_empty());
    }

    #[test]
    fn replies() {
        let one = proto::model(0x12B6).unwrap();
        let s = proto::status(&one, &[0x06, 0x12, 0x03, 0x46, 0x00]).unwrap();
        assert_eq!((s.connected, s.battery, s.chatmix), (true, Some(70), None));
        assert!(
            !proto::status(&one, &[0x06, 0x12, 0x01, 0x00])
                .unwrap()
                .connected
        );
        assert!(proto::status(&one, &[0x06, 0x18, 0x03, 0x46]).is_none());

        let seven = proto::model(0x12AD).unwrap();
        assert_eq!(
            proto::status(&seven, &[0x06, 0x18, 0x32]).unwrap().battery,
            Some(50)
        );
        assert!(
            !proto::status(&seven, &[0x06, 0x18, 0x00])
                .unwrap()
                .connected
        );

        let plus = proto::model(0x220E).unwrap();
        let s = proto::status(&plus, &[0xB0, 0x03, 0x02, 0x01, 0x64, 0x28]).unwrap();
        assert!(s.connected && s.charging);
        assert_eq!((s.battery, s.chatmix), (Some(50), Some((100, 40))));
        assert!(
            !proto::status(&plus, &[0xB0, 0x01, 0x02, 0x03, 0, 0])
                .unwrap()
                .connected
        );
        assert_eq!(
            proto::status(&plus, &[0xB0, 0x03, 0x07, 0x03])
                .unwrap()
                .battery,
            None
        );
        let p = proto::model(0x2212).unwrap();
        let s = proto::status(&p, &[0xB0, 0x03, 0x02, 0x03, 0x64, 0x28]).unwrap();
        assert_eq!((s.battery, s.charging, s.chatmix), (Some(50), false, None));
        assert!(proto::settings(&plus, &[0x20, 1, 2, 3]).is_empty());

        let pro = proto::model(0x1290).unwrap();
        assert!(proto::status(&pro, &[0x04]).unwrap().connected);
        assert!(!proto::status(&pro, &[0x02]).unwrap().connected);
        assert!(proto::status(&pro, &[]).is_none());
        assert!(pro.chatmix_switch(true).is_none());
    }
}
