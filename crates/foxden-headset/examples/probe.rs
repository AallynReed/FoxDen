//! Read-only: prints the status, audio and device-settings replies of each supported headset.

use std::time::Duration;

use foxden_battery::hid::{Dev, enumerate};

fn main() {
    for c in enumerate()
        .into_iter()
        .filter(|c| c.vid == 0x1038 && c.page == 0xFFC0 && c.usage == 1)
    {
        println!(
            "{:04X} iface {} in {} out {}",
            c.pid, c.iface, c.in_len, c.out_len
        );
        let Some(dev) = Dev::open(&c) else {
            println!("  cannot open");
            continue;
        };
        let _turn = foxden_core::hid_turn::take();
        for cmd in [0xB0u8, 0x20, 0xA0] {
            dev.flush();
            if !dev.write(&[0x00, cmd]) {
                println!("  {cmd:02X}: write failed");
                continue;
            }
            let r = dev.read_until(Duration::from_secs(1), |r| {
                (r.first() == Some(&cmd)).then(|| r.to_vec())
            });
            match r {
                Some(r) => println!("  {cmd:02X}: {:02X?}", &r[..r.len().min(16)]),
                None => println!("  {cmd:02X}: no reply"),
            }
        }
    }
}
