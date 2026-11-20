//! Captures for two seconds, prints timings and saves the last frame.
//! Usage: cargo run -p foxden-screen --example grab -- [out.ppm]

use foxden_screen::ScreenCapture;
use std::time::{Duration, Instant};

fn main() -> Result<(), String> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "screen.ppm".into());
    let capture = ScreenCapture::start()?;
    let started = Instant::now();
    let (mut seen, mut spent) = (0, Duration::ZERO);
    while started.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(100));
        let (captures, time) = capture.timing();
        if captures > seen {
            let each = (time - spent) / (captures - seen);
            println!(
                "captures {:>2}..{captures:<2}  {:.2} ms each",
                seen + 1,
                each.as_secs_f64() * 1e3
            );
        }
        (seen, spent) = (captures, time);
    }
    let frame = capture.frame();
    drop(capture);
    if seen == 0 || frame.pixels.is_empty() {
        return Err("no frame was captured".into());
    }

    let ran = started.elapsed().as_secs_f64();
    let each = (spent / seen).as_secs_f64() * 1e3;
    println!(
        "{seen} captures in {ran:.2} s ({:.1} per second), {each:.2} ms each on average",
        seen as f64 / ran
    );

    let count = frame.pixels.len() as u32;
    let sum = frame.pixels.iter().fold([0u32; 3], |s, c| {
        [s[0] + c.0 as u32, s[1] + c.1 as u32, s[2] + c.2 as u32]
    });
    println!(
        "average colour: rgb({}, {}, {})",
        sum[0] / count,
        sum[1] / count,
        sum[2] / count
    );

    let mut ppm = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    ppm.extend(frame.pixels.iter().flat_map(|c| [c.0, c.1, c.2]));
    std::fs::write(&out, ppm).map_err(|e| format!("{out}: {e}"))?;
    println!("wrote {out} ({}x{})", frame.width, frame.height);
    Ok(())
}
