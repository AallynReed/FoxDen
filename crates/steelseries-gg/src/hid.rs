//! Read-only list of SteelSeries USB HID interfaces. Nothing is ever written to a device here.

use crate::engine::STEELSERIES_VID;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidInterface {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub path: String,
}

pub fn list_steelseries() -> Result<Vec<HidInterface>> {
    let api = hidapi::HidApi::new().map_err(|e| Error::Device(e.to_string()))?;
    let mut out: Vec<HidInterface> = api
        .device_list()
        .filter(|d| d.vendor_id() == STEELSERIES_VID)
        .map(|d| HidInterface {
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            product: d.product_string().unwrap_or("").to_string(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            path: d.path().to_string_lossy().into_owned(),
        })
        .collect();
    out.sort_by_key(|d| (d.product_id, d.interface, d.usage_page));
    out.dedup_by(|a, b| a.path == b.path);
    Ok(out)
}
