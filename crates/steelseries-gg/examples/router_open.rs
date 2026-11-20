//! Opens a loopback route between two outputs with the master muted (silent), runs 2 s, stops.

use std::sync::{Arc, Mutex};
use steelseries_gg::Channel;
use steelseries_gg::router::{MixState, RouteSpec, Router, list_devices};

fn main() {
    let list = list_devices().expect("audio devices");
    let output = list.default_output.clone().expect("a default output");
    let source = list.outputs.iter().find(|o| **o != output).cloned().expect("a second output");
    println!("loopback from {source:?}\nplaying on   {output:?}");
    let mut state = MixState::default();
    state.master_muted = true;
    let controls = Arc::new(Mutex::new(state));
    let spec = RouteSpec { channel: Channel::Game, source, source_is_output: true, output };
    match Router::start(&[spec], controls.clone()) {
        Ok(router) => {
            std::thread::sleep(std::time::Duration::from_secs(2));
            drop(router);
            println!("router ran 2 s muted; peak seen on Game: {:.3}", controls.lock().unwrap().peaks[0]);
        }
        Err(e) => println!("router failed: {e}"),
    }
}
