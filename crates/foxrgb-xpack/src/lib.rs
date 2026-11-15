//! Device support learned from SignalRGB's plugins: lighting and USB fan control for hardware the
//! OpenRGB ports don't cover.

pub mod fan;
mod groups;
pub mod hid;

use foxrgb_core::Backend;
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::Detector;
use foxrgb_net::NetConfig;
use hidapi::HidApi;

/// Every HID lighting detector this crate adds, for `foxrgb_hid::probe_with`.
pub fn detectors() -> Vec<&'static Detector> {
    groups::detectors()
}

/// Ids (vid, pid, name) of every USB fan controller this crate drives.
pub fn fan_devices() -> Vec<(u16, u16, &'static str)> {
    groups::fan_ids()
}

/// Hubs and coolers an OpenRGB port lights and this crate cools: the service drives both sides.
pub fn combined() -> Vec<(u16, u16)> {
    let lit = foxrgb_hid::detectors();
    let mut v: Vec<(u16, u16)> = Vec::new();
    for (vid, pid, _) in fan_devices() {
        if lit.iter().any(|d| (d.vid, d.pid) == (vid, pid)) && !v.contains(&(vid, pid)) {
            v.push((vid, pid));
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
    groups::probe_other()
}

/// WLED, LeetDesk, SRGBmods Wifi LC, Twinkly and Cololight lights from the network settings.
pub fn probe_network(config: &NetConfig) -> Vec<Box<dyn Backend>> {
    groups::network::probe(config)
}

/// Fan and pump control for every USB controller this crate supports.
pub fn fan_backends() -> Vec<Box<dyn FanBackend>> {
    let Ok(api) = HidApi::new() else {
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

    // Where an OpenRGB port lights a cooler or hub, this crate adds only its fans.
    #[test]
    fn fan_hubs_leave_lighting_to_the_openrgb_ports() {
        let fox = foxrgb_hid::detectors();
        let fox_other = foxrgb_hid::other_devices();
        let ours = super::detectors();
        let mut fans: Vec<(u16, u16)> = Vec::new();
        for (v, p, n) in super::fan_devices() {
            assert!(
                !fox_other.iter().any(|f| (f.0, f.1) == (v, p)),
                "{n} is an OpenRGB port's"
            );
            assert!(!fans.contains(&(v, p)), "fan hub {n} twice");
            fans.push((v, p));
            if super::combined().contains(&(v, p)) {
                assert!(
                    !ours.iter().any(|d| (d.vid, d.pid) == (v, p)),
                    "{n} lit twice"
                );
            }
        }
        let combined = super::combined();
        assert_eq!(combined.len(), 64);
        assert!(
            combined
                .iter()
                .all(|id| fox.iter().any(|d| (d.vid, d.pid) == *id) && fans.contains(id))
        );
    }
}
