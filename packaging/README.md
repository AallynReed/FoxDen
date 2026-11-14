# Packaging

The installer is an NSIS setup program. It installs for the current user under
`%LOCALAPPDATA%\FoxRGB` and asks for no administrator rights. It carries `foxrgb.exe`,
`foxrgb-service.exe` beside it and the licence.

## Building the installer

The Tauri CLI is needed once:

```powershell
cargo install tauri-cli --version "^2" --locked
```

Then, from the repository root:

```powershell
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~"
cargo tauri build --config packaging/tauri.bundle.json
Remove-Item Env:CARGO_ENCODED_RUSTFLAGS
```

That builds the service, then the app, and writes
`target\release\bundle\nsis\FoxRGB_<version>_x64-setup.exe`. Leave `CARGO_TARGET_DIR` unset: the
service is picked up from `target\release`. The first build also downloads NSIS into
`%LOCALAPPDATA%\tauri`.

The setup program has been built this way. It has never been run.

The first line keeps your user folder, and with it your Windows user name, out of the binaries.
Without it they contain the paths of the dependency sources under `%USERPROFILE%\.cargo`.

`.github/workflows/build.yml` builds the same installer on GitHub. It only runs when started by
hand, from the Actions tab or with `gh workflow run build.yml`.

## The two config files

`src-tauri/tauri.conf.json` holds everything a development build may read, so `cargo run` never
depends on a release binary. `tauri.bundle.json` switches bundling on and names the files that ship
beside the app. Without it `cargo tauri build` makes no installer.

## The service

The installer copies `foxrgb-service.exe` and does nothing else with it. Registering it as a
Windows service stays a manual step from an elevated prompt. Do not register it from the per-user
folder: a service running as LocalSystem must not start from a directory the user can write to.

Quit FoxRGB from its tray menu before updating or uninstalling. A service the app started stops
when the app does, however the app was closed, but it needs a moment to hand the fans back, and
until then `foxrgb-service.exe` is in use. The installer only looks for a running `foxrgb.exe`.

Uninstalling leaves `%APPDATA%\com.foxrgb.desktop`, where the settings, the profiles, the web view's
data and the files of a service the app started are, unless "Delete the application data" is ticked.
The folder is named after the app's identifier because that is the one the box removes.

## Icons

`src-tauri/icons/make-icons.ps1` draws every icon file. Run it again after changing the design.

The app takes its window and tray icon from the exe at the large icon size, 32 px at 100 % scaling,
and Windows shrinks that for the title bar and the tray. Half-transparent edge pixels come out dark
when it does, which is why the outline has none.
