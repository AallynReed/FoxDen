// SPDX-License-Identifier: GPL-2.0-or-later

use std::mem::size_of;

use foxden_core::{Backend, Color, Device, DeviceKind};

use crate::nvidia;

const ID_GET_CONTROL: u32 = 0x3DBF5764;
const ID_SET_CONTROL: u32 = 0x197D065E;
/// NV_GPU_CLIENT_ILLUM_ZONE_CONTROL_PARAMS_V1, as the sniffed 72012.
const VERSION: u32 = (1 << 16) | size_of::<Params>() as u32;
const ZONES_MAX: usize = 32;
const CTRL_MODE_MANUAL_RGB: u32 = 0;
const ZONE_RGB: u32 = 1;
const ZONE_COLOR_FIXED: u32 = 2;
const ZONE_RGBW: u32 = 3;
const ZONE_SINGLE_COLOR: u32 = 4;
const DIRECT_BRIGHTNESS: u8 = 100;
const RECT: [f32; 4] = [0.06, 0.30, 0.40, 0.05];

/// device_list: vendor, device, subsystem vendor, subsystem device, treats RGBW as RGB, name.
#[rustfmt::skip]
const DEVICES: &[(u16, u16, u16, u16, bool, &str)] = &[
    (0x10DE, 0x1E89, 0x10DE, 0x1E89, false, "Palit GeForce RTX 2060"),
    (0x10DE, 0x1F08, 0x10DE, 0x12FB, false, "NVIDIA GeForce RTX 2060 FE"),
    (0x10DE, 0x1F06, 0x10DE, 0x13A2, false, "NVIDIA GeForce RTX 2060 SUPER FE"),
    (0x10DE, 0x1F07, 0x10DE, 0x12AD, false, "NVIDIA GeForce RTX 2070 FE"),
    (0x10DE, 0x1EC7, 0x10DE, 0x13AA, false, "NVIDIA GeForce RTX 2070 SUPER FE"),
    (0x10DE, 0x1E87, 0x10DE, 0x12A6, false, "NVIDIA GeForce RTX 2080 FE"),
    (0x10DE, 0x1E81, 0x10DE, 0x13A0, false, "NVIDIA GeForce RTX 2080 SUPER FE"),
    (0x10DE, 0x1E07, 0x10DE, 0x12A4, false, "NVIDIA GeForce RTX 2080 Ti FE"),
    (0x10DE, 0x1E07, 0x10DE, 0x1435, false, "NVIDIA GeForce RTX 2080 Ti Cyberpunk 2077"),
    (0x10DE, 0x1E02, 0x10DE, 0x12A3, false, "NVIDIA TITAN RTX"),
    (0x10DE, 0x2507, 0x10DE, 0x2507, true, "Gainward GeForce RTX 3050 LHR"),
    (0x10DE, 0x2544, 0x10DE, 0x2544, true, "Gainward GeForce RTX 3060 Pegasus"),
    (0x10DE, 0x2504, 0x10DE, 0x2504, true, "Palit GeForce RTX 3060 LHR"),
    (0x10DE, 0x2504, 0x196E, 0x138E, true, "PNY GeForce RTX 3060 XLR8 REVEL EPIC-X"),
    (0x10DE, 0x2504, 0x196E, 0x138F, true, "PNY GeForce RTX 3060 XLR8 REVEL EPIC-X LHR"),
    (0x10DE, 0x2487, 0x196E, 0x138E, true, "PNY GeForce RTX 3060 XLR8 REVEL EPIC-X"),
    (0x10DE, 0x2487, 0x10DE, 0x2487, true, "Palit GeForce RTX 3060 LHR"),
    (0x10DE, 0x2489, 0x196E, 0x1389, true, "PNY GeForce RTX 3060 Ti XLR8 REVEL EPIC-X"),
    (0x10DE, 0x2489, 0x10DE, 0x1389, true, "PNY GeForce RTX 3060 Ti XLR8 REVEL EPIC-X"),
    (0x10DE, 0x2414, 0x10DE, 0x2414, true, "NVIDIA GeForce RTX 3060 Ti V1 LHR"),
    (0x10DE, 0x2489, 0x10DE, 0x2489, true, "NVIDIA GeForce RTX 3060 Ti LHR"),
    (0x10DE, 0x2482, 0x196E, 0x138D, false, "PNY GeForce RTX 3070 Ti XLR8 Uprising EPIC-X"),
    (0x10DE, 0x2206, 0x10DE, 0x1467, false, "NVIDIA GeForce RTX 3080 FE"),
    (0x10DE, 0x2208, 0x10DE, 0x1535, false, "NVIDIA GeForce RTX 3080 Ti FE"),
    (0x10DE, 0x2208, 0x10DE, 0x2612, true, "MANLI GeForce RTX 3080 Ti GALLARDO"),
    (0x10DE, 0x2204, 0x10DE, 0x147D, false, "NVIDIA GeForce RTX 3090 FE"),
    (0x10DE, 0x2203, 0x10DE, 0x1618, false, "NVIDIA GeForce RTX 3090 Ti FE"),
    (0x10DE, 0x2882, 0x1569, 0xF311, true, "Palit GeForce RTX 4060 Dual"),
    (0x10DE, 0x2786, 0x1569, 0xF302, true, "Palit GeForce RTX 4070"),
    (0x10DE, 0x2783, 0x1569, 0xF302, true, "Palit GeForce RTX 4070 SUPER Dual"),
    (0x10DE, 0x2786, 0x10B0, 0xF303, false, "Gainward GeForce RTX 4070 Ghost"),
    (0x10DE, 0x2783, 0x10B0, 0xF303, false, "Gainward GeForce RTX 4070 SUPER Ghost"),
    (0x10DE, 0x2704, 0x10DE, 0x167A, false, "NVIDIA GeForce RTX 4080 FE"),
    (0x10DE, 0x2704, 0x10DE, 0x1794, false, "NVIDIA GeForce RTX 4080 FE"),
    (0x10DE, 0x2702, 0x10DE, 0x18CB, false, "NVIDIA GeForce RTX 4080 SUPER FE"),
    (0x10DE, 0x2684, 0x10DE, 0x165B, false, "NVIDIA GeForce RTX 4090 FE"),
    (0x10DE, 0x2684, 0x10DE, 0x16F4, false, "NVIDIA GeForce RTX 4090 FE"),
    (0x10DE, 0x2C02, 0x10DE, 0x2095, false, "NVIDIA GeForce RTX 5080 FE"),
    (0x10DE, 0x2B85, 0x10DE, 0x2057, false, "NVIDIA GeForce RTX 5090 FE"),
];

