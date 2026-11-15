//! Ported from OpenRGB Controllers/GigabyteRGBFusionGPUController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_smbus::i2c::{Result, SharedBus, SmBus, lock};

use super::{Entry, RECT, matching};

const REG_COLOR: u8 = 0x40;
const REG_MODE: u8 = 0x88;
const MODE_STATIC: u8 = 0x01;
const BRIGHTNESS_MAX: u8 = 0x63;
/// The RTX 3060 controller wants every command padded with four zero bytes.
const PADDED: u8 = 0x62;

/// One four-byte command, sent a byte at a time.
fn command(bus: &dyn SmBus, addr: u8, bytes: [u8; 4]) -> Result<()> {
    for b in bytes {
        bus.write_byte(addr, b)?;
    }
    if addr == PADDED {
        for _ in 0..4 {
            bus.write_byte(addr, 0x00)?;
        }
    }
    Ok(())
}

fn test_for(bus: &dyn SmBus, addr: u8) -> bool {
    if bus.write_byte(addr, 0xAB).is_err() {
        return false;
    }
    let padding = if addr == PADDED { 7 } else { 3 };
    for _ in 0..padding {
        let _ = bus.write_byte(addr, 0x00);
    }
    let mut pass = bus.read_byte(addr) == Ok(0xAB);
    if !matches!(bus.read_byte(addr), Ok(0x10 | 0x11 | 0x12 | 0x14)) {
        pass = false;
    }
    let _ = bus.read_byte(addr);
    let _ = bus.read_byte(addr);
    pass || addr == 0x48
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    let info = lock(bus).info().clone();
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for &(name, .., addr) in matching(&info, DEVICES) {
        if !test_for(lock(bus).as_ref(), addr) {
            continue;
        }
        let id = format!("gpu:gigabyte-fusion:{}:{addr:02x}", info.name);
        found.push(Box::new(FusionGpu {
            bus: bus.clone(),
            addr,
            device: Device::grid(&id, name, DeviceKind::Gpu, RECT, 1, 1),
            direct: false,
            last: None,
        }));
    }
    found
}

struct FusionGpu {
    bus: SharedBus,
    addr: u8,
    device: Device,
    direct: bool,
    last: Option<Color>,
}

impl Backend for FusionGpu {
    fn name(&self) -> &str {
        "Gigabyte RGB Fusion GPU"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> std::result::Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let &color = colors.first().ok_or("no colour")?;
        let bus = lock(&self.bus);
        // DeviceUpdateMode for Direct: static, speed 0, full brightness.
        if !self.direct {
            command(
                bus.as_ref(),
                self.addr,
                [REG_MODE, MODE_STATIC, 0x00, BRIGHTNESS_MAX],
            )?;
            self.direct = true;
        } else if self.last == Some(color) {
            return Ok(());
        }
        let Color(r, g, b) = color;
        command(bus.as_ref(), self.addr, [REG_COLOR, r, g, b])?;
        self.last = Some(color);
        Ok(())
    }
}

