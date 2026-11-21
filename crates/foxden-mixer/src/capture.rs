//! One app's sound, from its process and every process it started, with Windows' process
//! loopback (Windows 10 2004 and later). It is taken after the app's own volume in the Windows
//! mixer and before the output's volume and mute.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{BLOB, IAgileObject, IAgileObject_Impl};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{IUnknown, Interface, Ref, implement};

use crate::system::com_init;

/// Process loopback has no mix format of its own, so it is asked for this one.
pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
const BUFFER_100NS: i64 = 200_000;
const ACTIVATE_MS: u32 = 5_000;

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Activated(HANDLE);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        unsafe { SetEvent(self.0) }
    }
}

impl IAgileObject_Impl for Activated_Impl {}

struct Event(HANDLE);

impl Event {
    fn new() -> windows::core::Result<Event> {
        unsafe { CreateEventW(None, false, false, None) }.map(Event)
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32) -> windows::core::Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // Never cleared: the blob points at `params` on this stack.
    let mut prop = std::mem::ManuallyDrop::new(PROPVARIANT::default());
    unsafe {
        let inner = &mut *prop.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: &mut params as *mut _ as *mut u8,
        };
    }
    let done = Event::new()?;
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done.0).into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop),
            &handler,
        )
    }?;
    if unsafe { WaitForSingleObject(done.0, ACTIVATE_MS) } != WAIT_OBJECT_0 {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_ABORT,
        ));
    }
    let mut hr = windows::core::HRESULT(0);
    let mut iface: Option<IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut iface) }?;
    hr.ok()?;
    iface
        .ok_or_else(|| {
            windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOINTERFACE)
        })?
        .cast()
}

fn run(pid: u32, stop: &AtomicBool, sink: &mut dyn FnMut(&[f32])) -> windows::core::Result<()> {
    let client = open(pid)?;
    let block = (CHANNELS * 4) as u16;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        wBitsPerSample: 32,
        nBlockAlign: block,
        nAvgBytesPerSec: RATE * block as u32,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            BUFFER_100NS,
            0,
            &format,
            None,
        )
    }?;
    let ready = Event::new()?;
    unsafe { client.SetEventHandle(ready.0) }?;
    let capture: IAudioCaptureClient = unsafe { client.GetService() }?;
    unsafe { client.Start() }?;
    let mut silence = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(ready.0, 100) };
        while unsafe { capture.GetNextPacketSize() }? > 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }?;
            let n = frames as usize * CHANNELS;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                silence.resize(n, 0.0);
                sink(&silence);
            } else {
                sink(unsafe { std::slice::from_raw_parts(data as *const f32, n) });
            }
            unsafe { capture.ReleaseBuffer(frames) }?;
        }
    }
    unsafe { client.Stop() }?;
    Ok(())
}

/// A running capture of one process tree, stopped when dropped.
pub struct AppCapture {
    pub pid: u32,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl AppCapture {
    /// `sink` gets interleaved stereo at [`RATE`] on the capture's own thread.
    pub fn start(pid: u32, mut sink: impl FnMut(&[f32]) + Send + 'static) -> AppCapture {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let (s, e) = (stop.clone(), error.clone());
        let worker = thread::Builder::new()
            .name(format!("app-capture-{pid}"))
            .spawn(move || {
                com_init();
                if let Err(x) = run(pid, &s, &mut sink) {
                    *e.lock().unwrap_or_else(|p| p.into_inner()) = Some(x.message().to_string());
                }
            })
            .ok();
        AppCapture {
            pid,
            stop,
            error,
            worker,
        }
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for AppCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
