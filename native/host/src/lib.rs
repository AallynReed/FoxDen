//! The slice of the Tauri API the backend modules use, served by the native window instead.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub use foxden_host_macros::command;
use serde::Serialize;

pub type Wry = ();
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub enum RunEvent {
    Ready,
    Exit,
}

type Sink = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;
type Exec = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
type Assets = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;
type Notify = Box<dyn Fn(&str, &str) + Send + Sync>;
type ExitHook = Box<dyn Fn(&AppHandle, &RunEvent) + Send + Sync>;
type TrayHook = Arc<dyn Fn(&AppHandle, tray::TrayIconEvent) + Send + Sync>;

#[derive(Default)]
struct Inner {
    states: Mutex<HashMap<TypeId, &'static (dyn Any + Send + Sync)>>,
    sink: OnceLock<Sink>,
    main: OnceLock<Exec>,
    assets: OnceLock<Assets>,
    notify: OnceLock<Notify>,
    shown: AtomicBool,
    minimized: AtomicBool,
    exit_hooks: Mutex<Vec<ExitHook>>,
    tray_hooks: Mutex<Vec<TrayHook>>,
}

#[derive(Clone, Default)]
pub struct AppHandle(Arc<Inner>);

impl AppHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_sink(&self, f: impl Fn(&str, serde_json::Value) + Send + Sync + 'static) {
        let _ = self.0.sink.set(Box::new(f));
    }

    pub fn set_main_thread(&self, f: impl Fn(Box<dyn FnOnce() + Send>) + Send + Sync + 'static) {
        let _ = self.0.main.set(Box::new(f));
    }

    pub fn set_assets(&self, f: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) {
        let _ = self.0.assets.set(Box::new(f));
    }

    pub fn set_notify(&self, f: impl Fn(&str, &str) + Send + Sync + 'static) {
        let _ = self.0.notify.set(Box::new(f));
    }

    pub fn set_shown(&self, shown: bool, minimized: bool) {
        self.0.shown.store(shown, Ordering::Relaxed);
        self.0.minimized.store(minimized, Ordering::Relaxed);
    }

    pub fn fire_exit(&self) {
        for hook in self.0.exit_hooks.lock().unwrap().iter() {
            hook(self, &RunEvent::Exit);
        }
    }

    pub fn fire_tray(&self, event: tray::TrayIconEvent) {
        let hooks: Vec<TrayHook> = self.0.tray_hooks.lock().unwrap().clone();
        for hook in hooks {
            hook(self, event.clone());
        }
    }

    pub fn plugin(&self, plugin: plugin::TauriPlugin) -> Result<()> {
        if let Some(hook) = plugin.hook {
            self.0.exit_hooks.lock().unwrap().push(hook);
        }
        Ok(())
    }

    pub fn run_on_main_thread(&self, f: impl FnOnce() + Send + 'static) -> Result<()> {
        match self.0.main.get() {
            Some(exec) => {
                exec(Box::new(f));
                Ok(())
            }
            None => Err(Error("no main thread".into())),
        }
    }

    pub fn on_tray_icon_event(&self, f: impl Fn(&AppHandle, tray::TrayIconEvent) + Send + Sync + 'static) {
        self.0.tray_hooks.lock().unwrap().push(Arc::new(f));
    }

    pub fn asset_resolver(&self) -> AssetResolver {
        AssetResolver(self.clone())
    }
}

pub struct AssetResolver(AppHandle);

pub struct Asset {
    pub bytes: Vec<u8>,
}

impl AssetResolver {
    pub fn get(&self, path: String) -> Option<Asset> {
        let bytes = (self.0.0.assets.get()?)(&path)?;
        Some(Asset { bytes })
    }
}

pub trait Emitter {
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> Result<()>;
}

impl Emitter for AppHandle {
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> Result<()> {
        let Some(sink) = self.0.sink.get() else {
            return Ok(());
        };
        let value = serde_json::to_value(payload).map_err(|e| Error(e.to_string()))?;
        sink(event, value);
        Ok(())
    }
}

pub struct State<'a, T: 'static>(&'a T);

impl<T> Clone for State<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for State<'_, T> {}

impl<'a, T> State<'a, T> {
    pub fn inner(&self) -> &'a T {
        self.0
    }
}

impl<T> Deref for State<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

pub struct WebviewWindow(AppHandle);

