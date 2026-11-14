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

The setup program has been built this way and unpacked to see what it carries. It has never been
run.

The first line keeps your user folder, and with it your Windows user name, out of the binaries.
Without it they contain the paths of the dependency sources under `%USERPROFILE%\.cargo`.

Neither the setup program nor the binaries are signed.

## Releases

A setup program built on your own machine is for testing. Releases are built on GitHub.

`.github/workflows/build.yml` runs the tests and clippy and builds the same installer there. It
runs when started by hand, from the Actions tab or with `gh workflow run build.yml`, and as part
of a release.

`.github/workflows/release.yml` runs when a tag such as `v0.1.0` is pushed. It checks that the tag
is on `main` and names the version in `Cargo.toml` and `src-tauri/tauri.conf.json`, builds the
installer with `build.yml`, without the build cache, and drafts a release that carries the setup
program and its SHA-256 sum. The draft is published by hand.

A run that failed through no fault of the code can be run again from the Actions tab. When the
code needs a fix, take the tag away, commit the fix and tag again:

```powershell
git push origin :refs/tags/v0.1.0
git tag -d v0.1.0
```

Neither workflow has run yet.

## The two config files

`src-tauri/tauri.conf.json` holds everything a development build may read, so `cargo run` never
depends on a release binary. `tauri.bundle.json` switches bundling on and names the files that ship
beside the app. Without it `cargo tauri build` makes no installer.

## The service

The installer copies `foxrgb-service.exe` and does nothing else with it. Registering it as a
Windows service stays a manual step from an elevated prompt, and not from the per-user folder: a
service running as LocalSystem must not start from a directory the user can write to, so
`foxrgb-service install` refuses there. Copy the file to a folder under Program Files first and
run `install` from that copy.

Quit FoxRGB from its tray menu before updating or uninstalling. A service the app started stops
when the app does, however the app was closed, but it needs a moment to hand the fans back, and
until then `foxrgb-service.exe` is in use. The installer only looks for a running `foxrgb.exe`.

Uninstalling leaves `%APPDATA%\com.foxrgb.desktop`, where the settings, the profiles, the web view's
data and the files of a service the app started are, unless "Delete the application data" is ticked.
The folder is named after the app's identifier because that is the one the box removes.

## Icons

`src-tauri/icons/icon.svg` is the icon. `make-icons.ps1` beside it renders every icon file from it
with `cargo tauri icon`. Run it again after changing the drawing.

The app takes its window and tray icon from the exe at the large icon size, 32 px at 100 % scaling,
and Windows shrinks that for the title bar and the tray. Half-transparent edge pixels come out dark
when it does, which the icon's dark tile hides.
