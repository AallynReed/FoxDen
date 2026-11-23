//! Raw Win32 HID access for battery queries.
//!
//! Buffers follow hidapi conventions: on write and set-feature `data[0]` is the report id (0 when
//! reports are not numbered) and the buffer is padded to the report length; on read a leading 0
//! report id is stripped, so `buf[0]` is the first payload byte of an unnumbered report.

use std::ffi::{CString, c_void};
use std::sync::Once;
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDP_CAPS, HidD_FlushQueue, HidD_FreePreparsedData, HidD_GetFeature, HidD_GetInputReport,
    HidD_GetPreparsedData, HidD_SetFeature, HidD_SetOutputReport, HidP_GetCaps,
    PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventA, WaitForSingleObject};

/// One top-level HID collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coll {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    /// USB interface number, -1 when unknown.
    pub iface: i32,
    pub page: u16,
    pub usage: u16,
    pub in_len: u16,
    pub out_len: u16,
    pub feat_len: u16,
    pub product: String,
}

/// Every HID collection on the system, with its report lengths.
/// Listing opens every collection, so the last list is reused until the device list changes.
pub fn enumerate() -> Vec<Coll> {
    static LAST: std::sync::Mutex<Option<(u64, Vec<Coll>)>> = std::sync::Mutex::new(None);
    let print = fingerprint();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, list)) = last.as_ref()
        && *p == print
        && print != 0
    {
        return list.clone();
    }
    let list = list_now();
    *last = Some((print, list.clone()));
    list
}

fn list_now() -> Vec<Coll> {
    static ONCE: Once = Once::new();
    ONCE.call_once(hidapi::HidApi::disable_device_discovery);
    let Ok(mut api) = hidapi::HidApi::new() else {
        return Vec::new();
    };
    if api.refresh_devices().is_err() {
        return Vec::new();
    }
    api.device_list()
        .map(|d| {
            let path = d.path().to_string_lossy().into_owned();
            let caps = caps(&path).unwrap_or_default();
            Coll {
                vid: d.vendor_id(),
                pid: d.product_id(),
                iface: d.interface_number(),
                page: d.usage_page(),
                usage: d.usage(),
                in_len: caps.0,
                out_len: caps.1,
                feat_len: caps.2,
                product: d.product_string().unwrap_or_default().to_string(),
                path,
            }
        })
        .collect()
}

fn open_raw(path: &str, access: u32, overlapped: bool) -> Option<HANDLE> {
    let c = CString::new(path).ok()?;
    let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
    let h = unsafe {
        CreateFileA(
            c.as_ptr() as _,
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

/// Input, output and feature report lengths.
fn caps(path: &str) -> Option<(u16, u16, u16)> {
    let h = open_raw(path, 0, false)?;
    let mut out = None;
    unsafe {
        let mut pp: PHIDP_PREPARSED_DATA = 0;
        if HidD_GetPreparsedData(h, &mut pp) {
            let mut c: HIDP_CAPS = std::mem::zeroed();
            if HidP_GetCaps(pp, &mut c) >= 0 {
                out = Some((
                    c.InputReportByteLength,
                    c.OutputReportByteLength,
                    c.FeatureReportByteLength,
                ));
            }
            HidD_FreePreparsedData(pp);
        }
        CloseHandle(h);
    }
    out
}

/// An open HID collection.
pub struct Dev {
    h: HANDLE,
    overlapped: bool,
    pub coll: Coll,
}

unsafe impl Send for Dev {}

impl Drop for Dev {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.h) };
    }
}

impl Dev {
    /// Read and write.
    pub fn open(c: &Coll) -> Option<Self> {
        Self::with(c, GENERIC_READ | GENERIC_WRITE, true)
    }

    /// Read only: for push-only collections another app may hold.
    pub fn open_read_only(c: &Coll) -> Option<Self> {
        Self::with(c, GENERIC_READ, true)
    }

    /// No access rights: enough for feature reports, and opens on mice and keyboards.
    pub fn open_feature_only(c: &Coll) -> Option<Self> {
        Self::with(c, 0, false)
    }

    fn with(c: &Coll, access: u32, overlapped: bool) -> Option<Self> {
        open_raw(&c.path, access, overlapped).map(|h| Self {
            h,
            overlapped,
            coll: c.clone(),
        })
    }

    fn padded(&self, data: &[u8], len: u16) -> Vec<u8> {
        let mut v = data.to_vec();
        if v.len() < len as usize {
            v.resize(len as usize, 0);
        }
        v
    }

    /// True when the call just made completed or is still in flight.
    fn in_flight(started: bool) -> bool {
        started || unsafe { GetLastError() } == ERROR_IO_PENDING
    }

    /// Waits for an overlapped call; cancels it on timeout. Returns the transferred byte count.
    fn finish(&self, ov: &mut OVERLAPPED, pending: bool, timeout: Duration) -> Option<u32> {
        let mut n = 0u32;
        unsafe {
            if !pending {
                CloseHandle(ov.hEvent);
                return None;
            }
            let done =
                WaitForSingleObject(ov.hEvent, timeout.as_millis().min(u32::MAX as u128) as u32)
                    == WAIT_OBJECT_0;
            if !done {
                CancelIoEx(self.h, ov);
            }
            let ok = GetOverlappedResult(self.h, ov, &mut n, 1) != 0;
            CloseHandle(ov.hEvent);
            (done && ok).then_some(n)
        }
    }

