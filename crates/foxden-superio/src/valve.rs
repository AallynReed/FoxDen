//! Ported from OpenRGB Controllers/ValveSteamMachineController (GPL-2.0-or-later).

use foxden_core::{Backend, Color, Device, DeviceKind};

use crate::lights::{Dmi, Driver, Light, Pawn};

const LED_COUNT: u64 = 17;
/// Index of "manual" in the module's effect list, OpenRGB's Direct.
const EFFECT_MANUAL: u64 = 7;

struct SteamMachine {
    pawn: Box<dyn Pawn>,
    brightness: u64,
    delay: u64,
}

impl SteamMachine {
    fn set(&self, name: &str, input: &[u64]) {
        let _ = self.pawn.exec(name, input, 0);
    }

    /// DeviceUpdateMode for Direct.
    fn set_mode(&self) {
        let enabled = self.pawn.read("ioctl_get_enabled", &[]).unwrap_or(0) & 0xFF != 0;
        if !enabled {
            self.set("ioctl_set_enabled", &[1]);
        }
        self.set("ioctl_set_effect", &[EFFECT_MANUAL]);
        if self.brightness <= 255 {
            self.set("ioctl_set_brightness_scale", &[self.brightness]);
        }
        if self.delay <= 20 {
            self.set("ioctl_set_delay", &[self.delay]);
        }
    }
}

impl Driver for SteamMachine {
    fn write(&mut self, colors: &[Color], first: bool) {
        if first {
            self.set_mode();
        }
        for (led, c) in colors.iter().enumerate() {
            let index = (LED_COUNT - 1) - led as u64;
            let input = [index, c.0 as u64, c.1 as u64, c.2 as u64];
            self.set("ioctl_set_led_color", &input);
        }
    }
}

/// DetectValveSteamMachineControllers and the controller constructor.
pub(crate) fn detect(dmi: &Dmi, open: &dyn Fn() -> Option<Box<dyn Pawn>>) -> Vec<Box<dyn Backend>> {
    let ids = (dmi.manufacturer.as_str(), dmi.product_name.as_str());
    if !matches!(ids, ("OEM", "F7F") | ("Valve", "Fremont")) {
        return Vec::new();
    }
    let Some(pawn) = open() else {
        return Vec::new();
    };
    let _ = pawn.exec("ioctl_init", &[], 0);
    let ranged = |name, max| pawn.read(name, &[]).filter(|&v| v <= max).unwrap_or(0);
    let brightness = ranged("ioctl_get_brightness_scale", 255);
    let delay = ranged("ioctl_get_delay", 20);
    let device = Device::grid(
        "pawnio:steam-machine",
        "Valve Steam Machine",
        DeviceKind::Strip,
        [0.06, 0.30, 0.40, 0.05],
        LED_COUNT as u32,
        1,
    );
    let driver = SteamMachine {
        pawn,
        brightness,
        delay,
    };
    vec![Light::boxed("valve steam machine", device, driver)]
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::lights::fake::{self, Fake, Log};

    fn run(dmi: &Dmi, enabled: u64) -> (Log, Vec<Box<dyn Backend>>) {
        let log = Log::default();
        let open = || {
            let reads = HashMap::from([
                (("get_brightness_scale", 0), 0x80),
                (("get_delay", 0), 5),
                (("get_enabled", 0), enabled),
            ]);
            Some(Box::new(Fake {
                log: log.clone(),
                reads,
            }) as Box<dyn Pawn>)
        };
        let found = detect(dmi, &open);
        (log, found)
    }

    #[test]
    fn detection() {
        let (log, found) = run(&fake::dmi("Valve", "", "Fremont"), 1);
        assert_eq!(found.len(), 1);
        assert_eq!(
            fake::take(&log),
            ["init", "get_brightness_scale", "get_delay"]
        );
        let (_, found) = run(&fake::dmi("OEM", "", "F7F"), 1);
        assert_eq!(found.len(), 1);
        let (log, found) = run(&fake::dmi("Valve", "", "Jupiter"), 1);
        assert!(found.is_empty());
        assert!(fake::take(&log).is_empty());
    }

    // DeviceUpdateMode(Direct = "manual") then DeviceUpdateLEDs, LEDs reversed.
    #[test]
    fn direct_frame() {
        let (log, mut found) = run(&fake::dmi("Valve", "", "Fremont"), 0);
        fake::take(&log);
        let id = found[0].devices()[0].id.clone();
        let colors: Vec<Color> = (0..17).map(|i| Color(i, 0x10, 0xFF)).collect();
        found[0].push(&id, &colors).unwrap();
        let mut want = vec![
            "get_enabled".to_string(),
            "set_enabled 01".into(),
            "set_effect 07".into(),
            "set_brightness_scale 80".into(),
            "set_delay 05".into(),
        ];
        for i in 0..17u64 {
            want.push(format!("set_led_color {:02x} {i:02x} 10 ff", 16 - i));
        }
        assert_eq!(fake::take(&log), want);

        let mut colors = colors;
        colors[0] = Color(9, 9, 9);
        found[0].push(&id, &colors).unwrap();
        let log = fake::take(&log);
        assert_eq!(log.len(), 17);
        assert_eq!(log[0], "set_led_color 10 09 09 09");
    }

    #[test]
    fn an_enabled_strip_is_not_enabled_again() {
        let (log, mut found) = run(&fake::dmi("Valve", "", "Fremont"), 1);
        fake::take(&log);
        let id = found[0].devices()[0].id.clone();
        found[0].push(&id, &[Color::BLACK; 17]).unwrap();
        assert_eq!(fake::take(&log)[..2], ["get_enabled", "set_effect 07"]);
    }
}
