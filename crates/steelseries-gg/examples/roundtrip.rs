//! Changes Sonar's Game volume and chat mix, reads them back, then restores both.

use steelseries_gg::{Channel, Endpoints};

fn main() {
    let s = Endpoints::discover().ok().and_then(|e| e.sonar()).expect("Sonar not running");
    let before = s.classic_volumes().unwrap().channel(Channel::Game).classic.unwrap();
    s.set_volume(Channel::Game, 0.8).unwrap();
    let mid = s.classic_volumes().unwrap().channel(Channel::Game).classic.unwrap();
    s.set_volume(Channel::Game, before.volume).unwrap();
    let after = s.classic_volumes().unwrap().channel(Channel::Game).classic.unwrap();
    println!("game volume {} -> {} -> {}", before.volume, mid.volume, after.volume);

    let cm = s.chat_mix().unwrap();
    let set = s.set_chat_mix(0.3);
    let read = s.chat_mix().unwrap();
    let _ = s.set_chat_mix(cm.balance);
    println!("chat mix {} -> {:?} (read {}) -> {}", cm.balance, set.map(|c| c.balance), read.balance, s.chat_mix().unwrap().balance);
}
