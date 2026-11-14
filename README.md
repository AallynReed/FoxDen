# FoxRGB

Free RGB lighting control for Windows, written in Rust.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as a single animation instead of each device doing its own thing.

## Status

Early. The effects engine and a live preview work. No hardware is driven yet.

Planned backends:

- Native drivers for USB HID devices and SMBus RAM/GPU lighting
- A bridge to an OpenRGB server for everything else

## Layout

- `crates/foxrgb-core` - colours, devices, effects, the render engine and the `Backend` trait
- `src-tauri` - the desktop app (Tauri 2)
- `ui` - the preview and controls, plain HTML/CSS/JS with no build step

## Running

Needs Rust and the WebView2 runtime (included with Windows 11).

```
cargo run -p foxrgb
cargo test -p foxrgb-core
```