#[repr(C)]
#[derive(Clone, Copy)]
struct Zone {
    kind: u32,
    ctrl_mode: u32,
    data: [u8; 128],
    rsvd: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Params {
    version: u32,
    /// bDefault in bit 0, then rsvdField.
    flags: u32,
    count: u32,
    rsvd: [u8; 64],
    zones: [Zone; ZONES_MAX],
}

impl Params {
    fn zeroed() -> Params {
        let zone = Zone {
            kind: 0,
            ctrl_mode: 0,
            data: [0; 128],
            rsvd: [0; 64],
        };
        Params {
            version: 0,
            flags: 0,
            count: 0,
            rsvd: [0; 64],
            zones: [zone; ZONES_MAX],
        }
    }
}

/// `set` picks NvAPI_GPU_ClientIllumZonesSetControl over GetControl.
type Call = Box<dyn FnMut(bool, &mut Params) -> i32 + Send>;

fn pause() {
    // nvapi_accessor::nvapi_zone_control: NvAPI ignores calls that come faster than this
    #[cfg(not(test))]
    std::thread::sleep(std::time::Duration::from_millis(30));
}

struct Illumination {
    device: Device,
    call: Call,
    rgbw_as_rgb: bool,
    params: Box<Params>,
    sent: Option<Vec<Color>>,
}

impl Illumination {
    fn control(&mut self, set: bool) -> Result<(), String> {
        let status = (self.call)(set, &mut self.params);
        pause();
        match status {
            0 => Ok(()),
            e => Err(format!("NvAPI illumination call failed ({e})")),
        }
    }

    // NVIDIAIlluminationV1Controller::getControl
    fn get(&mut self) -> Result<(), String> {
        *self.params = Params::zeroed();
        self.params.version = VERSION;
        self.control(false)
    }

