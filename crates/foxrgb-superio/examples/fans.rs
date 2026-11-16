fn main() {
    for mut backend in foxrgb_superio::backends() {
        println!("{}", backend.name());
        for mut sensor in backend.sensors() {
            let value = sensor.read();
            println!("  {} ({}): {value:?} C", sensor.name(), sensor.id());
        }
        for mut fan in backend.fans() {
            let rpm = fan.rpm();
            println!("  {} ({}): {rpm:?} rpm", fan.name(), fan.id());
        }
    }
}
