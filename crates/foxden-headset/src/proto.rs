//! Frames and replies of the SteelSeries Arctis Nova headsets, and the controls of Corsair Bragi
//! headsets. Pure: no device access.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Nova 7 on its first firmware: settings are write-only.
    Nova7,
    /// Nova 7 Gen 2, or a Nova 7 on the newer firmware: settings read back.
    Nova7Gen2,
    Nova5,
    NovaPro,
    /// Corsair headsets: every setting is a Bragi property.
    Bragi,
    /// Logitech headsets over HID++ 2.0.
    Logitech,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub family: Family,
    /// The 7P's dial is its sidetone; the command does nothing there.
    pub sidetone: bool,
    /// Bragi only: highest sidetone level, microphone level and noise cancelling.
    pub sidetone_max: u8,
    pub mic: bool,
    pub anc: bool,
    pub wireless: bool,
}

const fn m(pid: u16, name: &'static str, family: Family, sidetone: bool) -> Model {
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
    m(0x2202, "Arctis Nova 7", Family::Nova7, true),
    m(0x2206, "Arctis Nova 7X", Family::Nova7, true),
    m(0x223A, "Arctis Nova 7 Diablo IV", Family::Nova7, true),
    m(0x227A, "Arctis Nova 7 WoW Edition", Family::Nova7, true),
    m(0x22A4, "Arctis Nova 7X", Family::Nova7, true),
    m(0x220A, "Arctis Nova 7P", Family::Nova7, false),
    m(0x22A1, "Arctis Nova 7", Family::Nova7Gen2, true),
    m(0x227E, "Arctis Nova 7 Gen 2", Family::Nova7Gen2, true),
    m(0x229E, "Arctis Nova 7X Gen 2", Family::Nova7Gen2, true),
    m(0x22A5, "Arctis Nova 7X", Family::Nova7Gen2, true),
    m(0x22AD, "Arctis Nova 7X White", Family::Nova7Gen2, true),
    m(0x22A9, "Arctis Nova 7 Diablo IV", Family::Nova7Gen2, true),
    m(0x22AB, "Arctis Nova 7 WoW Edition", Family::Nova7Gen2, true),
    m(0x22A7, "Arctis Nova 7P", Family::Nova7Gen2, false),
    m(0x2298, "Arctis Nova 7P Gen 2", Family::Nova7Gen2, false),
    m(0x2232, "Arctis Nova 5", Family::Nova5, true),
    m(0x2253, "Arctis Nova 5X", Family::Nova5, true),
    m(0x12E0, "Arctis Nova Pro Wireless", Family::NovaPro, true),
    m(0x12E5, "Arctis Nova Pro Wireless X", Family::NovaPro, true),
];

pub fn model(pid: u16) -> Option<Model> {
    MODELS.iter().copied().find(|m| m.pid == pid)
}

/// A headset FoxDen has settings for, SteelSeries or Corsair.
pub fn any_model(vid: u16, pid: u16) -> Option<Model> {
    match vid {
        0x1038 => model(pid),
        0x1B1C => {
            let row = foxden_battery::corsair_catalog::corsair(pid)?;
            let receiver = foxden_battery::corsair_catalog::served(row);
            let c = receiver.unwrap_or(row);
            let has = c.sidetone > 0 || c.mic || c.anc;
            (c.bragi && c.kind == foxden_battery::Kind::Headset && has).then_some(Model {
                vid,
                pid,
                name: c.name.strip_prefix("Corsair ").unwrap_or(c.name),
                family: Family::Bragi,
                sidetone: c.sidetone > 0,
                sidetone_max: c.sidetone,
                mic: c.mic,
                anc: c.anc,
                wireless: c.wireless || receiver.is_some(),
            })
        }
        crate::logitech::VID => crate::logitech::model(pid),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Sidetone,
    MicVolume,
    VolumeLimiter,
    AutoOff,
    MicLed,
    BtPowerOn,
    BtCall,
    EqPreset,
    Anc,
}

impl Control {
    pub fn id(self) -> &'static str {
        match self {
            Control::Sidetone => "sidetone",
            Control::MicVolume => "mic_volume",
            Control::VolumeLimiter => "volume_limiter",
            Control::AutoOff => "auto_off",
            Control::MicLed => "mic_led",
            Control::BtPowerOn => "bt_power_on",
            Control::BtCall => "bt_call",
            Control::EqPreset => "eq_preset",
            Control::Anc => "anc",
        }
    }
}

