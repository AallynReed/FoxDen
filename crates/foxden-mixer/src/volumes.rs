//! Windows' own volume, mute and peak level of each app's audio session and each device, as the
//! Volume mixer shows them. Nothing is rerouted. The COM objects stay on the thread that made them.

use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl,
    IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::core::Interface;

use crate::Result;
use crate::park::lower;
use crate::system::{enumerator, err, id_of, name_of, processes};

/// The session and device lists are read again after this; levels are read on every call.
const STALE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppVolume {
    pub exe: String,
    /// Id of the output it plays on.
    pub endpoint: String,
    pub volume: f32,
    pub muted: bool,
    pub peak: f32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceVolume {
    pub id: String,
    pub name: String,
    pub capture: bool,
    pub default: bool,
    pub volume: f32,
    pub muted: bool,
    pub peak: f32,
}

struct Session {
    key: (String, String),
    control: IAudioSessionControl,
    volume: ISimpleAudioVolume,
    meter: Option<IAudioMeterInformation>,
}

struct Device {
    info: DeviceVolume,
    volume: IAudioEndpointVolume,
    meter: Option<IAudioMeterInformation>,
}

/// Holds the sessions and devices between reads.
#[derive(Default)]
pub struct Volumes {
    sessions: Vec<Session>,
    devices: Vec<Device>,
    read_at: Option<Instant>,
}

impl Volumes {
    fn refresh(&mut self, skip_pid: u32) -> Result<()> {
        let en = enumerator()?;
        let procs = processes();
        let mut sessions = Vec::new();
        let mut devices = Vec::new();
        for capture in [false, true] {
            let flow = if capture { eCapture } else { eRender };
            let default = unsafe { en.GetDefaultAudioEndpoint(flow, eConsole) }
                .ok()
                .and_then(|d| id_of(&d).ok());
            let list = unsafe { en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }.map_err(err)?;
            for i in 0..unsafe { list.GetCount() }.map_err(err)? {
                let Ok(dev) = (unsafe { list.Item(i) }) else {
                    continue;
                };
                let Ok(id) = id_of(&dev) else { continue };
                if let Ok(volume) =
                    unsafe { dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }
                {
                    devices.push(Device {
                        info: DeviceVolume {
                            default: default.as_deref() == Some(id.as_str()),
                            id: id.clone(),
                            name: name_of(&dev),
                            capture,
                            volume: 0.0,
                            muted: false,
                            peak: 0.0,
                        },
                        volume,
                        meter: unsafe { dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) }
                            .ok(),
                    });
                }
                if capture {
                    continue;
                }
                let Ok(mgr) = (unsafe { dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
                else {
                    continue;
                };
                let Ok(all) = (unsafe { mgr.GetSessionEnumerator() }) else {
                    continue;
                };
                for k in 0..unsafe { all.GetCount() }.unwrap_or(0) {
                    let Ok(control) = (unsafe { all.GetSession(k) }) else {
                        continue;
                    };
                    let Ok(c2) = control.cast::<IAudioSessionControl2>() else {
                        continue;
                    };
                    if unsafe { c2.IsSystemSoundsSession() } == windows::Win32::Foundation::S_OK {
                        continue;
                    }
                    let pid = unsafe { c2.GetProcessId() }.unwrap_or(0);
                    if pid == 0 || pid == skip_pid {
                        continue;
                    }
                    let Some(p) = procs.iter().find(|p| p.pid == pid) else {
                        continue;
                    };
                    let Ok(volume) = control.cast::<ISimpleAudioVolume>() else {
                        continue;
                    };
                    sessions.push(Session {
                        key: (p.exe.clone(), id.clone()),
                        meter: control.cast::<IAudioMeterInformation>().ok(),
                        control,
                        volume,
                    });
                }
            }
        }
        self.sessions = sessions;
        self.devices = devices;
        self.read_at = Some(Instant::now());
        Ok(())
    }

    fn fresh(&mut self, skip_pid: u32) -> Result<()> {
        if self.read_at.is_none_or(|t| t.elapsed() >= STALE) {
            self.refresh(skip_pid)?;
        }
        Ok(())
    }

    /// Every app with a session on an active output, one row per app and output, and every
    /// active device. Sessions of `skip_pid` are left out.
    pub fn read(&mut self, skip_pid: u32) -> Result<(Vec<AppVolume>, Vec<DeviceVolume>)> {
        self.fresh(skip_pid)?;
        let mut apps: Vec<AppVolume> = Vec::new();
        for s in &self.sessions {
            let state = unsafe { s.control.GetState() }.ok();
            if state == Some(AudioSessionStateExpired) {
                continue;
            }
            let volume = unsafe { s.volume.GetMasterVolume() }.unwrap_or(1.0);
            let muted = unsafe { s.volume.GetMute() }.is_ok_and(|b| b.as_bool());
            let peak = s
                .meter
                .as_ref()
                .and_then(|m| unsafe { m.GetPeakValue() }.ok())
                .unwrap_or(0.0);
            let active = state == Some(AudioSessionStateActive);
            match apps
                .iter_mut()
                .find(|a| lower(&a.exe) == lower(&s.key.0) && a.endpoint == s.key.1)
            {
                Some(a) => {
                    a.peak = a.peak.max(peak);
                    a.active |= active;
                }
                None => apps.push(AppVolume {
                    exe: s.key.0.clone(),
                    endpoint: s.key.1.clone(),
                    volume,
                    muted,
                    peak,
                    active,
                }),
            }
        }
        apps.sort_by_key(|a| (!a.active, lower(&a.exe)));
        let devices = self
            .devices
            .iter()
            .map(|d| DeviceVolume {
                volume: unsafe { d.volume.GetMasterVolumeLevelScalar() }.unwrap_or(0.0),
                muted: unsafe { d.volume.GetMute() }.is_ok_and(|b| b.as_bool()),
                peak: d
                    .meter
                    .as_ref()
                    .and_then(|m| unsafe { m.GetPeakValue() }.ok())
                    .unwrap_or(0.0),
                ..d.info.clone()
            })
            .collect();
        Ok((apps, devices))
    }

    /// Sets every session of `exe` on `endpoint`.
    pub fn set_app(
        &mut self,
        exe: &str,
        endpoint: &str,
        volume: Option<f32>,
        muted: Option<bool>,
    ) -> Result<()> {
        let key = lower(exe);
        let mut found = false;
        for s in self
            .sessions
            .iter()
            .filter(|s| lower(&s.key.0) == key && s.key.1 == endpoint)
        {
            found = true;
            if let Some(v) = volume {
                unsafe {
                    s.volume
                        .SetMasterVolume(v.clamp(0.0, 1.0), std::ptr::null())
                }
                .map_err(err)?;
            }
            if let Some(m) = muted {
                unsafe { s.volume.SetMute(m, std::ptr::null()) }.map_err(err)?;
            }
        }
        if found {
            Ok(())
        } else {
            Err(format!("{exe} is no longer playing"))
        }
    }

    pub fn set_device(&mut self, id: &str, volume: Option<f32>, muted: Option<bool>) -> Result<()> {
        let d = self
            .devices
            .iter()
            .find(|d| d.info.id == id)
            .ok_or("That device is gone")?;
        if let Some(v) = volume {
            unsafe {
                d.volume
                    .SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null())
            }
            .map_err(err)?;
        }
        if let Some(m) = muted {
            unsafe { d.volume.SetMute(m, std::ptr::null()) }.map_err(err)?;
        }
        Ok(())
    }

    /// Reads the lists again on the next call, as after the default device changed.
    pub fn stale(&mut self) {
        self.read_at = None;
    }
}
