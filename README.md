<div align="center">

<img src="src-tauri/icons/icon.png" width="128" alt="FoxRGB icon">

# FoxRGB

<!-- Project status -->
[![GitHub Release](https://img.shields.io/github/v/release/AallynReed/FoxRGB?style=for-the-badge&color=blue&cacheSeconds=21600)](https://github.com/AallynReed/FoxRGB/releases/latest)
![GitHub Repo stars](https://img.shields.io/github/stars/AallynReed/FoxRGB?style=for-the-badge&color=gold&cacheSeconds=21600)
[![GitHub Downloads](https://img.shields.io/github/downloads/AallynReed/FoxRGB/total?style=for-the-badge&cacheSeconds=21600)](https://github.com/AallynReed/FoxRGB/releases)
![GitHub last commit](https://img.shields.io/github/last-commit/AallynReed/FoxRGB?style=for-the-badge&color=green&cacheSeconds=21600)
![GitHub issues](https://img.shields.io/github/issues/AallynReed/FoxRGB?style=for-the-badge&cacheSeconds=21600)
[![Build](https://img.shields.io/github/actions/workflow/status/AallynReed/FoxRGB/release.yml?style=for-the-badge&label=build&cacheSeconds=21600)](https://github.com/AallynReed/FoxRGB/actions)

<!-- Platforms -->
![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white)

<!-- Tech stack -->
![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri_2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white)

<!-- Project meta -->
![License](https://img.shields.io/badge/License-GPL--2.0--or--later-8A2BE2?style=for-the-badge)
[![Discord](https://img.shields.io/badge/Discord-Foxhearth-5865F2?style=for-the-badge&logo=discord&logoColor=white)](https://discord.gg/VGTaqW3vwn)
[![foxhearth.com](https://img.shields.io/badge/foxhearth.com-project_page-E8702A?style=for-the-badge)](https://foxhearth.com/projects/foxrgb)

</div>

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

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

- Committers and reviewers: [AallynReed](https://github.com/AallynReed)
- Approvers: [AallynReed](https://github.com/AallynReed)

## Privacy

FoxRGB only connects to:

- GitHub, a minute after start and then every six hours, to check for a new release. Nothing
  is sent beyond the request itself.
- `reports.foxhearth.com`, only if you switch on anonymous error reports (off by default). A
  report holds the error, the FoxRGB version and the hardware ids of your devices, never names,
  serial numbers, paths or addresses.
- The OpenRGB server you set, `127.0.0.1` unless you change it, to see whether it is running.
- Network lights (Hue, Wiz, Nanoleaf, LIFX, Govee and the rest), only if you switch them on (off by
  default), and only at addresses on your local network, never beyond it.

## Licence

GPL-2.0-or-later.
Bundled PawnIO modules are LGPL-2.1.