/// What values a control takes, and how the panel offers them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Shape {
    Steps {
        min: i32,
        max: i32,
    },
    Switch,
    Choice {
        options: Vec<(i32, &'static str)>,
    },
    /// Any whole number of minutes up to `max`, 0 for never; `presets` are offered.
    Minutes {
        max: i32,
        presets: Vec<i32>,
    },
}

impl Shape {
    pub fn accepts(&self, v: i32) -> bool {
        match self {
            Shape::Steps { min, max } => (*min..=*max).contains(&v),
            Shape::Switch => v == 0 || v == 1,
            Shape::Choice { options } => options.iter().any(|(o, _)| *o == v),
            Shape::Minutes { max, .. } => (0..=*max).contains(&v),
        }
    }
}

const LEVELS: [(i32, &str); 4] = [(0, "Off"), (1, "Low"), (2, "Medium"), (3, "High")];
const MINUTES: [i32; 8] = [0, 1, 5, 10, 15, 30, 60, 90];
/// The Nova Pro's auto-off steps, by wire value.
const PRO_OFF: [(i32, &str); 7] = [
    (0, "Never"),
    (1, "1 min"),
    (2, "5 min"),
    (3, "10 min"),
    (4, "15 min"),
    (5, "30 min"),
    (6, "60 min"),
];
/// Nova 5 mic LED wire values for levels 0..=3.
const NOVA5_LED: [u8; 4] = [0, 1, 4, 10];
pub const EQ_PRESETS: [(i32, &str); 4] = [(0, "Flat"), (1, "Bass"), (2, "Focus"), (3, "Smiley")];
/// Shown when custom bands were set last.
pub const EQ_CUSTOM: i32 = 4;
pub const EQ_BANDS: usize = 10;
/// Band gains are in half-dB steps.
pub const EQ_MIN: i32 = -20;
pub const EQ_MAX: i32 = 20;

/// After a setting change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Save {
    /// Nothing to save: changes stick on their own.
    None,
    /// Send this frame right after the change.
    Now(Vec<u8>),
    /// Send once changes have settled; the headset ignores commands for a while after it.
    Settled(Vec<u8>),
}

/// Pause after a settled save during which the headset takes no commands.
pub const SAVE_BUSY_MS: u64 = 1500;

