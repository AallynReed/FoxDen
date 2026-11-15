<p align="center"><img src="src-tauri/icons/icon.png" width="128" alt="FoxRGB icon"></p>

# FoxRGB

[![Discord](https://img.shields.io/badge/Discord-Foxhearth-5865F2?logo=discord&logoColor=white)](https://discord.gg/VGTaqW3vwn)

Free lighting and fan control for Windows, written in Rust. Standalone: it drives the hardware
with its own drivers, with nothing else to install.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as one animation.

## Devices

FoxRGB's own drivers cover about 1,000 devices, ported from
[OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB):

- USB: Razer, Logitech, Corsair, SteelSeries, Cooler Master, MSI, Gigabyte, ASRock, NZXT,
  HyperX, Roccat, Lian Li, Thermaltake, Wooting, Elgato Stream Deck, Windows LampArray devices
  and about 70 more brands
- Memory and motherboards over SMBus: ASUS Aura and ENE, Corsair, Kingston Fury, HyperX,
  Patriot, T-Force, Crucial, Gigabyte, EVGA
- Graphics cards over their own I2C bus: ASUS, Gigabyte, EVGA, Palit, Sapphire, PNY,
  PowerColor, Galax, Colorful, Zotac

Seen working on real hardware so far: MSI Mystic Light boards, Logitech G502 and G515
Lightspeed (wireless and on a cable), Corsair Vengeance RGB DDR5. The rest follow OpenRGB's
protocols byte for byte and need testing by their owners; reports are welcome.

An optional OpenRGB integration drives anything else through a running OpenRGB server.

## Features

- Sixteen effects, including audio and screen reactive ones
- Layout editor with undo and redo, profiles, tray icon
- Hardware detection, with devices picked up when they are plugged in or wake up
- Fan control: CPU, board and drive temperatures, curves for board fan headers, the AIO pump
  (never below 70%) and AMD graphics card fans
- Modules: switch Lighting or Fans off and FoxRGB lets go of that hardware
- Closes conflicting lighting and fan software
- Starts with Windows, in the tray
- Updates itself from GitHub releases and tells you when a new one is out

## Install

Download `FoxRGB_<version>_x64-setup.exe` from the [releases](../../releases) and run it. It asks
for administrator rights once, to:

- install FoxRGB into Program Files
- install [PawnIO](https://pawnio.eu), the signed driver that reaches the SMBus and the fan chip
- register the FoxRGB service, which drives RAM and fans so the app itself runs without admin

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

Releases are built on GitHub only: pushing a tag such as `v0.3.0` runs
`.github/workflows/release.yml`, which builds and signs the setup program and drafts the release.
[packaging/README.md](packaging/README.md) has the details.

## Layout

- `crates/foxrgb-core` - colours, devices, effects, the render engine and the `Backend` trait
- `crates/foxrgb-hid` - USB HID drivers
- `crates/foxrgb-smbus` - SMBus drivers through PawnIO
- `crates/foxrgb-gpu` - graphics card drivers over the AMD and NVIDIA I2C buses
- `crates/foxrgb-pawnio` - the PawnIO driver interface
- `crates/foxrgb-superio`, `crates/foxrgb-adlx` - board fan chip, CPU temperature, AMD card fans
- `crates/foxrgb-openrgb` - optional client for an OpenRGB server
- `crates/foxrgb-audio`, `crates/foxrgb-screen` - audio and desktop capture
- `crates/foxrgb-fan`, `crates/foxrgb-hwmon` - fan curves and temperature sensors
- `crates/foxrgb-service`, `crates/foxrgb-client` - the background service and its pipe client
- `src-tauri` - the desktop app (Tauri 2)
- `ui` - plain HTML, CSS and JS with no build step
- `packaging` - installer configuration

## Licence

GPL-2.0-or-later. Device protocols are ported from OpenRGB (GPL-2.0-or-later). The bundled
PawnIO modules are LGPL-2.1.
