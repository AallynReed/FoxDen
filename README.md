# FoxRGB

Free lighting and fan control for Windows, written in Rust.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as a single animation instead of each device doing its own thing. Fan curves run in a
background service and follow any temperature it can read.

## Status

Early. The app runs with simulated devices, which it marks as such: sixteen effects, a layout
editor with undo and redo, profiles, settings, zoom and a tray icon. Audio and screen effects read
live loopback audio and the desktop. The Devices tab lists the lighting hardware it recognises and
says what could drive it.

The Fans tab shows the temperatures the service can read (AMD graphics cards and drives so far)
and has a curve editor. So far it can only drive simulated fans. A fan goes to full speed when a
sensor its curve needs is lost, and back to the firmware when the service stops or gets stuck.

No real hardware has been driven:

- The OpenRGB bridge is written and tested against stand-in servers only.
- The native HID drivers (MSI Mystic Light, Logitech HID++) are written, untested on hardware and
  off by default.
- Fan headers and GPU fans need a hardware helper that is not built.
- The background service has only been run as an ordinary program, never installed as a Windows
  service.
- SMBus RAM and GPU lighting are not started.

There is no release yet. The setup program builds, and the release build of the app and the
service has been run from the build folder. The setup program itself has never been run, and
neither GitHub workflow has run.

[PLAN.md](PLAN.md) says what is done, how far each part was checked and what is left to do by hand.

## Install

There is nothing to download yet. Until the first release, build the setup program as described
under Build. It has been built and unpacked but never run, so what follows is what it is set up to
do, not what it has been seen doing.

`FoxRGB_<version>_x64-setup.exe` installs for the current user into `%LOCALAPPDATA%\FoxRGB`, asks
for no administrator rights and adds a Start menu entry. It is not signed, so Windows SmartScreen
warns about a downloaded copy. Windows 11 has the WebView2 runtime the app needs; on a machine
without it the setup program downloads it from Microsoft.

Uninstall from Settings, Apps, Installed apps. The settings and profiles in
`%APPDATA%\com.foxrgb.desktop` stay unless "Delete the application data" is ticked.

The setup program puts `foxrgb-service.exe` beside the app and does not register it. The app
starts it as an ordinary program from the Fans tab and stops it when it quits, which needs no
administrator rights.

Registering it as a Windows service, so that it runs before anyone logs in, is a step of its own.
It has never been tried, and there is little reason to yet, because the service cannot drive a real
fan. From an elevated PowerShell:

```powershell
$dir = "$env:ProgramFiles\FoxRGB"
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item "$env:LOCALAPPDATA\FoxRGB\foxrgb-service.exe" $dir
& "$dir\foxrgb-service.exe" install
& "$dir\foxrgb-service.exe" start
```

`install` only registers a copy that ordinary users cannot replace, which is why the file is
copied out of the per-user folder first. The installed service keeps its files in
`%ProgramData%\FoxRGB`, which it makes for SYSTEM and Administrators to write to and refuses to use
when anyone else can write there. To remove it again:

```powershell
& "$env:ProgramFiles\FoxRGB\foxrgb-service.exe" uninstall
```

## Build

Needs a current stable Rust (1.97 was used) and the WebView2 runtime.

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p foxrgb
```

Build the whole workspace first. The Fans tab starts `foxrgb-service.exe` from the folder the app
runs from, and `cargo run -p foxrgb` alone does not build it.

The setup program needs the Tauri CLI, once:

```powershell
cargo install tauri-cli --version "^2" --locked
```

Then, from the repository root:

```powershell
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~"
cargo tauri build --config packaging/tauri.bundle.json
Remove-Item Env:CARGO_ENCODED_RUSTFLAGS
```

That builds the service and the app in release mode and writes
`target\release\bundle\nsis\FoxRGB_<version>_x64-setup.exe`. The first line keeps your Windows user
name out of the binaries. [packaging/README.md](packaging/README.md) has the details.

A setup program built this way is for testing. Releases are built on GitHub: pushing a tag such as
`v0.1.0` runs `.github/workflows/release.yml`, which builds the setup program there and drafts a
release that carries it. The draft is published by hand.

## Running

Everything the app keeps is in `%APPDATA%\com.foxrgb.desktop`: the settings and profiles in
`config.json`, the files of a service the app started in `service`, and the web view's own data,
which holds the view options, in `webview`. Set `FOXRGB_CONFIG_DIR` to use another folder for all
of it.

The service also runs by hand, with `--sim` to start with simulated fans:

```powershell
cargo run -p foxrgb-service -- run --sim
```

Run that way it keeps its files in the same `service` folder as one the app starts, or in the
folder `FOXRGB_DATA_DIR` names. Ctrl+C stops it.

`ui/index.html` also opens in a plain browser, where `ui/mock.js` and `ui/fans-mock.js` stand in
for the app.

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
- `.github/workflows` - the build and the release on GitHub
