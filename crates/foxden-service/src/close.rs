//! Closes conflicting software from the built-in list, as SYSTEM.

use foxden_core::conflicts::{self, Closed, Process, Scope};
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};

/// Stops the listed services and ends listed processes in `session` only; not under `cargo test`.
pub fn run(scope: Scope, session: Option<u32>) -> Closed {
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
    let Some(session) = session else {
        return out;
    };
    let running: Vec<Process> = processes()
        .into_iter()
        .filter(|p| session_of(p.pid) == Some(session))
        .collect();
    for (process, app) in conflicts::targets(&running, scope, std::process::id()) {
        out.add(app, kill(process, session));
    }
    out
}

fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0;
    (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
}

/// The file name of the program a process runs.
fn image_name(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    };
    if ok == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit(['/', '\\']).next().map(str::to_string)
}

/// Whether the process behind an opened handle is still the one that was listed.
fn same(listed: &Process, image: Option<&str>, session: Option<u32>, wanted: u32) -> bool {
    session == Some(wanted) && image.is_some_and(|name| name.eq_ignore_ascii_case(&listed.name))
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

fn kill(process: &Process, session: u32) -> bool {
    unsafe {
        let access = PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        let handle = OpenProcess(access, 0, process.pid);
        if handle.is_null() {
            return false;
        }
        // Held open, the id can no longer pass to another process; it may have already.
        let image = image_name(handle);
        if !same(process, image.as_deref(), session_of(process.pid), session) {
            CloseHandle(handle);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_program_in_the_callers_session_is_ended() {
        let listed = Process {
            pid: 1234,
            name: "Conflict.exe".into(),
        };
        assert!(same(&listed, Some("conflict.EXE"), Some(2), 2));
        assert!(!same(&listed, Some("notepad.exe"), Some(2), 2));
        assert!(!same(&listed, None, Some(2), 2));
        assert!(!same(&listed, Some("Conflict.exe"), Some(3), 2));
        assert!(!same(&listed, Some("Conflict.exe"), None, 2));
    }

    #[test]
    fn this_process_reports_its_own_image_and_session() {
        let me = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, std::process::id()) };
        assert!(!me.is_null());
        let name = image_name(me).unwrap();
        unsafe { CloseHandle(me) };
        assert!(name.to_lowercase().ends_with(".exe"), "{name}");
        assert!(session_of(std::process::id()).is_some());
    }
}
