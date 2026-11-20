//! Opened HID collections a hub's lighting and fan drivers can share, with one lock per device.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::{self, ThreadId};

use foxden_core::{Backend, Color, Device as Lit};
use hidapi::HidDevice;
pub use hidapi::HidResult;

use crate::registry::Info;

/// The calls a driver makes on an opened collection.
pub trait Raw: Send {
    fn write(&self, data: &[u8]) -> HidResult<usize>;
    fn read_timeout(&self, buf: &mut [u8], ms: i32) -> HidResult<usize>;
    fn send_feature_report(&self, data: &[u8]) -> HidResult<()>;
    fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize>;
    fn get_input_report(&self, buf: &mut [u8]) -> HidResult<usize>;
}

impl Raw for HidDevice {
    fn write(&self, data: &[u8]) -> HidResult<usize> {
        HidDevice::write(self, data)
    }
    fn read_timeout(&self, buf: &mut [u8], ms: i32) -> HidResult<usize> {
        HidDevice::read_timeout(self, buf, ms)
    }
    fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
        HidDevice::send_feature_report(self, data)
    }
    fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        HidDevice::get_feature_report(self, buf)
    }
    fn get_input_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        HidDevice::get_input_report(self, buf)
    }
}

type Handle = Arc<Mutex<Box<dyn Raw>>>;
type Kept = Weak<Mutex<Box<dyn Raw>>>;
/// Writes a test fake saw, with whether the device lock was held for each.
#[cfg(test)]
pub(crate) type Seen = Arc<Mutex<Vec<(Vec<u8>, bool)>>>;

/// The names a device's drivers give themselves in `Device::set_active`.
pub const LIGHTS: &str = "lights";
pub const FANS: &str = "fans";

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One physical device: its lock, its open collections and what its drivers tell each other.
#[derive(Default)]
pub struct Device {
    owner: Mutex<(Option<ThreadId>, usize)>,
    free: Condvar,
    handles: Mutex<Vec<(String, Kept)>>,
    active: Mutex<Vec<&'static str>>,
    owed: AtomicBool,
    epoch: AtomicU32,
    seq: AtomicU8,
}

/// Holds a device's lock until dropped. The thread holding it may take it again.
pub struct Hold(Arc<Device>);

impl Drop for Hold {
    fn drop(&mut self) {
        let mut owner = locked(&self.0.owner);
        owner.1 -= 1;
        if owner.1 == 0 {
            owner.0 = None;
            self.0.free.notify_one();
        }
    }
}

impl Device {
    pub fn hold(self: &Arc<Self>) -> Hold {
        let me = thread::current().id();
        let mut owner = locked(&self.owner);
        while owner.0.is_some_and(|t| t != me) {
            owner = self.free.wait(owner).unwrap_or_else(|e| e.into_inner());
        }
        *owner = (Some(me), owner.1 + 1);
        Hold(self.clone())
    }

    /// True while this thread holds the lock.
    pub fn held_here(&self) -> bool {
        locked(&self.owner).0 == Some(thread::current().id())
    }

    /// Marks `who` as driving the device, or done with it.
    pub fn set_active(&self, who: &'static str, on: bool) {
        let mut active = locked(&self.active);
        active.retain(|w| *w != who);
        if on {
            active.push(who);
        }
    }

    /// Another driver is driving the device.
    pub fn others_active(&self, who: &'static str) -> bool {
        locked(&self.active).iter().any(|w| *w != who)
    }

    /// A hand-back to firmware was skipped and is owed by whoever lets go last.
    pub fn owe(&self, owed: bool) {
        self.owed.store(owed, Ordering::Relaxed);
    }

    pub fn owed(&self) -> bool {
        self.owed.load(Ordering::Relaxed)
    }

    /// Counts the times one driver changed device state the others may have to send again.
    pub fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// A packet sequence 1..=31 that every driver of the device steps.
    pub fn next_seq(&self) -> u8 {
        let step = |s: u8| Some(if s < 31 { s + 1 } else { 1 });
        let old = self
            .seq
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, step);
        step(old.unwrap_or(0)).unwrap_or(1)
    }
}

static SHARED: Mutex<Vec<(u16, u16)>> = Mutex::new(Vec::new());
static DEVICES: Mutex<Vec<(String, Weak<Device>)>> = Mutex::new(Vec::new());

