use serde::{Deserialize, Serialize};

use crate::Color;

/// Effects a device runs on its own, without frames from FoxDen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hardware {
    pub modes: Vec<HardwareMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HardwareMode {
    pub id: String,
    pub name: String,
    /// Lowest and highest value the device takes, when the mode has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<[u8; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<[u8; 2]>,
    #[serde(default)]
    pub colors: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directions: Vec<String>,
    #[serde(default)]
    pub random: bool,
}

impl HardwareMode {
    pub fn new(id: &str, name: &str) -> Self {
        HardwareMode {
            id: id.into(),
            name: name.into(),
            speed: None,
            brightness: None,
            colors: 0,
            directions: Vec::new(),
            random: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HardwareSetting {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default)]
    pub random: bool,
}

impl HardwareSetting {
    pub fn new(mode: &str) -> Self {
        HardwareSetting {
            mode: mode.into(),
            speed: None,
            brightness: None,
            colors: Vec::new(),
            direction: None,
            random: false,
        }
    }
}

/// A setting fitted to its mode: values in range, every colour present.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    pub speed: u8,
    pub brightness: u8,
    pub colors: Vec<Color>,
    pub direction: usize,
    pub random: bool,
}

impl Hardware {
    pub fn mode(&self, id: &str) -> Option<&HardwareMode> {
        self.modes.iter().find(|m| m.id == id)
    }

    /// The mode a setting names and the setting fitted to it.
    pub fn fit(&self, s: &HardwareSetting) -> Result<(&HardwareMode, Fitted), String> {
        let mode = self
            .mode(&s.mode)
            .ok_or_else(|| format!("unknown hardware mode {}", s.mode))?;
        let pick = |range: Option<[u8; 2]>, v: Option<u8>, default: fn([u8; 2]) -> u8| {
            range.map_or(0, |r| v.map_or(default(r), |v| v.clamp(r[0], r[1])))
        };
        let mut colors = s.colors.clone();
        colors.resize(mode.colors, Color(255, 255, 255));
        let direction = s
            .direction
            .as_ref()
            .and_then(|d| mode.directions.iter().position(|m| m == d))
            .unwrap_or(0);
        let fitted = Fitted {
            speed: pick(mode.speed, s.speed, |r| r[0] + (r[1] - r[0]) / 2),
            brightness: pick(mode.brightness, s.brightness, |r| r[1]),
            colors,
            direction,
            random: mode.random && s.random,
        };
        Ok((mode, fitted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_fitted_to_its_mode() {
        let mut m = HardwareMode::new("breathing", "Breathing");
        m.speed = Some([10, 20]);
        m.brightness = Some([0, 200]);
        m.colors = 2;
        m.directions = vec!["left".into(), "right".into()];
        let hw = Hardware { modes: vec![m] };
        let s: HardwareSetting =
            serde_json::from_str(r#"{"mode":"breathing","speed":99,"colors":[[1,2,3]],"direction":"right","random":true}"#)
                .unwrap();
        let (_, f) = hw.fit(&s).unwrap();
        assert_eq!(f.speed, 20);
        assert_eq!(f.brightness, 200);
        assert_eq!(f.colors, [Color(1, 2, 3), Color(255, 255, 255)]);
        assert_eq!(f.direction, 1);
        assert!(!f.random);
        let bare: HardwareSetting = serde_json::from_str(r#"{"mode":"breathing"}"#).unwrap();
        assert_eq!(hw.fit(&bare).unwrap().1.speed, 15);
        assert!(
            hw.fit(&HardwareSetting {
                mode: "x".into(),
                ..bare
            })
            .is_err()
        );
    }
}
