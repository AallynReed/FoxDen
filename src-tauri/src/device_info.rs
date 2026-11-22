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

fn build(vendor: u16, product: u16, shown: &str, collections: &[foxden_hid::Collection]) -> Value {
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

/// A probed report without its trailing zeros, with its full length.
fn trimmed(r: &[u8]) -> String {
    let end = r.iter().rposition(|&b| b != 0).map_or(1, |i| i + 1);
    format!("{} bytes: {}", r.len(), bytes(&r[..end]))
}

/// Report ids a descriptor declares for feature and for input reports. 0 when it numbers none.
fn report_ids(d: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut features, mut inputs) = (Vec::new(), Vec::new());
    let mut id = 0u8;
    let mut i = 0;
    while i < d.len() {
        let prefix = d[i];
        let size = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        let data = d.get(i + 1).copied().unwrap_or(0);
        match prefix & 0xFC {
            0x84 => id = data,
            0xB0 if !features.contains(&id) => features.push(id),
            0x80 if !inputs.contains(&id) => inputs.push(id),
            _ => {}
        }
        i += 1 + size;
    }
    (features, inputs)
}

/// What a device's vendor collections hold right now, to find where it keeps its battery level.
/// Only reads: feature reports are asked for, input reports are listened to for a moment.
fn probe(vendor: u16, product: u16, collections: &[foxden_hid::Collection]) -> Vec<Value> {
    use foxden_battery::hid::{Dev, enumerate};
    use std::time::{Duration, Instant};
    let mut out = Vec::new();
    for c in enumerate()
        .into_iter()
        .filter(|c| c.vid == vendor && c.pid == product && c.page >= 0xFF00)
    {
        let Some(desc) = collections
            .iter()
            .find(|d| d.usage_page == c.page && d.usage == c.usage && d.interface == c.iface)
            .and_then(|d| d.descriptor.as_ref().ok())
        else {
            continue;
        };
        let (features, inputs) = report_ids(desc);
        let mut feature = serde_json::Map::new();
        if c.feat_len > 0
            && let Some(dev) = Dev::open_feature_only(&c)
        {
            for id in features {
                if let Some(r) = dev.get_feature(id) {
                    feature.insert(format!("{id:02x}"), trimmed(&r).into());
                }
            }
        }
        let mut heard = Vec::new();
        if c.in_len > 0
            && !inputs.is_empty()
            && let Some(dev) = Dev::open_read_only(&c)
        {
            let end = Instant::now() + Duration::from_millis(1500);
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                if let Some(r) = dev.read(left)
                    && !heard.contains(&trimmed(&r))
                    && heard.len() < 8
                {
                    heard.push(trimmed(&r));
                }
            }
        }
        if !feature.is_empty() || !heard.is_empty() {
            out.push(json!({
                "usage_page": format!("{:04x}", c.page),
                "usage": format!("{:04x}", c.usage),
                "interface": c.iface,
                "feature": feature,
                "input": heard,
            }));
        }
    }
    out
}

#[tauri::command]
pub async fn device_info(id: String, name: String) -> Result<String, String> {
    let (vendor, product) = ids(&id).ok_or("Only USB devices can be reported.")?;
    let collections =
        tauri::async_runtime::spawn_blocking(move || foxden_hid::describe(vendor, product))
            .await
            .map_err(|e| e.to_string())?;
    if collections.is_empty() {
        return Err("The device is no longer connected.".into());
    }
    let mut report = build(vendor, product, &name, &collections);
    let probe = tauri::async_runtime::spawn_blocking(move || probe(vendor, product, &collections))
        .await
        .map_err(|e| e.to_string())?;
    if !probe.is_empty() {
        report["probe"] = Value::Array(probe);
    }
    if vendor == 0x046D {
        let hidpp = tauri::async_runtime::spawn_blocking(move || {
            foxden_battery::logitech_diagnose(product)
        })
        .await
        .map_err(|e| e.to_string())?;
        if hidpp.len() > 1 {
            report["hidpp"] = hidpp.into();
        }
    }
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
    fn report_ids_come_from_the_descriptor() {
        let d = [
            0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x10, 0x09, 0x01, 0x95, 0x06, 0x81,
            0x00, 0x85, 0x08, 0x09, 0x02, 0x95, 0x07, 0xb1, 0x02, 0xc0,
        ];
        assert_eq!(report_ids(&d), (vec![0x08], vec![0x10]));
    }

    #[test]
    fn only_usb_ids_parse() {
        assert_eq!(ids("usb:046d:c342"), Some((0x046D, 0xC342)));
        assert_eq!(ids("usb:046d"), None);
        assert_eq!(ids("usb:046d:c342:1"), None);
        assert_eq!(ids("board:msi"), None);
    }

    #[test]
    fn the_report_has_no_dropped_keys_or_long_hex() {
        let c = foxden_hid::Collection {
            interface: 1,
            usage_page: 0xFF43,
            usage: 0x0602,
            release: 0x0101,
            name: "G512 RGB".into(),
            manufacturer: "Logitech".into(),
            descriptor: Ok(vec![
                0x06, 0x43, 0xFF, 0x0A, 0x02, 0x06, 0xA1, 0x01, 0x85, 0x11, 0x75, 0x08, 0x95, 0x13,
            ]),
        };
        let r = build(0x046D, 0xC342, "Logitech G512", &[c]);
        let text = r.to_string();
        assert_eq!(r["product"], "G512 RGB");
        assert_eq!(r["collections"][0]["usage_page"], "ff43");
        assert!(r.get("name").is_none());
        assert!(text.contains("06 43 ff 0a"));
        assert!(
            !text
                .split(|ch: char| !ch.is_ascii_hexdigit())
                .any(|w| w.len() >= 24)
        );
    }
}
