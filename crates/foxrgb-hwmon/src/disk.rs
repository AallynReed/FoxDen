//! Solid-state drive temperatures through the storage temperature query, which needs no
//! administrator rights.

use std::mem::{size_of, size_of_val};
use std::ptr::{null, null_mut};
use std::time::Duration;

use foxrgb_fan::{FanBackend, Sensor};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    StorageDeviceProperty, StorageDeviceSeekPenaltyProperty, StorageDeviceTemperatureProperty,
};

use crate::{Pace, plausible, polled, text, wide};

/// Physical drive numbers 0 up to this are tried.
const DRIVES: u32 = 32;
/// Drives are asked seldom: a busy one can take over 100 ms to answer.
const PACE: Pace = Pace {
    every: Duration::from_secs(5),
    keep: Duration::from_secs(12),
};

/// A physical drive opened with no access rights, which is enough to query it.
struct Drive(HANDLE);

impl Drive {
    fn open(number: u32) -> Option<Drive> {
        let path = wide(&format!(r"\\.\PhysicalDrive{number}"));
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE;
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                share,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        (handle != INVALID_HANDLE_VALUE).then_some(Drive(handle))
    }

    /// Fills `out` with a property of the drive and returns the bytes written.
    fn query(&self, property: STORAGE_PROPERTY_ID, out: &mut [u64]) -> Option<usize> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: property,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut written = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                IOCTL_STORAGE_QUERY_PROPERTY,
                (&raw const query).cast(),
                size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                out.as_mut_ptr().cast(),
                size_of_val(out) as u32,
                &mut written,
                null_mut(),
            )
        };
        (ok != 0).then_some(written as usize)
    }

    /// Whether the drive says it has no seek penalty, as solid-state drives do.
    fn solid_state(&self) -> bool {
        let mut out = [0u64; 2];
        let written = self.query(StorageDeviceSeekPenaltyProperty, &mut out);
        // Two 32-bit header fields, then the flag byte.
        written.is_some_and(|written| written > 8) && out[1].to_ne_bytes()[0] == 0
    }

    // Fields are read through the pointer: a reply's flag bytes need not be valid `bool`s.
    fn temperature(&self) -> Option<f32> {
        let mut out = [0u64; 64];
        let written = self.query(StorageDeviceTemperatureProperty, &mut out)?;
        if written < size_of::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>() {
            return None;
        }
        let data: *const STORAGE_TEMPERATURE_DATA_DESCRIPTOR = out.as_ptr().cast();
        let (count, first) = unsafe { ((*data).InfoCount, (*data).TemperatureInfo[0].Temperature) };
        temperature(count, first)
    }

    fn model(&self) -> Option<String> {
        let mut out = [0u64; 128];
        let written = self.query(StorageDeviceProperty, &mut out)?;
        if written < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return None;
        }
        let descriptor: *const STORAGE_DEVICE_DESCRIPTOR = out.as_ptr().cast();
        let (vendor, product) =
            unsafe { ((*descriptor).VendorIdOffset, (*descriptor).ProductIdOffset) };
        let bytes = unsafe { std::slice::from_raw_parts(out.as_ptr().cast(), size_of_val(&out)) };
        model(bytes, vendor, product)
    }
}

impl Drop for Drive {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The drive's first reading, which is its overall temperature, if it sent any.
fn temperature(count: u16, first: i16) -> Option<f32> {
    if count == 0 {
        return None;
    }
    plausible(f32::from(first))
}

/// Vendor and product as one name. An offset of 0 means the drive did not say.
fn model(descriptor: &[u8], vendor: u32, product: u32) -> Option<String> {
    let part = |offset: u32| match descriptor.get(offset as usize..) {
        Some(tail) if offset != 0 => text(tail.iter().copied()),
        _ => String::new(),
    };
    let model = format!("{} {}", part(vendor), part(product));
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

struct DiskSensor {
    number: u32,
    id: String,
    name: String,
}

impl Sensor for DiskSensor {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self) -> Option<f32> {
        // Opened each time, so no handle is held that would block ejecting the drive.
        Drive::open(self.number)?.temperature()
    }
}

pub fn open() -> Option<Box<dyn FanBackend>> {
    let found = (0..DRIVES)
        .filter_map(|number| {
            let drive = Drive::open(number)?;
            // Spinning disks are left out: asking one may wake it or keep it awake.
            if !drive.solid_state() {
                return None;
            }
            let celsius = drive.temperature()?;
            let model = drive.model().unwrap_or_else(|| format!("Disk {number}"));
            let sensor = DiskSensor {
                number,
                id: format!("disk{number}/temperature"),
                name: format!("{model} temperature"),
            };
            Some((sensor, celsius))
        })
        .collect();
    polled("disk", found, PACE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_is_the_first_plausible_reading() {
        assert_eq!(temperature(1, 41), Some(41.0));
        assert_eq!(temperature(3, -5), Some(-5.0));
        assert_eq!(temperature(0, 41), None);
        assert_eq!(temperature(1, -273), None);
        assert_eq!(temperature(1, 200), None);
        // Windows fills in -32768 where a drive gave no value.
        assert_eq!(temperature(1, i16::MIN), None);
    }

    #[test]
    fn model_joins_vendor_and_product() {
        let mut descriptor = [0u8; 96];
        descriptor[40..48].copy_from_slice(b"WDC     ");
        descriptor[49..65].copy_from_slice(b"WD40EZRZ-00GXCB0");
        assert_eq!(
            model(&descriptor, 40, 49).as_deref(),
            Some("WDC WD40EZRZ-00GXCB0")
        );
        assert_eq!(
            model(&descriptor, 0, 49).as_deref(),
            Some("WD40EZRZ-00GXCB0")
        );
        assert_eq!(model(&descriptor, 0, 0), None);
        assert_eq!(model(&descriptor, 500, 600), None);
    }
}
