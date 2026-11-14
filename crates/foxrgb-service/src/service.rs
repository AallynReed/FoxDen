use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::{guard, host, pipe};

const NAME: &str = "foxrgb-service";
const DISPLAY_NAME: &str = "FoxRGB Service";
const DESCRIPTION: &str = "Controls fans and lighting for FoxRGB in the background.";
/// The argument the installed service is launched with.
pub const SERVICE_ARG: &str = "service";
/// Exit codes reported to the Service Control Manager.
const COULD_NOT_START: u32 = 1;
const GAVE_UP: u32 = 2;
const UNSAFE_FOLDER: u32 = 3;

static FAILED: AtomicBool = AtomicBool::new(false);

fn describe(e: windows_service::Error) -> String {
    match e {
        windows_service::Error::Winapi(io) => match io.raw_os_error() {
            Some(5) => "access denied: run this from an elevated (Administrator) prompt".into(),
            Some(1063) => {
                "this mode is started by the Service Control Manager; use `run` in a console".into()
            }
            _ => io.to_string(),
        },
        other => other.to_string(),
    }
}

fn open(access: ServiceAccess) -> Result<Service, String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(describe)?;
    manager.open_service(NAME, access).map_err(describe)
}

/// A service that gave up is started again twice, then left stopped until someone looks.
fn failure_actions() -> ServiceFailureActions {
    let after = |action_type, seconds| ServiceAction {
        action_type,
        delay: Duration::from_secs(seconds),
    };
    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            after(ServiceActionType::Restart, 5),
            after(ServiceActionType::Restart, 60),
            after(ServiceActionType::None, 0),
        ]),
    }
}

/// The path of this program with links resolved, as long as only administrators can replace
/// what is there. A service that runs as LocalSystem must not start from anywhere else.
fn program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| e.to_string())?;
    let exe = plain(exe);
    guard::program(&exe)?;
    Ok(exe)
}

/// Without the `\\?\` that resolving a path puts in front of a drive letter.
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

pub fn install() -> Result<(), String> {
    let executable_path = program()?;
    let access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let manager = ServiceManager::local_computer(None::<&str>, access).map_err(describe)?;
    // Only now, with administrator rights certain: made by anyone else it would not be trusted.
    guard::data_dir(&host::data_dir(host::Mode::Service))?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path,
        launch_arguments: vec![SERVICE_ARG.into()],
        dependencies: Vec::new(),
        // No account name means LocalSystem.
        account_name: None,
        account_password: None,
    };
    // Setting a restart action needs the right to start the service.
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .map_err(describe)?;
    service.set_description(DESCRIPTION).map_err(describe)?;
    service
        .update_failure_actions(failure_actions())
        .map_err(describe)?;
    // Otherwise only a crash counts as a failure, not a stop with an error code.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(describe)
}

/// Asks a running service to stop and waits until it has.
fn stop_and_wait(
    mut state: impl FnMut() -> Result<ServiceState, String>,
    mut stop: impl FnMut() -> Result<(), String>,
    patience: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + patience;
    let mut asked = false;
    loop {
        match state()? {
            ServiceState::Stopped => return Ok(()),
            // A service that is still starting or already stopping takes no stop request.
            ServiceState::Running | ServiceState::Paused if !asked => {
                stop()?;
                asked = true;
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("the service did not stop within {patience:?}"));
        }
        thread::sleep(patience / 100);
    }
}

pub fn uninstall() -> Result<(), String> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    // Deleting a service that still runs only marks it, and the name stays taken.
    stop_and_wait(
        || Ok(service.query_status().map_err(describe)?.current_state),
        || service.stop().map(|_| ()).map_err(describe),
        Duration::from_secs(20),
    )
    .map_err(|e| format!("{e}; it was not removed"))?;
    service.delete().map_err(describe)
}

pub fn start() -> Result<(), String> {
    let service = open(ServiceAccess::START)?;
    // The service checks this too, but has nowhere to say what is wrong.
    guard::data_dir(&host::data_dir(host::Mode::Service))?;
    service.start::<&str>(&[]).map_err(describe)
}

pub fn stop() -> Result<(), String> {
    open(ServiceAccess::STOP)?
        .stop()
        .map(|_| ())
        .map_err(describe)
}

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the Service Control Manager; returns when the service has stopped.
pub fn dispatch() -> Result<(), String> {
    service_dispatcher::start(NAME, ffi_service_main).map_err(describe)?;
    if FAILED.load(Ordering::Relaxed) {
        return Err("stopped after a failure, see service.log".into());
    }
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if run().is_err() {
        FAILED.store(true, Ordering::Relaxed);
    }
}

fn status(state: ServiceState, exit_code: u32) -> ServiceStatus {
    let (controls_accepted, wait_hint) = match state {
        ServiceState::Running => (
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            Duration::ZERO,
        ),
        ServiceState::StopPending => (ServiceControlAccept::empty(), Duration::from_secs(5)),
        _ => (ServiceControlAccept::empty(), Duration::ZERO),
    };
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        // ServiceSpecific(0) would still report ERROR_SERVICE_SPECIFIC_ERROR to the SCM.
        exit_code: match exit_code {
            0 => ServiceExitCode::NO_ERROR,
            code => ServiceExitCode::ServiceSpecific(code),
        },
        checkpoint: 0,
        wait_hint,
        process_id: None,
    }
}

