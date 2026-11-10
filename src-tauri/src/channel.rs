//! Which Kryoto Desktop this is: the release, or Kryoto Desktop Preview.
//!
//! The preview app is built from the `preview` branch for members of a Kryoto
//! Preview (kryo.to docs/preview.md) by `.github/workflows/preview-release.yml`,
//! with `KRYOTO_CHANNEL=preview` and `src-tauri/tauri.preview.conf.json`. It
//! installs beside the normal app, the way a debug build does: its own
//! identifier (so its own data folder, WebView profile and settings), its own
//! single-instance port, keychain entries and autostart name, it leaves
//! `kryoto://` links to the normal app, and it talks to preview.kryo.to.

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Built as Kryoto Desktop Preview.
pub const PREVIEW: bool = match option_env!("KRYOTO_CHANNEL") {
    Some(c) => same(c, "preview"),
    None => false,
};

/// The name people see for this copy: the tray, the autostart entry.
pub const APP_NAME: &str = if cfg!(debug_assertions) {
    "Kryoto Desktop Dev"
} else if PREVIEW {
    "Kryoto Desktop Preview"
} else {
    "Kryoto Desktop"
};
