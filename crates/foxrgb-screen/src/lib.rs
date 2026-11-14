mod gdi;

use foxrgb_core::{Color, ScreenFrame};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const WIDTH: usize = 64;
pub const HEIGHT: usize = 36;
const INTERVAL: Duration = Duration::from_millis(50);
/// Share of each new capture in the output; the rest is the previous frame.
const BLEND: f32 = 0.6;

#[derive(Default)]
struct Shared {
    frame: ScreenFrame,
    captures: u32,
    capture_time: Duration,
}

/// Captures the primary monitor, downscaled, for screen ambience.
pub struct ScreenCapture {
    shared: Arc<Mutex<Shared>>,
    worker: Option<(mpsc::Sender<()>, JoinHandle<()>)>,
}

impl ScreenCapture {
    /// Starts capture on a background thread and waits for its first frame.
    /// Capture stops when the value is dropped.
    pub fn start() -> Result<Self, String> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (stop, stopped) = mpsc::channel();
        let (ready, is_ready) = mpsc::channel();
        let worker_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("foxrgb-screen".into())
            .spawn(move || run(&worker_shared, &stopped, ready))
            .map_err(|e| format!("screen capture thread: {e}"))?;
        match is_ready.recv() {
            Ok(Ok(())) => Ok(ScreenCapture {
                shared,
                worker: Some((stop, thread)),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("screen capture thread exited early".into()),
        }
    }

    /// Latest downscaled frame. Cheap; called once per rendered frame.
    pub fn frame(&self) -> ScreenFrame {
        self.shared.lock().unwrap().frame.clone()
    }

    /// Number of captures so far and the total time they took.
    pub fn timing(&self) -> (u32, Duration) {
        let shared = self.shared.lock().unwrap();
        (shared.captures, shared.capture_time)
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        if let Some((stop, thread)) = self.worker.take() {
            drop(stop);
            let _ = thread.join();
        }
    }
}

fn run(
    shared: &Mutex<Shared>,
    stopped: &mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let mut grabber = match gdi::Grabber::new(WIDTH, HEIGHT) {
        Ok(grabber) => grabber,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // Reported after the first capture, so the caller never renders an empty frame.
    let mut ready = Some(ready);
    let mut next = Instant::now();
    loop {
        let started = Instant::now();
        // A failed grab (locked desktop, UAC prompt) keeps the last frame.
        if let Ok(bgra) = grabber.grab() {
            let frame = bgra_to_frame(bgra, WIDTH, HEIGHT);
            let took = started.elapsed();
            let mut shared = shared.lock().unwrap();
            blend(&mut shared.frame, &frame, BLEND);
            shared.captures = shared.captures.wrapping_add(1);
            shared.capture_time += took;
        }
        if let Some(ready) = ready.take() {
            let _ = ready.send(Ok(()));
        }
        let now = Instant::now();
        next = next_capture(next, now, now - started);
        if stopped.recv_timeout(next - now) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

/// Fixed deadlines hold the rate, but the wait is never shorter than the capture took,
/// so a slow screen read uses at most half the thread's time.
fn next_capture(deadline: Instant, now: Instant, took: Duration) -> Instant {
    (deadline + INTERVAL).max(now + took)
}

/// Converts top-down 32-bit BGRA rows into a frame. Empty if the buffer is too short.
fn bgra_to_frame(bgra: &[u8], width: usize, height: usize) -> ScreenFrame {
    if bgra.len() < width * height * 4 {
        return ScreenFrame::default();
    }
    let pixels = bgra
        .chunks_exact(4)
        .take(width * height)
        .map(|p| Color(p[2], p[1], p[0]))
        .collect();
    ScreenFrame {
        width,
        height,
        pixels,
    }
}

/// Moves `frame` towards `target` by `k` (0..=1). A frame of another size is replaced.
fn blend(frame: &mut ScreenFrame, target: &ScreenFrame, k: f32) {
    if (frame.width, frame.height, frame.pixels.len())
        != (target.width, target.height, target.pixels.len())
    {
        *frame = target.clone();
        return;
    }
    for (pixel, to) in frame.pixels.iter_mut().zip(&target.pixels) {
        *pixel = Color(
            mix(pixel.0, to.0, k),
            mix(pixel.1, to.1, k),
            mix(pixel.2, to.2, k),
        );
    }
}

/// Rounds towards `to`, so repeated blending lands on the target instead of stopping one short.
fn mix(from: u8, to: u8, k: f32) -> u8 {
    let v = from as f32 + (to as f32 - from as f32) * k;
    if to > from {
        v.ceil() as u8
    } else {
        v.floor() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_becomes_rgb_in_row_order() {
        let bgra = [1, 2, 3, 255, 4, 5, 6, 0, 7, 8, 9, 255, 10, 11, 12, 255];
        let frame = bgra_to_frame(&bgra, 2, 2);
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(
            frame.pixels,
            [
                Color(3, 2, 1),
                Color(6, 5, 4),
                Color(9, 8, 7),
                Color(12, 11, 10)
            ]
        );
        assert_eq!(frame.sample(0.9, 0.1), Color(6, 5, 4));
        assert_eq!(frame.sample(0.1, 0.9), Color(9, 8, 7));
    }

    #[test]
    fn extra_bytes_are_ignored_and_short_buffers_give_no_frame() {
        let bgra = [10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255];
        assert_eq!(
            bgra_to_frame(&bgra, 2, 1).pixels,
            [Color(30, 20, 10), Color(60, 50, 40)]
        );
        assert_eq!(bgra_to_frame(&bgra, 2, 2), ScreenFrame::default());
        assert_eq!(bgra_to_frame(&[], 0, 0), ScreenFrame::default());
    }

    fn solid(color: Color) -> ScreenFrame {
        ScreenFrame {
            width: 2,
            height: 1,
            pixels: vec![color; 2],
        }
    }

    #[test]
    fn first_blend_takes_the_target() {
        let mut frame = ScreenFrame::default();
        blend(&mut frame, &solid(Color(9, 9, 9)), 0.5);
        assert_eq!(frame, solid(Color(9, 9, 9)));
    }

    #[test]
    fn blend_moves_part_of_the_way() {
        let mut frame = solid(Color(0, 100, 200));
        blend(&mut frame, &solid(Color(100, 100, 0)), 0.5);
        assert_eq!(frame, solid(Color(50, 100, 100)));
    }

    #[test]
    fn blend_settles_on_the_target() {
        for (from, to) in [
            (Color(0, 255, 1), Color(255, 0, 0)),
            (Color(3, 2, 1), Color::BLACK),
        ] {
            let mut frame = solid(from);
            for _ in 0..12 {
                blend(&mut frame, &solid(to), BLEND);
            }
            assert_eq!(frame, solid(to));
        }
    }

    #[test]
    fn capture_can_be_shared_between_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<ScreenCapture>();
    }

    #[test]
    fn capture_rate_holds_and_slow_captures_back_off() {
        let ms = Duration::from_millis;
        let start = Instant::now();
        assert_eq!(
            next_capture(start, start + ms(14), ms(14)),
            start + INTERVAL
        );
        assert_eq!(next_capture(start, start + ms(80), ms(80)), start + ms(160));
        assert_eq!(next_capture(start, start + ms(500), ms(1)), start + ms(501));
    }

    fn gdi_objects() -> u32 {
        use windows_sys::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
    }

    // The only test that touches GDI, so the process-wide object count is its own.
    #[test]
    fn capture_runs_stops_and_frees_its_handles() {
        let Ok(capture) = ScreenCapture::start() else {
            return;
        };
        let first = capture.frame();
        thread::sleep(INTERVAL * 3);
        // Where the screen can be read at all, the frame is there as soon as start returns.
        if capture.timing().0 > 0 {
            for frame in [first, capture.frame()] {
                assert_eq!(
                    (frame.width, frame.height, frame.pixels.len()),
                    (WIDTH, HEIGHT, WIDTH * HEIGHT)
                );
            }
        }
        let shared = capture.shared.clone();
        drop(capture);
        assert_eq!(Arc::strong_count(&shared), 1, "worker outlived the capture");

        let before = gdi_objects();
        for _ in 0..8 {
            let capture = ScreenCapture::start().unwrap();
            thread::sleep(INTERVAL);
            drop(capture);
        }
        let after = gdi_objects();
        assert!(after <= before, "GDI objects went from {before} to {after}");

        let capture = ScreenCapture::start().unwrap();
        thread::sleep(INTERVAL * 2);
        let early = gdi_objects();
        thread::sleep(INTERVAL * 10);
        let late = gdi_objects();
        drop(capture);
        // One more is the screen DC of a grab in progress.
        assert!(
            late <= early + 1,
            "GDI objects went from {early} to {late} while capturing"
        );
    }
}
