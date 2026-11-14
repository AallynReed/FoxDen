use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;

use windows_service::service::{
    Service, ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
    ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::{host, pipe};

const NAME: &str = "foxrgb-service";
const DISPLAY_NAME: &str = "FoxRGB Service";
const DESCRIPTION: &str = "Controls fans and lighting for FoxRGB in the background.";
/// The argument the installed service is launched with.
pub const SERVICE_ARG: &str = "service";

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

pub fn install() -> Result<(), String> {
    let access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let manager = ServiceManager::local_computer(None::<&str>, access).map_err(describe)?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe().map_err(|e| e.to_string())?,
        launch_arguments: vec![SERVICE_ARG.into()],
        dependencies: Vec::new(),
        // No account name means LocalSystem.
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG)
        .map_err(describe)?;
    service.set_description(DESCRIPTION).map_err(describe)
}

pub fn uninstall() -> Result<(), String> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    if service.query_status().map_err(describe)?.current_state != ServiceState::Stopped {
        service.stop().map_err(describe)?;
    }
    service.delete().map_err(describe)
}

pub fn start() -> Result<(), String> {
    open(ServiceAccess::START)?
        .start::<&str>(&[])
        .map_err(describe)
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
    service_dispatcher::start(NAME, ffi_service_main).map_err(describe)
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run() {
        host::log(&host::data_dir(), &format!("service failed: {e}"));
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

    // No hardware backend exists yet, so the service idles with none.
    let running = match host::start(host::data_dir(), Vec::new(), pipe::PIPE_NAME) {
        Ok(running) => running,
        Err(e) => {
            let _ = handle.set_service_status(status(ServiceState::Stopped, 1));
            return Err(e);
        }
    };
    if let Err(e) = handle.set_service_status(status(ServiceState::Running, 0)) {
        running.stop();
        return Err(describe(e));
    }
    let _ = stop_rx.recv();
    let _ = handle.set_service_status(status(ServiceState::StopPending, 0));
    running.stop();
    handle
        .set_service_status(status(ServiceState::Stopped, 0))
        .map_err(describe)
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
        assert_eq!(
            status(ServiceState::Stopped, 1).exit_code,
            ServiceExitCode::ServiceSpecific(1)
        );
    }
}
