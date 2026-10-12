//! Player pictures live in app data, separate from the disposable network cache.
use std::io::{Read, Write};
use std::path::Path;

pub(crate) const MAX_BYTES: usize = 25 * 1024 * 1024;
pub(crate) const PREFIX: &str = "kryoto-art://local/";

pub(crate) fn managed_key(url: &str) -> Option<&str> {
    let key = url.strip_prefix(PREFIX)?;
    (key.len() == 64 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then_some(key)
}

pub(crate) fn mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => "image/avif",
        _ if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub(crate) fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a picture file.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Choose a picture smaller than 25 MB.".into());
    }
    if matches!(mime(&bytes), "application/octet-stream" | "image/svg+xml") {
        return Err("Choose a PNG, JPEG, WebP, GIF or AVIF picture.".into());
    }
    Ok(bytes)
}

pub(crate) fn store(dir: &Path, bytes: &[u8], key: &str) -> Result<String, String> {
    let uri = format!("{PREFIX}{key}");
    if managed_key(&uri).is_none() { return Err("Invalid artwork key.".into()); }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let dest = dir.join(key);
    if !dest.is_file() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp = dir.join(format!("{key}.{}.{nonce}.part", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
            file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temp, &dest).or_else(|e| if dest.is_file() { Ok(()) } else { Err(e) }).map_err(|e| e.to_string())
        })();
        let _ = std::fs::remove_file(&temp);
        result?;
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_urls_cannot_escape_the_picture_folder() {
        assert!(managed_key(&format!("{PREFIX}{}", "a".repeat(64))).is_some());
        for suffix in ["../library.json", "%2e%2e%2fsecret", "aa?file=secret", "/etc/passwd", "", "AB"] {
            assert!(managed_key(&format!("{PREFIX}{suffix}")).is_none());
        }
        assert!(managed_key(&format!("kryoto-art://other/{}", "a".repeat(64))).is_none());
    }

    #[test]
    fn imported_picture_survives_source_deletion_and_reuses_its_copy() {
        let root = std::env::temp_dir().join(format!("kryoto-art-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("original.png");
        let bytes = b"\x89PNG\r\n\x1a\nexample";
        std::fs::write(&source, bytes).unwrap();
        let owned = root.join("app-data");
        let key = "a".repeat(64);
        let uri = store(&owned, &read_image(&source).unwrap(), &key).unwrap();
        std::fs::remove_file(&source).unwrap();
        assert_eq!(read_image(&owned.join(managed_key(&uri).unwrap())).unwrap(), bytes);
        assert_eq!(store(&owned, bytes, &key).unwrap(), uri);
        assert_eq!(std::fs::read_dir(&owned).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn arbitrary_files_and_svg_are_not_imported() {
        let path = std::env::temp_dir().join(format!("kryoto-art-invalid-{}", std::process::id()));
        for bytes in [b"MZ executable".as_slice(), b"<svg onload='alert(1)'/>", b""] {
            std::fs::write(&path, bytes).unwrap();
            assert!(read_image(&path).is_err());
        }
        std::fs::File::create(&path).unwrap().set_len(MAX_BYTES as u64 + 1).unwrap();
        assert!(read_image(&path).unwrap_err().contains("25 MB"));
        std::fs::remove_file(path).unwrap();
    }
}
