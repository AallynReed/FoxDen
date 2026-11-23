//! Lighting through the background service: memory without admin rights, and the fan hubs.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use foxden_client::{Client, HubFrame, PIPE};
use foxden_core::{Backend, Color, Device};

use crate::state::lock;

pub struct ServiceLights {
    client: Client,
    devices: Vec<Device>,
    sent: Vec<(String, Vec<Color>)>,
}

/// The service's lighting devices, or none when no service with SMBus access is running.
pub fn probe() -> Option<ServiceLights> {
    // The first ask makes the service scan every bus, which takes a while on AMD chipsets.
    let devices = Client::new(PIPE, Duration::from_secs(60)).lights().ok()?;
    let client = Client::new(PIPE, Duration::from_secs(5));
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
    lengths: BTreeMap<String, usize>,
    generation: u64,
    devices: Vec<Device>,
    frame: Vec<HubFrame>,
    sent: HashMap<String, (String, Instant)>,
    failed: HashMap<String, String>,
}

/// An unchanged frame is sent again after this long, for hubs that fall back to their own effect.
const RESEND: Duration = Duration::from_secs(1);

static HUB_ZONES: Mutex<Vec<foxden_hid::Zone>> = Mutex::new(Vec::new());
/// Whether a service answered when the hubs were last opened; `None` before that.
static SERVICE_UP: Mutex<Option<bool>> = Mutex::new(None);

/// The hubs' lighting through the service, or `None` when the app has to light them itself.
pub fn hubs(lengths: &BTreeMap<String, usize>) -> Option<ServiceHubs> {
    lock(&HUB_ZONES).clear();
    let client = Client::new(PIPE, Duration::from_secs(20));
    let found = client.hub_lights(lengths);
    *lock(&SERVICE_UP) = Some(found.is_ok() || client.status().is_ok());
    let found = found.ok()?;
    *lock(&HUB_ZONES) = found
        .zones
        .into_iter()
        .map(|z| foxden_hid::Zone {
            device: z.device,
            index: z.index,
            name: z.name,
            leds: z.leds,
            min: z.min,
            max: z.max,
            step: z.step.max(1),
        })
        .collect();
    Some(ServiceHubs {
        client,
        lengths: lengths.clone(),
        generation: found.generation,
        devices: found.devices,
        frame: Vec::new(),
        sent: HashMap::new(),
        failed: HashMap::new(),
    })
}

/// The resizable zones of the hubs the last `hubs` call found.
pub fn hub_zones() -> Vec<foxden_hid::Zone> {
    std::mem::take(&mut *lock(&HUB_ZONES))
}

/// True when the service came or went since the hubs were opened, so they must be opened again.
pub fn service_changed(up: bool) -> bool {
    lock(&SERVICE_UP)
        .as_mut()
        .is_some_and(|seen| std::mem::replace(seen, up) != up)
}

/// Whether any hub the service would own is plugged in.
pub fn hubs_present() -> bool {
    let owned = foxden_xpack::combined();
    foxden_hid::enumerate()
        .iter()
        .any(|d| owned.contains(&(d.vendor, d.product)))
}

/// The ids the app's own probe leaves to the service: all combined hubs once it answered.
pub fn left_to_service(answered: bool) -> Vec<(u16, u16)> {
    if answered {
        foxden_xpack::combined()
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
        let colors = foxden_core::to_hex(colors);
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
        let mut sent = self.client.set_hub_lights(&frame)?;
        // A service that restarted, or a stale release, left nothing open.
        if !sent.open {
            self.generation = self.client.hub_lights(&self.lengths)?.generation;
            sent = self.client.set_hub_lights(&frame)?;
        }
        let failed = sent.failed;
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
        let _ = self.client.release_hub_lights(self.generation);
    }
}

#[cfg(test)]
mod tests {
    use foxden_hid::Pick;

    #[test]
    fn the_app_skips_the_combined_hubs_only_when_the_service_answered() {
        let skip = super::left_to_service(true);
        assert_eq!(skip, foxden_xpack::combined());
        assert!(skip.iter().all(|&(v, p)| !Pick::Skip(&skip).wants(v, p)));
        let kept = super::left_to_service(false);
        assert!(skip.iter().all(|&(v, p)| Pick::Skip(&kept).wants(v, p)));
    }

    #[test]
    fn a_service_that_comes_or_goes_reopens_the_hubs_once() {
        assert!(!super::service_changed(true));
        *super::lock(&super::SERVICE_UP) = Some(false);
        assert!(super::service_changed(true));
        assert!(!super::service_changed(true));
        assert!(super::service_changed(false));
    }
}
