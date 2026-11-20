//! Closes conflicting software from the built-in list, as SYSTEM.

use foxden_core::conflicts::{self, Closed, Process, Scope};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

/// Stops the listed services and ends the listed processes. Never runs under `cargo test`.
pub fn run(scope: Scope) -> Closed {
    let mut out = Closed::default();
    if cfg!(test) {
        return out;
    }
    if let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    {
        for (name, app) in conflicts::services(scope) {
            let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
            let Ok(service) = manager.open_service(name, access) else {
                continue;
            };
            let running = service
                .query_status()
                .is_ok_and(|s| s.current_state != ServiceState::Stopped);
            if running {
                out.add(app, service.stop().is_ok());
            }
        }
    }
    let running = processes();
    for (process, app) in conflicts::targets(&running, scope, std::process::id()) {
        out.add(app, kill(process.pid));
    }
    out
}

fn processes() -> Vec<Process> {
    let mut found = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            found.push(Process {
                pid: entry.th32ProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    found
}

fn kill(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let ok = TerminateProcess(handle, 1) != 0;
        // Until it has exited, the process may still hold its devices.
        if ok {
            WaitForSingleObject(handle, 2000);
        }
        CloseHandle(handle);
        ok
    }
}