impl WebviewWindow {
    pub fn is_visible(&self) -> Result<bool> {
        Ok(self.0.0.shown.load(Ordering::Relaxed))
    }

    pub fn is_minimized(&self) -> Result<bool> {
        Ok(self.0.0.minimized.load(Ordering::Relaxed))
    }
}

pub trait Manager {
    fn manage<T: Send + Sync + 'static>(&self, value: T) -> bool;
    fn try_state<T: Send + Sync + 'static>(&self) -> Option<State<'static, T>>;
    fn state<T: Send + Sync + 'static>(&self) -> State<'static, T> {
        self.try_state().expect("state not managed")
    }
    fn get_webview_window(&self, label: &str) -> Option<WebviewWindow>;
}

impl Manager for AppHandle {
    fn manage<T: Send + Sync + 'static>(&self, value: T) -> bool {
        let mut states = self.0.states.lock().unwrap();
        if states.contains_key(&TypeId::of::<T>()) {
            return false;
        }
        let leaked: &'static T = Box::leak(Box::new(value));
        states.insert(TypeId::of::<T>(), leaked);
        true
    }

    fn try_state<T: Send + Sync + 'static>(&self) -> Option<State<'static, T>> {
        let states = self.0.states.lock().unwrap();
        let any: &'static (dyn Any + Send + Sync) = *states.get(&TypeId::of::<T>())?;
        any.downcast_ref::<T>().map(State)
    }

    fn get_webview_window(&self, label: &str) -> Option<WebviewWindow> {
        (label == "main" && self.0.shown.load(Ordering::Relaxed)).then(|| WebviewWindow(self.clone()))
    }
}

pub trait NotificationExt {
    fn notification(&self) -> Notification;
}

impl NotificationExt for AppHandle {
    fn notification(&self) -> Notification {
        Notification(self.clone())
    }
}

pub struct Notification(AppHandle);

impl Notification {
    pub fn builder(&self) -> NotificationBuilder {
        NotificationBuilder {
            app: self.0.clone(),
            title: String::new(),
            body: String::new(),
        }
    }
}

pub struct NotificationBuilder {
    app: AppHandle,
    title: String,
    body: String,
}

impl NotificationBuilder {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn show(self) -> Result<()> {
        if let Some(notify) = self.app.0.notify.get() {
            notify(&self.title, &self.body);
        }
        Ok(())
    }
}

pub mod plugin {
    use super::*;

    pub struct TauriPlugin {
        pub(crate) hook: Option<ExitHook>,
    }

    pub struct Builder<R> {
        hook: Option<ExitHook>,
        _r: PhantomData<R>,
    }

    impl<R> Builder<R> {
        pub fn new(_name: &'static str) -> Self {
            Self {
                hook: None,
                _r: PhantomData,
            }
        }

        pub fn on_event(mut self, f: impl Fn(&AppHandle, &RunEvent) + Send + Sync + 'static) -> Self {
            self.hook = Some(Box::new(f));
            self
        }

        pub fn build(self) -> TauriPlugin {
            TauriPlugin { hook: self.hook }
        }
    }
}

pub mod tray {
    pub use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
}

pub mod ipc {
    use std::marker::PhantomData;

    pub enum InvokeBody {
        Json(serde_json::Value),
        Raw(Vec<u8>),
    }

    pub struct Request<'a> {
        body: InvokeBody,
        _a: PhantomData<&'a ()>,
    }

    impl Request<'_> {
        pub fn new(body: InvokeBody) -> Self {
            Self {
                body,
                _a: PhantomData,
            }
        }

        pub fn body(&self) -> &InvokeBody {
            &self.body
        }
    }
}

pub mod async_runtime {
    use std::future::Future;
    use std::sync::LazyLock;

    pub use tokio::task::JoinHandle;

    static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    });

    pub fn spawn<F: Future + Send + 'static>(f: F) -> JoinHandle<F::Output>
    where
        F::Output: Send + 'static,
    {
        RUNTIME.spawn(f)
    }

    pub fn spawn_blocking<F: FnOnce() -> R + Send + 'static, R: Send + 'static>(f: F) -> JoinHandle<R> {
        RUNTIME.spawn_blocking(f)
    }

    pub fn block_on<F: Future>(f: F) -> F::Output {
        RUNTIME.block_on(f)
    }
}
