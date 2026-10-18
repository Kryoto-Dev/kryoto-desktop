# Developing Kryoto Desktop

The details behind the README's quick start.

## Running and building

```text
pnpm install
pnpm app:dev      # the app
pnpm dev          # the UI alone in a browser, on sample data (http://localhost:1421)
pnpm app:build    # the signed release installer (NSIS on Windows, AppImage/.deb on Linux)
pnpm app:build:local   # an unsigned installer for this PC, copied out of the repo
```

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

Native tests run with `cargo test --locked --manifest-path src-tauri/Cargo.toml` and keep SQLCipher enabled. On Windows GNU builds, OpenSSL needs a complete GNU-compatible Perl and GNU make. Browser checks use `test/ui.html` and `test/bridge.html` through the dev server: with Playwright and Chromium available, run `node scripts/check-ui.mjs`. Set `DESKTOP_TEST_URL` for a different dev port or `PLAYWRIGHT_BROWSER_CHANNEL=msedge` to use installed Edge.

Website-triggered downloads require the matching website command-delivery migration and protocol 2 endpoints; see [the delivery contract](../kryo.to/docs/desktop-delivery.md). Older clients leave actions waiting for an update.

## Tests

```text
pnpm build                   # typecheck + build the UI
cd src-tauri && cargo test
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

CI (`.github/workflows/ci.yml`) runs all of that on every push and pull
request, on Ubuntu and on Windows, and checks that the three version fields
agree.

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
the Windows installer, the AppImage and the .deb with `pnpm app:build`, and
publishes them as release `v<version>`. kryo.to/desktop offers the newest
release by itself.

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
| `src-tauri/src/storage.rs` | library folders, drive space, moving games between drives |
| `src-tauri/src/system.rs` | tray, single instance, start with Windows, window state |
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