    // NVIDIAIlluminationV1Controller::setZone for NVIDIA_ILLUMINATION_DIRECT
    fn zone(&mut self, zone: usize, c: Color, brightness: u8) -> Result<(), String> {
        let got = self.get();
        let rgbw_as_rgb = self.rgbw_as_rgb;
        let z = &mut self.params.zones[zone];
        let Color(mut r, mut g, mut b) = c;
        let mut w = 0;
        z.ctrl_mode = CTRL_MODE_MANUAL_RGB;
        match z.kind {
            ZONE_RGB => z.data[..4].copy_from_slice(&[r, g, b, brightness]),
            ZONE_RGBW => {
                if !rgbw_as_rgb {
                    let lo = r.min(g).min(b) as i32;
                    let hi = r.max(g).max(b) as i32;
                    if hi - lo <= 10 {
                        (r, g, b) = (0, 0, 0);
                        w = ((hi + lo) / 2) as u8;
                    }
                }
                z.data[..5].copy_from_slice(&[r, g, b, w, brightness]);
            }
            ZONE_SINGLE_COLOR | ZONE_COLOR_FIXED => {
                z.data[0] = if (r, g, b) == (0, 0, 0) {
                    0
                } else {
                    brightness
                };
            }
            _ => {}
        }
        let set = self.control(true);
        got.and(set)
    }
}

impl Backend for Illumination {
    fn name(&self) -> &str {
        "NVIDIA Illumination"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    // RGBController_NVIDIAIlluminationV1::DeviceUpdateZoneLEDs for each zone that changed
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if colors.len() != self.device.leds.len() {
            return Err(format!(
                "{} expects {} colours, got {}",
                self.device.name,
                self.device.leds.len(),
                colors.len()
            ));
        }
        let mut result = Ok(());
        for (i, &c) in colors.iter().enumerate() {
            if self.sent.as_ref().is_some_and(|s| s[i] == c) {
                continue;
            }
            let r = self.zone(i, c, DIRECT_BRIGHTNESS);
            result = result.and(r);
        }
        if result.is_ok() {
            self.sent = Some(colors.to_vec());
        }
        result
    }
}

/// DetectNVIDIAIllumGPUs for one GPU: its table row, then the zones getInfo reports.
fn found(index: usize, ids: [u16; 4], call: Call) -> Option<Box<dyn Backend>> {
    let &(.., rgbw_as_rgb, name) = DEVICES.iter().find(|d| [d.0, d.1, d.2, d.3] == ids)?;
    let mut backend = Illumination {
        device: Device::grid("", name, DeviceKind::Gpu, RECT, 1, 1),
        call,
        rgbw_as_rgb,
        params: Box::new(Params::zeroed()),
        sent: None,
    };
    let _ = backend.get();
    let zones = (backend.params.count as usize).min(ZONES_MAX) as u32;
    if zones == 0 {
        return None;
    }
    let id = format!("gpu:nvidia-illumination:{index}");
    backend.device = Device::grid(&id, name, DeviceKind::Gpu, RECT, zones, 1);
    Some(Box::new(backend))
}

type ZoneControl = unsafe extern "C" fn(*mut i32, *mut Params) -> i32;

/// Every NVIDIA card in the device list. Empty without the driver.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let Some((query_interface, gpus)) = nvidia::load() else {
        return Vec::new();
    };
    let get = unsafe { nvidia::query::<ZoneControl>(query_interface, ID_GET_CONTROL) };
    let set = unsafe { nvidia::query::<ZoneControl>(query_interface, ID_SET_CONTROL) };
    let (Some(get), Some(set)) = (get, set) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, (gpu, ids)) in gpus.into_iter().enumerate() {
        let Some(ids) = ids else { continue };
        let call: Call = Box::new(move |is_set, params| unsafe {
            if is_set {
                set(gpu.0, params)
            } else {
                get(gpu.0, params)
            }
        });
        out.extend(found(index, ids, call));
    }
    out
}

