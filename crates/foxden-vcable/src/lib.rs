//! VB-Audio's VB-CABLE as FoxDen's virtual microphone: finding it by hardware id, installing and
//! removing it with VB-Audio's own setup, and naming its two endpoints.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_PRESENT, SP_DEVINFO_DATA, SPDRP_HARDWAREID, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{ERROR_SUCCESS, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED, EDataFlow,
    ERole, IMMDevice, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, eAll, eCapture,
    eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PVCHF_DEFAULT, PropVariantChangeType,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
    STGM_READWRITE,
};
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, REG_DWORD, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::core::{GUID, HRESULT, IUnknown, Interface, PCWSTR, w};

pub const HARDWARE_ID: &str = "VBAudioVACWDM";
pub const SETUP: &str = "VBCABLE_Setup_x64.exe";
pub const DONATE: &str = "https://vb-audio.com/Cable/";
const BASE: &str = "FoxDen VB-Cable";
const MIC: &str = "Microphone";
const FEED: &str = "Mic feed";
pub const MIC_NAME: &str = "FoxDen VB-Cable (Microphone)";
pub const FEED_NAME: &str = "FoxDen VB-Cable (Mic feed)";

const MEDIA_CLASS: GUID = GUID::from_u128(0x4d36e96c_e325_11ce_bfc1_08002be10318);
const MATCHING_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa8b865dd_2e3d_4094_ad97_e593a70c75d6),
    pid: 8,
};
const DEVICE_DESC: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 2,
};
const INTERFACE_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc),
    pid: 6,
};
const FORM_FACTOR: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e),
    pid: 0,
};
const LINE_LEVEL: &str = "2";
const SETUP_TIMEOUT: Duration = Duration::from_secs(180);
const ENDPOINT_WAIT: Duration = Duration::from_secs(30);

pub type Result<T> = std::result::Result<T, String>;

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// Windows' endpoint id.
    pub id: String,
    /// The name apps and cpal show.
    pub name: String,
    pub capture: bool,
    pub active: bool,
    /// Not the extra 16-channel playback endpoint.
    pub main: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The driver's device is present.
    pub driver: bool,
    /// FoxDen ran VB-Audio's setup, so FoxDen may remove it.
    pub ours: bool,
    /// The recording endpoint apps pick as a microphone.
    pub mic: Option<Endpoint>,
    /// The playback endpoint FoxDen plays the mic into.
    pub feed: Option<Endpoint>,
}

impl Status {
    pub fn ready(&self) -> bool {
        self.mic.as_ref().is_some_and(|e| e.active) && self.feed.as_ref().is_some_and(|e| e.active)
    }

    pub fn renamed(&self) -> bool {
        self.mic.as_ref().is_some_and(|e| e.name == MIC_NAME)
            && self.feed.as_ref().is_some_and(|e| e.name == FEED_NAME)
    }
}

pub fn com_init() {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

/// What is on the machine now. COM must be initialised on this thread.
pub fn status() -> Status {
    let (mic, feed) = pick(endpoints().unwrap_or_default());
    Status {
        driver: driver_present(),
        ours: ours(),
        mic,
        feed,
    }
}

/// The capture and render endpoints, active ones first.
fn pick(mut list: Vec<Endpoint>) -> (Option<Endpoint>, Option<Endpoint>) {
    list.retain(|e| e.main);
    list.sort_by_key(|e| !e.active);
    let mic = list.iter().find(|e| e.capture).cloned();
    let feed = list.iter().find(|e| !e.capture).cloned();
    (mic, feed)
}

/// Whether a present device of the media class carries VB-CABLE's hardware id.
pub fn driver_present() -> bool {
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(Some(&MEDIA_CLASS), None, None, DIGCF_PRESENT) else {
            return false;
        };
        let mut found = false;
        let mut i = 0;
        loop {
            let mut data = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, i, &mut data).is_err() {
                break;
            }
            i += 1;
            let mut buf = [0u8; 1024];
            let mut needed = 0;
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                &data,
                SPDRP_HARDWAREID,
                None,
                Some(&mut buf),
                Some(&mut needed),
            )
            .is_err()
            {
                continue;
            }
            let wide: Vec<u16> = buf[..(needed as usize).min(buf.len())]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if multi_sz(&wide).any(|id| id.eq_ignore_ascii_case(HARDWARE_ID)) {
                found = true;
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

fn multi_sz(wide: &[u16]) -> impl Iterator<Item = String> + '_ {
    wide.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(err)
}

fn text(dev: &IMMDevice, key: &PROPERTYKEY) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(key))
    }
    .map(|v| v.to_string())
    .unwrap_or_default()
}

fn id_of(dev: &IMMDevice) -> Result<String> {
    unsafe {
        let id = dev.GetId().map_err(err)?;
        let out = id.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(id.0 as *const c_void));
        out
    }
}