/// Has this process open each listed device once and serialize its drivers. Off by default.
pub fn share(ids: &[(u16, u16)]) {
    let mut shared = locked(&SHARED);
    for id in ids {
        if !shared.contains(id) {
            shared.push(*id);
        }
    }
}

pub fn is_shared(vid: u16, pid: u16) -> bool {
    locked(&SHARED).contains(&(vid, pid))
}

/// The shared device a collection belongs to, or `None` when sharing is off for it.
pub fn device_for(info: &Info) -> Option<Arc<Device>> {
    if !is_shared(info.vid, info.pid) {
        return None;
    }
    let key = if info.serial.is_empty() {
        let at = usb_parent(&info.path).unwrap_or_else(|| info.path.clone());
        format!("{:04x}{:04x}{at}", info.vid, info.pid)
    } else {
        info.key()
    };
    let mut devices = locked(&DEVICES);
    devices.retain(|(_, d)| d.strong_count() > 0);
    if let Some(d) = devices
        .iter()
        .find_map(|(k, d)| (*k == key).then(|| d.upgrade()))
    {
        return d;
    }
    let d = Arc::new(Device::default());
    devices.push((key, Arc::downgrade(&d)));
    Some(d)
}

/// The device instance id in a HID or WinUSB path: `\\?\hid#a#b#{guid}` is `HID\A\B`.
pub fn instance_id(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))?;
    let (id, _) = rest.rsplit_once('#')?;
    Some(id.replace('#', "\\").to_ascii_uppercase())
}

/// A whole USB device, not one of its interfaces.
pub fn is_usb_device(id: &str) -> bool {
    id.starts_with("USB\\VID_") && !id.contains("&MI_")
}

#[cfg(test)]
pub(crate) static PARENTS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The USB device a collection or interface sits on, so a serial-less device keeps one key.
pub fn usb_parent(path: &str) -> Option<String> {
    #[cfg(test)]
    if let Some((_, p)) = locked(&PARENTS).iter().find(|(c, _)| c == path) {
        return Some(p.clone());
    }
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
    };
    let wide: Vec<u16> = instance_id(path)?.encode_utf16().chain(Some(0)).collect();
    let mut node = 0u32;
    // SAFETY: every buffer handed to CfgMgr outlives its call.
    unsafe {
        if CM_Locate_DevNodeW(&mut node, wide.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return None;
        }
        for _ in 0..8 {
            let mut buf = [0u16; 256];
            if CM_Get_Device_IDW(node, buf.as_mut_ptr(), buf.len() as u32, 0) != CR_SUCCESS {
                return None;
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let id = String::from_utf16_lossy(&buf[..end]).to_ascii_uppercase();
            if is_usb_device(&id) {
                return Some(id);
            }
            let mut parent = 0u32;
            if CM_Get_Parent(&mut parent, node, 0) != CR_SUCCESS {
                return None;
            }
            node = parent;
        }
    }
    None
}

/// One opened collection. Every call takes its device's lock.
#[derive(Clone)]
pub struct Port {
    device: Arc<Device>,
    raw: Handle,
}

impl Port {
    /// The shared handle for this collection when there is one, else a handle `open` makes.
    pub fn open(
        info: &Info,
        open: impl FnOnce() -> Result<Box<dyn Raw>, String>,
    ) -> Result<Port, String> {
        let Some(device) = device_for(info) else {
            return Ok(Port::alone(open()?));
        };
        let mut handles = locked(&device.handles);
        handles.retain(|(_, h)| h.strong_count() > 0);
        let kept = handles
            .iter()
            .find_map(|(p, h)| (*p == info.path).then(|| h.upgrade()).flatten());
        let raw = match kept {
            Some(raw) => raw,
            None => {
                let raw: Handle = Arc::new(Mutex::new(open()?));
                handles.push((info.path.clone(), Arc::downgrade(&raw)));
                raw
            }
        };
        drop(handles);
        Ok(Port { device, raw })
    }

    /// A handle no other driver shares.
    pub fn alone(raw: Box<dyn Raw>) -> Port {
        Port {
            device: Arc::default(),
            raw: Arc::new(Mutex::new(raw)),
        }
    }

    pub fn device(&self) -> &Arc<Device> {
        &self.device
    }

