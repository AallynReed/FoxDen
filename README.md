# FoxRGB

Free lighting and fan control for Windows, written in Rust.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as a single animation instead of each device doing its own thing. Fan curves run in a
background service and follow any temperature it can read.

## Status

Early. The app runs with virtual devices: sixteen effects, a layout editor with undo and redo,
profiles, settings and a tray icon. Audio and screen effects read live loopback audio and the
desktop. The Devices tab lists the lighting hardware it recognises and says what could drive it.

The Fans tab shows the temperatures the service can read (AMD graphics cards and drives so far)
and has a curve editor. So far it can only drive simulated fans.

No real hardware has been driven:

- The OpenRGB bridge is written and tested against stand-in servers only.
- The native HID drivers (MSI Mystic Light, Logitech HID++) are written, untested on hardware and
  off by default.
- Fan headers and GPU fans need a hardware helper that is not built.
- The background service has only been run as an ordinary program, never installed as a Windows
  service.
- SMBus RAM and GPU lighting are not started.

[PLAN.md](PLAN.md) says what is done, how far each part was checked and what comes next.

## Layout

- `crates/foxrgb-core` - colours, devices, effects, the render engine and the `Backend` trait
- `crates/foxrgb-openrgb` - client for an OpenRGB SDK server
- `crates/foxrgb-hid` - native USB HID drivers
- `crates/foxrgb-audio` - loopback capture and a 16-band spectrum
- `crates/foxrgb-screen` - desktop capture
- `crates/foxrgb-fan` - fan curves, the fan controller and simulated fans
- `crates/foxrgb-hwmon` - read-only temperature sensors
- `crates/foxrgb-service` - background service that runs the fan loop
- `crates/foxrgb-client` - client for the service's local pipe
- `src-tauri` - the desktop app (Tauri 2)
- `ui` - the preview and controls, plain HTML/CSS/JS with no build step
- `packaging` - installer configuration, see [packaging/README.md](packaging/README.md)

## Running

Needs Rust and the WebView2 runtime (included with Windows 11).

```powershell
cargo build --workspace
cargo run -p foxrgb
cargo test --workspace
```

Build the whole workspace first. The Fans tab starts `foxrgb-service.exe` from the folder the app
runs from, and `cargo run -p foxrgb` alone does not build it. The app starts the service as an
ordinary program for the session, needs no administrator rights for that, and stops it when it
quits.

Everything the app keeps is in `%APPDATA%\com.foxrgb.desktop`: the settings and profiles in
`config.json`, the files of a service the app started in `service`, and the web view's own data,
which holds the view options, in `webview`. Set `FOXRGB_CONFIG_DIR` to use another folder for all
of it.

The service also runs by hand, with `--sim` to start with simulated fans:

```powershell
cargo run -p foxrgb-service -- run --sim
```

Run that way it keeps its files in the same `service` folder as one the app starts, or in the
folder `FOXRGB_DATA_DIR` names.

Registering it as a Windows service is written but has never been tried. `foxrgb-service install`
only registers a copy that ordinary users cannot replace, such as one in a folder under Program
Files. An installed service keeps its files in `%ProgramData%\FoxRGB`, which it makes for SYSTEM
and Administrators only and refuses to use when anyone else can write there.

`ui/index.html` also opens in a plain browser, where `ui/mock.js` and `ui/fans-mock.js` stand in
for the app.
