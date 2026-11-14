use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Motherboard,
    Memory,
    Gpu,
    Keyboard,
    Mouse,
    Strip,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Placement on the shared effect canvas: `[x, y, w, h]`, each 0..=1.
    pub rect: [f32; 4],
    /// Columns and rows, used to size LEDs when drawing.
    pub grid: [u32; 2],
    /// LED positions inside `rect`, each 0..=1.
    pub leds: Vec<[f32; 2]>,
}

impl Device {
    /// A `cols` x `rows` matrix with LEDs at cell centres, row-major.
    pub fn grid(id: &str, name: &str, kind: DeviceKind, rect: [f32; 4], cols: u32, rows: u32) -> Self {
        let leds = (0..rows)
            .flat_map(|r| {
                (0..cols).map(move |c| {
                    [(c as f32 + 0.5) / cols as f32, (r as f32 + 0.5) / rows as f32]
                })
            })
            .collect();
        Device { id: id.into(), name: name.into(), kind, rect, grid: [cols, rows], leds }
    }

    /// Canvas position of each LED.
    pub fn canvas_positions(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let [x, y, w, h] = self.rect;
        self.leds.iter().map(move |[lx, ly]| (x + lx * w, y + ly * h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_layout() {
        let d = Device::grid("d", "D", DeviceKind::Other, [0.5, 0.0, 0.5, 1.0], 2, 1);
        assert_eq!(d.leds.len(), 2);
        let pos: Vec<_> = d.canvas_positions().collect();
        assert_eq!(pos, vec![(0.625, 0.5), (0.875, 0.5)]);
    }
}