    /// Both ports are one handle.
    pub fn same(&self, other: &Port) -> bool {
        Arc::ptr_eq(&self.raw, &other.raw)
    }

    pub fn write(&self, data: &[u8]) -> HidResult<usize> {
        let _hold = self.device.hold();
        locked(&self.raw).write(data)
    }
    pub fn read_timeout(&self, buf: &mut [u8], ms: i32) -> HidResult<usize> {
        let _hold = self.device.hold();
        locked(&self.raw).read_timeout(buf, ms)
    }
    pub fn read(&self, buf: &mut [u8]) -> HidResult<usize> {
        self.read_timeout(buf, -1)
    }
    pub fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
        let _hold = self.device.hold();
        locked(&self.raw).send_feature_report(data)
    }
    pub fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        let _hold = self.device.hold();
        locked(&self.raw).get_feature_report(buf)
    }
    pub fn get_input_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        let _hold = self.device.hold();
        locked(&self.raw).get_input_report(buf)
    }
}

impl From<HidDevice> for Port {
    fn from(hid: HidDevice) -> Port {
        Port::alone(Box::new(hid))
    }
}

/// A lighting backend that holds its device's lock for each frame and for its release.
struct Guarded {
    inner: Option<Box<dyn Backend>>,
    device: Arc<Device>,
}

pub fn guard(inner: Box<dyn Backend>, device: Arc<Device>) -> Box<dyn Backend> {
    Box::new(Guarded {
        inner: Some(inner),
        device,
    })
}

impl Backend for Guarded {
    fn name(&self) -> &str {
        self.inner.as_ref().map_or("", |b| b.name())
    }
    fn devices(&self) -> Vec<Lit> {
        self.inner.as_ref().map(|b| b.devices()).unwrap_or_default()
    }
    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        let _hold = self.device.hold();
        match self.inner.as_mut() {
            Some(b) => b.push(device_id, colors),
            None => Ok(()),
        }
    }
    fn flush(&mut self) -> Result<(), String> {
        let _hold = self.device.hold();
        self.inner.as_mut().map_or(Ok(()), |b| b.flush())
    }
}

