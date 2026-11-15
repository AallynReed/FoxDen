//! ASUS mouse models (protocol learned from SignalRGB's Asus_Mouse.js device library).

use super::{Model, Wire};
use foxrgb_hid::registry::Detector;

pub static ROG_KERIS: Model = Model {
    size: [7, 8],
    cells: &[[3, 5], [3, 0]],
    wire: Wire::Legacy,
    brightness: 4,
};

pub static GLADIUS_III_AIMPOINT: Model = Model {
    size: [3, 3],
    cells: &[[1, 0]],
    wire: Wire::Modern,
    brightness: 4,
};

pub static ROG_HARPE_ACE_AIM_LAB_EDITION: Model = Model {
    size: [3, 5],
    cells: &[[1, 0]],
    wire: Wire::Modern,
    brightness: 4,
};

pub static ROG_KERIS_II_ORIGIN: Model = Model {
    size: [5, 6],
    cells: &[[2, 5], [2, 0], [0, 3]],
    wire: Wire::Legacy,
    brightness: 100,
};

pub static PIDS: &[(u16, &Model)] = &[
    (0x1A59, &ROG_KERIS),
    (0x1A92, &ROG_HARPE_ACE_AIM_LAB_EDITION),
    (0x1A94, &ROG_HARPE_ACE_AIM_LAB_EDITION),
    (0x1B0C, &GLADIUS_III_AIMPOINT),
    (0x1C0C, &ROG_KERIS_II_ORIGIN),
];

pub static DETECTORS: &[Detector] = &[
    super::mouse("ASUS ROG Keris", 0x1A59, 0, 0xFF01, 1),
    super::mouse("ASUS ROG Harpe Ace AIM LAB Edition", 0x1A92, 0, 0xFF01, 1),
    super::mouse("ASUS ROG Harpe Ace AIM LAB Edition", 0x1A94, 0, 0xFF01, 1),
    super::mouse("ASUS Gladius III Aimpoint", 0x1B0C, 0, 0xFF01, 1),
    super::mouse("ASUS ROG Keris II Origin", 0x1C0C, 0, 0xFF01, 1),
];
