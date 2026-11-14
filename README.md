# FoxRGB

Free RGB lighting control for Windows, written in Rust.

One effect runs across a shared canvas, so a wave moves from the keyboard to the RAM to the
motherboard as a single animation instead of each device doing its own thing.

## Status

Early. The app runs with virtual devices: sixteen effects, a layout editor, profiles, settings and
a tray icon. Audio and screen effects read live loopback audio and the desktop.

No real hardware has been driven yet:

- The OpenRGB bridge is written and tested against a mock server only.
- The native HID drivers (MSI Mystic Light, Logitech HID++) are written, untested on hardware and
  off by default.
- Fan control has a curve engine and simulated fans only. The background service is written but
  has never been installed.
- SMBus RAM and GPU lighting are not started.

[PLAN.md](PLAN.md) says what is done, what is untested and what comes next.

## Layout

- `crates/foxrgb-core` - colours, devices, effects, the render engine and the `Backend` trait
- `crates/foxrgb-openrgb` - client for an OpenRGB SDK server
- `crates/foxrgb-hid` - native USB HID drivers
- `crates/foxrgb-audio` - loopback capture and a 16-band spectrum
- `crates/foxrgb-screen` - desktop capture
- `crates/foxrgb-fan` - fan curves and the fan controller
- `crates/foxrgb-service` - Windows service host for background control
- `src-tauri` - the desktop app (Tauri 2)
- `ui` - the preview and controls, plain HTML/CSS/JS with no build step

## Running

Needs Rust and the WebView2 runtime (included with Windows 11).

```
cargo run -p foxrgb
cargo test --workspace
```

Settings are kept in `%APPDATA%\FoxRGB\config.json`. Set `FOXRGB_CONFIG_DIR` to use another folder.

`ui/index.html` also opens in a plain browser, where `ui/mock.js` stands in for the app.
