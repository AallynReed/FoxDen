//! Device support learned from SignalRGB's plugins: lighting and USB fan control for hardware the
//! OpenRGB ports don't cover.

pub mod fan;
mod groups;
pub mod hid;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use foxrgb_net::NetConfig;

/// Every HID lighting detector this crate adds, for `foxrgb_hid::probe_with`.
pub fn detectors() -> Vec<&'static Detector> {
    groups::detectors()
}

/// Ids (vid, pid, name) of every USB fan controller this crate drives.
pub fn fan_devices() -> Vec<(u16, u16, &'static str)> {
    groups::fan_ids()
}

/// Fan devices the service leaves the app to light, with why.
pub const NOT_OWNED: &[(u16, u16, &str)] = &[
    (0x3904, 0xF001, "no lighting driver"),
    (0x1532, 0x0F3C, "no lighting driver"),
    (0x0B05, 0x1988, "no lighting driver"),
    (0x0B05, 0x1AA2, "no lighting driver"),
    (0x0B05, 0x1ADA, "no lighting driver"),
    (0x1E71, 0x300E, "no lighting driver"),
    (0x1E71, 0x3014, "no lighting driver"),
    (0x1E71, 0x300C, "no lighting driver"),
    (0x1E71, 0x1711, "no lighting driver"),
    (0x1E71, 0x2015, "no lighting driver"),
    (0x0B05, 0x1BCB, "no lighting driver"),
    (0x0B05, 0x1ADE, "no lighting driver"),
    (0x0DB0, 0xB130, "no lighting driver"),
    (0x1B1C, 0x0C04, "no lighting driver"),
    (0x0C70, 0xF011, "no lighting driver"),
    (0x0C70, 0xF00D, "no lighting driver"),
    (0x0C70, 0xF00A, "no lighting driver"),
    (
        0x0C70,
        0xF010,
        "temperature probes, no fan or pump to drive",
    ),
    (0x1B1C, 0x0C42, "a coolant probe, no fan or pump to drive"),
    (
        0x0416,
        0x7372,
        "Nuvoton keyboards share the id, and their driver refuses the TL controller",
    ),
];

/// Every fan device with lighting: the service drives both sides whenever it runs.
pub fn combined() -> Vec<(u16, u16)> {
    let lit: Vec<(u16, u16)> = foxrgb_hid::detectors()
        .iter()
        .chain(&detectors())
        .map(|d| (d.vid, d.pid))
        .chain(other_devices().iter().map(|d| (d.0, d.1)))
        .chain(foxrgb_hid::other_devices().iter().map(|d| (d.0, d.1)))
        .collect();
    let mut v: Vec<(u16, u16)> = Vec::new();
    for (vid, pid, _) in fan_devices() {
        let id = (vid, pid);
        if lit.contains(&id) && !NOT_OWNED.iter().any(|n| (n.0, n.1) == id) && !v.contains(&id) {
            v.push(id);
        }
    }
    v
}

/// Ids of the raw-USB and serial lighting devices `probe_other` opens.
pub fn other_devices() -> Vec<(u16, u16, &'static str)> {
    groups::other_ids()
}

/// Raw-USB and serial lighting devices, which never show in the HID list.
pub fn probe_other() -> Vec<Box<dyn Backend>> {
    groups::probe_other(foxrgb_hid::Pick::All)
}

/// Like `probe_other`, opening only what `pick` wants.
pub fn probe_other_picked(pick: foxrgb_hid::Pick) -> Vec<Box<dyn Backend>> {
    groups::probe_other(pick)
}

/// WLED, LeetDesk, SRGBmods Wifi LC, Twinkly and Cololight lights from the network settings.
pub fn probe_network(config: &NetConfig) -> Vec<Box<dyn Backend>> {
    groups::network::probe(config)
}

/// Fan and pump control for every USB controller this crate supports.
pub fn fan_backends() -> Vec<Box<dyn FanBackend>> {
    let ids = fan_devices();
    let Some(api) = foxrgb_hid::hid_api(|c| ids.iter().any(|d| (d.0, d.1) == (c.vid, c.pid)))
    else {
        return Vec::new();
    };
    groups::fan_backends(&api)
}

#[cfg(test)]
mod tests {
    /// Keychron boards on SignalRGB's QMK firmware; stock firmware fails the handshake and falls
    /// through to the OpenRGB Keychron matcher.
    const FIRMWARE_GATED: [u16; 1] = [0x3434];