#[rustfmt::skip]
const DEVICES: &[Entry] = &[
    ("Gigabyte GTX 1050 G1 Gaming", 0x10DE, 0x1C81, 0x1458, 0x372B, 0x47),
    ("Gigabyte GeForce GTX 1050 Ti G1 Gaming Rev A1", 0x10DE, 0x1C82, 0x1458, 0x372A, 0x47),
    ("Gigabyte GeForce GTX 1050 Ti G1 Gaming", 0x10DE, 0x1C82, 0x1458, 0x372A, 0x48),
    ("Gigabyte GeForce GTX 1060 G1 Gaming", 0x10DE, 0x1C03, 0x1458, 0x3716, 0x48),
    ("Gigabyte GeForce GTX 1060 G1 Gaming OC", 0x10DE, 0x1C03, 0x1458, 0x3739, 0x47),
    ("Gigabyte GeForce GTX 1060 Xtreme Gaming V1", 0x10DE, 0x1C03, 0x1458, 0x3757, 0x47),
    ("Gigabyte GeForce GTX 1060 Xtreme Gaming V1", 0x10DE, 0x1C03, 0x1458, 0x3758, 0x47),
    ("Gigabyte GeForce GTX 1060 Xtreme Gaming V2", 0x10DE, 0x1C03, 0x1458, 0x3776, 0x47),
    ("Gigabyte GeForce GTX 1060 Xtreme Gaming V2", 0x10DE, 0x1C03, 0x1458, 0x3777, 0x47),
    ("Gigabyte GeForce GTX 1070 Xtreme Gaming", 0x10DE, 0x1B81, 0x1458, 0x3778, 0x47),
    ("Gigabyte GeForce GTX 1070 Xtreme Gaming", 0x10DE, 0x1B81, 0x1458, 0x3779, 0x47),
    ("Gigabyte GeForce GTX 1070 G1 Gaming V1", 0x10DE, 0x1B81, 0x1458, 0x3772, 0x47),
    ("Gigabyte GeForce GTX 1070 Ti Gaming", 0x10DE, 0x1B82, 0x1458, 0x3794, 0x47),
    ("Gigabyte GeForce GTX 1080 G1 Gaming", 0x10DE, 0x1B80, 0x1458, 0x3702, 0x48),
    ("Gigabyte GeForce GTX 1080 Ti Gaming", 0x10DE, 0x1B06, 0x1458, 0x3752, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Gaming OC", 0x10DE, 0x1B06, 0x1458, 0x374C, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Gaming OC BLACK", 0x10DE, 0x1B06, 0x1458, 0x377A, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Xtreme Edition", 0x10DE, 0x1B06, 0x1458, 0x374E, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Xtreme Edition", 0x10DE, 0x1B06, 0x1458, 0x3751, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Xtreme Waterforce Edition", 0x10DE, 0x1B06, 0x1458, 0x3761, 0x47),
    ("Gigabyte GeForce GTX 1080 Ti Xtreme Waterforce Edition", 0x10DE, 0x1B06, 0x1458, 0x3762, 0x47),
    ("Gigabyte GeForce GTX 1650 Gaming OC", 0x10DE, 0x1F82, 0x1458, 0x3FE4, 0x55),
    ("Gigabyte GeForce GTX 1660 Gaming OC 6G", 0x10DE, 0x2184, 0x1458, 0x3FC7, 0x47),
    ("Gigabyte GeForce GTX 1660 SUPER Gaming OC", 0x10DE, 0x21C4, 0x1458, 0x4014, 0x47),
    ("Gigabyte GeForce GTX 1660 Ti Gaming OC", 0x10DE, 0x2182, 0x1458, 0x3FC5, 0x47),
    ("Gigabyte GeForce RTX 2060 Gaming OC", 0x10DE, 0x1F08, 0x1458, 0x37CE, 0x47),
    ("Gigabyte GeForce RTX 2060 Gaming OC PRO", 0x10DE, 0x1F08, 0x1458, 0x3FC2, 0x47),
    ("Gigabyte GeForce RTX 2060 Gaming OC PRO V2", 0x10DE, 0x1E89, 0x1458, 0x3FC9, 0x47),
    ("Gigabyte GeForce RTX 2060 Gaming OC PRO V3", 0x10DE, 0x1F08, 0x1458, 0x3FC9, 0x47),
    ("Gigabyte GeForce RTX 2060 Gaming OC PRO White", 0x10DE, 0x1F08, 0x1458, 0x3FD0, 0x47),
    ("Gigabyte GeForce RTX 2060 SUPER Gaming", 0x10DE, 0x1F06, 0x1458, 0x404A, 0x47),
    ("Gigabyte GeForce RTX 2060 SUPER Gaming OC", 0x10DE, 0x1F06, 0x1458, 0x3FED, 0x47),
    ("Gigabyte GeForce RTX 2060 SUPER Gaming OC 3X White", 0x10DE, 0x1F06, 0x1458, 0x3FFE, 0x47),
    ("Gigabyte GeForce RTX 2060 SUPER Gaming OC 3X V2", 0x10DE, 0x1F06, 0x1458, 0x4009, 0x47),
    ("Gigabyte GeForce RTX 2070 Gaming OC", 0x10DE, 0x1F02, 0x1458, 0x37AD, 0x47),
    ("Gigabyte GeForce RTX 2070 Gaming OC", 0x10DE, 0x1F07, 0x1458, 0x37AD, 0x47),
    ("Gigabyte GeForce RTX 2070 Gaming OC 8GC", 0x10DE, 0x1F02, 0x1458, 0x37C8, 0x47),
    ("Gigabyte GeForce RTX 2070 Windforce", 0x10DE, 0x1F02, 0x1458, 0x37C2, 0x47),
    ("Gigabyte GeForce RTX 2070 SUPER Gaming OC", 0x10DE, 0x1E84, 0x1458, 0x3FEB, 0x47),
    ("Gigabyte GeForce RTX 2070 SUPER Gaming OC 3X", 0x10DE, 0x1E84, 0x1458, 0x4008, 0x47),
    ("Gigabyte GeForce RTX 2070 SUPER Gaming OC 3X", 0x10DE, 0x1E84, 0x1458, 0x4008, 0x55),
    ("Gigabyte GeForce RTX 2070 SUPER Gaming OC 3X White", 0x10DE, 0x1E84, 0x1458, 0x400D, 0x47),
    ("Gigabyte GeForce RTX 2080 Gaming OC", 0x10DE, 0x1E82, 0x1458, 0x37D6, 0x47),
    ("Gigabyte GeForce RTX 2080 Gaming OC", 0x10DE, 0x1E87, 0x1458, 0x37A7, 0x47),
    ("Gigabyte GeForce RTX 2080 Ti GAMING OC", 0x10DE, 0x1E07, 0x1458, 0x37A9, 0x47),
    ("Gigabyte GeForce RTX 2080 SUPER Gaming OC", 0x10DE, 0x1E81, 0x1458, 0x3FE9, 0x47),
    ("Gigabyte GeForce RTX 3050 Gaming OC", 0x10DE, 0x2507, 0x1458, 0x40AA, 0x62),
    ("Gigabyte GeForce RTX 3060 EAGLE OC", 0x10DE, 0x2503, 0x1458, 0x4072, 0x63),
    ("Gigabyte GeForce RTX 3060 EAGLE OC", 0x10DE, 0x2487, 0x1458, 0x4072, 0x63),
    ("Gigabyte GeForce RTX 3060 EAGLE OC V2", 0x10DE, 0x2504, 0x1458, 0x4072, 0x63),
    ("Gigabyte GeForce RTX 3060 EAGLE LHR V2", 0x10DE, 0x2504, 0x1458, 0x4071, 0x63),
    ("Gigabyte GeForce RTX 3060 Vision OC", 0x10DE, 0x2503, 0x1458, 0x4073, 0x63),
    ("Gigabyte GeForce RTX 3060 Vision OC LHR", 0x10DE, 0x2504, 0x1458, 0x4073, 0x63),
    ("Gigabyte GeForce RTX 3060 Vision OC V3", 0x10DE, 0x2487, 0x1458, 0x4073, 0x63),
    ("Gigabyte GeForce RTX 3060 Gaming OC", 0x10DE, 0x2503, 0x1458, 0x4074, 0x62),
    ("Gigabyte GeForce RTX 3060 Gaming OC V2", 0x10DE, 0x2487, 0x1458, 0x4074, 0x62),
    ("Gigabyte GeForce RTX 3060 Gaming OC", 0x10DE, 0x2504, 0x1458, 0x4074, 0x62),
    ("Gigabyte GeForce RTX 3060 Ti Gaming OC LHR V2", 0x10DE, 0x2489, 0x1458, 0x405A, 0x32),
    ("Gigabyte GeForce RTX 3060 Ti EAGLE OC", 0x10DE, 0x2486, 0x1458, 0x405B, 0x63),
    ("Gigabyte GeForce RTX 3060 Ti EAGLE OC LHR V2", 0x10DE, 0x2489, 0x1458, 0x405B, 0x63),
    ("Gigabyte GeForce RTX 3060 Ti EAGLE OC LHR V2", 0x10DE, 0x2489, 0x1458, 0x4060, 0x63),
    ("Gigabyte GeForce RTX 3060 Ti Vision OC", 0x10DE, 0x2489, 0x1458, 0x4077, 0x63),
    ("Gigabyte GeForce RTX 3070 Gaming OC", 0x10DE, 0x2484, 0x1458, 0x404C, 0x62),
    ("Gigabyte GeForce RTX 3070 Gaming OC LHR V3", 0x10DE, 0x2488, 0x1458, 0x404C, 0x62),
    ("Gigabyte GeForce RTX 3070 Vision", 0x10DE, 0x2484, 0x1458, 0x404D, 0x63),
    ("Gigabyte GeForce RTX 3070 Vision LHR V2", 0x10DE, 0x2488, 0x1458, 0x404D, 0x63),
    ("Gigabyte GeForce RTX 3070 Eagle OC", 0x10DE, 0x2484, 0x1458, 0x404E, 0x63),
    ("Gigabyte GeForce RTX 3070 Eagle OC LHR V2", 0x10DE, 0x2488, 0x1458, 0x404E, 0x63),
    ("Gigabyte GeForce RTX 3070 Ti Gaming OC", 0x10DE, 0x2482, 0x1458, 0x408F, 0x62),
    ("Gigabyte GeForce RTX 3070 Ti EAGLE", 0x10DE, 0x2482, 0x1458, 0x408C, 0x63),
    ("Gigabyte GeForce RTX 3070 Ti Vision OC", 0x10DE, 0x2482, 0x1458, 0x4090, 0x63),
    ("Gigabyte GeForce RTX 3080 Gaming OC", 0x10DE, 0x2206, 0x1458, 0x403F, 0x62),
    ("Gigabyte GeForce RTX 3080 Vision OC", 0x10DE, 0x2206, 0x1458, 0x404B, 0x63),
    ("Gigabyte GeForce RTX 3080 EAGLE OC", 0x10DE, 0x2206, 0x1458, 0x4040, 0x63),
    ("Gigabyte GeForce RTX 3080 Gaming OC LHR", 0x10DE, 0x2216, 0x1458, 0x403F, 0x62),
    ("Gigabyte GeForce RTX 3080 Vision OC LHR V2", 0x10DE, 0x2216, 0x1458, 0x404B, 0x63),
    ("Gigabyte GeForce RTX 3080 12G Gaming OC", 0x10DE, 0x220A, 0x1458, 0x40A2, 0x62),
    ("Gigabyte GeForce RTX 3080 Ti Gaming OC", 0x10DE, 0x2208, 0x1458, 0x4088, 0x62),
    ("Gigabyte GeForce RTX 3080 Ti EAGLE", 0x10DE, 0x2208, 0x1458, 0x4085, 0x63),
    ("Gigabyte GeForce RTX 3080 Ti EAGLE OC", 0x10DE, 0x2208, 0x1458, 0x4086, 0x63),
    ("Gigabyte GeForce RTX 3090 Gaming OC", 0x10DE, 0x2204, 0x1458, 0x4043, 0x62),
    ("Gigabyte GeForce RTX 4060 Gaming OC", 0x10DE, 0x2882, 0x1458, 0x4109, 0x55),
    ("Gigabyte GeForce RTX 4060 Ti Gaming OC", 0x10DE, 0x2803, 0x1458, 0x40F8, 0x71),
    ("Gigabyte GeForce RTX 4060 Ti Gaming OC", 0x10DE, 0x2805, 0x1458, 0x4112, 0x71),
    ("Gigabyte GeForce RTX 4060 Ti AERO OC", 0x10DE, 0x2803, 0x1458, 0x4113, 0x71),
    ("Gigabyte GeForce RTX 4070 Ti SUPER AERO OC", 0x10DE, 0x2705, 0x1458, 0x413F, 0x71),
    ("Gigabyte GeForce RTX 4070 Ti SUPER EAGLE OC", 0x10DE, 0x2705, 0x1458, 0x413E, 0x71),
];

#[cfg(test)]
mod tests {
    use super::super::fake::Script;
    use super::*;
    use foxrgb_smbus::i2c::BusInfo;

    fn info(device: u16, sub_device: u16) -> BusInfo {
        BusInfo {
            name: "Nvidia NvAPI I2C on GPU 0".into(),
            pci_vendor: 0x10DE,
            pci_device: device,
            pci_subsystem_vendor: 0x1458,
            pci_subsystem_device: sub_device,
        }
    }

    fn card(info: BusInfo, answers: &[u8]) -> Script {
        let s = Script::new(info);
        s.stream.lock().unwrap().extend(answers);
        s
    }

    /// The first entry of the table at `addr`.
    fn entry(addr: u8) -> &'static Entry {
        DEVICES.iter().find(|e| e.5 == addr).unwrap()
    }

    fn info_for(e: &Entry) -> BusInfo {
        BusInfo {
            name: "Nvidia NvAPI I2C on GPU 0".into(),
            pci_vendor: e.1,
            pci_device: e.2,
            pci_subsystem_vendor: e.3,
            pci_subsystem_device: e.4,
        }
    }

    // GigabyteRGBFusionGPUControllerDetect.cpp has 87 REGISTER_I2C_PCI_DETECTOR lines
    #[test]
    fn table() {
        assert_eq!(DEVICES.len(), 87);
        assert_eq!(
            DEVICES[0],
            (
                "Gigabyte GTX 1050 G1 Gaming",
                0x10DE,
                0x1C81,
                0x1458,
                0x372B,
                0x47
            )
        );
        assert!(DEVICES.iter().all(|e| e.1 == 0x10DE));
    }

    // TestForGigabyteRGBFusionGPUController at an ordinary address
    #[test]
    fn detection() {
        let (bus, log) = card(info(0x1C81, 0x372B), &[0xAB, 0x14, 0x00, 0x00]).share();
        let found = detect(&bus);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].devices()[0].name, "Gigabyte GTX 1050 G1 Gaming");
        assert_eq!(
            *log.lock().unwrap(),
            [
                "byte 47 ab",
                "byte 47 00",
                "byte 47 00",
                "byte 47 00",
                "read 47",
                "read 47",
                "read 47",
                "read 47",
            ]
        );
    }

    // TestForGigabyteRGBFusionGPUController: bad answers, and a failed first write
    #[test]
    fn detection_rejects() {
        let (bus, log) = card(info(0x1C81, 0x372B), &[0xAB, 0x13, 0, 0]).share();
        assert!(detect(&bus).is_empty());
        assert_eq!(log.lock().unwrap().len(), 8);
        let (bus, _) = card(info(0x1C81, 0x372B), &[0xAA, 0x14, 0, 0]).share();
        assert!(detect(&bus).is_empty());
        let mut s = card(info(0x1C81, 0x372B), &[0xAB, 0x14, 0, 0]);
        s.fail_writes = true;
        let (bus, log) = s.share();
        assert!(detect(&bus).is_empty());
        assert_eq!(*log.lock().unwrap(), ["byte 47 ab"]);
    }

    // TestForGigabyteRGBFusionGPUController at 0x62: four extra zero bytes
    #[test]
    fn detection_padded() {
        let (bus, log) = card(info_for(entry(0x62)), &[0xAB, 0x10, 0, 0]).share();
        assert_eq!(detect(&bus).len(), 1);
        let log = log.lock().unwrap();
        assert_eq!(
            log[..8],
            ["byte 62 ab"]
                .into_iter()
                .chain(["byte 62 00"; 7])
                .collect::<Vec<_>>()
        );
        assert_eq!(log[8..], ["read 62"; 4]);
    }

    // TestForGigabyteRGBFusionGPUController at 0x48: passes whatever it reads
    #[test]
    fn detection_48() {
        let (bus, log) = Script::new(info_for(entry(0x48))).share();
        let found = detect(&bus);
        assert_eq!(found.len(), 1);
        assert!(found[0].devices()[0].id.ends_with(":48"));
        // The Rev A1 line shares these ids at 0x47 and is probed first, as OpenRGB runs every match.
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 16);
        assert_eq!(
            (log[0].as_str(), log[8].as_str()),
            ("byte 47 ab", "byte 48 ab")
        );
    }

    // RGBFusionGPUController::SetMode for Direct, then SetColor
    #[test]
    fn direct_frames() {
        let (bus, log) = card(info(0x1C81, 0x372B), &[0xAB, 0x14, 0, 0]).share();
        let mut found = detect(&bus);
        let id = found[0].devices()[0].id.clone();
        log.lock().unwrap().clear();
        found[0].push(&id, &[Color(1, 2, 3)]).unwrap();
        found[0].push(&id, &[Color(1, 2, 3)]).unwrap();
        found[0].push(&id, &[Color(4, 5, 6)]).unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            [
                "byte 47 88",
                "byte 47 01",
                "byte 47 00",
                "byte 47 63",
                "byte 47 40",
                "byte 47 01",
                "byte 47 02",
                "byte 47 03",
                "byte 47 40",
                "byte 47 04",
                "byte 47 05",
                "byte 47 06",
            ]
        );
    }

    // RGBFusionGPUController::SetMode and SetColor at 0x62, each padded
    #[test]
    fn direct_frames_padded() {
        let (bus, log) = card(info_for(entry(0x62)), &[0xAB, 0x10, 0, 0]).share();
        let mut found = detect(&bus);
        let id = found[0].devices()[0].id.clone();
        log.lock().unwrap().clear();
        found[0].push(&id, &[Color(0xFF, 0, 0x80)]).unwrap();
        let zeros = ["byte 62 00"; 4];
        let mut expected = vec!["byte 62 88", "byte 62 01", "byte 62 00", "byte 62 63"];
        expected.extend(zeros);
        expected.extend(["byte 62 40", "byte 62 ff", "byte 62 00", "byte 62 80"]);
        expected.extend(zeros);
        assert_eq!(*log.lock().unwrap(), expected);
    }
}
