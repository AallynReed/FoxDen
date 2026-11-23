//! Which pads speak the config protocol, and where each setting sits in their config block.

use crate::proto::Wire;

pub const VID: u16 = 0x2DC8;

/// Offsets into one family's config block. Every per-profile section holds `slots` records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: usize,
    pub slots: usize,
    pub crc: usize,
    pub mode: usize,
    pub slot: usize,
    pub names: usize,
    pub rumble: usize,
    pub sticks: usize,
    pub triggers: usize,
    pub features: usize,
    pub keys: usize,
    pub key_count: usize,
    /// Fixed macros (`macro_fun_record_t_Pro2`, 392 bytes a profile).
    pub macros: Option<usize>,
    /// Recorded macros (`record_macro_fun_record_t`, 216 bytes a profile).
    pub recorded: Option<usize>,
    pub xinput_rumble: Option<usize>,
    pub motion: Option<usize>,
    pub light: Option<LightLayout>,
    /// HitBox 2 `aw20144s_rgb_config_t`, 376 bytes a profile.
    pub rgb_raw: Option<usize>,
    /// HitBox 2 combos, 164 bytes a profile.
    pub combos: Option<usize>,
}

/// The light-ring sections of the Ultimate 2 family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightLayout {
    pub dynamic: usize,
    pub wheel: usize,
    pub single: usize,
    pub leds: usize,
}

pub const NAME_LEN: usize = 32;
pub const MACRO_LEN: usize = 392;
pub const RECORDED_LEN: usize = 216;
pub const RGB_RAW_LEN: usize = 376;
pub const COMBO_LEN: usize = 164;

const fn three(size: usize, key_count: usize) -> Layout {
    Layout {
        size,
        slots: 3,
        crc: 12,
        mode: 16,
        slot: 18,
        names: 20,
        rumble: 116,
        sticks: 152,
        triggers: 176,
        features: 200,
        keys: 224,
        key_count,
        macros: None,
        recorded: None,
        xinput_rumble: None,
        motion: None,
        light: None,
        rgb_raw: None,
        combos: None,
    }
}

/// Pro 2, Ultimate Bluetooth, Ultimate 2.4G and Ultimate PC.
pub const CLASSIC: Layout = Layout {
    macros: Some(476),
    recorded: Some(1652),
    xinput_rumble: Some(2300),
    ..three(0x914, 20)
};

/// Pro 3 and the HitBox arcade controller.
pub const PRO3: Layout = Layout {
    macros: Some(500),
    recorded: Some(1676),
    xinput_rumble: Some(2324),
    ..three(0x92C, 22)
};

/// Ultimate 2 Wireless.
pub const ULTIMATE2: Layout = Layout {
    recorded: Some(500),
    xinput_rumble: Some(1148),
    motion: Some(1172),
    light: Some(LightLayout {
        dynamic: 1208,
        wheel: 1244,
        single: 1292,
        leds: 24,
    }),
    ..three(0x638, 22)
};

/// Ultimate 2 Bluetooth.
pub const ULTIMATE2_BT: Layout = Layout {
    macros: Some(500),
    recorded: Some(1676),
    xinput_rumble: Some(2324),
    motion: Some(2348),
    light: Some(LightLayout {
        dynamic: 2384,
        wheel: 2420,
        single: 2468,
        leds: 24,
    }),
    ..three(0xAD0, 22)
};

/// HitBox 2: two profiles.
pub const HITBOX2: Layout = Layout {
    size: 0xA68,
    slots: 2,
    crc: 8,
    mode: 12,
    slot: 14,
    names: 16,
    rumble: 80,
    sticks: 104,
    triggers: 120,
    features: 136,
    keys: 152,
    key_count: 24,
    macros: Some(352),
    recorded: Some(1136),
    xinput_rumble: Some(1568),
    motion: None,
    light: None,
    rgb_raw: Some(1584),
    combos: Some(2336),
};

/// One pad, by the product id it reports on its vendor channel (`cmd::INFO`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub pid: u16,
    pub name: &'static str,
    pub layout: Layout,
    pub wire: Wire,
}

const LONG: Wire = Wire {
    long: true,
    crc: true,
};
const CRC: Wire = Wire {
    long: false,
    crc: true,
};
const PLAIN: Wire = Wire {
    long: false,
    crc: false,
};

