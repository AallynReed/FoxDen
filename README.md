<p align="center"><img src="src-tauri/icons/icon.png" width="128" alt="FoxRGB icon"></p>

# FoxRGB

[![Discord](https://img.shields.io/badge/Discord-Foxhearth-5865F2?logo=discord&logoColor=white)](https://discord.gg/VGTaqW3vwn)

Free lighting and fan control for Windows, written in Rust. Standalone: it drives the hardware
with its own drivers, with nothing else to install.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as one animation.

## Devices

Seen working on real hardware:

- MSI Mystic Light boards (185-byte generation, such as the B760M Gaming Plus WiFi)
- Logitech Lightspeed mice and keyboards over HID++ (G502 Lightspeed, G515 Lightspeed TKL)
- Corsair DRAM: Vengeance RGB, Vengeance RGB Pro and RS, Dominator Platinum and Titanium, DDR4 and DDR5

More are being ported from [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB). An optional
bridge to a running OpenRGB server covers the rest until then.

## Features

- Sixteen effects, including audio and screen reactive ones
- Layout editor with undo and redo, profiles, tray icon
- Hardware detection, with devices picked up when they are plugged in or wake up
- Fan curves in a background service (temperatures are read; fan headers are not driven yet)
- Starts with Windows, in the tray
- Updates itself from GitHub releases

## Install

Download `FoxRGB_<version>_x64-setup.exe` from the [releases](../../releases) and run it. It asks
for administrator rights once, to:

- install FoxRGB into Program Files
- install [PawnIO](https://pawnio.eu), the signed driver that reaches the RAM's SMBus
- register the FoxRGB service, which drives the RAM so the app itself runs without admin

Two programs cannot drive the same device, so FoxRGB closes other lighting and fan software
(SignalRGB, Logitech G HUB, iCUE, Fan Control and similar) when it starts. Turn this off under
Settings, Conflicting software. With both modules (Lighting and Fans) off, FoxRGB removes its
service and PawnIO; turning one on puts them back.

The setup program is not code signed, so Windows SmartScreen warns about it. Uninstall from
Settings, Apps; PawnIO stays installed.

## Build

Needs a current stable Rust and the WebView2 runtime.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p foxrgb
```

Releases are built on GitHub only: pushing a tag such as `v0.2.0` runs
`.github/workflows/release.yml`, which builds and signs the setup program and drafts the release.
[packaging/README.md](packaging/README.md) has the details.

## Layout

- `crates/foxrgb-core` - colours, devices, effects, the render engine and the `Backend` trait
- `crates/foxrgb-hid` - USB HID drivers
- `crates/foxrgb-smbus` - SMBus drivers through PawnIO
- `crates/foxrgb-openrgb` - optional client for an OpenRGB server
- `crates/foxrgb-audio`, `crates/foxrgb-screen` - audio and desktop capture
- `crates/foxrgb-fan`, `crates/foxrgb-hwmon` - fan curves and temperature sensors
- `crates/foxrgb-service`, `crates/foxrgb-client` - the background service and its pipe client
- `src-tauri` - the desktop app (Tauri 2)
- `ui` - plain HTML, CSS and JS with no build step
- `packaging` - installer configuration

## Licence

GPL-2.0-or-later. Device protocols are ported from OpenRGB (GPL-2.0-or-later). The bundled
PawnIO module is LGPL-2.1.
