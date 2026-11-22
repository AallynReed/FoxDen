//! Sound profiles: named processing for an output channel or the mic, shareable as a short code.

use crate::share;
use foxden_mixer::dsp::{self, Band, FilterType};
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "foxden-sound:";
const VERSION: u32 = 1;
const MAX_CODE: usize = 2000;
pub const MAX_PROFILES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Output,
    Mic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Eq {
    pub on: bool,
    pub bands: Vec<Band>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    pub on: bool,
    pub db: f32,
}

/// What a profile sets; anything `None` is left as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub kind: Kind,
    #[serde(default, skip_deserializing)]
    pub builtin: bool,
    #[serde(default)]
    pub eq: Option<Eq>,
    #[serde(default)]
    pub volume: Option<f32>,
    /// Mic only.
    #[serde(default)]
    pub gate: Option<Gate>,
}

fn num(v: f32, lo: f32, hi: f32, or: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { or }
}

pub fn clean_name(name: &str) -> String {
    share::clean_name(name, "")
}

impl Profile {
    /// Values in range, a bounded size, nothing a mic-only profile carries on an output.
    pub fn clean(mut self) -> Profile {
        self.name = share::clean_name(&self.name, "Shared profile");
        self.builtin = false;
        if let Some(eq) = &mut self.eq {
            eq.bands.truncate(dsp::MAX_BANDS);
            for b in &mut eq.bands {
                b.frequency = num(b.frequency, dsp::MIN_FREQ, dsp::MAX_FREQ, 1000.0);
                b.gain_db = num(b.gain_db, -24.0, 24.0, 0.0);
                b.q = num(b.q, 0.1, 20.0, 1.414);
            }
            if eq.bands.is_empty() {
                self.eq = None;
            }
        }
        self.volume = self.volume.map(|v| num(v, 0.0, 1.0, 1.0));
        self.gate = match self.kind {
            Kind::Mic => self.gate.map(|g| Gate {
                on: g.on,
                db: num(g.db, -90.0, 0.0, -50.0),
            }),
            Kind::Output => None,
        };
        self
    }
}

fn gains(g: [f32; 10]) -> Vec<Band> {
    dsp::default_bands()
        .into_iter()
        .zip(g)
        .map(|(b, gain_db)| Band { gain_db, ..b })
        .collect()
}

fn band(filter: FilterType, frequency: f32, gain_db: f32, q: f32) -> Band {
    Band {
        enabled: true,
        filter,
        frequency,
        gain_db,
        q,
    }
}

fn builtin(name: &str, kind: Kind, eq: Option<Vec<Band>>, gate: Option<f32>) -> Profile {
    Profile {
        name: name.into(),
        kind,
        builtin: true,
        eq: eq.map(|bands| Eq { on: true, bands }),
        volume: None,
        gate: gate.map(|db| Gate { on: true, db }),
    }
}

pub fn builtins() -> Vec<Profile> {
    use FilterType::*;
    use Kind::*;
    vec![
        builtin("Flat", Output, Some(gains([0.0; 10])), None),
        builtin(
            "Bass boost",
            Output,
            Some(gains([6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])),
            None,
        ),
        builtin(
            "Voice clarity",
            Output,
            Some(gains([
                -4.0, -3.0, -2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 2.0, 0.0,
            ])),
            None,
        ),
        builtin(
            "Treble",
            Output,
            Some(gains([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 5.0, 6.0])),
            None,
        ),
        builtin(
            "Clean voice",
            Mic,
            Some(vec![
                band(HighPass, 80.0, 0.0, 0.707),
                band(PeakingEQ, 250.0, -2.0, 1.0),
                band(PeakingEQ, 3000.0, 2.0, 1.0),
            ]),
            Some(-50.0),
        ),
        builtin(
            "Broadcast",
            Mic,
            Some(vec![
                band(HighPass, 70.0, 0.0, 0.707),
                band(LowShelving, 120.0, 3.0, 0.707),
                band(PeakingEQ, 400.0, -2.0, 1.2),
                band(PeakingEQ, 3500.0, 3.0, 0.9),
                band(HighShelving, 10000.0, 2.0, 0.707),
            ]),
            Some(-45.0),
        ),
        builtin("Noise gate", Mic, None, Some(-40.0)),
    ]
}

/// Filter index, Hz, dB, Q, enabled.
type ShareBand = (u8, f32, f32, f32, bool);

/// The share code's JSON, with short keys to keep codes small.
#[derive(Serialize, Deserialize)]
struct Share {
    v: u32,
    n: String,
    k: Kind,
    /// EQ on and bands as [filter, Hz, dB, Q, enabled].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    e: Option<(bool, Vec<ShareBand>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    o: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    g: Option<(bool, f32)>,
}