impl Model {
    pub fn brand(&self) -> &'static str {
        match self.vid {
            0x1B1C => "Corsair",
            crate::logitech::VID => "Logitech",
            _ => "SteelSeries",
        }
    }

    /// Report id of every frame: 0 on the unnumbered Nova collections, 6 on the Nova Pro base station.
    fn id(&self) -> u8 {
        if self.family == Family::NovaPro {
            0x06
        } else {
            0x00
        }
    }

    /// The vendor collection: interface, usage page and usage. The Nova Pro has no vendor usage there.
    pub fn collection(&self) -> (i32, Option<(u16, u16)>) {
        match self.family {
            Family::NovaPro => (4, None),
            _ => (3, Some((0xFFC0, 0x0001))),
        }
    }

    pub fn controls(&self) -> Vec<(Control, Shape)> {
        let levels = || Shape::Choice {
            options: LEVELS.to_vec(),
        };
        let minutes = || Shape::Minutes {
            max: 255,
            presets: MINUTES.to_vec(),
        };
        let mut out = Vec::new();
        match self.family {
            Family::Nova7 | Family::Nova7Gen2 => {
                if self.sidetone {
                    out.push((Control::Sidetone, levels()));
                }
                out.extend([
                    (Control::MicVolume, Shape::Steps { min: 0, max: 7 }),
                    (Control::VolumeLimiter, Shape::Switch),
                    (Control::AutoOff, minutes()),
                    (Control::MicLed, levels()),
                    (Control::BtPowerOn, Shape::Switch),
                    (
                        Control::BtCall,
                        Shape::Choice {
                            options: vec![(0, "No change"), (1, "-12 dB"), (2, "Mute game")],
                        },
                    ),
                ]);
            }
            Family::Nova5 => out.extend([
                (Control::Sidetone, Shape::Steps { min: 0, max: 10 }),
                (Control::MicVolume, Shape::Steps { min: 0, max: 15 }),
                (Control::VolumeLimiter, Shape::Switch),
                (Control::AutoOff, minutes()),
                (Control::MicLed, levels()),
            ]),
            Family::Bragi => {
                if self.sidetone {
                    out.push((
                        Control::Sidetone,
                        Shape::Steps {
                            min: 0,
                            max: self.sidetone_max as i32,
                        },
                    ));
                }
                if self.mic {
                    out.push((Control::MicVolume, Shape::Steps { min: 0, max: 100 }));
                }
                if self.anc {
                    out.push((Control::Anc, Shape::Switch));
                }
            }
            Family::Logitech => out.extend(crate::logitech::controls(self)),
            Family::NovaPro => out.extend([
                (Control::Sidetone, levels()),
                (
                    Control::AutoOff,
                    Shape::Choice {
                        options: PRO_OFF.to_vec(),
                    },
                ),
                (
                    Control::EqPreset,
                    Shape::Choice {
                        options: EQ_PRESETS.to_vec(),
                    },
                ),
            ]),
        }
        out
    }

    pub fn has_eq(&self) -> bool {
        self.family == Family::NovaPro
    }

    /// The frames that set `c` to `v`, without the save. `Err` for a control this model lacks or a value out of range.
    pub fn set(&self, c: Control, v: i32) -> Result<Vec<Vec<u8>>, String> {
        let shape = self
            .controls()
            .into_iter()
            .find(|(k, _)| *k == c)
            .map(|(_, s)| s)
            .ok_or_else(|| format!("{} has no {} setting", self.name, c.id()))?;
        if !shape.accepts(v) {
            return Err(format!("{v} is out of range for {}", c.id()));
        }
        let byte = v as u8;
        let cmd = match (self.family, c) {
            (Family::Logitech, _) => return Err("Logitech settings are sent over HID++".into()),
            (_, Control::Sidetone) => 0x39,
            (_, Control::MicVolume) => 0x37,
            (Family::Nova5, Control::VolumeLimiter) => 0x27,
            (_, Control::VolumeLimiter) => 0x3A,
            (Family::NovaPro, Control::AutoOff) => 0xC1,
            (_, Control::AutoOff) => 0xA3,
            (_, Control::MicLed) => 0xAE,
            (_, Control::BtPowerOn) => 0xB2,
            (_, Control::BtCall) => 0xB3,
            (_, Control::EqPreset) => 0x2E,
            (_, Control::Anc) => return Err(format!("{} has no {} setting", self.name, c.id())),
        };
        let byte = match (self.family, c) {
            (Family::Nova5, Control::MicLed) => NOVA5_LED[v as usize],
            _ => byte,
        };
        Ok(vec![vec![self.id(), cmd, byte]])
    }

    /// Custom EQ, in half-dB steps per band: selects the custom slot, then sends the gains.
    pub fn eq(&self, bands: &[i32]) -> Result<Vec<Vec<u8>>, String> {
        if !self.has_eq() {
            return Err(format!("{} has no custom EQ here", self.name));
        }
        if bands.len() != EQ_BANDS || bands.iter().any(|b| !(EQ_MIN..=EQ_MAX).contains(b)) {
            return Err("the EQ takes 10 bands of -10 to +10 dB".into());
        }
        let mut gains = vec![self.id(), 0x33];
        gains.extend(bands.iter().map(|b| (20 + b) as u8));
        Ok(vec![vec![self.id(), 0x2E, EQ_CUSTOM as u8], gains])
    }

    /// What follows a change of `c`.
    pub fn save(&self, c: Option<Control>) -> Save {
        match self.family {
            Family::Nova7 | Family::Nova7Gen2 => Save::Settled(vec![0x00, 0x09]),
            Family::Nova5 | Family::Bragi | Family::Logitech => Save::None,
            Family::NovaPro if c.is_some() => Save::Now(vec![0x06, 0x09]),
            Family::NovaPro => Save::None,
        }
    }

    pub fn settled_save(&self) -> Option<Vec<u8>> {
        match self.save(None) {
            Save::Settled(frame) => Some(frame),
            _ => None,
        }
    }

    /// Status request; the reply starts with the same bytes after the report id is dropped.
    pub fn status_request(&self) -> Vec<u8> {
        vec![self.id(), 0xB0]
    }

    /// Requests whose replies carry current settings, in order.
    pub fn reads(&self) -> Vec<Vec<u8>> {
        match self.family {
            Family::Nova7Gen2 | Family::Nova5 => vec![vec![0x00, 0x20], vec![0x00, 0xA0]],
            _ => Vec::new(),
        }
    }

    /// Turns the base station's ChatMix reports on or off. The base does not store it.
    pub fn chatmix_switch(&self, on: bool) -> Option<Vec<u8>> {
        (self.family == Family::NovaPro).then(|| vec![self.id(), 0x49, on as u8])
    }

    /// The first bytes a reply to `req` starts with.
    pub fn reply_head<'a>(&self, req: &'a [u8]) -> &'a [u8] {
        if self.id() == 0 { &req[1..] } else { req }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub connected: bool,
    pub battery: Option<u8>,
    pub charging: bool,
    /// Game and chat sides of the ChatMix dial, 0..=100.
    pub chatmix: Option<(u8, u8)>,
    /// Settings that come with the status.
    #[serde(skip)]
    pub values: Vec<(Control, i32)>,
}

