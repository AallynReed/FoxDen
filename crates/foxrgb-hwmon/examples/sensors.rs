//! Prints every sensor once a second, for five seconds unless told otherwise.
//! Usage: cargo run -p foxrgb-hwmon --example sensors -- [seconds]

use std::time::{Duration, Instant};

fn main() {
    let seconds = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(5);

    let started = Instant::now();
    let mut backends = foxrgb_hwmon::backends();
    let took = started.elapsed().as_secs_f64() * 1e3;
    println!("{} sensor sources found in {took:.0} ms", backends.len());

    let mut sensors = Vec::new();
    for backend in &mut backends {
        let found = backend.sensors();
        let fans = backend.fans().len();
        println!("{}: {} sensors, {fans} fans", backend.name(), found.len());
        sensors.extend(found);
    }

    for second in 0..seconds {
        if second > 0 {
            std::thread::sleep(Duration::from_secs(1));
        }
        println!("t+{second}s");
        for sensor in &mut sensors {
            let at = Instant::now();
            let reading = sensor.read();
            let took = at.elapsed().as_secs_f64() * 1e3;
            let reading = reading.map_or("none".to_string(), |celsius| format!("{celsius:.1} C"));
            println!(
                "  {:<18} {reading:>8}  {took:5.2} ms  {}",
                sensor.id(),
                sensor.name()
            );
        }
    }
}
