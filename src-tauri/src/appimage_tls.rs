//! The AppImage's web views on a distro that keeps its CA list elsewhere.
//!
//! The AppImage carries Ubuntu's GnuTLS, whose one trust file is built in as
//! `BUILT_IN`, and nothing (GnuTLS, glib-networking, libsoup, WebKit) can point
//! it anywhere else. Fedora has no such file, so every HTTPS request in the web
//! views failed (kryoto-bugs KB-0150). Where it is missing, this gives WebKit's
//! processes a copy of the library that reads a link of ours to the distro's
//! list instead. This process has loaded the bundled one already and does no
//! TLS through it; WebKit's start later and take `LD_LIBRARY_PATH` as it is set
//! here. The .deb and .rpm use the system's library and never come here.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};

const BUILT_IN: &str = "/etc/ssl/certs/ca-certificates.crt";

/// Where distros without `BUILT_IN` keep the same list.
const BUNDLES: [&str; 6] = [
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/var/lib/ca-certificates/ca-bundle.pem",
    "/etc/ssl/cert.pem",
];

/// Short on purpose: the path replaces `BUILT_IN` in the library, so it can
/// never be longer, whatever the user id. Not a home folder, which can be.
const PREFIX: &str = "/tmp/kryoto-ca-";

/// The folder for this user: `/tmp/kryoto-ca-1000`.
fn folder(uid: u32) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{uid}"))
}

/// A folder of ours in `LD_LIBRARY_PATH`, which programs we start must not get.
pub fn is_ours(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.starts_with(PREFIX))
}

/// The distro's list, when the built-in path is not there.
fn bundle(exists: impl Fn(&Path) -> bool, chosen: Option<PathBuf>) -> Option<PathBuf> {
    if exists(Path::new(BUILT_IN)) {
        return None;
    }
    chosen.filter(|p| exists(p)).or_else(|| BUNDLES.iter().map(PathBuf::from).find(|p| exists(p)))
}

/// The library reading `to` instead, when the path is there once and `to` fits.
fn patched(library: &[u8], to: &str) -> Option<Vec<u8>> {
    let from = BUILT_IN.as_bytes();
    if to.len() > from.len() {
        return None;
    }
    let mut found = library.windows(from.len() + 1).enumerate().filter(|(_, w)| w[..from.len()] == *from && w[from.len()] == 0).map(|(at, _)| at);
    let at = found.next()?;
    if found.next().is_some() {
        return None;
    }
    let mut out = library.to_vec();
    out[at..at + from.len()].fill(0);
    out[at..at + to.len()].copy_from_slice(to.as_bytes());
    Some(out)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::sync::OnceLock;

    static NOTE: OnceLock<String> = OnceLock::new();

    /// Ours and nobody else's: anyone can make a folder in /tmp first, and what
    /// is in it decides which certificates are trusted.
    fn private_folder(uid: u32) -> Result<PathBuf, String> {
        let dir = folder(uid);
        let made = std::fs::DirBuilder::new().mode(0o700).create(&dir);
        if let Err(e) = made {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(format!("{}: {e}", dir.display()));
            }
        }
        let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(format!("{} is not private", dir.display()));
        }
        Ok(dir)
    }

    fn prepare(appdir: &Path, bundle: &Path, uid: u32) -> Result<PathBuf, String> {
        let dir = private_folder(uid)?;
        let list = dir.join("ca.crt");
        let library = std::fs::read(appdir.join("usr/lib/libgnutls.so.30")).map_err(|e| format!("bundled GnuTLS: {e}"))?;
        let copy = patched(&library, &list.to_string_lossy()).ok_or("the bundled GnuTLS has no trust path to replace")?;
        // Beside and renamed over: another start of Kryoto may be running on the last one.
        let id = std::process::id();
        let link = dir.join(format!("ca.crt.{id}"));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(bundle, &link).map_err(|e| e.to_string())?;
        std::fs::rename(&link, &list).map_err(|e| e.to_string())?;
        let target = dir.join("libgnutls.so.30");
        if std::fs::read(&target).ok().as_deref() != Some(&copy) {
            let fresh = dir.join(format!("libgnutls.so.30.{id}"));
            std::fs::write(&fresh, &copy).map_err(|e| e.to_string())?;
            std::fs::rename(&fresh, &target).map_err(|e| e.to_string())?;
        }
        Ok(dir)
    }

    pub fn apply() {
        let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
        let Some(appdir) = std::env::var_os("APPDIR").filter(|d| appimage && !d.is_empty()) else { return };
        let chosen = std::env::var_os("SSL_CERT_FILE").map(PathBuf::from);
        let Some(bundle) = bundle(|p| p.exists(), chosen) else { return };
        // SAFETY: geteuid only reads this process's id.
        let uid = unsafe { libc::geteuid() };
        match prepare(Path::new(&appdir), &bundle, uid) {
            Ok(dir) => {
                let paths = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
                let all = std::iter::once(dir.clone()).chain(std::env::split_paths(&paths));
                if let Ok(value) = std::env::join_paths(all) {
                    std::env::set_var("LD_LIBRARY_PATH", value);
                }
                let _ = NOTE.set(format!("HTTPS trusts {} through {}", bundle.display(), dir.display()));
            }
            Err(e) => {
                let _ = NOTE.set(format!("HTTPS cannot use {}: {e}", bundle.display()));
            }
        }
    }

    pub fn describe() -> Option<String> {
        NOTE.get().cloned()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Somebody else made the folder first: this process owns it, the id asked for does not.
        #[test]
        fn a_folder_someone_else_made_is_refused() {
            let other = 4_000_000_000;
            let dir = folder(other);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir(&dir).unwrap();
            let answer = private_folder(other);
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(answer.unwrap_err().contains("not private"));
        }

        #[test]
        fn a_folder_others_can_write_to_is_refused() {
            // SAFETY: geteuid only reads this process's id.
            let uid = unsafe { libc::geteuid() };
            let dir = private_folder(uid).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            let open = private_folder(uid);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(open.is_err());
            assert_eq!(private_folder(uid).unwrap(), dir);
        }
    }
}

