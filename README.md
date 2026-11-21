<div align="center">

<img src="src-tauri/icons/icon.png" width="128" alt="FoxDen icon">

# FoxDen

<!-- Project status -->
[![GitHub Release](https://img.shields.io/github/v/release/AallynReed/FoxDen?style=for-the-badge&color=blue&cacheSeconds=21600)](https://github.com/AallynReed/FoxDen/releases/latest)
![GitHub Repo stars](https://img.shields.io/github/stars/AallynReed/FoxDen?style=for-the-badge&color=gold&cacheSeconds=21600)
[![GitHub Downloads](https://img.shields.io/github/downloads/AallynReed/FoxDen/total?style=for-the-badge&cacheSeconds=21600)](https://github.com/AallynReed/FoxDen/releases)
![GitHub last commit](https://img.shields.io/github/last-commit/AallynReed/FoxDen?style=for-the-badge&color=green&cacheSeconds=21600)
![GitHub issues](https://img.shields.io/github/issues/AallynReed/FoxDen?style=for-the-badge&cacheSeconds=21600)
[![Build](https://img.shields.io/github/actions/workflow/status/AallynReed/FoxDen/release.yml?style=for-the-badge&label=build&cacheSeconds=21600)](https://github.com/AallynReed/FoxDen/actions)

<!-- Platforms -->
![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white)

<!-- Tech stack -->
![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri_2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white)

<!-- Project meta -->
![License](https://img.shields.io/badge/License-GPL--2.0--or--later-8A2BE2?style=for-the-badge)
[![Discord](https://img.shields.io/badge/Discord-Foxhearth-5865F2?style=for-the-badge&logo=discord&logoColor=white)](https://discord.gg/VGTaqW3vwn)
[![foxhearth.com](https://img.shields.io/badge/foxhearth.com-project_page-E8702A?style=for-the-badge)](https://foxhearth.com/projects/foxden)

</div>

Free lighting, fan, audio and battery control for Windows, with its own drivers and no vendor
software. One effect runs across all your devices as a single animation.

## Devices

Over 2,000 devices: keyboards, mice, headsets, RAM, motherboards, graphics cards, coolers, fan
hubs, LED strips and network lights.

<p align="center">
<img src=".github/brands/razer.svg" height="28" alt="Razer" title="Razer">&nbsp;&nbsp;
<img src=".github/brands/logitechg.svg" height="28" alt="Logitech G" title="Logitech G">&nbsp;&nbsp;
<img src=".github/brands/corsair.svg" height="28" alt="Corsair" title="Corsair">&nbsp;&nbsp;
<img src=".github/brands/steelseries.svg" height="28" alt="SteelSeries" title="SteelSeries">&nbsp;&nbsp;
<img src=".github/brands/hyperx.svg" height="28" alt="HyperX" title="HyperX">&nbsp;&nbsp;
<img src=".github/brands/asus.svg" height="28" alt="ASUS" title="ASUS">&nbsp;&nbsp;
<img src=".github/brands/msi.svg" height="28" alt="MSI" title="MSI">&nbsp;&nbsp;
<img src=".github/brands/gigabyte.svg" height="16" alt="Gigabyte" title="Gigabyte">&nbsp;&nbsp;
<img src=".github/brands/asrock.svg" height="16" alt="ASRock" title="ASRock">&nbsp;&nbsp;
<img src=".github/brands/nvidia.svg" height="28" alt="NVIDIA" title="NVIDIA">&nbsp;&nbsp;
<img src=".github/brands/alienware.svg" height="28" alt="Alienware" title="Alienware">&nbsp;&nbsp;
<img src=".github/brands/kingstontechnology.svg" height="28" alt="Kingston" title="Kingston">&nbsp;&nbsp;
<img src=".github/brands/nzxt.svg" height="28" alt="NZXT" title="NZXT">&nbsp;&nbsp;
<img src=".github/brands/coolermaster.svg" height="28" alt="Cooler Master" title="Cooler Master">&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset=".github/brands/lianli-dark.svg"><img src=".github/brands/lianli.svg" height="16" alt="Lian Li" title="Lian Li"></picture>&nbsp;&nbsp;
<img src=".github/brands/thermaltake.svg" height="16" alt="Thermaltake" title="Thermaltake">&nbsp;&nbsp;
<img src=".github/brands/aquacomputer.gif" height="16" alt="Aqua Computer" title="Aqua Computer">&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset=".github/brands/keychron-dark.svg"><img src=".github/brands/keychron.svg" height="16" alt="Keychron" title="Keychron"></picture>&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset=".github/brands/wooting-dark.svg"><img src=".github/brands/wooting.svg" height="28" alt="Wooting" title="Wooting"></picture>&nbsp;&nbsp;
<img src=".github/brands/elgato.svg" height="28" alt="Elgato" title="Elgato">&nbsp;&nbsp;
<img src=".github/brands/philipshue.svg" height="28" alt="Philips Hue" title="Philips Hue">&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset=".github/brands/nanoleaf-dark.svg"><img src=".github/brands/nanoleaf.svg" height="16" alt="Nanoleaf" title="Nanoleaf"></picture>&nbsp;&nbsp;
<img src=".github/brands/lifx.svg" height="28" alt="LIFX" title="LIFX">&nbsp;&nbsp;
<picture><source media="(prefers-color-scheme: dark)" srcset=".github/brands/govee-dark.svg"><img src=".github/brands/govee.svg" height="16" alt="Govee" title="Govee"></picture>
</p>

And many more.

Tested so far on MSI Mystic Light boards, Logitech G502 and G515, and Corsair Vengeance RGB
DDR5. If yours works or doesn't, [open an issue](../../issues/new/choose).

OpenRGB can optionally drive anything else through its server.

## Features

- Effects, including audio, screen and key-press reactive ones
- Layers over the background with blend modes and opacity, dragged into order
- Chosen keys in your own colours, and layers that show only while a game or app is open or in front
- Layout editor that draws keyboards, mice, RAM, fans and boards as their real shapes, plus per-device backgrounds and profiles
- Fan curves for motherboard headers, coolers, fan hubs and AMD graphics cards, with zero RPM
- Battery levels of wireless mice, keyboards, headsets and controllers as tray icons, for over 100 brands
- Audio mixer: send each app to a Game, Chat, Media or Aux channel with its own volume, EQ and chat
  mix, no virtual driver needed; a headset's ChatMix dial drives the mix
- Headset settings for SteelSeries Arctis Nova headsets (sidetone, mic volume, limiter, auto-off)
- LED strips and network lights
- Lighting and Fans modules you can switch off
- Closes conflicting software such as SignalRGB, G HUB and iCUE, unless you keep it running
- Send device info for anything not supported yet
- Starts with Windows and updates itself

## Install

Download the setup program from [releases](../../releases) and run it. It installs FoxDen, the
[PawnIO](https://pawnio.eu) driver and a small service for RAM and fans.

Windows SmartScreen warns about it because it is not code signed.

## Build

```powershell
cargo build --workspace
cargo run -p foxden
```

## Privacy

FoxDen only connects to the places below. Its window's WebView2 runs with Edge's background
services, SmartScreen and telemetry switched off.

- GitHub, a minute after start and then every hour, to check for a new release. Nothing
  is sent beyond the request itself.
- `reports.foxhearth.com`, only if you switch on anonymous error reports (off by default). A
  report holds the error, the FoxDen version and the hardware ids of your devices, never names,
  serial numbers, paths or addresses.
- The OpenRGB server you set, `127.0.0.1` unless you change it, to see whether it is running.
- Network lights (Hue, Wiz, Nanoleaf, LIFX, Govee and the rest), only if you switch them on (off by
  default), and only at addresses on your local network, never beyond it.

## Licence

GPL-2.0-or-later.
Bundled PawnIO modules are LGPL-2.1.
