//! Memory lighting through the installed background service, for an app without administrator rights.

use std::time::Duration;

use foxrgb_client::{Client, PIPE};
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
