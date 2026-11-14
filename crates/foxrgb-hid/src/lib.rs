//! Native USB HID lighting drivers. Experimental: none of this has run against hardware yet.

mod hidpp;
mod msi;

use foxrgb_core::Backend;
use hidapi::HidApi;

fn supported(vid: u16, pid: u16) -> bool {
    matches!((vid, pid), (msi::VID, msi::PID) | (hidpp::VID, hidpp::G502_RECEIVER | hidpp::G515_RECEIVER))
}

/// HID collections of the supported devices, read from the system's device list without opening any.
pub fn list() -> Vec<String> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    api.device_list()
        .filter(|d| supported(d.vendor_id(), d.product_id()))
        .map(|d| {
            format!(
                "{:04x}:{:04x} usage {:04x}:{:04x} interface {}",
                d.vendor_id(),
                d.product_id(),
                d.usage_page(),
                d.usage(),
                d.interface_number()
            )
        })
        .collect()
}

/// Detects supported USB HID lighting devices and returns one backend per device.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let Ok(api) = HidApi::new() else {
        return Vec::new();
    };
    let mut found: Vec<Box<dyn Backend>> = Vec::new();
    let mut board = false;
    for info in api.device_list() {
        match (info.vendor_id(), info.product_id()) {
            (msi::VID, msi::PID) if !board => {
                if let Ok(hid) = info.open_device(&api)
                    && let Ok(device) = msi::Board::open(hid)
                {
                    found.push(Box::new(device));
                    board = true;
                }
            }
            (hidpp::VID, pid @ (hidpp::G502_RECEIVER | hidpp::G515_RECEIVER))
                if hidpp::is_long_channel(info.usage_page(), info.usage()) =>
            {
                if let Ok(hid) = info.open_device(&api)
                    && let Ok(device) = hidpp::Lightspeed::open(hid, pid)
                {
                    found.push(Box::new(device));
                }
            }
            _ => {}
        }
    }
    found
}
