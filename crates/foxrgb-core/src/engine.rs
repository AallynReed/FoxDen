use crate::{Backend, Color, Device, Effect, Inputs};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    pub device: String,
    pub colors: Vec<Color>,
}

pub struct Engine {
    backends: Vec<(Box<dyn Backend>, Vec<Device>)>,
    pub effect: Effect,
    pub brightness: f32,
}

impl Engine {
    pub fn new(backends: Vec<Box<dyn Backend>>) -> Self {
        let backends = backends
            .into_iter()
            .map(|b| {
                let devices = b.devices();
                (b, devices)
            })
            .collect();
        Engine { backends, effect: Effect::default(), brightness: 1.0 }
    }

    pub fn devices(&self) -> Vec<Device> {
        self.backends.iter().flat_map(|(_, d)| d.iter().cloned()).collect()
    }

    /// Renders the effect at time `t`, pushes it to every device and returns what was sent.
    pub fn render(&mut self, t: f32, inputs: &Inputs) -> Vec<Frame> {
        let (effect, brightness) = (&self.effect, self.brightness);
        let mut frames = Vec::new();
        for (backend, devices) in &mut self.backends {
            for device in devices.iter() {
                let colors: Vec<Color> = device
                    .canvas_positions()
                    .map(|(x, y)| effect.sample(x, y, t, inputs).scale(brightness))
                    .collect();
                if let Err(e) = backend.push(&device.id, &colors) {
                    eprintln!("{}: {}: {e}", backend.name(), device.id);
                }
                frames.push(Frame { device: device.id.clone(), colors });
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualBackend;

    #[test]
    fn one_frame_per_device_one_color_per_led() {
        let mut engine = Engine::new(vec![Box::new(VirtualBackend)]);
        let devices = engine.devices();
        let frames = engine.render(1.0, &Inputs::default());
        assert_eq!(frames.len(), devices.len());
        for (frame, device) in frames.iter().zip(&devices) {
            assert_eq!(frame.device, device.id);
            assert_eq!(frame.colors.len(), device.leds.len());
        }
    }

    #[test]
    fn zero_brightness_is_black() {
        let mut engine = Engine::new(vec![Box::new(VirtualBackend)]);
        engine.brightness = 0.0;
        let frames = engine.render(3.0, &Inputs::default());
        assert!(frames.iter().all(|f| f.colors.iter().all(|c| *c == Color::BLACK)));
    }
}