pub const MODELS: [Model; 11] = [
    Model {
        pid: 0x6012,
        name: "8BitDo Ultimate 2 Wireless",
        layout: ULTIMATE2,
        wire: LONG,
    },
    Model {
        pid: 0x600F,
        name: "8BitDo Ultimate 2 Bluetooth",
        layout: ULTIMATE2_BT,
        wire: LONG,
    },
    Model {
        pid: 0x6009,
        name: "8BitDo Pro 3",
        layout: PRO3,
        wire: LONG,
    },
    Model {
        pid: 0x600B,
        name: "8BitDo HitBox Arcade Controller",
        layout: PRO3,
        wire: LONG,
    },
    Model {
        pid: 0x2062,
        name: "8BitDo HitBox 2 Arcade Controller",
        layout: HITBOX2,
        wire: LONG,
    },
    Model {
        pid: 0x6007,
        name: "8BitDo Ultimate Bluetooth",
        layout: CLASSIC,
        wire: CRC,
    },
    Model {
        pid: 0x3011,
        name: "8BitDo Ultimate Wired for PC",
        layout: CLASSIC,
        wire: CRC,
    },
    Model {
        pid: 0x3010,
        name: "8BitDo Pro 2 Wired",
        layout: CLASSIC,
        wire: CRC,
    },
    Model {
        pid: 0x6006,
        name: "8BitDo Pro 2",
        layout: CLASSIC,
        wire: CRC,
    },
    Model {
        pid: 0x6003,
        name: "8BitDo Pro 2",
        layout: CLASSIC,
        wire: PLAIN,
    },
    Model {
        pid: 0x3012,
        name: "8BitDo Ultimate 2.4G",
        layout: CLASSIC,
        wire: PLAIN,
    },
];

/// Receivers, docks and second ids that stand for a pad from `MODELS`.
const ALIASES: [(u16, u16); 8] = [
    (0x6013, 0x6012),
    (0x6011, 0x600F),
    (0x600A, 0x6009),
    (0x600D, 0x6009),
    (0x600C, 0x600B),
    (0x2085, 0x2062),
    (0x3106, 0x6007),
    (0x3013, 0x3012),
];

pub fn model(pid: u16) -> Option<&'static Model> {
    let pid = ALIASES.iter().find(|a| a.0 == pid).map_or(pid, |a| a.1);
    MODELS.iter().find(|m| m.pid == pid)
}

/// Light ring themes the Ultimate 2 switches to at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    /// Follows the sticks, in `dynamic` colours.
    Dynamic,
    /// Spins, in `wheel` colours and speed.
    Wheel,
    /// The 24 colours of `single`.
    Custom,
    Off,
    /// The off theme of the alternate shell colour.
    OffAlt,
    Other(u16),
}

impl Theme {
    pub fn code(self) -> u16 {
        match self {
            Theme::Dynamic => 2,
            Theme::Wheel => 3,
            Theme::Custom => 4,
            Theme::Off => 5,
            Theme::OffAlt => 6,
            Theme::Other(n) => n,
        }
    }

    pub fn from_code(n: u16) -> Theme {
        match n {
            2 => Theme::Dynamic,
            3 => Theme::Wheel,
            4 => Theme::Custom,
            5 => Theme::Off,
            6 => Theme::OffAlt,
            n => Theme::Other(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(l: &Layout) -> usize {
        let per = 4 + 4 * l.key_count;
        let mut end = l.keys + per * l.slots;
        for (at, len) in [
            (l.macros, MACRO_LEN),
            (l.recorded, RECORDED_LEN),
            (l.xinput_rumble, 8),
            (l.motion, 12),
            (l.rgb_raw, RGB_RAW_LEN),
            (l.combos, COMBO_LEN),
        ] {
            if let Some(at) = at {
                end = end.max(at + len * l.slots);
            }
        }
        if let Some(li) = l.light {
            end = end.max(li.single + (4 + 4 * li.leds) * l.slots);
        }
        end
    }

    // The sizes the vendor library's exports declare: writeUltimate2@1592, ANSI_CRC_16_Pro3@2348...
    #[test]
    fn layouts_fill_their_blocks() {
        for (l, size) in [
            (CLASSIC, 2324),
            (PRO3, 2348),
            (ULTIMATE2, 1592),
            (ULTIMATE2_BT, 2768),
            (HITBOX2, 2664),
        ] {
            assert_eq!(l.size, size);
            assert_eq!(end(&l), size, "{l:?}");
        }
    }

    #[test]
    fn receivers_resolve() {
        assert_eq!(model(0x6013).unwrap().pid, 0x6012);
        assert_eq!(model(0x600F).unwrap().layout, ULTIMATE2_BT);
        assert!(model(0x310B).is_none());
    }
}