fn vb_devices() -> Result<Vec<(IMMDevice, Endpoint)>> {
    let states =
        DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0);
    let devs = unsafe { enumerator()?.EnumAudioEndpoints(eAll, states) }.map_err(err)?;
    let mut out = Vec::new();
    for i in 0..unsafe { devs.GetCount() }.map_err(err)? {
        let Ok(dev) = (unsafe { devs.Item(i) }) else {
            continue;
        };
        if !text(&dev, &MATCHING_ID).eq_ignore_ascii_case(HARDWARE_ID) {
            continue;
        }
        let capture = dev
            .cast::<IMMEndpoint>()
            .and_then(|e| unsafe { e.GetDataFlow() })
            .is_ok_and(|f| f == eCapture);
        let active = unsafe { dev.GetState() }.is_ok_and(|s| s == DEVICE_STATE_ACTIVE);
        let e = Endpoint {
            id: id_of(&dev)?,
            name: text(&dev, &PKEY_Device_FriendlyName),
            capture,
            active,
            main: capture || text(&dev, &FORM_FACTOR) != LINE_LEVEL,
        };
        out.push((dev, e));
    }
    Ok(out)
}

/// VB-CABLE's endpoints, found by hardware id whatever they are called.
pub fn endpoints() -> Result<Vec<Endpoint>> {
    Ok(vb_devices()?.into_iter().map(|(_, e)| e).collect())
}

/// A string value of the type Windows keeps endpoint names in.
fn wide_text(s: &str) -> Result<PROPVARIANT> {
    let mut out = PROPVARIANT::default();
    unsafe { PropVariantChangeType(&mut out, &PROPVARIANT::from(s), PVCHF_DEFAULT, VT_LPWSTR) }
        .map_err(err)?;
    Ok(out)
}

/// Names the endpoints FoxDen's way. Needs administrator rights; returns how many changed.
pub fn rename() -> Result<usize> {
    let mut changed = 0;
    let mut seen = (false, false);
    for (dev, e) in vb_devices()? {
        let first = if e.capture { &mut seen.0 } else { &mut seen.1 };
        if !e.main || std::mem::replace(first, true) {
            continue;
        }
        let want = if e.capture { MIC_NAME } else { FEED_NAME };
        if e.name == want {
            continue;
        }
        let part = if e.capture { MIC } else { FEED };
        unsafe {
            let store = dev.OpenPropertyStore(STGM_READWRITE).map_err(err)?;
            store
                .SetValue(&DEVICE_DESC, &wide_text(BASE)?)
                .map_err(err)?;
            store
                .SetValue(&INTERFACE_NAME, &wide_text(part)?)
                .map_err(err)?;
            store.Commit().map_err(err)?;
        }
        changed += 1;
    }
    changed += hide_extras()?;
    Ok(changed)
}

#[repr(C)]
struct PolicyVtbl {
    _before: [usize; 13],
    set_default: unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT,
    set_visibility: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
}

#[repr(transparent)]
#[derive(Clone)]
struct PolicyConfig(IUnknown);

unsafe impl Interface for PolicyConfig {
    type Vtable = PolicyVtbl;
    const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
}

const POLICY_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const ROLES: [ERole; 3] = [eConsole, eMultimedia, eCommunications];

/// Windows' default playback and recording endpoints for each role.
#[derive(Clone, Debug, Default)]
pub struct Defaults(Vec<(EDataFlow, ERole, String)>);

/// The defaults now, to put back after VB-Audio's setup makes VB-CABLE the default.
pub fn defaults() -> Defaults {
    let Ok(en) = enumerator() else {
        return Defaults::default();
    };
    let mut out = Vec::new();
    for flow in [eRender, eCapture] {
        for role in ROLES {
            if let Ok(id) = unsafe { en.GetDefaultAudioEndpoint(flow, role) }
                .map_err(err)
                .and_then(|d| id_of(&d))
            {
                out.push((flow, role, id));
            }
        }
    }
    Defaults(out)
}

/// Puts the defaults back where VB-CABLE took them over. Returns how many were restored.
/// Disables VB-CABLE's extra 16-channel playback endpoint, as Sound settings' Disable does.
/// FoxDen never uses it, and anything played into it would come out of the virtual mic.
pub fn hide_extras() -> Result<usize> {
    let extra: Vec<String> = endpoints()?
        .into_iter()
        .filter(|e| !e.main && e.active)
        .map(|e| e.id)
        .collect();
    if extra.is_empty() {
        return Ok(0);
    }
    let policy: PolicyConfig =
        unsafe { CoCreateInstance(&POLICY_CLIENT, None, CLSCTX_ALL) }.map_err(err)?;
    let vt = unsafe { &**(policy.as_raw() as *const *const PolicyVtbl) };
    for id in &extra {
        let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
        unsafe { (vt.set_visibility)(policy.as_raw(), PCWSTR(wide.as_ptr()), 0) }
            .ok()
            .map_err(err)?;
    }
    Ok(extra.len())
}

pub fn restore(before: &Defaults) -> Result<usize> {
    let ours: Vec<String> = endpoints()?.into_iter().map(|e| e.id).collect();
    let now = defaults();
    let mut done = 0;
    for (flow, role, id) in &before.0 {
        let taken = now
            .0
            .iter()
            .any(|(f, r, cur)| f == flow && r == role && ours.contains(cur));
        if !taken || ours.contains(id) {
            continue;
        }
        set_default_for(id, &[*role])?;
        done += 1;
    }
    Ok(done)
}