    #[test]
    fn no_device_is_detected_by_two_families() {
        let ours = super::detectors();
        let fox = foxrgb_hid::detectors();
        for a in &ours {
            assert!(
                !fox.iter().any(|b| (a.vid, a.pid) == (b.vid, b.pid)),
                "{} {:04x}:{:04x} is an OpenRGB port's",
                a.name,
                a.vid,
                a.pid
            );
            let vendor_wide = foxrgb_hid::matchers().into_iter().any(|m| {
                m.vid == Some(a.vid)
                    && (m.usage_page.is_none()
                        || a.usage_page.is_none()
                        || m.usage_page == a.usage_page)
            });
            assert!(
                !vendor_wide || FIRMWARE_GATED.contains(&a.vid),
                "{} is claimed by a vendor-wide matcher",
                a.name
            );
        }
        for (i, a) in ours.iter().enumerate() {
            for b in &ours[i + 1..] {
                let twin = (a.vid, a.pid, a.interface, a.usage_page, a.usage)
                    == (b.vid, b.pid, b.interface, b.usage_page, b.usage);
                assert!(!twin, "{} / {} {:04x}:{:04x}", a.name, b.name, a.vid, a.pid);
            }
        }
        let fox_other = foxrgb_hid::other_devices();
        let mut seen: Vec<(u16, u16)> = Vec::new();
        for (v, p, n) in super::other_devices() {
            assert!(
                !fox_other.iter().any(|f| (f.0, f.1) == (v, p)),
                "{n} is an OpenRGB port's"
            );
            assert!(!seen.contains(&(v, p)), "{n} twice");
            seen.push((v, p));
        }
    }

    /// Uni Hub SL and AL: HID lighting while on the HID driver, the same reports over WinUSB after.
    const HID_OR_WINUSB: [(u16, u16); 2] = [(0x0CF2, 0xA100), (0x0CF2, 0xA101)];

    // Where an OpenRGB port lights a cooler or hub, this crate adds only its fans.
    #[test]
    fn fan_hubs_leave_lighting_to_the_openrgb_ports() {
        let fox = foxrgb_hid::detectors();
        let fox_other = foxrgb_hid::other_devices();
        let ours = super::detectors();
        let ours_other = super::other_devices();
        let mut fans: Vec<(u16, u16)> = Vec::new();
        for (v, p, n) in super::fan_devices() {
            let id = (v, p);
            assert!(
                !fox_other.iter().any(|f| (f.0, f.1) == id),
                "{n} is an OpenRGB port's"
            );
            assert!(!fans.contains(&id), "fan hub {n} twice");
            fans.push(id);
            if fox.iter().any(|d| (d.vid, d.pid) == id) {
                assert!(!ours.iter().any(|d| (d.vid, d.pid) == id), "{n} lit twice");
                let winusb = ours_other.iter().any(|d| (d.0, d.1) == id);
                assert_eq!(winusb, HID_OR_WINUSB.contains(&id), "{n} lit twice");
            }
        }
    }

    #[test]
    fn every_fan_device_is_owned_by_the_service_or_excluded_with_a_reason() {
        let combined = super::combined();
        let lit = |id: (u16, u16)| {
            foxrgb_hid::detectors()
                .iter()
                .chain(&super::detectors())
                .any(|d| (d.vid, d.pid) == id)
                || super::other_devices().iter().any(|d| (d.0, d.1) == id)
        };
        for (v, p, n) in super::fan_devices() {
            let out = super::NOT_OWNED.iter().find(|x| (x.0, x.1) == (v, p));
            assert!(
                combined.contains(&(v, p)) != out.is_some(),
                "{n} {v:04x}:{p:04x} is neither owned nor excluded, or both"
            );
            if let Some(&(_, _, why)) = out {
                assert!(!why.is_empty());
                assert_eq!(why == "no lighting driver", !lit((v, p)), "{n}: {why}");
            }
        }
        for (v, p, why) in super::NOT_OWNED {
            let fan = super::fan_devices().iter().any(|f| (f.0, f.1) == (*v, *p));
            assert!(fan, "{v:04x}:{p:04x} ({why}) is no fan device");
        }
        assert_eq!(combined.len(), 84);
        for id in [
            (0x1B1C, 0x0C2E),
            (0x1B1C, 0x0C2F),
            (0x264A, 0x1FA4),
            (0x264A, 0x2339),
            (0x1044, 0x7A4D),
            (0x1E71, 0x3008),
            (0x0416, 0x7395),
            (0x0CF2, 0x7750),
            (0x0CF2, 0xA106),
            (0x0416, 0x8051),
            (0x1E71, 0x170E),
            (0x1E71, 0x1714),
            (0x0B05, 0x1887),
            (0x1B1C, 0x0C56),
            (0x0C70, 0xF00E),
        ] {
            assert!(combined.contains(&id), "{id:04x?} is lit by this crate");
        }
    }

    // `probe_other_picked` only picks from fans_c, so every owned raw-USB device must be there.
    #[test]
    fn owned_raw_usb_devices_are_all_picked() {
        let picked = crate::groups::picked_other_ids();
        for (v, p, n) in super::other_devices() {
            if super::combined().contains(&(v, p)) {
                assert!(picked.iter().any(|x| (x.0, x.1) == (v, p)), "{n}");
            }
        }
    }
}