fn run() -> Result<(), String> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(describe)?;

    let dir = host::data_dir(host::Mode::Service);
    // Before anything is written, a log line included: the folder may be somebody else's.
    if let Err(e) = guard::data_dir(&dir) {
        let _ = handle.set_service_status(status(ServiceState::Stopped, UNSAFE_FOLDER));
        return Err(e);
    }
    let fail = |e: String| {
        host::log(&dir, &format!("service failed: {e}"));
        e
    };
    let started = host::start(
        dir.clone(),
        foxrgb_hwmon::backends,
        false,
        host::Mode::Service,
        pipe::PIPE_NAME,
    );
    let running = match started {
        Ok(running) => running,
        Err(e) => {
            let _ = handle.set_service_status(status(ServiceState::Stopped, COULD_NOT_START));
            return Err(fail(e));
        }
    };
    if let Err(e) = handle.set_service_status(status(ServiceState::Running, 0)) {
        running.stop();
        return Err(fail(describe(e)));
    }
    // Ends on a stop request, or when the host has given up on its control loop.
    let outcome = loop {
        match stop_rx.recv_timeout(Duration::from_millis(200)) {
            Err(RecvTimeoutError::Timeout) => {
                if let Some(outcome) = running.ended() {
                    break outcome;
                }
            }
            _ => break Ok(()),
        }
    };
    let _ = handle.set_service_status(status(ServiceState::StopPending, 0));
    running.stop();
    let exit_code = if outcome.is_ok() { 0 } else { GAVE_UP };
    handle
        .set_service_status(status(ServiceState::Stopped, exit_code))
        .map_err(describe)?;
    outcome.map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_failure_reports_an_exit_code() {
        for state in [
            ServiceState::Running,
            ServiceState::StopPending,
            ServiceState::Stopped,
        ] {
            assert_eq!(status(state, 0).exit_code, ServiceExitCode::NO_ERROR);
        }
        for code in [COULD_NOT_START, GAVE_UP, UNSAFE_FOLDER] {
            assert_eq!(
                status(ServiceState::Stopped, code).exit_code,
                ServiceExitCode::ServiceSpecific(code)
            );
        }
    }

    #[test]
    fn a_resolved_path_loses_only_the_prefix_of_a_drive() {
        let plain = |path: &str| plain(PathBuf::from(path));
        assert_eq!(
            plain(r"\\?\C:\Program Files\FoxRGB\foxrgb-service.exe"),
            PathBuf::from(r"C:\Program Files\FoxRGB\foxrgb-service.exe")
        );
        let share = r"\\?\UNC\server\share\foxrgb-service.exe";
        assert_eq!(plain(share), PathBuf::from(share));
        assert_eq!(plain(r"C:\Tools\x.exe"), PathBuf::from(r"C:\Tools\x.exe"));
    }

    #[test]
    fn a_failed_service_is_restarted_twice_and_then_left_alone() {
        let actions = failure_actions().actions.unwrap();
        let kinds: Vec<_> = actions.iter().map(|action| action.action_type).collect();
        let (restart, none) = (ServiceActionType::Restart, ServiceActionType::None);
        assert_eq!(kinds, [restart, restart, none]);
        assert!(actions[0].delay < actions[1].delay);
    }

    /// Runs `stop_and_wait` against a service that goes through `states`; returns the result
    /// and how often it was asked to stop.
    fn uninstall_from(states: &[ServiceState]) -> (Result<(), String>, u32) {
        let mut states = states.iter().copied().cycle();
        let mut stops = 0;
        let result = stop_and_wait(
            || Ok(states.next().unwrap()),
            || {
                stops += 1;
                Ok(())
            },
            Duration::from_millis(200),
        );
        (result, stops)
    }

    #[test]
    fn uninstall_waits_for_the_service_to_stop() {
        use ServiceState::{Paused, Running, StartPending, StopPending, Stopped};
        assert_eq!(uninstall_from(&[Stopped]), (Ok(()), 0));
        assert_eq!(uninstall_from(&[StopPending, Stopped]), (Ok(()), 0));
        assert_eq!(
            uninstall_from(&[Running, StopPending, StopPending, Stopped]),
            (Ok(()), 1)
        );
        assert_eq!(uninstall_from(&[Paused, Running, Stopped]), (Ok(()), 1));
        assert_eq!(
            uninstall_from(&[StartPending, Running, StopPending, Stopped]),
            (Ok(()), 1)
        );
    }

    #[test]
    fn uninstall_gives_up_on_a_service_that_never_stops() {
        let (result, stops) = uninstall_from(&[ServiceState::Running]);
        assert_eq!(result, Err("the service did not stop within 200ms".into()));
        assert_eq!(stops, 1);
    }

    #[test]
    fn uninstall_passes_on_a_refused_stop() {
        let result = stop_and_wait(
            || Ok(ServiceState::Running),
            || Err("access denied".into()),
            Duration::from_secs(1),
        );
        assert_eq!(result, Err("access denied".into()));
    }
}
