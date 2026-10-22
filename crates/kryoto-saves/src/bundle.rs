//! `.ksave`: one backup of a game's saves.
//!
//! A zip: every save file under `files/<Root>/<path>`, and `manifest.json`
//! saying what each one is (root, path, size, SHA-256), which rules found
//! them (so another PC knows where they go, and what to back up next time),
//! registry values, and where the backup was made. Paths in the zip are never
//! trusted on the way back: a file is written where its manifest entry says,
//! under a root this PC resolved, after [`crate::rules::path_problem`].

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::rules::{join, path_problem, Kind, Root, Rule, SaveRoots};
use crate::scan::{hex, Found};

pub const MANIFEST: &str = "manifest.json";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleFile {
    pub root: Root,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub mtime: u64,
    /// The rule ([`Rule::key`]) it belongs to.
    #[serde(default)]
    pub rule: String,
}

/// One registry value under `HKCU`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegValue {
    /// The key under HKCU, with `/` (`Software/Studio/Game`).
    pub path: String,
    pub name: String,
    /// The registry type (REG_SZ = 1, REG_BINARY = 3, REG_DWORD = 4...).
    pub kind: u32,
    /// The raw bytes, base64.
    pub data: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub v: u32,
    pub game_key: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub title: String,
    pub rules: Vec<Rule>,
    pub files: Vec<BundleFile>,
    #[serde(default)]
    pub registry: Vec<RegValue>,
    #[serde(default)]
    pub steam_id: Option<String>,
    #[serde(default)]
    pub machine: String,
    pub created_at: u64,
    #[serde(default)]
    pub parent_version: Option<String>,
    /// [`content_hash`] of what it holds.
    pub content_hash: String,
}

/// What a game's saves are, as one hash: every file (by where it is, its
/// size and its content) and every registry value. Equal hashes, equal saves.
pub fn content_hash(files: &[BundleFile], registry: &[RegValue]) -> String {
    let mut lines: Vec<String> = files.iter().map(|f| format!("f\t{}\t{}\t{}\t{}", f.root.name(), f.path.to_lowercase(), f.size, f.sha256)).collect();
    lines.extend(registry.iter().map(|r| format!("r\t{}\t{}\t{}\t{}", r.path.to_lowercase(), r.name.to_lowercase(), r.kind, r.data)));
    lines.sort();
    let mut h = Sha256::new();
    for l in lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// The backup file itself: its size and the hashes kryo.to checks it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub size: u64,
    /// MD5, base64 (the `Content-MD5` header and the storage ETag).
    pub md5: String,
    pub sha256: String,
}

fn entry_name(root: Root, path: &str) -> String {
    format!("files/{}/{}", root.name(), path)
}

/// Pack `files` (and the template's registry values) into `out`. Each file's
/// hash is taken from what was actually written, so a file the game changed a
/// moment ago is described as it is in the backup. A file that vanished is
/// left out. Returns the final manifest and the backup's own hashes.
pub fn write(template: &Manifest, files: &[Found], out: &Path) -> Result<(Manifest, Written), String> {
    write_with(template, files, out, &mut |_, _| {})
}

/// [`write`], saying how far it is: `progress(bytes packed, bytes in all)`.
pub fn write_with(template: &Manifest, files: &[Found], out: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<(Manifest, Written), String> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done = 0u64;
    progress(0, total);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("ksave.part");
    let file = std::fs::File::create(&tmp).map_err(|e| format!("Could not write the backup: {e}"))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let mut written = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    for f in files {
        if path_problem(&f.path).is_some() || f.root == Root::HKCU {
            continue;
        }
        let Ok(mut src) = std::fs::File::open(&f.abs) else { continue };
        zip.start_file(entry_name(f.root, &f.path), deflate).map_err(|e| e.to_string())?;
        let mut h = Sha256::new();
        let mut size = 0u64;
        loop {
            let n = src.read(&mut buf).map_err(|e| format!("Could not read {}: {e}", f.abs.display()))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            zip.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            size += n as u64;
            done += n as u64;
            progress(done.min(total), total);
        }
        written.push(BundleFile { root: f.root, path: f.path.clone(), size, sha256: hex(&h.finalize()), mtime: f.mtime, rule: f.rule.clone() });
    }
    let mut manifest = template.clone();
    manifest.v = VERSION;
    manifest.content_hash = content_hash(&written, &manifest.registry);
    manifest.files = written;
    zip.start_file(MANIFEST, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated))
        .map_err(|e| e.to_string())?;
    zip.write_all(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut inner = zip.finish().map_err(|e| e.to_string())?;
    inner.flush().map_err(|e| e.to_string())?;
    drop(inner);
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    Ok((manifest, digest(out)?))
}

/// Size, MD5 and SHA-256 of a file.
pub fn digest(path: &Path) -> Result<Written, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let (mut md5, mut sha) = (Md5::new(), Sha256::new());
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha.update(&buf[..n]);
        size += n as u64;
    }
    Ok(Written { size, md5: base64::engine::general_purpose::STANDARD.encode(md5.finalize()), sha256: hex(&sha.finalize()) })
}