/// Makes an endpoint Windows' default for every role of its flow.
pub fn set_default(id: &str) -> Result<()> {
    set_default_for(id, &ROLES)
}

fn set_default_for(id: &str, roles: &[ERole]) -> Result<()> {
    let policy: PolicyConfig =
        unsafe { CoCreateInstance(&POLICY_CLIENT, None, CLSCTX_ALL) }.map_err(err)?;
    let vt = unsafe { &**(policy.as_raw() as *const *const PolicyVtbl) };
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    for role in roles {
        unsafe { (vt.set_default)(policy.as_raw(), PCWSTR(wide.as_ptr()), *role) }
            .ok()
            .map_err(err)?;
    }
    Ok(())
}

/// Where VB-Audio's files sit: `vbcable` next to the program.
pub fn files_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join("vbcable")
}

const KEY: windows::core::PCWSTR = w!(r"SOFTWARE\FoxDen");
const VALUE: windows::core::PCWSTR = w!("VirtualMicInstalled");

/// Whether FoxDen installed VB-CABLE, as recorded in the machine's registry.
pub fn ours() -> bool {
    let mut v = 0u32;
    let mut len = 4u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut len),
        )
    };
    read == ERROR_SUCCESS && v == 1
}

fn set_ours(on: bool) -> Result<()> {
    let one = 1u32;
    let done = unsafe {
        if on {
            RegSetKeyValueW(
                HKEY_LOCAL_MACHINE,
                KEY,
                VALUE,
                REG_DWORD.0,
                Some(&one as *const u32 as *const c_void),
                4,
            )
        } else {
            RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, KEY, VALUE)
        }
    };
    if done == ERROR_SUCCESS || (!on && done.0 == 2) {
        Ok(())
    } else {
        Err(format!("cannot record the install: error {}", done.0))
    }
}

fn run_setup(dir: &Path, flag: &str) -> Result<()> {
    let setup = dir.join(SETUP);
    if !setup.exists() {
        return Err(format!("{} is missing", setup.display()));
    }
    let mut child = Command::new(&setup)
        .args([flag, "-h"])
        .current_dir(dir)
        .spawn()
        .map_err(|e| format!("cannot start VB-Audio's setup: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if start.elapsed() > SETUP_TIMEOUT => {
                let _ = child.kill();
                return Err("VB-Audio's setup did not finish".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn wait_for(what: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < ENDPOINT_WAIT {
        if what() {
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }
    what()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// It was already there and is kept as it is.
    Reused,
    /// FoxDen installed it and the endpoints are up.
    Ready,
    /// FoxDen installed it; the endpoints appear after a restart.
    Reboot,
}

/// Installs VB-CABLE from `dir` unless it is there already, then names its endpoints.
/// Needs administrator rights and COM on this thread.
pub fn install(dir: &Path) -> Result<Installed> {
    if driver_present() {
        let _ = wait_for(|| status().ready());
        rename()?;
        return Ok(Installed::Reused);
    }
    let before = defaults();
    run_setup(dir, "-i")?;
    if !wait_for(driver_present) {
        return Err("VB-Audio's setup ran but VB-CABLE is not there".into());
    }
    set_ours(true)?;
    if !wait_for(|| status().ready()) {
        return Ok(Installed::Reboot);
    }
    let _ = restore(&before);
    rename()?;
    Ok(Installed::Ready)
}

/// Removes VB-CABLE only if FoxDen installed it. Returns whether it ran the uninstall.
pub fn remove(dir: &Path) -> Result<bool> {
    if !ours() {
        return Ok(false);
    }
    if driver_present() {
        run_setup(dir, "-u")?;
    }
    set_ours(false)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(capture: bool, active: bool, name: &str) -> Endpoint {
        Endpoint {
            id: name.into(),
            name: name.into(),
            capture,
            active,
            main: true,
        }
    }

    #[test]
    fn active_endpoints_are_picked_first() {
        let (mic, feed) = pick(vec![
            ep(false, false, "old out"),
            ep(true, true, "in"),
            ep(false, true, "out"),
        ]);
        assert_eq!(mic.unwrap().name, "in");
        assert_eq!(feed.unwrap().name, "out");
        assert_eq!(pick(Vec::new()), (None, None));
    }

    #[test]
    fn hardware_ids_split_on_nulls() {
        let wide: Vec<u16> = "VBAudioVACWDM\0ROOT\\X\0\0".encode_utf16().collect();
        assert_eq!(
            multi_sz(&wide).collect::<Vec<_>>(),
            [HARDWARE_ID, "ROOT\\X"]
        );
    }

    #[test]
    fn names_follow_the_pattern() {
        assert_eq!(MIC_NAME, format!("{BASE} ({MIC})"));
        assert_eq!(FEED_NAME, format!("{BASE} ({FEED})"));
    }
}
