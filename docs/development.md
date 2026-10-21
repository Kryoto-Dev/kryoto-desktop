# Developing Kryoto Desktop

The details behind the README's quick start.

## Running and building

The supported build toolchain is pinned in the repository: Node `22.14.0`
(`.nvmrc`), pnpm `10.28.0` (`package.json`), and Rust `1.97.1`
(`rust-toolchain.toml`). `pnpm-lock.yaml` and `src-tauri/Cargo.lock` are part
of the source of truth; do not replace them with an unlocked install.

```text
pnpm install --frozen-lockfile
pnpm app:dev      # the app
pnpm dev          # the UI alone in a browser, on sample data (http://localhost:1421)
pnpm build        # typecheck + build the UI
pnpm preview      # serve dist/ after pnpm build (http://localhost:4173)
pnpm app:build    # release bundle for the current OS
pnpm app:build:local   # an unsigned installer for this PC, copied out of the repo
pnpm app:build:mac     # macOS only: explicitly build the app and DMG targets
```

The native crate pins the `kryoto-messaging` and `kryoto-padmap` git revisions
in `Cargo.toml`, so a clean checkout no longer needs sibling repositories.

### Linux prerequisites

Ubuntu 22.04/24.04 needs the Tauri/WebKitGTK and native build packages used by
CI and the Dockerfile:

Run the following as a system administrator, or use the Dockerfile for a
non-host installation:

```bash
apt-get update
apt-get install -y build-essential pkg-config perl \
  libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev libssl-dev libsqlite3-dev libxdo-dev libdbus-1-dev
```

The exact package names vary by distribution. A headless build/test run that
does not need a display is reproducible with `docker build -t kryoto-desktop .`
followed by `docker run --rm kryoto-desktop`.

### macOS

Build on macOS with Xcode Command Line Tools, Node/pnpm and the pinned Rust
toolchain. The Sikarugir backend requires macOS 14.6 or newer; Apple silicon
also needs Rosetta 2. `pnpm app:build:mac` merges `src-tauri/tauri.macos.conf.json`, which
selects the `.app` and `.dmg` targets, bundles uBlock resources and sets the
minimum system version. `src-tauri/Info.plist` declares `kryoto://`; the native
side handles Launch Services `Opened` events and keeps the link pending until
the shell is ready. Compatibility wrappers are discovered in
`~/Applications/Sikarugir`.

The native Compatibility settings can install the official Sikarugir Creator
Homebrew cask (and Rosetta on Apple silicon) after the user presses the install
button. Creator is intentionally opened separately: its public project does not
document a command-line or IPC interface for creating wrappers. A person must
select the Windows executable and engine in Creator, save the wrapper to
`~/Applications/Sikarugir`, and refresh Kryoto before the wrapper is detected.

macOS local builds are intentionally unsigned unless the machine has an Apple
Developer signing identity. Notarization is a release-owner operation and is
not attempted by local builds or preview CI.

`app:build` signs for the updater, so with `TAURI_SIGNING_PRIVATE_KEY` set it asks
for that key's password (or reads `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). To try a
build on your own PC use `app:build:local`: no key, no password, and the installer
is copied to `D:\kryo-builds\kryoto-desktop` (or `~/kryoto-builds/kryoto-desktop`;
set `KRYOTO_LOCAL_OUT` to change it). Run it from there: a program started from
inside a workspace folder can be limited to writing inside it, which stops the
installer and the app from saving anything.

`app:build` remaps the build machine's paths out of the binary and refuses to
finish if the home folder is still in it.

Debug builds run as `to.kryo.desktop.dev`, next to an installed copy without
sharing its data, and never send error reports.

`pnpm test` is the single offline-friendly project test entry point: it builds
the UI and runs the desktop and shared repair-engine suites with locked
dependencies and SQLCipher enabled. On Windows GNU builds, OpenSSL needs a complete
GNU-compatible Perl and GNU make. Browser checks use `test/ui.html` and
`test/bridge.html` through the dev server: with Playwright and Chromium
available, run `node scripts/check-ui.mjs`. Set `DESKTOP_TEST_URL` for a
different dev port or `PLAYWRIGHT_BROWSER_CHANNEL=msedge` to use installed
Edge.

Website-triggered downloads require the matching website command-delivery migration and protocol 2 endpoints; see [the delivery contract](../kryo.to/docs/desktop-delivery.md). Older clients leave actions waiting for an update.

## Tests

```text
pnpm test                    # UI build + desktop + repair-engine tests
cd src-tauri && cargo clippy --locked --all-targets -- -D warnings
pnpm preview --host 127.0.0.1 # preview the production UI after pnpm test
```