    fn overlapped() -> OVERLAPPED {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = unsafe { CreateEventA(std::ptr::null(), 1, 0, std::ptr::null()) };
        ov
    }

    pub fn write(&self, data: &[u8]) -> bool {
        self.write_timeout(data, Duration::from_millis(500))
    }

    pub fn write_timeout(&self, data: &[u8], timeout: Duration) -> bool {
        if !self.overlapped {
            return false;
        }
        let buf = self.padded(data, self.coll.out_len);
        let mut ov = Self::overlapped();
        let started = unsafe {
            WriteFile(
                self.h,
                buf.as_ptr(),
                buf.len() as u32,
                std::ptr::null_mut(),
                &mut ov,
            ) != 0
        };
        let pending = Self::in_flight(started);
        self.finish(&mut ov, pending, timeout).is_some()
    }

    /// One input report, or `None` on timeout or error.
    pub fn read(&self, timeout: Duration) -> Option<Vec<u8>> {
        if !self.overlapped {
            return None;
        }
        let mut buf = vec![0u8; (self.coll.in_len as usize).max(65)];
        let mut ov = Self::overlapped();
        let started = unsafe {
            ReadFile(
                self.h,
                buf.as_mut_ptr() as *mut c_void as _,
                buf.len() as u32,
                std::ptr::null_mut(),
                &mut ov,
            ) != 0
        };
        let pending = Self::in_flight(started);
        let n = self.finish(&mut ov, pending, timeout)? as usize;
        buf.truncate(n);
        if buf.first() == Some(&0) {
            buf.remove(0);
        }
        Some(buf)
    }

    /// Reads until `pick` accepts a report or the time runs out.
    pub fn read_until<T>(
        &self,
        timeout: Duration,
        mut pick: impl FnMut(&[u8]) -> Option<T>,
    ) -> Option<T> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            if let Some(r) = self.read(left.min(Duration::from_millis(50)))
                && let Some(v) = pick(&r)
            {
                return Some(v);
            }
        }
    }

    /// For collections without an interrupt OUT pipe: sends with HidD_SetOutputReport, then reads.
    pub fn set_output_and_read(&self, data: &[u8], timeout: Duration) -> Option<Vec<u8>> {
        let buf = self.padded(data, self.coll.out_len);
        let mut ov = Self::overlapped();
        let mut rbuf = vec![0u8; (self.coll.in_len as usize).max(65)];
        let started = unsafe {
            ReadFile(
                self.h,
                rbuf.as_mut_ptr() as _,
                rbuf.len() as u32,
                std::ptr::null_mut(),
                &mut ov,
            ) != 0
        };
        let pending = Self::in_flight(started);
        let sent = unsafe { HidD_SetOutputReport(self.h, buf.as_ptr() as _, buf.len() as u32) };
        if !sent && pending {
            unsafe { CancelIoEx(self.h, &ov) };
        }
        let n = self.finish(&mut ov, pending, timeout)? as usize;
        if !sent {
            return None;
        }
        rbuf.truncate(n);
        if rbuf.first() == Some(&0) {
            rbuf.remove(0);
        }
        Some(rbuf)
    }

    /// Drops queued input reports.
    pub fn flush(&self) {
        unsafe { HidD_FlushQueue(self.h) };
    }

    pub fn set_feature(&self, data: &[u8]) -> bool {
        let buf = self.padded(data, self.coll.feat_len);
        unsafe { HidD_SetFeature(self.h, buf.as_ptr() as _, buf.len() as u32) }
    }

    /// The feature report `id`; `[0]` is the report id.
    pub fn get_feature(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; (self.coll.feat_len as usize).max(2)];
        buf[0] = id;
        (unsafe { HidD_GetFeature(self.h, buf.as_mut_ptr() as _, buf.len() as u32) }).then_some(buf)
    }

    /// The current input report `id` over the control pipe; `[0]` is the report id.
    pub fn get_input_report(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; (self.coll.in_len as usize).max(2)];
        buf[0] = id;
        (unsafe { HidD_GetInputReport(self.h, buf.as_mut_ptr() as _, buf.len() as u32) })
            .then_some(buf)
    }
}

/// A cheap hash of the present HID and USB device interfaces: it changes when something is
/// plugged in or out, including devices without a HID interface.
pub fn fingerprint() -> u64 {
    use windows_sys::core::GUID;
    const HID: GUID = GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);
    const USB_DEVICE: GUID = GUID::from_u128(0xa5dcbf10_6530_11d2_901f_00c04fb951ed);
    let mut h = 0xcbf29ce484222325u64;
    for class in [HID, USB_DEVICE] {
        let mut paths = interfaces(&class);
        paths.sort();
        for c in paths.iter().flat_map(|p| p.iter().chain([&0])) {
            h = (h ^ *c as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn interfaces(class: &windows_sys::core::GUID) -> Vec<Vec<u16>> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
        CM_Get_Device_Interface_ListW, CR_SUCCESS,
    };
    let mut len = 0u32;
    let mut list = Vec::new();
    unsafe {
        if CM_Get_Device_Interface_List_SizeW(
            &mut len,
            class,
            std::ptr::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return Vec::new();
        }
        list.resize(len as usize, 0u16);
        if CM_Get_Device_Interface_ListW(
            class,
            std::ptr::null(),
            list.as_mut_ptr(),
            len,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return Vec::new();
        }
    }
    list.split(|&c| c == 0)
        .filter(|p| !p.is_empty())
        .map(<[u16]>::to_vec)
        .collect()
}
