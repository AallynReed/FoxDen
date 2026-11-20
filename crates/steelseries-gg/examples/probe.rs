//! Read-only: prints what the library sees on this computer.

use steelseries_gg::{Channel, Endpoints};

fn main() {
    let ep = match Endpoints::discover() {
        Ok(e) => e,
        Err(e) => return println!("GG not found: {e}"),
    };
    println!("GG {:?}\nSonar {:?}\nEngine {:?} (cert pinned: {})\nGameSense {:?}", ep.gg, ep.sonar, ep.engine, ep.engine_cert_pem.is_some(), ep.gamesense);
    if let Some(s) = ep.sonar() {
        println!("mode {:?}", s.mode());
        match s.classic_volumes() {
            Ok(v) => {
                for c in Channel::ALL {
                    println!("  {:<6} {:?}", c.label(), v.channel(c).classic);
                }
            }
            Err(e) => println!("volumes: {e}"),
        }
        println!("chat mix {:?}", s.chat_mix());
        println!("redirections {:?}", s.classic_redirections().map(|r| r.len()));
    }
    if let Some(e) = ep.engine() {
        match e.devices() {
            Ok(devs) => {
                for d in devs {
                    println!("  device {} {} connected={} fw_update={}", d.id, d.title(), d.is_connected(), d.firmware_update_available());
                }
            }
            Err(err) => println!("engine: {err}"),
        }
        println!("mm metadata {:?}", e.mm_devices_metadata().map(|m| m.iter().map(|x| x.has_chat_mix()).collect::<Vec<_>>()));
    }
    if let Some(g) = ep.gamesense() {
        println!("gamesense ping {:?}", g.ping());
    }
}
