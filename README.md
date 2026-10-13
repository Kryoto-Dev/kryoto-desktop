<p align="center"><img src="docs/logo.png" width="80" alt="Kryoto Desktop"></p>

# Kryoto Desktop

Find, download and play Kryoto games in one app. Windows and Linux.

**Kryoto** · Tauri 2 (Rust) + React · MIT · [Download](https://kryo.to/desktop) · [All repos](https://github.com/Kryoto-Dev)

![Library](docs/screenshots/library.png)

## Quick start

Clone [`kryoto-messaging`](https://github.com/Kryoto-Dev/kryoto-messaging) and
[`kryoto-padmap`](https://github.com/Kryoto-Dev/kryoto-padmap) next to this repo, then:

```bash
pnpm install
pnpm app:dev      # the app
pnpm dev          # the UI alone in a browser, on sample data (http://localhost:1421)
pnpm app:build    # installers (NSIS on Windows, AppImage/.deb on Linux)
```

Debug builds run as `to.kryo.desktop.dev`, beside an installed copy, without sharing its data.

## Layout

| Path | What's there |
|---|---|
| `src-tauri/src/` | native side: downloads, launching, library, storage, tray, updater, logs |
| `src/` | the UI: `shell`, `library`, `downloads`, `settings`, `community`, `friends`, `boot` |
| `crates/kryoto-repair` | the game repair engine (shared with the auto-fixer) |
| `scripts/` | build, brand art and UI checks |
| `docs/` | [development guide](docs/development.md), logo and screenshots |

## Checks

```bash
pnpm build                                          # typecheck + build the UI
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
```

CI runs these on Ubuntu and Windows for every push and pull request.

## Release

Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
together, add the entry to kryo.to's changelog, and push to `main`. The release workflow builds,
signs and publishes `v<version>`; installed copies update themselves.
Signing keys and the rest: [docs/development.md](docs/development.md#releasing).

## License

[MIT](LICENSE).
