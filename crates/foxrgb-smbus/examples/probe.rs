fn main() {
    let bus = match foxrgb_smbus::Bus::open() {
        Ok(bus) => bus,
        Err(e) => return println!("open: {e}"),
    };
    println!("bus open");
    for addr in (0x18..=0x1Fu8).chain(0x50..=0x5F) {
        let quick = bus.write_quick(addr).is_ok();
        let r43 = bus.read_byte(addr, 0x43);
        let r44 = bus.read_byte(addr, 0x44);
        println!("{addr:#04x}: quick={quick} 0x43={r43:02x?} 0x44={r44:02x?}");
    }
    match foxrgb_smbus::probe() {
        Ok(mut found) => {
            println!("{} devices", found.len());
            for backend in &mut found {
                for d in backend.devices() {
                    let red = vec![foxrgb_core::Color(255, 0, 0); d.leds.len()];
                    println!(
                        "{} {} leds={}: {:?}",
                        d.id,
                        d.name,
                        d.leds.len(),
                        backend.push(&d.id, &red)
                    );
                }
            }
        }
        Err(e) => println!("probe: {e}"),
    }
    std::io::stdin().read_line(&mut String::new()).ok();
}