CI (`.github/workflows/ci.yml`) runs the UI/browser checks on Ubuntu and native
tests/clippy on Ubuntu, Windows and macOS for every push and pull request. It
also starts `pnpm preview` and verifies that the production UI responds. The
workflow checks that the three version fields agree.

## Linux graphics

WebKitGTK's fast (DMA-BUF) renderer breaks on NVIDIA's driver and on machines
without a GPU render device, and works everywhere else. Kryoto picks per
machine (Settings > General > Graphics: Automatic, Compatible, Full), and if a
start never draws its window, the next one switches to Compatible by itself.
To force a mode for one run: `KRYOTO_GPU=compatible kryoto` (or `full`).
`WEBKIT_DISABLE_DMABUF_RENDERER` and `WEBKIT_DISABLE_COMPOSITING_MODE`, when
you set them yourself, always win.

## Releasing

Bump the version in `package.json`, `src-tauri/Cargo.toml` and
`src-tauri/tauri.conf.json` together, add the entry to kryo.to's changelog,
and push to main. `.github/workflows/release.yml` sees the new version, builds
the Windows installer, Linux AppImage/.deb/.rpm, and the macOS app/DMG with
`pnpm app:build`, and publishes them as release `v<version>`. kryo.to/desktop
offers the newest release by itself.

The same release carries the updater's files: a `.sig` beside each installer
and `latest.json`, which every installed copy reads from
`releases/latest/download/latest.json`. They are signed with the repository
secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
a key of Kryoto Desktop's own whose public half is `plugins.updater.pubkey` in
`tauri.conf.json`. The release refuses to build without them, because a
release with no `latest.json` is one no installed copy ever hears about, and
refuses a key that does not match that public half.

Keep the private key and its password somewhere besides the repository
secrets (GitHub never shows a secret again). Losing it means a new key pair,
and copies installed with the old public key cannot update past that point:
they need one manual reinstall.

The macOS release job runs on `macos-14` and can produce unsigned artifacts
without any Apple credentials. To ship a Gatekeeper-friendly build, the
release runner must also have an Apple Developer signing identity in its
keychain and a separate notarization step with organization-owned credentials.
Those credentials are deliberately not present in this repository or its
workflows.

Keep `@tauri-apps/api` and `@tauri-apps/cli` on the same minor version as the
`tauri` crate in `Cargo.lock`: `tauri build` stops on a mismatch.

`KRYOTO_STAND_IN=<folder with "Captain Hardcore.exe"> cargo test -- --ignored`
also starts a real process the way Play does, against a stand-in that writes
its arguments to `launch-log.txt`.

Changes are listed in kryo.to's changelog; this app has none of its own.

## Where things are (full map)

| | |
|---|---|
| `src-tauri/src/lib.rs` | the Store web view, what kryo.to reports (account, inbox, lists), commands |
| `src-tauri/src/downloads.rs` | taking over kryo.to downloads, resume, space checks, unpacking, install |
| `src-tauri/src/launch.rs` | what Play runs: Steam launch entries, launch options, Wine/Proton |
| `src-tauri/src/phone.rs` | the phone server: pairing, the library, Play/Stop, downloads, achievements for [Kryoto Mobile](https://github.com/Kryoto-Dev/kryoto-mobile) on the same network |
| `src-tauri/src/library.rs` | `library.json`, play/stop, play time, uninstall |
| `src-tauri/src/storage.rs` | library folders, volume space/labels, moving games between volumes |
| `src-tauri/src/system.rs` | tray, single instance, desktop autostart, window state |
| `src-tauri/src/menus.rs` | the menu view: menus drawn in a web view stacked over the Store, inside the main window |
| `src-tauri/src/linux_overlay.rs` | Linux: laying the Store and the menu view over the shell, and resizing from the window's edges |
| `src-tauri/src/display_env.rs` | Linux: picking WebKitGTK's renderer per machine, and falling back by itself when a start never draws |
| `src/lib/history.ts` | the one Back / Forward history for the Library, the Store's pages and everything else |
| `src/lib/updates.ts`, `src/shell/UpdatePrompt.tsx` | the in-app updater: the check on launch, the prompt, Check for updates in About |
| `src-tauri/src/logging.rs` | the log file, crash capture, reports to kryo.to |
| `src/boot` | the start box and sign-in |
| `src/shell`, `src/library`, `src/downloads`, `src/settings`, `src/community`, `src/friends` | the app |
| `src/ui/ascii` | the K// mark (traced from kryo.to's) and the block lettering, as vectors |
| `scripts/brand.mjs` | icons and installer art, generated from the same code |
