<p align="center"><img src="src-tauri/icons/icon.png" width="128" alt="FoxRGB icon"></p>

# FoxRGB

[![Release](https://img.shields.io/github/v/release/AallynReed/FoxRGB)](https://github.com/AallynReed/FoxRGB/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/AallynReed/FoxRGB/total)](https://github.com/AallynReed/FoxRGB/releases)
[![Stars](https://img.shields.io/github/stars/AallynReed/FoxRGB?style=flat)](https://github.com/AallynReed/FoxRGB/stargazers)
[![Build](https://img.shields.io/github/actions/workflow/status/AallynReed/FoxRGB/release.yml?label=build)](https://github.com/AallynReed/FoxRGB/actions)
[![Issues](https://img.shields.io/github/issues/AallynReed/FoxRGB)](https://github.com/AallynReed/FoxRGB/issues)
[![Rust](https://img.shields.io/badge/-Rust-B7410E?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Tauri](https://img.shields.io/badge/-Tauri%202-24C8D8?logo=tauri&logoColor=white)](https://tauri.app)
[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4)](https://github.com/AallynReed/FoxRGB/releases/latest)
[![Licence](https://img.shields.io/github/license/AallynReed/FoxRGB)](LICENSE)
[![Discord](https://img.shields.io/badge/Discord-Foxhearth-5865F2?logo=discord&logoColor=white)](https://discord.gg/VGTaqW3vwn)

Free lighting and fan control for Windows. One effect runs across all your devices as a single
animation, with its own drivers and nothing else to install.

## Devices

Keyboards, mice, headsets, coolers, RAM, motherboards and graphics cards from Razer, Logitech,
Corsair, SteelSeries, ASUS, MSI, Gigabyte, NZXT, HyperX, Elgato Stream Deck and many more.

Tested so far on MSI Mystic Light boards, Logitech G502 and G515, and Corsair Vengeance RGB
DDR5. If yours works or doesn't, [open an issue](../../issues/new/choose).

OpenRGB can optionally drive anything else through its server.

## Features

- Effects, including audio, screen and key-press reactive ones
- Layout editor and profiles
- Fan curves for board headers, AIO pumps and AMD graphics cards
- Lighting and Fans modules you can switch off
- Closes conflicting software such as SignalRGB, G HUB and iCUE
- Starts with Windows and updates itself

## Install

Download the setup program from [releases](../../releases) and run it. It installs FoxRGB, the
[PawnIO](https://pawnio.eu) driver and a small service for RAM and fans.

Windows SmartScreen warns about it because it is not code signed.

## Build

```powershell
cargo build --workspace
cargo run -p foxrgb
```

## Licence

GPL-2.0-or-later.
Bundled PawnIO modules are LGPL-2.1.