impl Drop for Guarded {
    fn drop(&mut self) {
        let _hold = self.device.hold();
        self.inner.take();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::registry::Bus;
    use std::sync::atomic::AtomicUsize;

    /// Records each write, and whether the device lock was held for it.
    pub struct Fake {
        pub writes: super::Seen,
        pub device: Arc<Mutex<Option<Arc<Device>>>>,
    }

    impl Raw for Fake {
        fn write(&self, data: &[u8]) -> HidResult<usize> {
            let held = locked(&self.device).as_ref().is_some_and(|d| d.held_here());
            locked(&self.writes).push((data.to_vec(), held));
            Ok(data.len())
        }
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            Ok(())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
        fn get_input_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
    }

    pub fn info(pid: u16, path: &str) -> Info {
        Info {
            vid: 0xF0F0,
            pid,
            interface: 0,
            usage_page: 0xFF00,
            usage: 1,
            path: path.into(),
            serial: "S1".into(),
            product: String::new(),
            bus: Bus::Usb,
            release: 0,
        }
    }

    #[test]
    fn a_shared_collection_is_opened_once_and_every_write_holds_the_lock() {
        share(&[(0xF0F0, 1)]);
        let opens = AtomicUsize::new(0);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(None));
        let open = || {
            opens.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Fake {
                writes: writes.clone(),
                device: seen.clone(),
            }) as Box<dyn Raw>)
        };
        let lights = Port::open(&info(1, "a"), open).unwrap();
        let fans = Port::open(&info(1, "a"), open).unwrap();
        let sibling = Port::open(&info(1, "b"), open).unwrap();
        assert!(lights.same(&fans) && !lights.same(&sibling));
        assert!(Arc::ptr_eq(lights.device(), sibling.device()));
        assert_eq!(opens.load(Ordering::Relaxed), 2);
        *locked(&seen) = Some(lights.device().clone());
        lights.write(&[1]).unwrap();
        fans.write(&[2]).unwrap();
        assert_eq!(*locked(&writes), [(vec![1], true), (vec![2], true)]);
        drop((lights, fans, sibling));
        *locked(&seen) = None;
        Port::open(&info(1, "a"), open).unwrap();
        assert_eq!(
            opens.load(Ordering::Relaxed),
            3,
            "a dropped handle is closed"
        );
    }

    #[test]
    fn without_sharing_every_open_is_its_own() {
        let opens = AtomicUsize::new(0);
        let open = || {
            opens.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Fake {
                writes: Arc::default(),
                device: Arc::default(),
            }) as Box<dyn Raw>)
        };
        let a = Port::open(&info(2, "a"), open).unwrap();
        let b = Port::open(&info(2, "a"), open).unwrap();
        assert!(!a.same(&b) && !Arc::ptr_eq(a.device(), b.device()));
        assert_eq!(opens.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_pick_skips_or_keeps_only_the_listed_ids() {
        use crate::Pick;
        let ids = [(1, 2)];
        assert!(Pick::All.wants(1, 2));
        assert!(!Pick::Skip(&ids).wants(1, 2) && Pick::Skip(&ids).wants(1, 3));
        assert!(Pick::Only(&ids).wants(1, 2) && !Pick::Only(&ids).wants(1, 3));
        assert!(Pick::Skip(&[]).wants(1, 2));
    }

    // Serial-less collections of one USB device: two handles, one lock and one sequence.
    #[test]
    fn serial_less_collections_share_their_usb_device() {
        share(&[(0xF0F0, 3)]);
        let bare = |path: &str| Info {
            serial: String::new(),
            ..info(3, path)
        };
        locked(&PARENTS).extend([
            ("lights".into(), r"USB\VID_F0F0&PID_0003\5&1&0&2".into()),
            ("fans".into(), r"USB\VID_F0F0&PID_0003\5&1&0&2".into()),
            ("other".into(), r"USB\VID_F0F0&PID_0003\5&1&0&3".into()),
        ]);
        let open = || {
            Ok(Box::new(Fake {
                writes: Arc::default(),
                device: Arc::default(),
            }) as Box<dyn Raw>)
        };
        let lights = Port::open(&bare("lights"), open).unwrap();
        let fans = Port::open(&bare("fans"), open).unwrap();
        let other = Port::open(&bare("other"), open).unwrap();
        assert!(!lights.same(&fans));
        assert!(Arc::ptr_eq(lights.device(), fans.device()));
        assert!(!Arc::ptr_eq(lights.device(), other.device()));
        let seq = [
            lights.device().next_seq(),
            fans.device().next_seq(),
            lights.device().next_seq(),
        ];
        assert_eq!(seq, [1, 2, 3]);
        assert_eq!(other.device().next_seq(), 1);
    }

    #[test]
    fn paths_name_their_device_instance() {
        let hid = r"\\?\hid#vid_1b1c&pid_0c2e&mi_00&col01#7&2c5a&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let id = instance_id(hid).unwrap();
        assert_eq!(id, r"HID\VID_1B1C&PID_0C2E&MI_00&COL01\7&2C5A&0&0000");
        assert!(!is_usb_device(&id));
        let usb = instance_id(
            r"\\?\usb#vid_0cf2&pid_a100#6&1&0&2#{a5dcbf10-6530-11d2-901f-00c04fb951ed}",
        );
        assert!(is_usb_device(&usb.unwrap()));
        assert!(!is_usb_device(r"USB\VID_0CF2&PID_A100&MI_01\7&1&0&1"));
        assert_eq!(instance_id("no prefix"), None);
    }

    #[test]
    fn the_shared_sequence_runs_from_1_to_31() {
        let d = Device::default();
        let seq: Vec<u8> = (0..33).map(|_| d.next_seq()).collect();
        assert_eq!(seq[..3], [1, 2, 3]);
        assert_eq!(seq[30..], [31, 1, 2]);
    }

    #[test]
    fn the_lock_keeps_other_threads_out_and_lets_its_holder_back_in() {
        let d = Arc::new(Device::default());
        let outer = d.hold();
        let inner = d.hold();
        let other = {
            let d = d.clone();
            thread::spawn(move || {
                let _h = d.hold();
            })
        };
        thread::sleep(std::time::Duration::from_millis(30));
        assert!(!other.is_finished());
        drop(inner);
        thread::sleep(std::time::Duration::from_millis(30));
        assert!(!other.is_finished());
        drop(outer);
        other.join().unwrap();
        assert!(!d.held_here());
    }
}
