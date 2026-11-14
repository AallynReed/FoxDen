mod host;
mod pipe;
mod service;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use foxrgb_fan::{FanBackend, SimBackend};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

const USAGE: &str = "usage: foxrgb-service <command>

  run [--sim]  run in this console until Ctrl+C (--sim uses simulated fans)
  install      register the Windows service, started at boot
  uninstall    stop and remove the service
  start        start the installed service
  stop         stop the installed service

install, uninstall, start and stop need an elevated prompt.";

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_console_event(kind: u32) -> i32 {
    INTERRUPTED.store(true, Ordering::Relaxed);
    // Past Ctrl+C and Ctrl+Break the process dies when this returns, so wait for the release.
    if kind > 1 {
        thread::sleep(Duration::from_secs(4));
    }
    1
}

fn run(sim: bool) -> Result<(), String> {
    let mut backends: Vec<Box<dyn FanBackend>> = Vec::new();
    if sim {
        backends.push(Box::<SimBackend>::default());
    }
    if unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) } == 0 {
        return Err("cannot install the Ctrl+C handler".into());
    }
    let running = host::start(host::data_dir(), backends, pipe::PIPE_NAME)?;
    while !INTERRUPTED.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
    }
    running.stop();
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["run"] => run(false),
        ["run", "--sim"] => run(true),
        ["install"] => service::install(),
        ["uninstall"] => service::uninstall(),
        ["start"] => service::start(),
        ["stop"] => service::stop(),
        [service::SERVICE_ARG] => service::dispatch(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("foxrgb-service: {e}");
            ExitCode::FAILURE
        }
    }
}
