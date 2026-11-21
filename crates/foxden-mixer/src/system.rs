//! Windows' outputs, audio sessions, processes and per-app default outputs. COM is used only on
//! the calling thread, which must have called [`com_init`].

use std::ffi::c_void;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eCommunications,
    eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, h};

use crate::Result;

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

pub fn com_init() {
    // Already initialised in another mode is fine too.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(err)
}

fn name_of(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
    }
    .map(|v| v.to_string())
    .unwrap_or_default()
}

fn id_of(dev: &IMMDevice) -> Result<String> {
    unsafe {
        let id = dev.GetId().map_err(err)?;
        let out = id.to_string().map_err(|e| e.to_string());
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const c_void));
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    /// The same name cpal gives the device.
    pub name: String,
    pub default: bool,
}

/// Active outputs.
pub fn endpoints() -> Result<Vec<Endpoint>> {
    let en = enumerator()?;
    let default = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) }
        .ok()
        .and_then(|d| id_of(&d).ok());
    let devs = unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.map_err(err)?;
    let mut out = Vec::new();
    for i in 0..unsafe { devs.GetCount() }.map_err(err)? {
        let Ok(dev) = (unsafe { devs.Item(i) }) else {
            continue;
        };
        let id = id_of(&dev)?;
        out.push(Endpoint {
            default: default.as_deref() == Some(id.as_str()),
            name: name_of(&dev),
            id,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: u32,
    /// Id of the output it plays on.
    pub endpoint: String,
    pub active: bool,
}

/// Every app's audio session on every active output, system sounds left out.
pub fn sessions() -> Result<Vec<Session>> {
    let en = enumerator()?;
    let devs = unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.map_err(err)?;
    let mut out = Vec::new();
    for i in 0..unsafe { devs.GetCount() }.map_err(err)? {
        let Ok(dev) = (unsafe { devs.Item(i) }) else {
            continue;
        };
        let endpoint = id_of(&dev)?;
        let Ok(mgr) = (unsafe { dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else {
            continue;
        };
        let Ok(list) = (unsafe { mgr.GetSessionEnumerator() }) else {
            continue;
        };
        for k in 0..unsafe { list.GetCount() }.unwrap_or(0) {
            let Ok(s) = (unsafe { list.GetSession(k) }) else {
                continue;
            };
            let Ok(s2) = s.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if unsafe { s2.IsSystemSoundsSession() } == windows::Win32::Foundation::S_OK {
                continue;
            }
            let pid = unsafe { s2.GetProcessId() }.unwrap_or(0);
            if pid == 0 {
                continue;
            }
            let active = unsafe { s.GetState() }.is_ok_and(|st| st == AudioSessionStateActive);
            out.push(Session {
                pid,
                endpoint: endpoint.clone(),
                active,
            });
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

pub fn processes() -> Vec<Process> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            out.push(Process {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                exe: String::from_utf16_lossy(&e.szExeFile[..len]),
            });
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn endpoint_volume(id: &str) -> Result<IAudioEndpointVolume> {
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let dev = unsafe { enumerator()?.GetDevice(PCWSTR(wide.as_ptr())) }.map_err(err)?;
    unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(err)
}

pub fn endpoint_muted(id: &str) -> Result<bool> {
    unsafe { endpoint_volume(id)?.GetMute() }
        .map(|b| b.as_bool())
        .map_err(err)
}

pub fn set_endpoint_muted(id: &str, muted: bool) -> Result<()> {
    unsafe { endpoint_volume(id)?.SetMute(muted, std::ptr::null()) }.map_err(err)
}

#[repr(C)]
struct PolicyVtbl {
    _inspectable: [usize; 6],
    _other: [usize; 19],
    set: unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, *mut c_void) -> HRESULT,
    get: unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, *mut *mut c_void) -> HRESULT,
}

macro_rules! factory {
    ($name:ident, $iid:expr) => {
        #[repr(transparent)]
        #[derive(Clone)]
        struct $name(IUnknown);
        unsafe impl Interface for $name {
            type Vtable = PolicyVtbl;
            const IID: GUID = GUID::from_u128($iid);
        }
    };
}
// The layout from Windows 10 21H2 on, then the one before it.
factory!(Policy21H2, 0xab3d4648_e242_459f_b02f_541c70306324);
factory!(PolicyOld, 0x2a59116d_6c4f_45e0_a74f_707e3fef9258);

const POLICY_CLASS: &HSTRING = h!("Windows.Media.Internal.AudioPolicyConfig");
const MMDEVAPI: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER_INTERFACE: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";

/// The per-app default output that Windows' "App volume and device preferences" sets. Windows
/// keeps it per executable, across restarts, and only takes a pid that has used audio.
pub struct AppOutputs {
    factory: IUnknown,
}

impl AppOutputs {
    pub fn new() -> Result<AppOutputs> {
        let factory = match unsafe { RoGetActivationFactory::<Policy21H2>(POLICY_CLASS) } {
            Ok(f) => f.0,
            Err(_) => {
                unsafe { RoGetActivationFactory::<PolicyOld>(POLICY_CLASS) }
                    .map_err(err)?
                    .0
            }
        };
        Ok(AppOutputs { factory })
    }

    fn vtbl(&self) -> &PolicyVtbl {
        unsafe { &**(self.factory.as_raw() as *const *const PolicyVtbl) }
    }

    /// The endpoint id chosen for this app, empty for the system default.
    pub fn get(&self, pid: u32) -> Result<String> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe { (self.vtbl().get)(self.factory.as_raw(), pid, eRender, eConsole, &mut raw) }
            .ok()
            .map_err(err)?;
        let full = unsafe { std::mem::transmute::<*mut c_void, HSTRING>(raw) }.to_string();
        Ok(full
            .strip_prefix(MMDEVAPI)
            .and_then(|s| s.strip_suffix(RENDER_INTERFACE))
            .unwrap_or(&full)
            .to_string())
    }

    /// Sends the app to an endpoint id, or back to the system default with an empty one.
    pub fn set(&self, pid: u32, endpoint: &str) -> Result<()> {
        let full = (!endpoint.is_empty())
            .then(|| HSTRING::from(format!("{MMDEVAPI}{endpoint}{RENDER_INTERFACE}")));
        let raw = full.as_ref().map_or(std::ptr::null_mut(), |h| unsafe {
            std::mem::transmute_copy::<HSTRING, *mut c_void>(h)
        });
        for role in [eMultimedia, eConsole, eCommunications] {
            unsafe { (self.vtbl().set)(self.factory.as_raw(), pid, eRender, role, raw) }
                .ok()
                .map_err(err)?;
        }
        Ok(())
    }
}
