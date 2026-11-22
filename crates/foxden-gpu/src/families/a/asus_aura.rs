// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_smbus::i2c::{SharedBus, SmBus, lock};

use super::{Entry, RECT, matching};

const AMD_GPU_VEN: u16 = 0x1002;
/// Bus names an AMD card may sit on.
const AMD_BUS_NAMES: [&str; 3] = [
    "AMD ADL",
    "AMDGPU DM i2c OEM bus",
    "AMDGPU i2c bit bus OEM 0x97",
];
const MAGIC: u16 = 0x1589;
const REG_RED: u8 = 0x04;
const REG_GREEN: u8 = 0x05;
const REG_BLUE: u8 = 0x06;
const REG_MODE: u8 = 0x07;
const MODE_STATIC: u8 = 1;

/// A failed read is the s32 -1 truncated to a byte.
fn read(bus: &dyn SmBus, addr: u8, reg: u8) -> u8 {
    bus.read_byte_data(addr, reg).unwrap_or(0xFF)
}

fn test_for(bus: &dyn SmBus, addr: u8) -> bool {
    let info = bus.info();
    if info.pci_vendor == AMD_GPU_VEN && !AMD_BUS_NAMES.iter().any(|n| info.name.starts_with(n)) {
        return false;
    }
    let high = read(bus, addr, 0x20);
    let low = read(bus, addr, 0x21);
    ((high as u16) << 8) + low as u16 == MAGIC
}

pub fn detect(bus: &SharedBus) -> Vec<Box<dyn Backend>> {
    let info = lock(bus).info().clone();
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    for &(name, .., addr) in matching(&info, DEVICES) {
        let guard = lock(bus);
        if !test_for(guard.as_ref(), addr) {
            continue;
        }
        // Setup reads the colour, then the mode.
        for reg in [REG_RED, REG_GREEN, REG_BLUE, REG_MODE] {
            read(guard.as_ref(), addr, reg);
        }
        drop(guard);
        let id = format!("gpu:asus-aura:{}:{addr:02x}", info.name);
        found.push(Box::new(AuraGpu {
            bus: bus.clone(),
            addr,
            device: Device::grid(&id, name, DeviceKind::Gpu, RECT, 1, 1),
            direct: false,
            last: None,
        }));
    }
    found
}

struct AuraGpu {
    bus: SharedBus,
    addr: u8,
    device: Device,
    direct: bool,
    last: Option<Color>,
}

impl Backend for AuraGpu {
    fn name(&self) -> &str {
        "ASUS Aura GPU"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let &color = colors.first().ok_or("no colour")?;
        let bus = lock(&self.bus);
        // Direct mode: Static, without the apply register.
        if !self.direct {
            bus.write_byte_data(self.addr, REG_MODE, MODE_STATIC)?;
            self.direct = true;
        } else if self.last == Some(color) {
            return Ok(());
        }
        let Color(r, g, b) = color;
        bus.write_byte_data(self.addr, REG_RED, r)?;
        bus.write_byte_data(self.addr, REG_GREEN, g)?;
        bus.write_byte_data(self.addr, REG_BLUE, b)?;
        self.last = Some(color);
        Ok(())
    }
}

