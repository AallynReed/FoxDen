//! Group fans_a: USB fan controllers and AIO coolers.

mod aquacomputer;
mod arctic;
mod msi_coreliquid;
mod nuvoton;
mod nuvoton_layouts;
mod nzxt_grid;
mod nzxt_kraken;
mod nzxt_smart2;
mod nzxt_x2;
mod razer_pwm;
mod ryujin;
mod util;
mod waterforce;

use foxden_fan::FanBackend;
use foxden_hid::registry::Detector;
use hidapi::HidApi;

pub fn detectors() -> Vec<&'static Detector> {
    waterforce::DETECTORS
        .iter()
        .chain(nzxt_kraken::DETECTORS.iter())
        .chain(nuvoton::DETECTORS.iter())
        .collect()
}

pub fn fan_ids() -> Vec<(u16, u16, &'static str)> {
    let mut v = arctic::IDS.to_vec();
    v.extend_from_slice(razer_pwm::IDS);
    v.extend(ryujin::ids());
    v.extend_from_slice(waterforce::IDS);
    v.extend(nzxt_kraken::ids());
    v.extend(nzxt_smart2::ids());
    v.extend_from_slice(nuvoton::IDS);
    v.extend(nzxt_x2::ids());
    v.extend(nzxt_grid::ids());
    v.extend(msi_coreliquid::ids());
    v.extend(aquacomputer::ids());
    v
}

pub fn fan_backends(api: &HidApi) -> Vec<Box<dyn FanBackend>> {
    let mut seen = Vec::new();
    let mut v = arctic::backends(api, &mut seen);
    v.extend(razer_pwm::backends(api, &mut seen));
    v.extend(ryujin::backends(api, &mut seen));
    v.extend(waterforce::backends(api, &mut seen));
    v.extend(nzxt_kraken::backends(api, &mut seen));
    v.extend(nzxt_smart2::backends(api, &mut seen));
    v.extend(nuvoton::backends(api, &mut seen));
    v.extend(nzxt_x2::backends(api, &mut seen));
    v.extend(nzxt_grid::backends(api, &mut seen));
    v.extend(msi_coreliquid::backends(api, &mut seen));
    v.extend(aquacomputer::backends(api, &mut seen));
    v
}
