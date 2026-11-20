//! Installs or removes PawnIO and the FoxDen service as the Lighting and Fans modules need them.

use std::path::Path;
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_FAILED};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

const SERVICE: &str = "foxden-service";
const PAWNIO: &str = "PawnIO";
const ERROR_CANCELLED: u32 = 1223;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    InstallPawnio,
    InstallService,
    StartService,
    UninstallService,
    UninstallPawnio,
}

impl Step {
    fn words(self) -> &'static str {
        match self {
            Step::InstallPawnio => "install PawnIO",
            Step::InstallService => "install the FoxDen service",
            Step::StartService => "start the FoxDen service",
            Step::UninstallService => "remove the FoxDen service",
            Step::UninstallPawnio => "remove PawnIO",
        }
    }

    fn command(self, dir: &Path) -> String {
        let service = dir.join("foxden-service.exe");
        let pawnio = dir.join("PawnIO_setup.exe");
        let (exe, args) = match self {
            Step::InstallPawnio => (pawnio, "-install -silent"),
            Step::InstallService => (service, "install"),
            Step::StartService => (service, "start"),
            Step::UninstallService => (service, "uninstall"),
            Step::UninstallPawnio => (pawnio, "-uninstall -silent"),
        };
        format!("\"{}\" {args}", exe.display())
    }
}

/// What is on the machine now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Present {
    pub pawnio: bool,
    pub service: bool,
    pub running: bool,
}

/// Both modules need PawnIO and the service; with neither on, both go.
pub fn plan(wanted: bool, present: Present) -> Vec<Step> {
    let mut steps = Vec::new();
    if wanted {
        if !present.pawnio {
            steps.push(Step::InstallPawnio);
        }
        if !present.service {
            steps.push(Step::InstallService);
        }
        if !present.running {
            steps.push(Step::StartService);
        }
    } else {
        if present.service {
            steps.push(Step::UninstallService);
        }
        if present.pawnio {
            steps.push(Step::UninstallPawnio);
        }
    }
    steps
}

/// The arguments for one elevated `cmd.exe`; `/s` strips only the outer quotes.
pub fn cmd_args(steps: &[Step], dir: &Path) -> String {
    let chain: Vec<String> = steps.iter().map(|s| s.command(dir)).collect();
    format!("/d /s /c \"{}\"", chain.join(" && "))
}

fn list(steps: &[Step]) -> String {
    let words: Vec<&str> = steps.iter().map(|s| s.words()).collect();
    words.join(", then ")
}

pub fn describe(steps: &[Step]) -> String {
    let mut text = list(steps);
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

fn present() -> Present {
    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    else {
        return Present::default();
    };
    let state = |name: &str| {
        manager
            .open_service(name, ServiceAccess::QUERY_STATUS)
            .ok()
            .map(|s| s.query_status().map(|s| s.current_state))
    };
    let service = state(SERVICE);
    Present {
        pawnio: state(PAWNIO).is_some(),
        service: service.is_some(),
        running: matches!(service, Some(Ok(ServiceState::Running))),
    }
}

#[derive(Clone, Serialize)]
struct Outcome {
    ok: bool,
    text: String,
}

/// Brings the machine in line with the modules on a thread of its own, then emits "modules".
pub fn apply(app: &AppHandle, wanted: bool) {
    let app = app.clone();
    thread::spawn(move || {
        // The fan config is kept and the fans handed back before the service can go.
        crate::fans::settle();
        if !wanted {
            crate::fans::stop_child();
        }
        let steps = plan(wanted, present());
        let outcome = if steps.is_empty() {
            Outcome {
                ok: true,
                text: "Nothing to install or remove".into(),
            }
        } else if cfg!(debug_assertions) {
            Outcome {
                ok: true,
                text: format!(
                    "Development build, so nothing was changed. Would: {}",
                    list(&steps)
                ),
            }
        } else {
            match elevated(&steps) {
                Ok(()) => Outcome {
                    ok: true,
                    text: format!("Done: {}", list(&steps)),
                },
                Err(e) => Outcome { ok: false, text: e },
            }
        };
        let _ = app.emit("modules", outcome);
    });
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Runs the steps in one elevated `cmd.exe`, so there is one UAC prompt.
fn elevated(steps: &[Step]) -> Result<(), String> {
    let dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("no install folder")?
        .to_path_buf();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let cmd = wide(&format!(r"{system}\System32\cmd.exe"));
    let args = wide(&cmd_args(steps, &dir));
    let verb = wide("runas");
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = cmd.as_ptr();
        info.lpParameters = args.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            return Err(match GetLastError() {
                ERROR_CANCELLED => "Cancelled at the administrator prompt".into(),
                code => format!("Could not get administrator rights (error {code})"),
            });
        }
        if info.hProcess.is_null() {
            return Err("The helper did not start".into());
        }
        let waited = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0;
        let read = GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if waited == WAIT_FAILED || read == 0 {
            return Err("Lost track of the helper".into());
        }
        match code {
            0 => Ok(()),
            code => Err(format!("{} failed (exit code {code})", describe(steps))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Present = Present {
        pawnio: true,
        service: true,
        running: true,
    };

    #[test]
    fn only_what_is_missing_or_left_over_is_touched() {
        assert_eq!(
            plan(true, Present::default()),
            [
                Step::InstallPawnio,
                Step::InstallService,
                Step::StartService
            ]
        );
        assert!(plan(true, ALL).is_empty());
        let stopped = Present {
            running: false,
            ..ALL
        };
        assert_eq!(plan(true, stopped), [Step::StartService]);
        assert_eq!(
            plan(false, ALL),
            [Step::UninstallService, Step::UninstallPawnio]
        );
        let no_service = Present {
            pawnio: true,
            ..Present::default()
        };
        assert_eq!(plan(false, no_service), [Step::UninstallPawnio]);
        assert!(plan(false, Present::default()).is_empty());
    }

    #[test]
    fn the_command_keeps_paths_with_spaces_whole() {
        let dir = Path::new(r"C:\Program Files\FoxDen");
        let args = cmd_args(&[Step::UninstallService, Step::UninstallPawnio], dir);
        assert_eq!(
            args,
            r#"/d /s /c ""C:\Program Files\FoxDen\foxden-service.exe" uninstall && "C:\Program Files\FoxDen\PawnIO_setup.exe" -uninstall -silent""#
        );
    }

    #[test]
    fn the_steps_read_as_a_sentence() {
        assert_eq!(
            describe(&[Step::UninstallService, Step::UninstallPawnio]),
            "Remove the FoxDen service, then remove PawnIO"
        );
    }
}
