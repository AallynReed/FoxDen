//! Wi-Fi and LAN lights foxden-net lacks
//! (Twinkly, Cololight, LeetDesk, WLED, SRGBmods Wifi LC).

mod cololight;
mod net;
mod twinkly;
mod wled;

use foxden_core::Backend;
use foxden_net::NetConfig;
use std::net::Ipv4Addr;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

/// Whole probe, discovery and follow-up requests included.
const PROBE_BUDGET: Duration = Duration::from_millis(1900);

/// Where one family looks: the listed addresses, plus a broadcast search when `discover` is on.
#[derive(Clone)]
pub struct Targets {
    pub ips: Vec<Ipv4Addr>,
    pub discover: bool,
}

type Probe = fn(Sender<Box<dyn Backend>>, Instant, Targets);

fn ips(list: &[foxden_net::IpOnly]) -> Vec<Ipv4Addr> {
    list.iter()
        .filter_map(|e| e.ip.trim().parse().ok())
        .filter(|ip| net::is_local(*ip))
        .collect()
}

/// Opens the configured lights of these families, and searches for more when discovery is on.
pub fn probe(config: &NetConfig) -> Vec<Box<dyn Backend>> {
    let at = |list| Targets {
        ips: ips(list),
        discover: config.discover,
    };
    let families: [(Probe, Targets); 3] = [
        (wled::probe, at(&config.wled)),
        (twinkly::probe, at(&config.twinkly)),
        (cololight::probe, at(&config.cololight)),
    ];
    let deadline = Instant::now() + PROBE_BUDGET;
    let (tx, rx) = mpsc::channel::<Box<dyn Backend>>();
    for (f, t) in families {
        if t.ips.is_empty() && !t.discover {
            continue;
        }
        let tx = tx.clone();
        std::thread::Builder::new().name("foxden-net-probe".into()).spawn(move || f(tx, deadline, t)).expect("thread start");
    }
    drop(tx);
    let mut out: Vec<Box<dyn Backend>> = Vec::new();
    while let Ok(b) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        let id = b.devices().first().map(|d| d.id.clone());
        if !out
            .iter()
            .any(|o| o.devices().first().map(|d| d.id.clone()) == id)
        {
            out.push(b);
        }
    }
    out
}