const NOVA7_DISCRETE: &[u16] = &[0x2202, 0x2206, 0x220A, 0x223A, 0x227A, 0x22A4];

/// Parses the reply to [`Model::status_request`].
pub fn status(m: &Model, r: &[u8]) -> Option<Status> {
    let pct = |v: u8| v.min(100);
    match m.family {
        Family::Nova7 | Family::Nova7Gen2 => {
            if r.len() < 6 || r[0] != 0xB0 {
                return None;
            }
            if r[3] == 0 {
                return Some(Status::default());
            }
            let battery = if NOVA7_DISCRETE.contains(&m.pid) {
                (r[2] <= 4).then(|| r[2] * 25)
            } else {
                Some(pct(r[2]))
            };
            let mut values = Vec::new();
            if m.family == Family::Nova7Gen2 && r.len() >= 9 {
                values.extend(bool_value(Control::BtPowerOn, r[7]));
                if r[8] <= 2 {
                    values.push((Control::BtCall, r[8] as i32));
                }
            }
            Some(Status {
                connected: true,
                battery,
                charging: matches!(r[3], 1 | 2),
                chatmix: Some((pct(r[4]), pct(r[5]))),
                values,
            })
        }
        Family::Nova5 => {
            if r.len() < 7 || r[0] != 0xB0 {
                return None;
            }
            if r[1] == 0x02 {
                return Some(Status::default());
            }
            Some(Status {
                connected: true,
                battery: Some(pct(r[3])),
                charging: r[4] == 1,
                chatmix: Some((pct(r[5]), pct(r[6]))),
                values: Vec::new(),
            })
        }
        Family::Bragi | Family::Logitech => None,
        Family::NovaPro => {
            if r.len() < 16 || r[0] != 0x06 || r[1] != 0xB0 {
                return None;
            }
            if r[15] == 0x01 {
                return Some(Status::default());
            }
            Some(Status {
                connected: true,
                battery: (r[6] <= 8).then(|| (r[6] as u16 * 100 / 8) as u8),
                charging: r[15] == 0x02,
                chatmix: None,
                values: Vec::new(),
            })
        }
    }
}

