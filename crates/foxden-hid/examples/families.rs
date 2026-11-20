fn main() {
    let all = foxden_hid::family_devices();
    let mut seen = std::collections::HashMap::new();
    for (vid, pid, name) in &all {
        seen.entry((*vid, *pid))
            .or_insert_with(Vec::new)
            .push(*name);
    }
    println!("{} detectors, {} distinct vid:pid", all.len(), seen.len());
    let present: Vec<_> = foxden_hid::enumerate()
        .iter()
        .filter(|i| i.usb)
        .map(|i| (i.vendor, i.product))
        .collect();
    for ((vid, pid), names) in &seen {
        if present.contains(&(*vid, *pid)) {
            println!("on this PC: {vid:04x}:{pid:04x} {names:?}");
        }
    }
}
