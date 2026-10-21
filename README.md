<p align="center"><img src="docs/logo.png" width="80" alt="Kryoto Desktop"></p>

# Kryoto Desktop

Find, download and play Kryoto games in one app. Windows, Linux and macOS.

**Kryoto** · Tauri 2 (Rust) + React · MIT · [Download](https://kryo.to/desktop) · [All repos](https://github.com/Kryoto-Dev)

![Library](docs/screenshots/library.png)

## Quick start

Use Node `22.14.0`, pnpm `10.28.0` and Rust `1.97.1` (the repository pins
these in `.nvmrc`, `package.json` and `rust-toolchain.toml`). On Linux, install
the Tauri/WebKitGTK packages listed in [the development guide](docs/development.md#linux-prerequisites).
Then:

```bash
pnpm install --frozen-lockfile
pnpm app:dev      # the app
pnpm dev          # the UI alone in a browser, on sample data (http://localhost:1421)
pnpm build        # typecheck and build the UI
pnpm preview      # serve dist/ after pnpm build (http://localhost:4173)
pnpm app:build    # platform installers: NSIS, AppImage/.deb, or macOS app/DMG
pnpm app:build:local # unsigned local installer copied outside the repository
```

On a Mac, `pnpm app:build:mac` explicitly selects the `.app` and `.dmg`
targets. The Sikarugir backend requires macOS 14.6 or newer; Apple silicon
also needs Rosetta 2, as required by Sikarugir. A packaged macOS app declares
the `kryoto://` URL scheme; the first launch registers that bundle with Launch
Services.

On macOS, open **Settings > Compatibility** and choose **Install Sikarugir
Creator**. Kryoto runs the official Homebrew cask and installs Rosetta on
Apple silicon when needed. Once Creator is ready, choose **Open Sikarugir
Creator**, select the game's Windows executable and an engine in Creator, and
save the wrapper under `~/Applications/Sikarugir`. Return to Kryoto and refresh
Compatibility; the wrapper will then appear in the compatibility picker. Creator
is a GUI-only tool with no supported command-line or IPC interface, so wrapper
creation itself cannot be automated safely by Kryoto.

Debug builds run as `to.kryo.desktop.dev`, beside an installed copy, without sharing its data.

## Layout

| Path | What's there |
|---|---|
| `src-tauri/src/` | native side: downloads, launching, library, storage, tray, updater, logs |
| `src/` | the UI: `shell`, `library`, `downloads`, `settings`, `community`, `friends`, `boot` |
| `crates/kryoto-repair` | the game repair engine (shared with the auto-fixer) |
| `scripts/` | build, brand art and UI checks |
| `docs/` | [development guide](docs/development.md), logo and screenshots |
| `src-tauri/tauri.macos.conf.json` | macOS app/DMG targets and bundled resources |
| `src-tauri/Info.plist` | macOS `kryoto://` URL-scheme declaration |

## Checks

```bash
pnpm test                                           # UI + desktop + repair-engine tests
cargo clippy --locked --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
```

CI runs the UI checks on Ubuntu, native tests/clippy on Ubuntu and Windows,
and the same native checks on macOS. The preview workflow can attach unsigned
Windows, Linux and macOS installers to a pull request without creating a
release. A clean Linux build/test environment is available with:

```bash
docker build -t kryoto-desktop .
docker run --rm kryoto-desktop
```

## Release

Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
together, add the entry to kryo.to's changelog, and push to `main`. The release workflow builds,
signs and publishes `v<version>` for Windows, Linux and macOS; installed copies update themselves.
Signing keys and the rest: [docs/development.md](docs/development.md#releasing).

The macOS job produces an app and DMG on the hosted macOS runner. Apple
Developer code signing and notarization require an organization-owned
certificate/keychain and notarization credentials; no Apple credentials are
stored in this repository, so local and preview macOS artifacts may be
unsigned and Gatekeeper can warn until the release runner is configured.

## License

[MIT](LICENSE).