/// Game and chat from a report the Nova Pro base pushes when its ChatMix dial turns.
pub fn chatmix_report(m: &Model, r: &[u8]) -> Option<(u8, u8)> {
    (m.family == Family::NovaPro && r.len() >= 4 && matches!(r[0], 0x06 | 0x07) && r[1] == 0x45)
        .then(|| (r[2].min(100), r[3].min(100)))
}

fn bool_value(c: Control, b: u8) -> Option<(Control, i32)> {
    (b <= 1).then_some((c, b as i32))
}

fn level(c: Control, b: u8, max: u8) -> Option<(Control, i32)> {
    (b <= max).then_some((c, b as i32))
}

/// Settings in a reply to one of [`Model::reads`]. Values outside their range are left out.
pub fn settings(m: &Model, r: &[u8]) -> Vec<(Control, i32)> {
    let mut out = Vec::new();
    match (m.family, r.first()) {
        (Family::Nova7Gen2, Some(0x20)) if r.len() >= 4 => {
            out.extend(level(Control::MicVolume, r[1], 7));
            if m.sidetone {
                out.extend(level(Control::Sidetone, r[2], 3));
            }
            out.extend(bool_value(Control::VolumeLimiter, r[3]));
        }
        (Family::Nova5, Some(0x20)) if r.len() >= 64 => {
            out.extend(level(Control::MicVolume, r[61], 15));
            out.extend(level(Control::Sidetone, r[62], 10));
            out.extend(bool_value(Control::VolumeLimiter, r[63]));
        }
        (Family::Nova7Gen2, Some(0xA0)) if r.len() >= 3 => {
            out.push((Control::AutoOff, r[1] as i32));
            out.extend(level(Control::MicLed, r[2], 3));
        }
        (Family::Nova5, Some(0xA0)) if r.len() >= 3 => {
            out.push((Control::AutoOff, r[1] as i32));
            if let Some(l) = NOVA5_LED.iter().position(|w| *w == r[2]) {
                out.push((Control::MicLed, l as i32));
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nova7() -> Model {
        model(0x22A1).unwrap()
    }

    #[test]
    fn models() {
        assert_eq!(nova7().family, Family::Nova7Gen2);
        assert_eq!(model(0x2202).unwrap().family, Family::Nova7);
        assert!(model(0x2258).is_none());
        assert!(!model(0x22A7).unwrap().sidetone);
        assert!(
            !model(0x220A)
                .unwrap()
                .controls()
                .iter()
                .any(|(c, _)| *c == Control::Sidetone)
        );
        assert_eq!(model(0x12E0).unwrap().collection(), (4, None));
    }

    #[test]
    fn corsair_headsets_come_from_the_table() {
        let hs80 = any_model(0x1B1C, 0x0A69).unwrap();
        assert_eq!((hs80.family, hs80.brand()), (Family::Bragi, "Corsair"));
        let ids: Vec<_> = hs80.controls().into_iter().map(|c| c.0).collect();
        assert!(ids.contains(&Control::Sidetone) && ids.contains(&Control::MicVolume));
        let receiver = any_model(0x1B1C, 0x0A6B).unwrap();
        assert_eq!((receiver.name, receiver.wireless), ("HS80", true));
        assert!(any_model(0x1B1C, 0x1B2D).is_none());
        assert_eq!(any_model(0x1038, 0x22A1).unwrap().brand(), "SteelSeries");
    }

    #[test]
    fn nova7_frames() {
        let m = nova7();
        assert_eq!(m.set(Control::Sidetone, 2).unwrap(), vec![vec![0, 0x39, 2]]);
        assert_eq!(
            m.set(Control::MicVolume, 7).unwrap(),
            vec![vec![0, 0x37, 7]]
        );
        assert_eq!(
            m.set(Control::VolumeLimiter, 1).unwrap(),
            vec![vec![0, 0x3A, 1]]
        );
        assert_eq!(
            m.set(Control::AutoOff, 30).unwrap(),
            vec![vec![0, 0xA3, 30]]
        );
        assert_eq!(m.set(Control::MicLed, 3).unwrap(), vec![vec![0, 0xAE, 3]]);
        assert_eq!(
            m.set(Control::BtPowerOn, 0).unwrap(),
            vec![vec![0, 0xB2, 0]]
        );
        assert_eq!(m.set(Control::BtCall, 2).unwrap(), vec![vec![0, 0xB3, 2]]);
        assert_eq!(
            m.save(Some(Control::Sidetone)),
            Save::Settled(vec![0, 0x09])
        );
    }

    #[test]
    fn ranges_are_checked() {
        let m = nova7();
        assert!(m.set(Control::Sidetone, 4).is_err());
        assert!(m.set(Control::MicVolume, 8).is_err());
        assert!(m.set(Control::MicVolume, -1).is_err());
        assert!(m.set(Control::AutoOff, 256).is_err());
        assert!(m.set(Control::BtCall, 3).is_err());
        assert!(m.set(Control::VolumeLimiter, 2).is_err());
        assert!(m.set(Control::EqPreset, 0).is_err());
        assert!(m.eq(&[0; 10]).is_err());
        assert!(model(0x22A7).unwrap().set(Control::Sidetone, 1).is_err());
    }

    #[test]
    fn nova5_frames() {
        let m = model(0x2232).unwrap();
        assert_eq!(
            m.set(Control::Sidetone, 10).unwrap(),
            vec![vec![0, 0x39, 10]]
        );
        assert_eq!(
            m.set(Control::MicVolume, 15).unwrap(),
            vec![vec![0, 0x37, 15]]
        );
        assert_eq!(
            m.set(Control::VolumeLimiter, 1).unwrap(),
            vec![vec![0, 0x27, 1]]
        );
        assert_eq!(m.set(Control::MicLed, 2).unwrap(), vec![vec![0, 0xAE, 4]]);
        assert_eq!(m.set(Control::MicLed, 3).unwrap(), vec![vec![0, 0xAE, 10]]);
        assert!(m.set(Control::BtPowerOn, 1).is_err());
        assert_eq!(m.save(Some(Control::Sidetone)), Save::None);
    }

    #[test]
    fn nova_pro_frames() {
        let m = model(0x12E0).unwrap();
        assert_eq!(m.set(Control::Sidetone, 3).unwrap(), vec![vec![6, 0x39, 3]]);
        assert_eq!(m.set(Control::AutoOff, 5).unwrap(), vec![vec![6, 0xC1, 5]]);
        assert!(m.set(Control::AutoOff, 7).is_err());
        assert_eq!(m.set(Control::EqPreset, 1).unwrap(), vec![vec![6, 0x2E, 1]]);
        assert_eq!(m.save(Some(Control::Sidetone)), Save::Now(vec![6, 0x09]));
        let mut bands = [0; 10];
        bands[0] = -20;
        bands[9] = 7;
        let f = m.eq(&bands).unwrap();
        assert_eq!(f[0], vec![6, 0x2E, 4]);
        assert_eq!(f[1], vec![6, 0x33, 0, 20, 20, 20, 20, 20, 20, 20, 20, 27]);
        bands[3] = 21;
        assert!(m.eq(&bands).is_err());
        assert!(m.eq(&[0; 9]).is_err());
    }

    #[test]
    fn nova7_replies() {
        let m = nova7();
        // Read from this PC's headset.
        let b0 = [0xB0, 0x03, 0x2E, 0x03, 0x64, 0x46, 0x00, 0x00, 0x00, 0x01];
        let s = status(&m, &b0).unwrap();
        assert!(s.connected && !s.charging);
        assert_eq!((s.battery, s.chatmix), (Some(46), Some((100, 70))));
        assert_eq!(
            s.values,
            vec![(Control::BtPowerOn, 0), (Control::BtCall, 0)]
        );
        assert!(!status(&m, &[0xB0, 0, 0, 0, 0, 0]).unwrap().connected);
        assert_eq!(
            settings(&m, &[0x20, 0x07, 0x00, 0x01]),
            vec![
                (Control::MicVolume, 7),
                (Control::Sidetone, 0),
                (Control::VolumeLimiter, 1)
            ]
        );
        assert_eq!(
            settings(&m, &[0xA0, 0x00, 0x01, 0x01, 0x00, 0xFF]),
            vec![(Control::AutoOff, 0), (Control::MicLed, 1)]
        );
        assert_eq!(
            settings(&m, &[0x20, 0x09, 0x05, 0x02]),
            Vec::<(Control, i32)>::new()
        );
        let old = model(0x2202).unwrap();
        assert_eq!(
            status(&old, &[0xB0, 0, 3, 1, 0, 0]).unwrap().battery,
            Some(75)
        );
        assert!(
            status(&old, &[0xB0, 0, 3, 1, 0, 0])
                .unwrap()
                .values
                .is_empty()
        );
        assert!(settings(&old, &[0x20, 0x07, 0x00, 0x01]).is_empty());
        assert_eq!(m.reply_head(&[0x00, 0x20]), &[0x20]);
    }

    #[test]
    fn nova5_and_pro_replies() {
        let m = model(0x2232).unwrap();
        let s = status(&m, &[0xB0, 0x03, 0, 60, 1, 50, 40]).unwrap();
        assert_eq!((s.battery, s.charging), (Some(60), true));
        assert!(!status(&m, &[0xB0, 0x02, 0, 0, 0, 0, 0]).unwrap().connected);
        let mut a = [0u8; 64];
        a[0] = 0x20;
        a[61] = 12;
        a[62] = 4;
        a[63] = 1;
        assert_eq!(
            settings(&m, &a),
            vec![
                (Control::MicVolume, 12),
                (Control::Sidetone, 4),
                (Control::VolumeLimiter, 1)
            ]
        );
        assert_eq!(
            settings(&m, &[0xA0, 10, 4]),
            vec![(Control::AutoOff, 10), (Control::MicLed, 2)]
        );
        let p = model(0x12E0).unwrap();
        let mut r = [0u8; 16];
        r[0] = 0x06;
        r[1] = 0xB0;
        r[6] = 4;
        r[15] = 0x02;
        let s = status(&p, &r).unwrap();
        assert_eq!((s.battery, s.charging), (Some(50), true));
        assert_eq!(p.reply_head(&[0x06, 0xB0]), &[0x06, 0xB0]);
    }

    #[test]
    fn nova_pro_chatmix() {
        let p = model(0x12E0).unwrap();
        assert_eq!(p.chatmix_switch(true), Some(vec![0x06, 0x49, 0x01]));
        assert_eq!(p.chatmix_switch(false), Some(vec![0x06, 0x49, 0x00]));
        assert_eq!(
            model(0x12E5).unwrap().chatmix_switch(true),
            Some(vec![0x06, 0x49, 0x01])
        );
        assert_eq!(nova7().chatmix_switch(true), None);
        let mut r = [0u8; 64];
        r[..4].copy_from_slice(&[0x07, 0x45, 64, 32]);
        assert_eq!(chatmix_report(&p, &r), Some((64, 32)));
        r[0] = 0x06;
        assert_eq!(chatmix_report(&p, &r), Some((64, 32)));
        assert_eq!(chatmix_report(&p, &[0x07, 0x45, 0, 100]), Some((0, 100)));
        assert_eq!(
            chatmix_report(&p, &[0x07, 0x45, 200, 101]),
            Some((100, 100))
        );
        assert_eq!(chatmix_report(&p, &[0x07, 0x25, 10, 0]), None);
        assert_eq!(chatmix_report(&p, &[0x07, 0x31, 2, 20]), None);
        assert_eq!(chatmix_report(&p, &[0x06, 0xB0, 0, 0]), None);
        assert_eq!(chatmix_report(&p, &[0x05, 0x45, 50, 50]), None);
        assert_eq!(chatmix_report(&p, &[0x07, 0x45, 50]), None);
        assert_eq!(chatmix_report(&nova7(), &[0x07, 0x45, 50, 50]), None);
    }
}
