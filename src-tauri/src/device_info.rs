//! A device report the user reads, then sends or copies, to help add a driver.

use serde_json::{Value, json};

fn ids(id: &str) -> Option<(u16, u16)> {
    let mut parts = id.strip_prefix("usb:")?.split(':');
    let vendor = u16::from_str_radix(parts.next()?, 16).ok()?;
    let product = u16::from_str_radix(parts.next()?, 16).ok()?;
    parts.next().is_none().then_some((vendor, product))
}

/// Space-separated so the collector does not take it for a token.
fn bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn build(vendor: u16, product: u16, shown: &str, collections: &[foxrgb_hid::Collection]) -> Value {
    let first = collections.iter().find(|c| !c.name.is_empty());
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "kind": "device-info",
        "os": "windows",
        "vid": format!("{vendor:04x}"),
        "pid": format!("{product:04x}"),
        "product": first.map_or(shown, |c| c.name.as_str()),
        "manufacturer": first.map_or("", |c| c.manufacturer.as_str()),
        "release": collections.first().map_or(0, |c| c.release),
        "collections": collections.iter().map(|c| {
            let mut entry = json!({
                "interface": c.interface,
                "usage_page": format!("{:04x}", c.usage_page),
                "usage": format!("{:04x}", c.usage),
            });
            match &c.descriptor {
                Ok(d) => entry["descriptor"] = bytes(d).into(),
                Err(e) => entry["unreadable"] = crate::report::scrub(e).into(),
            }
            entry
        }).collect::<Vec<_>>(),
    })
}

#[tauri::command]
pub async fn device_info(id: String, name: String) -> Result<String, String> {
    let (vendor, product) = ids(&id).ok_or("Only USB devices can be reported.")?;
    let collections = tauri::async_runtime::spawn_blocking(move || {
        foxrgb_hid::describe(vendor, product)
    })
    .await
    .map_err(|e| e.to_string())?;
    if collections.is_empty() {
        return Err("The device is no longer connected.".into());
    }
    let report = build(vendor, product, &name, &collections);
    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_device_info(text: String) -> Result<(), String> {
    let report: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    crate::report::post(&report).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_usb_ids_parse() {
        assert_eq!(ids("usb:046d:c342"), Some((0x046D, 0xC342)));
        assert_eq!(ids("usb:046d"), None);
        assert_eq!(ids("usb:046d:c342:1"), None);
        assert_eq!(ids("board:msi"), None);
    }

    #[test]
    fn the_report_has_no_dropped_keys_or_long_hex() {
        let c = foxrgb_hid::Collection {
            interface: 1,
            usage_page: 0xFF43,
            usage: 0x0602,
            release: 0x0101,
            name: "G512 RGB".into(),
            manufacturer: "Logitech".into(),
            descriptor: Ok(vec![0x06, 0x43, 0xFF, 0x0A, 0x02, 0x06, 0xA1, 0x01, 0x85, 0x11, 0x75, 0x08, 0x95, 0x13]),
        };
        let r = build(0x046D, 0xC342, "Logitech G512", &[c]);
        let text = r.to_string();
        assert_eq!(r["product"], "G512 RGB");
        assert_eq!(r["collections"][0]["usage_page"], "ff43");
        assert!(r.get("name").is_none());
        assert!(text.contains("06 43 ff 0a"));
        assert!(!text.split(|ch: char| !ch.is_ascii_hexdigit()).any(|w| w.len() >= 24));
    }
}