fn open(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("Could not open the backup: {e}"))?;
    zip::ZipArchive::new(f).map_err(|_| "That backup is damaged.".to_string())
}

/// The manifest of a backup, without unpacking it.
pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let mut zip = open(path)?;
    let mut entry = zip.by_name(MANIFEST).map_err(|_| "That backup has no manifest.".to_string())?;
    if entry.size() > 32 << 20 {
        return Err("That backup's manifest is too big.".into());
    }
    let mut text = Vec::new();
    entry.read_to_end(&mut text).map_err(|e| e.to_string())?;
    let m: Manifest = serde_json::from_slice(&text).map_err(|_| "That backup's manifest could not be read.".to_string())?;
    if m.v > VERSION {
        return Err("That backup was made by a newer Kryoto Desktop. Update to restore it.".into());
    }
    Ok(m)
}

/// Check a whole backup: its manifest reads, every file in it is there with
/// the size and SHA-256 it claims, and no path could leave its folder.
pub fn verify(path: &Path) -> Result<Manifest, String> {
    let m = read_manifest(path)?;
    let mut zip = open(path)?;
    let mut buf = vec![0u8; 1 << 16];
    for f in &m.files {
        if path_problem(&f.path).is_some() || f.root == Root::HKCU {
            return Err("That backup names a file outside its game's folders.".into());
        }
        let mut entry = zip.by_name(&entry_name(f.root, &f.path)).map_err(|_| format!("{} is missing from the backup.", f.path))?;
        let mut h = Sha256::new();
        let mut size = 0u64;
        loop {
            let n = entry.read(&mut buf).map_err(|_| "That backup is damaged.".to_string())?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > f.size {
                return Err(format!("{} is not what the backup says.", f.path));
            }
            h.update(&buf[..n]);
        }
        if size != f.size || hex(&h.finalize()) != f.sha256 {
            return Err(format!("{} is not what the backup says.", f.path));
        }
    }
    for r in &m.registry {
        if !r.path.to_ascii_lowercase().starts_with("software/") || r.path.split('/').any(|s| s.is_empty() || s == "..") {
            return Err("That backup names a registry key outside HKCU\\Software.".into());
        }
    }
    Ok(m)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Restored {
    pub written: usize,
    pub removed: usize,
    pub registry: usize,
}

/// Put a verified backup's files back on this PC. Each file is written beside
/// its place and then swapped in, so a crash leaves the old file or the new
/// one. Inside each save folder the backup had files for, files the backup
/// does not have are removed, so the folder is what it was when backed up;
/// the caller keeps a local backup of what was there first.
///
/// `local` is what is on this PC now (from [`crate::scan::collect`] with the
/// same rules), for those removals.
pub fn restore(path: &Path, m: &Manifest, roots: &SaveRoots, local: &[Found]) -> Result<Restored, String> {
    let mut zip = open(path)?;
    let mut out = Restored::default();
    let mut kept: HashSet<(Root, String)> = HashSet::new();
    for f in &m.files {
        if path_problem(&f.path).is_some() || f.root == Root::HKCU || crate::scan::blocked(&f.path) {
            continue;
        }
        let Some(base) = roots.get(f.root) else { continue };
        let target = join(base, &f.path);
        kept.insert((f.root, f.path.to_lowercase()));
        let mut entry = zip.by_name(&entry_name(f.root, &f.path)).map_err(|_| format!("{} is missing from the backup.", f.path))?;
        let dir = target.parent().ok_or("A save has nowhere to go.")?.to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
        let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let staged = dir.join(format!(".{name}.kryoto-restore"));
        {
            let mut w = std::fs::File::create(&staged).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
            std::io::copy(&mut entry, &mut w).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
            w.flush().map_err(|e| e.to_string())?;
            if f.mtime > 0 {
                let _ = w.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(f.mtime));
            }
            let _ = w.sync_all();
        }
        if let Err(e) = std::fs::rename(&staged, &target) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("Could not put {} back: {e}", target.display()));
        }
        out.written += 1;
    }
    // Mirror the folders the backup has files for.
    let backed_rules: HashSet<&str> = m.files.iter().map(|f| f.rule.as_str()).filter(|r| !r.is_empty()).collect();
    for l in local {
        if backed_rules.contains(l.rule.as_str()) && !kept.contains(&(l.root, l.path.to_lowercase())) && std::fs::remove_file(&l.abs).is_ok() {
            out.removed += 1;
        }
    }
    #[cfg(windows)]
    {
        out.registry = crate::registry::write(&m.registry)?;
    }
    Ok(out)
}

/// The rules a backup was made with, for files only (registry rules are
/// carried by their values).
pub fn file_rules(m: &Manifest) -> Vec<Rule> {
    m.rules.iter().filter(|r| r.kind == Kind::Files).cloned().collect()
}

