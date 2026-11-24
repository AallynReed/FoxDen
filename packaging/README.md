# Packaging

The installer is an NSIS setup program built from `native.nsi`. It installs for all users into Program
Files and asks for administrator rights once. `installer-hooks.nsh` then:

- runs PawnIO's official setup silently (`PawnIO_setup.exe -install -silent`) and keeps it in the
  install folder, so the app can remove or reinstall PawnIO when its modules are switched
- registers and starts `foxden-service.exe` as a Windows service
- stops the service before an update and removes it on uninstall
- on uninstall, removes VB-CABLE only if FoxDen installed it

FoxDen does not ship VB-Audio's VB-CABLE. For the virtual microphone the user installs it from
https://vb-audio.com/Cable/; FoxDen then finds it by hardware id, renames its endpoints and uses it.

## Releases

Releases are built on GitHub only. A setup program built on your own machine is for testing.

- `.github/workflows/build.yml` runs the tests and clippy, downloads PawnIO's setup (checked
  against its SHA-256) and builds the installer with its update signature. Start it by hand to
  test.
- `.github/workflows/release.yml` runs on a tag such as `v0.2.0`. It checks the tag is on `main`
  and matches the workspace version in `Cargo.toml`, builds with
  `build.yml` and drafts a release with the installer, its signature, `latest.json` and
  `SHA256SUMS.txt`. Publish the draft by hand.

To retry a release after a fix:

```powershell
git push origin :refs/tags/v0.2.0
git tag -d v0.2.0
```

## Updates

The app checks `releases/latest/download/latest.json` a minute after start and every hour,
and installs signed updates by itself. The signing key is the `TAURI_SIGNING_PRIVATE_KEY`
secret; the public key is `PUBKEY` in `native/src/updater.rs`. Losing the private key means no
further update can reach installed copies.

Updates run the installer with `/P /UPDATE`: it skips its pages and starts the app again when done.

## Local test build

```powershell
Invoke-WebRequest https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe -OutFile packaging/PawnIO_setup.exe
cargo build --release -p foxden -p foxden-service -p foxden-mcp
makensis /DVERSION=0.18.0 /DAPP=target\release\foxden.exe /DSERVICE=target\release\foxden-service.exe /DMCP=target\release\foxden-mcp.exe /DPAWNIO=packaging\PawnIO_setup.exe /DOUT=FoxDen-setup.exe packaging\native.nsi
```

## Icons

`native/icons/icon.svg` is the icon. `native/icons/make-icons.ps1` renders every icon file
from it.
