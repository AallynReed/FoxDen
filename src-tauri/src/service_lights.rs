//! Lighting through the installed background service: memory for an app without administrator
//! rights, and the fan hubs whose fans the service drives.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use foxrgb_client::{Client, HubFrame, PIPE};
use foxrgb_core::{Backend, Color, Device};

pub struct ServiceLights {
    client: Client,
    devices: Vec<Device>,
    sent: Vec<(String, Vec<Color>)>,
}

/// The service's lighting devices, or none when no service with SMBus access is running.
pub fn probe() -> Option<ServiceLights> {
    let client = Client::new(PIPE, Duration::from_secs(5));
    let devices = client.lights().ok()?;
    (!devices.is_empty()).then_some(ServiceLights {
        client,
        devices,
        sent: Vec::new(),
    })
}

impl Backend for ServiceLights {
    fn name(&self) -> &str {
        "service"
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if self
            .sent
            .iter()
            .any(|(id, last)| id == device_id && last == colors)
        {
            return Ok(());
        }
        self.client.set_lights(device_id, colors)?;
        self.sent.retain(|(id, _)| id != device_id);
        self.sent.push((device_id.to_string(), colors.to_vec()));
        Ok(())
    }
}

/// Fan hubs whose lighting the service drives with their fans, one batch per frame.
pub struct ServiceHubs {
    client: Client,
    devices: Vec<Device>,
    frame: Vec<HubFrame>,
    sent: HashMap<String, (String, Instant)>,
    failed: HashMap<String, String>,
}

/// An unchanged frame is sent again after this long, for hubs that fall back to their own effect.
const RESEND: Duration = Duration::from_secs(1);

static HUB_ZONES: Mutex<Vec<foxrgb_hid::Zone>> = Mutex::new(Vec::new());

/// The hubs' lighting through the service, or `None` when no service that offers it answers;
/// the app then lights them itself.
pub fn hubs(lengths: &BTreeMap<String, usize>) -> Option<ServiceHubs> {
    HUB_ZONES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let client = Client::new(PIPE, Duration::from_secs(20));
    let found = client.hub_lights(lengths).ok()?;
    *HUB_ZONES.lock().unwrap_or_else(|e| e.into_inner()) = found
        .zones
        .into_iter()
        .map(|z| foxrgb_hid::Zone {
            device: z.device,
            index: z.index,
            name: z.name,
            leds: z.leds,
            min: z.min,
            max: z.max,
        })
        .collect();
    Some(ServiceHubs {
        client,
        devices: found.devices,
        frame: Vec::new(),
        sent: HashMap::new(),
        failed: HashMap::new(),
    })
}

/// The resizable zones of the hubs the last `hubs` call found.
pub fn hub_zones() -> Vec<foxrgb_hid::Zone> {
    std::mem::take(&mut *HUB_ZONES.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The ids the app's own probe leaves to the service: all combined hubs once it answered.
pub fn left_to_service(answered: bool) -> Vec<(u16, u16)> {
    if answered {
        foxrgb_xpack::combined()
    } else {
        Vec::new()
    }
}

impl Backend for ServiceHubs {
    fn name(&self) -> &str {
        "service hubs"
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        let colors = foxrgb_core::to_hex(colors);
        let fresh = self
            .sent
            .get(device_id)
            .is_some_and(|(last, at)| *last == colors && at.elapsed() < RESEND);
        self.frame.retain(|f| f.device != device_id);
        if !fresh {
            self.frame.push(HubFrame {
                device: device_id.to_string(),
                colors,
            });
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        if self.frame.is_empty() {
            return Ok(());
        }
        let frame = std::mem::take(&mut self.frame);
        let failed = self.client.set_hub_lights(&frame)?;
        for f in frame {
            if !failed.contains_key(&f.device) {
                self.failed.remove(&f.device);
            }
            self.sent.insert(f.device, (f.colors, Instant::now()));
        }
        // One hub that fails is logged, not allowed to stop the others.
        for (device, e) in failed {
            self.sent.remove(&device);
            if self.failed.get(&device) != Some(&e) {
                crate::log::warn("devices", format!("{device}: {e}"));
                self.failed.insert(device, e);
            }
        }
        Ok(())
    }
}

impl Drop for ServiceHubs {
    fn drop(&mut self) {
        let _ = self.client.release_hub_lights();
    }
}

#[cfg(test)]
mod tests {
    use foxrgb_hid::Pick;

    #[test]
    fn the_app_skips_the_combined_hubs_only_when_the_service_answered() {
        let skip = super::left_to_service(true);
        assert_eq!(skip, foxrgb_xpack::combined());
        assert!(skip.iter().all(|&(v, p)| !Pick::Skip(&skip).wants(v, p)));
        let kept = super::left_to_service(false);
        assert!(skip.iter().all(|&(v, p)| Pick::Skip(&kept).wants(v, p)));
    }
}
