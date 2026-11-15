mod close;
mod guard;
mod host;
mod pipe;
mod service;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

const USAGE: &str = "usage: foxrgb-service <command>

  run [--parent <pid>]
               run in this console until Ctrl+C or a shutdown request
               (--parent also ends the run when the process with that id ends)
  install      register the Windows service, started at boot
  uninstall    stop and remove the service
  start        start the installed service
  stop         stop the installed service

install, uninstall, start and stop need an elevated prompt. install only registers
a copy of this program that ordinary users cannot replace, such as one in a folder
under Program Files, and the service keeps its files in a folder closed to them.";

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_console_event(kind: u32) -> i32 {
    INTERRUPTED.store(true, Ordering::Relaxed);
    // Past Ctrl+C and Ctrl+Break the process dies when this returns, so wait for the release.
    if kind > 1 {
        thread::sleep(Duration::from_secs(4));
    }
    1
}

/// The process that started this one and must not be outlived: the app.
struct Parent(HANDLE);

impl Parent {
    fn open(pid: u32) -> Result<Parent, String> {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            let e = std::io::Error::last_os_error();
            return Err(format!("cannot watch process {pid}: {e}"));
        }
        Ok(Parent(handle))
    }

    fn gone(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// What follows `run`: the process to end with. `None` if it makes no sense.
fn run_options(args: &[&str]) -> Option<Option<u32>> {
    match args {
        [] => Some(None),
        ["--parent", pid] => Some(Some(pid.parse().ok()?)),
        _ => None,
    }
}

fn run(parent: Option<u32>) -> Result<(), String> {
    // Opened before anything is driven: a parent that has already gone leaves nothing running.
    let parent = parent.map(Parent::open).transpose()?;
    if unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) } == 0 {
        return Err("cannot install the Ctrl+C handler".into());
    }
    let dir = host::data_dir(host::Mode::Console);
    let running = host::start(
        dir.clone(),
        host::hardware,
        host::Mode::Console,
        pipe::PIPE_NAME,
    )?;
    let outcome = loop {
        if INTERRUPTED.load(Ordering::Relaxed) {
            break Ok(());
        }
        // The app may have been killed, which leaves it no chance to ask for a shutdown.
        if parent.as_ref().is_some_and(Parent::gone) {
            host::log(&dir, "the process that started this service has ended");
            break Ok(());
        }
        if let Some(outcome) = running.ended() {
            break outcome;
        }
        thread::sleep(Duration::from_millis(100));
    };
    running.stop();
    outcome
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["run", options @ ..] => match run_options(options) {
            Some(parent) => run(parent),
            None => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        },
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

#[cfg(test)]
mod tests {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use super::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[test]
    fn run_takes_an_optional_parent() {
        assert_eq!(run_options(&[]), Some(None));
        assert_eq!(run_options(&["--parent", "4321"]), Some(Some(4321)));
        for bad in [
            &["--parent"][..],
            &["--parent", "app"],
            &["--parent", "-4"],
            &["--fast"],
            &["--sim"],
            &["--parent", "8", "--parent", "9"],
        ] {
            assert_eq!(run_options(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_parent_is_gone_once_its_process_has_ended() {
        let me = Parent::open(std::process::id()).unwrap();
        assert!(!me.gone());

        let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd".into());
        let mut child = Command::new(shell)
            .args(["/C", "exit"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let parent = Parent::open(child.id()).unwrap();
        child.wait().unwrap();
        assert!(parent.gone());
    }

    #[test]
    fn a_process_that_is_not_there_cannot_be_a_parent() {
        // Process ids are multiples of four, so nothing has this one.
        let refused = Parent::open(u32::MAX - 2).err().unwrap();
        assert!(refused.contains("cannot watch process"), "{refused}");
    }
}
