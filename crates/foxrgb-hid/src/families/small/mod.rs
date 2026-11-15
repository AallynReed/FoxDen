//! Ported from OpenRGB Controllers (small HID families) (GPL-2.0-or-later).

use crate::registry::Detector;

mod acer_nitro;
mod aoc_keyboard;
mod aoc_mouse;
mod aoc_mousemat;
mod areson;
mod asus_monitor;
mod clevo_lightbar;
mod common;
mod cryorig;
mod darkproject;
mod dreamcheeky;
mod ducky;
mod edifier;
mod ek;
mod gamesir;
mod hp_omen30l;
mod hyte;
mod instant_mouse;
mod intel_arc;
mod ionico;
mod jginyue;
mod keychron;
mod keyrox;
pub(crate) mod klm;
mod laview;
mod lego_toypad;
mod lexip;
mod lg_monitor;
mod lightsalt;
mod luxafor;
mod madcatz;
mod mintaka;
mod mnt;
mod n5312a;
mod nvidia_esa;
mod oks;
mod patriot_viper;
mod robobloq;
mod sayo;
mod skydimo;
mod skyloong;
mod tecknet;
mod thingm;
mod trust;
mod turtlebeach_kp7;
mod valkyrie;
mod winbond;
mod witmod;
mod wraith_prism;
mod wushi;
mod xpg_summoner;
mod zalman;
mod zet;

pub fn detectors() -> Vec<&'static Detector> {
    [
        acer_nitro::DETECTORS,
        aoc_keyboard::DETECTORS,
        aoc_mouse::DETECTORS,
        aoc_mousemat::DETECTORS,
        areson::DETECTORS,
        asus_monitor::DETECTORS,
        clevo_lightbar::DETECTORS,
        cryorig::DETECTORS,
        darkproject::DETECTORS,
        dreamcheeky::DETECTORS,
        ducky::DETECTORS,
        edifier::DETECTORS,
        ek::DETECTORS,
        gamesir::DETECTORS,
        hp_omen30l::DETECTORS,
        hyte::DETECTORS,
        instant_mouse::DETECTORS,
        intel_arc::DETECTORS,
        ionico::DETECTORS,
        jginyue::DETECTORS,
        keychron::DETECTORS,
        keyrox::DETECTORS,
        laview::DETECTORS,
        lego_toypad::DETECTORS,
        lexip::DETECTORS,
        lg_monitor::DETECTORS,
        lightsalt::DETECTORS,
        luxafor::DETECTORS,
        madcatz::DETECTORS,
        mintaka::DETECTORS,
        mnt::DETECTORS,
        n5312a::DETECTORS,
        nvidia_esa::DETECTORS,
        oks::DETECTORS,
        patriot_viper::DETECTORS,
        robobloq::DETECTORS,
        sayo::DETECTORS,
        skydimo::DETECTORS,
        skyloong::DETECTORS,
        tecknet::DETECTORS,
        thingm::DETECTORS,
        trust::DETECTORS,
        turtlebeach_kp7::DETECTORS,
        valkyrie::DETECTORS,
        winbond::DETECTORS,
        witmod::DETECTORS,
        wraith_prism::DETECTORS,
        wushi::DETECTORS,
        xpg_summoner::DETECTORS,
        zalman::DETECTORS,
        zet::DETECTORS,
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn detectors_are_distinct() {
        let d = super::detectors();
        println!("small detectors: {}", d.len());
        for (i, a) in d.iter().enumerate() {
            for b in &d[i + 1..] {
                let same = (a.vid, a.pid, a.interface, a.usage_page, a.usage)
                    == (b.vid, b.pid, b.interface, b.usage_page, b.usage);
                assert!(!same, "{} duplicates {}", a.name, b.name);
            }
        }
    }
}
