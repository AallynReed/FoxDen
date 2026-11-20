//! Ported from OpenRGB Controllers/EspurnaController (GPL-2.0-or-later).

use foxden_core::{Color, DeviceKind};

use crate::net::tcp_once;
use crate::{Driver, EspurnaDevice, Light, local};

/// `SetLEDs`: the colour as `%23RRGGBB` in a bare GET.
fn request(apikey: &str, host: &str, c: Color) -> String {
    format!(
        "GET /api/rgb?apikey={apikey}&value=%23{:02X}{:02X}{:02X} HTTP/1.1\r\nHost: {host}\r\n\r\n",
        c.0, c.1, c.2
    )
}

struct Espurna {
    cfg: EspurnaDevice,
}

impl Driver for Espurna {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let c = colors.first().copied().unwrap_or_default();
        let req = request(&self.cfg.apikey, &self.cfg.ip, c);
        tcp_once(&self.cfg.ip, self.cfg.port, req.as_bytes(), false).map(drop)
    }
}

pub fn open(cfg: &EspurnaDevice) -> Option<Light> {
    local(&cfg.ip).ok()?;
    Some(Light::new(
        format!("net:espurna:{}:{}", cfg.ip, cfg.port),
        "Espurna".into(),
        DeviceKind::Other,
        1,
        Box::new(Espurna { cfg: cfg.clone() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request_bytes() {
        assert_eq!(
            request("C0FFEE", "192.168.1.30", Color(255, 10, 0)),
            "GET /api/rgb?apikey=C0FFEE&value=%23FF0A00 HTTP/1.1\r\nHost: 192.168.1.30\r\n\r\n"
        );
    }
}