/// Before anything starts a thread or a web view (Linux only).
pub fn apply() {
    #[cfg(target_os = "linux")]
    linux::apply();
}

/// For the log, once logging is up.
pub fn describe() -> Option<String> {
    #[cfg(target_os = "linux")]
    return linux::describe();
    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Vec<u8> {
        [b"\x7fELF...".as_slice(), BUILT_IN.as_bytes(), b"\0/etc/other\0"].concat()
    }

    #[test]
    fn the_path_is_replaced_in_place_and_the_rest_stays() {
        let to = "/tmp/kryoto-ca-1000/ca.crt";
        let before = library();
        let after = patched(&before, to).unwrap();
        assert_eq!(after.len(), before.len());
        let start = 7;
        assert_eq!(&after[start..start + to.len()], to.as_bytes());
        assert!(after[start + to.len()..start + BUILT_IN.len() + 1].iter().all(|b| *b == 0));
        assert!(after.ends_with(b"\0/etc/other\0"));
    }

    #[test]
    fn the_longest_user_id_still_fits() {
        let list = folder(u32::MAX).join("ca.crt");
        assert!(patched(&library(), list.to_str().unwrap()).is_some());
    }

    #[test]
    fn a_library_it_does_not_recognise_is_left_alone() {
        assert_eq!(patched(b"no path here", "/tmp/x"), None);
        let twice = [library(), library()].concat();
        assert_eq!(patched(&twice, "/tmp/x"), None);
        assert_eq!(patched(&library(), &"x".repeat(BUILT_IN.len() + 1)), None);
    }

    #[test]
    fn nothing_happens_where_the_built_in_path_exists() {
        assert_eq!(bundle(|_| true, None), None);
    }

    #[test]
    fn the_distros_list_is_found() {
        let fedora = |p: &Path| p == Path::new(BUNDLES[0]);
        assert_eq!(bundle(fedora, None).as_deref(), Some(Path::new(BUNDLES[0])));
        let suse = |p: &Path| p == Path::new("/etc/ssl/ca-bundle.pem");
        assert_eq!(bundle(suse, None).as_deref(), Some(Path::new("/etc/ssl/ca-bundle.pem")));
        assert_eq!(bundle(|_| false, None), None);
    }

    #[test]
    fn a_list_the_player_named_comes_first_when_it_is_there() {
        let mine = PathBuf::from("/opt/mine.pem");
        let some = |p: &Path| p == mine || p == Path::new(BUNDLES[0]);
        assert_eq!(bundle(some, Some(mine.clone())), Some(mine.clone()));
        let only_fedora = |p: &Path| p == Path::new(BUNDLES[0]);
        assert_eq!(bundle(only_fedora, Some(mine)).as_deref(), Some(Path::new(BUNDLES[0])));
    }

    #[test]
    fn only_our_folder_counts_as_ours() {
        assert!(is_ours(&folder(1000)));
        assert!(!is_ours(Path::new("/tmp/other")));
    }
}