/// Whether the device list has this card.
pub fn known(ids: [u16; 4]) -> bool {
    DEVICES.iter().any(|d| [d.0, d.1, d.2, d.3] == ids)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// (set, version, flags, count, per zone: type, control mode, first five data bytes)
    type Seen = (bool, u32, u32, u32, Vec<(u32, u32, [u8; 5])>);

    /// A card whose GetControl reports these zone types.
    fn card(kinds: &'static [u32], log: Arc<Mutex<Vec<Seen>>>) -> Call {
        Box::new(move |set, p| {
            if !set {
                p.count = kinds.len() as u32;
                for (z, &k) in p.zones.iter_mut().zip(kinds) {
                    z.kind = k;
                    z.data[..5].copy_from_slice(&[9, 9, 9, 9, 9]);
                }
            }
            let zones = p.zones[..p.count as usize]
                .iter()
                .map(|z| (z.kind, z.ctrl_mode, z.data[..5].try_into().unwrap()))
                .collect();
            log.lock()
                .unwrap()
                .push((set, p.version, p.flags, p.count, zones));
            0
        })
    }

    const FE_3080: [u16; 4] = [0x10DE, 0x2206, 0x10DE, 0x1467];
    const PALIT_3060: [u16; 4] = [0x10DE, 0x2504, 0x10DE, 0x2504];

    // NV_GPU_CLIENT_ILLUM_ZONE_CONTROL_PARAMS_V1 and NV_STRUCT_VERSION as NvAPISpy logs it
    #[test]
    fn params_layout() {
        assert_eq!(size_of::<Zone>(), 200);
        assert_eq!(size_of::<Params>(), 6476);
        assert_eq!(VERSION, 72012);
    }

    // DetectNVIDIAIllumGPUs: the four ids pick the row, getInfo counts the zones.
    #[test]
    fn detect() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let b = found(
            0,
            FE_3080,
            card(&[ZONE_RGBW, ZONE_SINGLE_COLOR], log.clone()),
        )
        .unwrap();
        let d = &b.devices()[0];
        assert_eq!(
            (d.id.as_str(), d.leds.len()),
            ("gpu:nvidia-illumination:0", 2)
        );
        assert_eq!(d.name, "NVIDIA GeForce RTX 3080 FE");
        assert_eq!(log.lock().unwrap()[0].1, 72012);
        assert!(found(0, [0x10DE, 0x2206, 0x10DE, 0x0000], card(&[1], log.clone())).is_none());
        assert!(found(0, FE_3080, card(&[], log)).is_none());
        assert!(known(PALIT_3060) && !known([0x10DE, 0x2206, 0, 0]));
        assert_eq!(DEVICES.len(), 39);
    }

    // setZone: Get, the zone in MANUAL_RGB with its colour, Set; near-grey RGBW turns white.
    #[test]
    fn direct_rgbw() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut b = found(
            0,
            FE_3080,
            card(&[ZONE_RGBW, ZONE_SINGLE_COLOR], log.clone()),
        )
        .unwrap();
        log.lock().unwrap().clear();
        let id = b.devices()[0].id.clone();
        b.push(&id, &[Color(200, 205, 210), Color(1, 0, 0)])
            .unwrap();
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 4);
        let other = (ZONE_SINGLE_COLOR, 0, [9; 5]);
        assert_eq!(
            log[0],
            (false, 72012, 0, 2, vec![(ZONE_RGBW, 0, [9; 5]), other])
        );
        assert_eq!(
            log[1],
            (
                true,
                72012,
                0,
                2,
                vec![(ZONE_RGBW, 0, [0, 0, 0, 205, 100]), other]
            )
        );
        let first = (ZONE_RGBW, 0, [9; 5]);
        assert_eq!(
            log[3],
            (
                true,
                72012,
                0,
                2,
                vec![first, (ZONE_SINGLE_COLOR, 0, [100, 9, 9, 9, 9])]
            )
        );
    }

    // RGBW-as-RGB cards keep the colour, RGB zones take it, and only changed zones are sent.
    #[test]
    fn direct_rgb() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut b = found(1, PALIT_3060, card(&[ZONE_RGBW, ZONE_RGB], log.clone())).unwrap();
        let id = b.devices()[0].id.clone();
        b.push(&id, &[Color(200, 205, 210), Color(1, 2, 3)])
            .unwrap();
        log.lock().unwrap().clear();
        b.push(&id, &[Color(200, 205, 210), Color(0, 0, 0)])
            .unwrap();
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].4[1], (ZONE_RGB, 0, [0, 0, 0, 100, 9]));
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut b = found(1, PALIT_3060, card(&[ZONE_RGBW], log.clone())).unwrap();
        b.push(&id, &[Color(200, 205, 210)]).unwrap();
        assert_eq!(
            log.lock().unwrap()[2].4[0],
            (ZONE_RGBW, 0, [200, 205, 210, 0, 100])
        );
    }
}
