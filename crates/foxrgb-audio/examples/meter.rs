use std::thread;
use std::time::Duration;

use foxrgb_audio::AudioCapture;

/// Prints the loopback spectrum for five seconds.
fn main() -> Result<(), String> {
    let capture = AudioCapture::start()?;
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        let frame = capture.frame();
        let bars: String = frame
            .bands
            .iter()
            .map(|b| b" .:-=+*#%@"[(b * 9.0).round() as usize] as char)
            .collect();
        println!("|{bars}| level {:.2}", frame.level);
    }
    Ok(())
}