#[rustfmt::skip]
pub(super) const DEVICES: &[Entry] = &[
    ("ASUS ROG STRIX GeForce GTX 1050 Gaming OC", 0x10DE, 0x1C81, 0x1043, 0x85D8, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1050 Ti Gaming", 0x10DE, 0x1C82, 0x1043, 0x8611, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1050 Ti Gaming OC", 0x10DE, 0x1C82, 0x1043, 0x85CD, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1050 Ti Gaming OC", 0x10DE, 0x1C82, 0x1043, 0x85D1, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1060 Gaming", 0x10DE, 0x1C03, 0x1043, 0x85A4, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1060 Gaming", 0x10DE, 0x1C03, 0x1043, 0x85AC, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1060 Gaming", 0x10DE, 0x1C03, 0x1043, 0x865B, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1070 Gaming", 0x10DE, 0x1B81, 0x1043, 0x8598, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1070 Gaming OC", 0x10DE, 0x1B81, 0x1043, 0x8599, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1070 OC", 0x10DE, 0x1B81, 0x1043, 0x85A2, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1070 Ti Gaming", 0x10DE, 0x1B82, 0x1043, 0x861D, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1070 A8G Ti Gaming", 0x10DE, 0x1B82, 0x1043, 0x861E, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Gaming OC", 0x10DE, 0x1B80, 0x1043, 0x8592, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 A8G Gaming", 0x10DE, 0x1B80, 0x1043, 0x85AA, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Gaming OC", 0x10DE, 0x1B80, 0x1043, 0x85F9, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Gaming OC 11Gbps", 0x10DE, 0x1B80, 0x1043, 0x85E8, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Ti Gaming", 0x10DE, 0x1B06, 0x1043, 0x85EB, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Ti Gaming", 0x10DE, 0x1B06, 0x1043, 0x85F1, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Ti Gaming OC", 0x10DE, 0x1B06, 0x1043, 0x85EA, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1080 Ti Gaming OC", 0x10DE, 0x1B06, 0x1043, 0x85E4, 0x29),
    ("ASUS ROG Poseidon GeForce GTX 1080 Ti", 0x10DE, 0x1B06, 0x1043, 0x85EC, 0x29),
    ("ASUS ROG STRIX GeForce GTX 1650 SUPER A4G Gaming", 0x10DE, 0x2187, 0x1043, 0x8750, 0x2A),
    ("ASUS ROG STRIX GeForce GTX 1650 SUPER Gaming OC", 0x10DE, 0x2187, 0x1043, 0x874F, 0x2A),
    ("ASUS ROG STRIX GeForce GTX 1660 SUPER Gaming OC", 0x10DE, 0x21C4, 0x1043, 0x8752, 0x2A),
    ("ASUS ROG STRIX GeForce GTX 1660 SUPER Gaming", 0x10DE, 0x21C4, 0x1043, 0x8754, 0x2A),
    ("ASUS ROG STRIX GeForce GTX 1660 Ti Gaming OC", 0x10DE, 0x2182, 0x1043, 0x86A5, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 Gaming", 0x10DE, 0x1F08, 0x1043, 0x8690, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 Gaming OC", 0x10DE, 0x1F08, 0x1043, 0x868E, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 Gaming OC", 0x10DE, 0x1F08, 0x1043, 0x86D2, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 EVO Gaming", 0x10DE, 0x1F08, 0x1043, 0x86D3, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 EVO V2 Gaming", 0x10DE, 0x1F08, 0x1043, 0x880B, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 EVO Gaming OC", 0x10DE, 0x1E89, 0x1043, 0x8775, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER Gaming", 0x10DE, 0x1F06, 0x1043, 0x8730, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER A8G EVO Gaming", 0x10DE, 0x1F47, 0x1043, 0x8703, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER Gaming OC", 0x10DE, 0x1F06, 0x1043, 0x86FB, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER Gaming", 0x10DE, 0x1F47, 0x1043, 0x8702, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER A8G Gaming", 0x10DE, 0x1F06, 0x1043, 0x86FC, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2060 SUPER A8G Gaming", 0x10DE, 0x1F06, 0x1043, 0x86FD, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 A8G Gaming", 0x10DE, 0x1F07, 0x1043, 0x8671, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 Gaming OC", 0x10DE, 0x1F07, 0x1043, 0x8670, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER A8G Gaming", 0x10DE, 0x1EC7, 0x1043, 0x86FF, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER A8G Gaming", 0x10DE, 0x1E84, 0x1043, 0x8728, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER A8G Gaming", 0x10DE, 0x1E84, 0x1043, 0x8706, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER Gaming OC", 0x10DE, 0x1E84, 0x1043, 0x8729, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER Gaming", 0x10DE, 0x1E84, 0x1043, 0x8707, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2070 SUPER Gaming OC", 0x10DE, 0x1E84, 0x1043, 0x8727, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Gaming", 0x10DE, 0x1E82, 0x1043, 0x867F, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Gaming OC", 0x10DE, 0x1E87, 0x1043, 0x865F, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 V2 Gaming OC", 0x10DE, 0x1E87, 0x1043, 0x8661, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 SUPER A8G Gaming", 0x10DE, 0x1E81, 0x1043, 0x8712, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 SUPER Gaming OC", 0x10DE, 0x1E81, 0x1043, 0x8711, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 SUPER White OC", 0x10DE, 0x1E81, 0x1043, 0x876B, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Ti Gaming", 0x10DE, 0x1E04, 0x1043, 0x8687, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Ti Gaming", 0x10DE, 0x1E07, 0x1043, 0x866C, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Ti A11G Gaming", 0x10DE, 0x1E07, 0x1043, 0x866B, 0x2A),
    ("ASUS ROG STRIX GeForce RTX 2080 Ti Gaming OC", 0x10DE, 0x1E07, 0x1043, 0x866A, 0x2A),
    ("ASUS TUF GeForce RTX 3060 Ti Gaming OC", 0x10DE, 0x2489, 0x1043, 0x87C6, 0x2A),
    ("ASUS AREZ STRIX Radeon RX Vega 56 Gaming OC", 0x1002, 0x687F, 0x1043, 0x0555, 0x29),
    ("ASUS ROG STRIX Radeon Vega 64", 0x1002, 0x687F, 0x1043, 0x04C4, 0x29),
    ("ASUS ROG STRIX Radeon RX 470 Gaming OC", 0x1002, 0x67DF, 0x1043, 0x04B0, 0x29),
    ("ASUS ROG STRIX Radeon RX 480 Gaming", 0x1002, 0x67DF, 0x1043, 0x04FD, 0x29),
    ("ASUS ROG STRIX Radeon RX 480 Gaming OC", 0x1002, 0x67DF, 0x1043, 0x04FB, 0x29),
    ("ASUS ROG STRIX Radeon RX 560 Gaming", 0x1002, 0x67FF, 0x1043, 0x04BC, 0x29),
    ("ASUS ROG STRIX Radeon RX 560 Gaming", 0x1002, 0x67FF, 0x1043, 0x04BE, 0x29),
    ("ASUS ROG STRIX Radeon RX 570 Gaming OC", 0x1002, 0x67DF, 0x1043, 0x04C2, 0x29),
    ("ASUS ROG STRIX Radeon RX 570 Gaming OC", 0x1002, 0x67DF, 0x1043, 0x0588, 0x29),
    ("ASUS ROG STRIX Radeon RX 580 Gaming OC", 0x1002, 0x67DF, 0x1043, 0x0517, 0x29),
    ("ASUS ROG STRIX Radeon RX 580 Gaming TOP", 0x1002, 0x67DF, 0x1043, 0x0519, 0x29),
    ("ASUS ROG STRIX Radeon RX 590 Gaming", 0x1002, 0x67DF, 0x1043, 0x056A, 0x29),
    ("ASUS ROG STRIX Radeon RX 5600 XT Gaming OC", 0x1002, 0x731F, 0x1043, 0x04EC, 0x2A),
    ("ASUS ROG STRIX Radeon RX 5700 Gaming OC", 0x1002, 0x731F, 0x1043, 0x04E4, 0x2A),
    ("ASUS ROG STRIX Radeon RX 5700 XT Gaming OC", 0x1002, 0x731F, 0x1043, 0x04E2, 0x2A),
    ("ASUS ROG STRIX Radeon RX 5700 XT Gaming OC", 0x1002, 0x731F, 0x1043, 0x05C1, 0x2A),
    ("ASUS ROG STRIX Radeon RX 5700 XT Gaming OC", 0x1002, 0x731F, 0x1043, 0x05C3, 0x2A),
    ("ASUS GTX 1050Ti Strix Gaming OC", 0x10DE, 0x1C82, 0x1043, 0x864B, 0x29),
    ("ASUS GTX 1060 Strix Gaming", 0x10DE, 0x1C03, 0x1043, 0x854A, 0x29),
    ("ASUS GTX 1080 Strix Gaming OC", 0x10DE, 0x1B80, 0x1043, 0x8593, 0x29),
    ("ASUS ROG Strix GTX 1650 Super Gaming OC", 0x10DE, 0x2187, 0x1043, 0x8751, 0x2A),
    ("ASUS ROG Strix GTX 1660 Super Gaming Advanced", 0x10DE, 0x21C4, 0x1043, 0x8753, 0x2A),
    ("ASUS ROG Strix GTX 1660TI OC", 0x10DE, 0x2182, 0x1043, 0x86A6, 0x2A),
    ("ASUS ROG Strix GTX 1660Ti Strix Gaming", 0x10DE, 0x2182, 0x1043, 0x86A7, 0x2A),
    ("ASUS ROG Strix RTX 2060 Gaming", 0x10DE, 0x1F08, 0x1043, 0x86D1, 0x2A),
    ("ASUS ROG Strix RTX 2060 Gaming 6gb", 0x10DE, 0x1F08, 0x1043, 0x868F, 0x2A),
    ("ASUS ROG Strix RTX 2060 Super EVO Gaming OC", 0x10DE, 0x1F47, 0x1043, 0x8730, 0x2A),
    ("ASUS ROG Strix RTX 2060 Super Gaming", 0x10DE, 0x1F06, 0x1043, 0x872F, 0x2A),
    ("ASUS ROG Strix RTX 2060 Super Gaming OC", 0x10DE, 0x1F06, 0x1043, 0x879F, 0x2A),
    ("ASUS ROG Strix RTX 2060 Super EVO Gaming", 0x10DE, 0x1F06, 0x1043, 0x87A0, 0x2A),
    ("ASUS ROG Strix RTX 2060 OC", 0x10DE, 0x1F08, 0x1043, 0x86F5, 0x2A),
    ("ASUS ROG Strix RTX 2070 Gaming", 0x10DE, 0x1F02, 0x1043, 0x867E, 0x2A),
    ("ASUS ROG Strix RTX 2070 Gaming", 0x10DE, 0x1F02, 0x1043, 0x8796, 0x2A),
    ("ASUS ROG Strix RTX 2070 Super 8GB", 0x10DE, 0x1EC7, 0x1043, 0x8701, 0x2A),
    ("ASUS ROG Strix RTX 2080 Gaming Advanced", 0x10DE, 0x1E87, 0x1043, 0x8660, 0x2A),
    ("ASUS ROG Strix RTX 2060 Evo V2", 0x10DE, 0x1E89, 0x1043, 0x8776, 0x2A),
    ("ASUS ROG Strix RTX 2080Ti Call of Duty BO4", 0x10DE, 0x1E07, 0x1043, 0x8689, 0x2A),
    ("ASUS ROG Strix RTX 2060 Evo", 0x10DE, 0x1F08, 0x1043, 0x880C, 0x2A),
    ("ASUS ROG Strix RTX 2080 Super Gaming 8G", 0x10DE, 0x1E81, 0x1043, 0x8713, 0x2A),
    ("ASUS ROG Strix RTX 2080Ti", 0x10DE, 0x1E07, 0x1043, 0x8759, 0x2A),
    ("ASUS ROG Strix RTX 2080Ti Gaming OC White Edition", 0x10DE, 0x1E07, 0x1043, 0x875A, 0x2A),
];

#[cfg(test)]
mod tests {
    use super::super::fake::Script;
    use super::*;
    use foxden_smbus::i2c::BusInfo;

    fn strix_2080(name: &str) -> BusInfo {
        BusInfo {
            name: name.into(),
            pci_vendor: 0x10DE,
            pci_device: 0x1E87,
            pci_subsystem_vendor: 0x1043,
            pci_subsystem_device: 0x865F,
        }
    }

    fn card(info: BusInfo, addr: u8, magic: [u8; 2]) -> Script {
        let mut s = Script::new(info);
        s.registers.insert((addr, 0x20), magic[0]);
        s.registers.insert((addr, 0x21), magic[1]);
        for reg in 4..=7 {
            s.registers.insert((addr, reg), 1);
        }
        s
    }

    // 74 base rows, then 24 more
    #[test]
    fn table() {
        assert_eq!(DEVICES.len(), 74 + 24);
        assert!(DEVICES.contains(&(
            "ASUS ROG STRIX Radeon RX 5700 XT Gaming OC",
            0x1002,
            0x731F,
            0x1043,
            0x04E2,
            0x2A
        )));
    }

    // Probe, then the setup reads
    #[test]
    fn detection_reads() {
        let info = strix_2080("Nvidia NvAPI I2C on GPU 0");
        assert!(matching(&info, DEVICES).count() == 1);
        let (bus, log) = card(info, 0x2A, [0x15, 0x89]).share();
        let found = detect(&bus);
        assert_eq!(found.len(), 1);
        let d = &found[0].devices()[0];
        assert_eq!(d.name, "ASUS ROG STRIX GeForce RTX 2080 Gaming OC");
        assert_eq!(d.leds.len(), 1);
        assert_eq!(
            *log.lock().unwrap(),
            [
                "read_data 2a 20",
                "read_data 2a 21",
                "read_data 2a 04",
                "read_data 2a 05",
                "read_data 2a 06",
                "read_data 2a 07",
            ]
        );
    }

    // Probe: wrong magic, and absent device read as 0xFF
    #[test]
    fn detection_rejects() {
        let (bus, log) = card(strix_2080("x"), 0x2A, [0x15, 0x88]).share();
        assert!(detect(&bus).is_empty());
        assert_eq!(*log.lock().unwrap(), ["read_data 2a 20", "read_data 2a 21"]);
        let (bus, _) = Script::new(strix_2080("x")).share();
        assert!(detect(&bus).is_empty());
        let (bus, log) = card(BusInfo::default(), 0x2A, [0x15, 0x89]).share();
        assert!(detect(&bus).is_empty());
        assert!(log.lock().unwrap().is_empty());
    }

    // Probe: AMD cards only on a recognised bus
    #[test]
    fn amd_bus_name() {
        let vega = |name: &str| BusInfo {
            name: name.into(),
            pci_vendor: 0x1002,
            pci_device: 0x687F,
            pci_subsystem_vendor: 0x1043,
            pci_subsystem_device: 0x04C4,
        };
        let (bus, log) = card(vega("Other AMD bus"), 0x29, [0x15, 0x89]).share();
        assert!(detect(&bus).is_empty());
        assert!(log.lock().unwrap().is_empty());
        let (bus, _) = card(vega(crate::amd::BUS_NAME), 0x29, [0x15, 0x89]).share();
        assert_eq!(detect(&bus).len(), 1);
    }

    // Direct mode, then the LEDs
    #[test]
    fn direct_frames() {
        let (bus, log) = card(strix_2080("x"), 0x2A, [0x15, 0x89]).share();
        let mut found = detect(&bus);
        let id = found[0].devices()[0].id.clone();
        log.lock().unwrap().clear();
        found[0].push(&id, &[Color(0x11, 0x22, 0x33)]).unwrap();
        found[0].push(&id, &[Color(0x11, 0x22, 0x33)]).unwrap();
        found[0].push(&id, &[Color(0, 0, 0xFF)]).unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            [
                "byte_data 2a 07 01",
                "byte_data 2a 04 11",
                "byte_data 2a 05 22",
                "byte_data 2a 06 33",
                "byte_data 2a 04 00",
                "byte_data 2a 05 00",
                "byte_data 2a 06 ff",
            ]
        );
        assert!(found[0].push("other", &[Color::BLACK]).is_err());
    }
}