fn round(v: f32, places: i32) -> f32 {
    let m = 10f32.powi(places);
    (v * m).round() / m
}

pub fn encode(p: &Profile) -> String {
    let share = Share {
        v: VERSION,
        n: p.name.clone(),
        k: p.kind,
        e: p.eq.as_ref().map(|e| {
            let bands = e
                .bands
                .iter()
                .map(|b| {
                    let f = FilterType::ALL
                        .iter()
                        .position(|x| *x == b.filter)
                        .unwrap_or(7);
                    (
                        f as u8,
                        round(b.frequency, 0),
                        round(b.gain_db, 1),
                        round(b.q, 3),
                        b.enabled,
                    )
                })
                .collect();
            (e.on, bands)
        }),
        o: p.volume.map(|v| round(v, 2)),
        g: p.gate.map(|g| (g.on, round(g.db, 1))),
    };
    share::encode(PREFIX, &share)
}

/// Reads a share code. It is only ever data: checked, clamped and bounded.
pub fn decode(code: &str) -> Result<Profile, String> {
    let s: Share = share::decode(code, PREFIX, MAX_CODE, "sound")?;
    let eq = match s.e {
        Some((on, bands)) => {
            let mut out = Vec::new();
            for (f, frequency, gain_db, q, enabled) in bands.into_iter().take(dsp::MAX_BANDS) {
                let filter = *FilterType::ALL
                    .get(f as usize)
                    .ok_or("The share code has an unknown filter")?;
                out.push(Band {
                    enabled,
                    filter,
                    frequency,
                    gain_db,
                    q,
                });
            }
            Some(Eq { on, bands: out })
        }
        None => None,
    };
    Ok(Profile {
        name: s.n,
        kind: s.k,
        builtin: false,
        eq,
        volume: s.o,
        gate: s.g.map(|(on, db)| Gate { on, db }),
    }
    .clean())
}

#[tauri::command]
pub fn sound_code(profile: Profile) -> String {
    encode(&profile.clean())
}

#[tauri::command]
pub fn sound_decode(code: String) -> Result<Profile, String> {
    decode(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    #[test]
    fn codes_round_trip_and_stay_short() {
        for p in builtins() {
            let code = encode(&p);
            assert!(code.starts_with(PREFIX) && code.len() < 600, "{code}");
            let back = decode(&code).unwrap();
            assert_eq!((back.name.as_str(), back.kind), (p.name.as_str(), p.kind));
            assert_eq!(back.eq, p.eq);
            assert_eq!(back.gate, p.gate);
            assert!(!back.builtin);
        }
    }

    #[test]
    fn values_are_clamped_and_output_profiles_carry_no_gate() {
        let p = Profile {
            name: "  Loud\u{7}  ".into(),
            kind: Kind::Output,
            builtin: true,
            eq: Some(Eq {
                on: true,
                bands: vec![band(FilterType::PeakingEQ, 99_999.0, 80.0, 0.0); 30],
            }),
            volume: Some(4.0),
            gate: Some(Gate {
                on: true,
                db: -20.0,
            }),
        };
        let back = decode(&encode(&p)).unwrap();
        assert_eq!(back.name, "Loud");
        let b = &back.eq.as_ref().unwrap().bands;
        assert_eq!(b.len(), dsp::MAX_BANDS);
        assert_eq!(
            (b[0].frequency, b[0].gain_db, b[0].q),
            (dsp::MAX_FREQ, 24.0, 0.1)
        );
        assert_eq!(back.volume, Some(1.0));
        assert_eq!(back.gate, None);
    }

    #[test]
    fn bad_codes_are_refused() {
        let wrap = |json: &str| format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json));
        assert!(decode("hello").is_err());
        assert!(decode(&format!("{PREFIX}!!!")).is_err());
        assert!(decode(&wrap("[1,2]")).is_err());
        let newer = decode(&wrap(r#"{"v":2,"n":"x","k":"mic"}"#)).unwrap_err();
        assert!(newer.contains("newer"));
        assert!(decode(&wrap(r#"{"v":1,"n":"x","k":"speaker"}"#)).is_err());
        assert!(
            decode(&wrap(
                r#"{"v":1,"n":"x","k":"mic","e":[true,[[42,100,0,1,true]]]}"#
            ))
            .is_err()
        );
        assert!(decode(&format!("{PREFIX}{}", "A".repeat(MAX_CODE))).is_err());
        let ok = decode(&wrap(r#"{"v":1,"n":"","k":"mic","g":[true,-500]}"#)).unwrap();
        assert_eq!(ok.name, "Shared profile");
        assert_eq!(
            ok.gate,
            Some(Gate {
                on: true,
                db: -90.0
            })
        );
    }
}