/// Every save file in the backup, by where it goes on this PC.
pub fn targets(m: &Manifest, roots: &SaveRoots) -> HashMap<PathBuf, BundleFile> {
    m.files.iter().filter_map(|f| Some((join(roots.get(f.root)?, &f.path), f.clone()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Source;
    use crate::scan::{collect, MAX_FILES};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryo-saves-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn template(rules: Vec<Rule>) -> Manifest {
        Manifest { game_key: "steam:620".into(), rules, created_at: 1, machine: "PC".into(), ..Default::default() }
    }

    #[test]
    fn round_trip_restores_files_and_mirrors_the_folder() {
        let d = tmp("round");
        let docs = d.join("docs");
        std::fs::create_dir_all(docs.join("Game/Saves/sub")).unwrap();
        std::fs::write(docs.join("Game/Saves/slot1.sav"), b"first").unwrap();
        std::fs::write(docs.join("Game/Saves/sub/meta.bin"), vec![7u8; 100_000]).unwrap();
        let roots = SaveRoots { documents: Some(docs.clone()), ..Default::default() };
        let rules = vec![Rule::files(Root::Documents, "Game/Saves", Source::Known)];
        let found = collect(&rules, &roots, MAX_FILES);
        let out = d.join("b.ksave");
        let (m, w) = write(&template(rules.clone()), &found.files, &out).unwrap();
        assert_eq!(m.files.len(), 2);
        assert_eq!(w, digest(&out).unwrap());
        assert_eq!(verify(&out).unwrap(), m);
        // Play on: one save changes, a new one appears.
        std::fs::write(docs.join("Game/Saves/slot1.sav"), b"changed later").unwrap();
        std::fs::write(docs.join("Game/Saves/slot2.sav"), b"new").unwrap();
        let local = collect(&rules, &roots, MAX_FILES);
        assert_ne!(content_hash(&local.files.iter().map(|f| BundleFile { root: f.root, path: f.path.clone(), size: f.size, sha256: crate::scan::sha256_file(&f.abs).unwrap(), mtime: f.mtime, rule: f.rule.clone() }).collect::<Vec<_>>(), &[]), m.content_hash);
        let r = restore(&out, &m, &roots, &local.files).unwrap();
        assert_eq!((r.written, r.removed), (2, 1));
        assert_eq!(std::fs::read(docs.join("Game/Saves/slot1.sav")).unwrap(), b"first");
        assert!(!docs.join("Game/Saves/slot2.sav").exists());
        assert_eq!(std::fs::read(docs.join("Game/Saves/sub/meta.bin")).unwrap().len(), 100_000);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tampered_backup_is_refused() {
        let d = tmp("tamper");
        std::fs::create_dir_all(d.join("docs/G")).unwrap();
        std::fs::write(d.join("docs/G/s.sav"), b"good").unwrap();
        let roots = SaveRoots { documents: Some(d.join("docs")), ..Default::default() };
        let rules = vec![Rule::files(Root::Documents, "G", Source::Known)];
        let found = collect(&rules, &roots, MAX_FILES);
        let out = d.join("b.ksave");
        let (mut m, _) = write(&template(rules), &found.files, &out).unwrap();
        // A manifest that lies about a file, written into a copy.
        m.files[0].sha256 = "0".repeat(64);
        let bad = d.join("bad.ksave");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("files/Documents/G/s.sav", o).unwrap();
            z.write_all(b"good").unwrap();
            z.start_file(MANIFEST, o).unwrap();
            z.write_all(&serde_json::to_vec(&m).unwrap()).unwrap();
            z.finish().unwrap();
        }
        assert!(verify(&bad).is_err());
        // And one that tries to climb out of its folder.
        m.files[0].sha256 = crate::scan::sha256_file(&d.join("docs/G/s.sav")).unwrap();
        m.files[0].path = "../../escape.txt".into();
        let evil = d.join("evil.ksave");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("files/Documents/../../escape.txt", o).unwrap();
            z.write_all(b"good").unwrap();
            z.start_file(MANIFEST, o).unwrap();
            z.write_all(&serde_json::to_vec(&m).unwrap()).unwrap();
            z.finish().unwrap();
        }
        assert!(verify(&evil).is_err());
        assert!(restore(&evil, &m, &roots, &[]).unwrap().written == 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_content_hash_ignores_order_and_case() {
        let a = BundleFile { root: Root::Documents, path: "G/A.sav".into(), size: 1, sha256: "aa".into(), mtime: 1, rule: String::new() };
        let b = BundleFile { root: Root::AppData, path: "G/b.sav".into(), size: 2, sha256: "bb".into(), mtime: 2, rule: String::new() };
        let mut a2 = a.clone();
        a2.path = "g/a.sav".into();
        a2.mtime = 99;
        assert_eq!(content_hash(&[a.clone(), b.clone()], &[]), content_hash(&[b.clone(), a2], &[]));
        let mut changed = a.clone();
        changed.sha256 = "ab".into();
        assert_ne!(content_hash(&[a, b.clone()], &[]), content_hash(&[changed, b], &[]));
    }
}
