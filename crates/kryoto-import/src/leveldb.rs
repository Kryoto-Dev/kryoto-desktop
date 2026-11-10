//! A read-only LevelDB reader: the store Hydra keeps its library in
//! (`classic-level`, which is Google's LevelDB).
//!
//! It reads the files and nothing else. A real LevelDB open takes the `LOCK`
//! file and may write a new log and manifest, which would change another
//! launcher's folder; this never opens a file for writing. What it reads is
//! the live set the manifest names (`CURRENT` -> `MANIFEST-*`): the sorted
//! tables (`.ldb`, `.sst`) and the write-ahead logs not yet folded into them.
//! For each key the newest entry wins, and a deletion hides the key.

use crate::snappy::{self, varint};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// No database in that folder.
    Missing,
    /// Another program has its files open (on Windows, the launcher itself
    /// while it runs).
    Locked,
    /// A file could not be read for another reason.
    Io(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Missing => f.write_str("No library database there."),
            ReadError::Locked => f.write_str("The database is in use by another program."),
            ReadError::Io(e) => f.write_str(e),
        }
    }
}

/// Every live key and its value.
pub fn read_all(dir: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, ReadError> {
    if !dir.join("CURRENT").is_file() && !has_files(dir) {
        return Err(ReadError::Missing);
    }
    let files = live_files(dir)?;
    // key -> (sequence, value; None for a deletion)
    let mut newest: BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)> = BTreeMap::new();
    let mut keep = |key: Vec<u8>, seq: u64, value: Option<Vec<u8>>| match newest.get(&key) {
        Some((s, _)) if *s >= seq => {}
        _ => {
            newest.insert(key, (seq, value));
        }
    };
    // A file the manifest names but that is gone was compacted away while we
    // looked: what replaced it is newer and read too.
    let read_live = |path: &Path| match read(path) {
        Err(ReadError::Missing) => Ok(Vec::new()),
        other => other,
    };
    for table in &files.tables {
        let bytes = read_live(table)?;
        // A damaged table is skipped rather than failing the whole read: the
        // rest of the library is still worth importing.
        let _ = read_table(&bytes, &mut keep);
    }
    for log in &files.logs {
        let bytes = read_live(log)?;
        for record in log_records(&bytes) {
            read_batch(&record, &mut keep);
        }
    }
    Ok(newest.into_iter().filter_map(|(k, (_, v))| Some((k, v?))).collect())
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|r| {
        r.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.ends_with(".ldb") || n.ends_with(".sst") || n.ends_with(".log") && n != "log"
        })
    })
}

fn read(path: &Path) -> Result<Vec<u8>, ReadError> {
    std::fs::read(path).map_err(|e| match e.raw_os_error() {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        Some(32) | Some(33) if cfg!(windows) => ReadError::Locked,
        _ if e.kind() == std::io::ErrorKind::NotFound => ReadError::Missing,
        _ => ReadError::Io(format!("{}: {e}", path.display())),
    })
}

struct Files {
    tables: Vec<PathBuf>,
    logs: Vec<PathBuf>,
}

/// The tables and logs the current manifest says are live; every table and
/// log in the folder when there is no readable manifest.
fn live_files(dir: &Path) -> Result<Files, ReadError> {
    let numbered = |ext: &[&str]| -> Vec<(u64, PathBuf)> {
        let mut out: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
            .map(|r| {
                r.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let (stem, x) = name.rsplit_once('.')?;
                        (ext.contains(&x.to_ascii_lowercase().as_str())).then_some(())?;
                        Some((stem.parse::<u64>().ok()?, e.path()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    };
    let tables = numbered(&["ldb", "sst"]);
    let logs = numbered(&["log"]);
    let manifest = std::fs::read_to_string(dir.join("CURRENT"))
        .ok()
        .map(|c| dir.join(c.trim()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("MANIFEST-")));
    let Some(manifest) = manifest else {
        return Ok(Files { tables: tables.into_iter().map(|t| t.1).collect(), logs: logs.into_iter().map(|l| l.1).collect() });
    };
    let edits = read(&manifest)?;
    let mut live: HashSet<u64> = HashSet::new();
    let mut log_number = 0u64;
    let mut prev_log = 0u64;
    for record in log_records(&edits) {
        let Some(edit) = version_edit(&record) else { continue };
        for n in edit.added {
            live.insert(n);
        }
        for n in edit.deleted {
            live.remove(&n);
        }
        log_number = edit.log_number.unwrap_or(log_number);
        prev_log = edit.prev_log.unwrap_or(prev_log);
    }
    Ok(Files {
        tables: tables.into_iter().filter(|(n, _)| live.contains(n)).map(|t| t.1).collect(),
        logs: logs.into_iter().filter(|(n, _)| *n >= log_number || (*n == prev_log && prev_log != 0)).map(|l| l.1).collect(),
    })
}

#[derive(Default)]
struct VersionEdit {
    added: Vec<u64>,
    deleted: Vec<u64>,
    log_number: Option<u64>,
    prev_log: Option<u64>,
}

fn version_edit(buf: &[u8]) -> Option<VersionEdit> {
    let mut edit = VersionEdit::default();
    let mut pos = 0;
    let slice = |pos: usize| -> Option<usize> {
        let (len, p) = varint(buf, pos)?;
        let end = p.checked_add(len as usize)?;
        (end <= buf.len()).then_some(end)
    };
    while pos < buf.len() {
        let (tag, p) = varint(buf, pos)?;
        pos = p;
        match tag {
            1 => pos = slice(pos)?, // comparator name
            2 => {
                let (v, p) = varint(buf, pos)?;
                edit.log_number = Some(v);
                pos = p;
            }
            3 | 4 => pos = varint(buf, pos)?.1, // next file number, last sequence
            5 => {
                pos = varint(buf, pos)?.1; // level
                pos = slice(pos)?; // compaction pointer key
            }
            6 => {
                pos = varint(buf, pos)?.1;
                let (n, p) = varint(buf, pos)?;
                edit.deleted.push(n);
                pos = p;
            }
            7 => {
                pos = varint(buf, pos)?.1;
                let (n, p) = varint(buf, pos)?;
                edit.added.push(n);
                pos = varint(buf, p)?.1; // file size
                pos = slice(pos)?; // smallest
                pos = slice(pos)?; // largest
            }
            9 => {
                let (v, p) = varint(buf, pos)?;
                edit.prev_log = Some(v);
                pos = p;
            }
            _ => return None,
        }
    }
    Some(edit)
}

/// The records of a LevelDB log (the write-ahead log and the manifest share
/// one format): 32 KiB blocks of chunks, each a FULL record or the FIRST,
/// MIDDLE and LAST pieces of one. A torn tail (a write the program never
/// finished) is dropped, as LevelDB itself does.
pub(crate) fn log_records(bytes: &[u8]) -> Vec<Vec<u8>> {
    const BLOCK: usize = 32 * 1024;
    let mut out = Vec::new();
    let mut partial: Option<Vec<u8>> = None;
    let mut block_start = 0;
    while block_start < bytes.len() {
        let block = &bytes[block_start..(block_start + BLOCK).min(bytes.len())];
        let mut pos = 0;
        while pos + 7 <= block.len() {
            let len = u16::from_le_bytes([block[pos + 4], block[pos + 5]]) as usize;
            let kind = block[pos + 6];
            if kind == 0 && len == 0 {
                break; // zero padding: the rest of this block is empty
            }
            let Some(data) = block.get(pos + 7..pos + 7 + len) else { break };
            pos += 7 + len;
            match kind {
                1 => {
                    partial = None;
                    out.push(data.to_vec());
                }
                2 => partial = Some(data.to_vec()),
                3 => {
                    if let Some(p) = partial.as_mut() {
                        p.extend_from_slice(data);
                    }
                }
                4 => {
                    if let Some(mut p) = partial.take() {
                        p.extend_from_slice(data);
                        out.push(p);
                    }
                }
                _ => partial = None,
            }
        }
        block_start += BLOCK;
    }
    out
}

/// One write batch from the log: a starting sequence, then puts and deletes.
fn read_batch(rec: &[u8], keep: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) {
    if rec.len() < 12 {
        return;
    }
    let seq = u64::from_le_bytes(rec[0..8].try_into().unwrap_or_default());
    let count = u32::from_le_bytes(rec[8..12].try_into().unwrap_or_default()) as u64;
    let mut pos = 12;
    let take = |pos: usize| -> Option<(&[u8], usize)> {
        let (len, p) = varint(rec, pos)?;
        let end = p.checked_add(len as usize)?;
        Some((rec.get(p..end)?, end))
    };
    for i in 0..count {
        let Some(&tag) = rec.get(pos) else { return };
        pos += 1;
        let Some((key, p)) = take(pos) else { return };
        pos = p;
        if tag == 1 {
            let Some((value, p)) = take(pos) else { return };
            pos = p;
            keep(key.to_vec(), seq + i, Some(value.to_vec()));
        } else if tag == 0 {
            keep(key.to_vec(), seq + i, None);
        } else {
            return;
        }
    }
}

/// A sorted table: its index block points at each data block, whose keys are
/// LevelDB's internal keys (the key, then 8 bytes of sequence and kind).
fn read_table(bytes: &[u8], keep: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) -> Option<()> {
    const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
    if bytes.len() < 48 {
        return None;
    }
    let footer = &bytes[bytes.len() - 48..];
    if u64::from_le_bytes(footer[40..48].try_into().ok()?) != MAGIC {
        return None;
    }
    let (_, p) = handle(footer, 0)?; // metaindex
    let (index, _) = handle(footer, p)?;
    let index = block(bytes, index)?;
    for (_, value) in entries(&index)? {
        let (data, _) = handle(&value, 0)?;
        let Some(data) = block(bytes, data) else { continue };
        for (ikey, value) in entries(&data).unwrap_or_default() {
            if ikey.len() < 8 {
                continue;
            }
            let (user, trailer) = ikey.split_at(ikey.len() - 8);
            let trailer = u64::from_le_bytes(trailer.try_into().ok()?);
            let (seq, kind) = (trailer >> 8, trailer & 0xff);
            keep(user.to_vec(), seq, (kind == 1).then_some(value));
        }
    }
    Some(())
}

fn handle(buf: &[u8], pos: usize) -> Option<((usize, usize), usize)> {
    let (offset, p) = varint(buf, pos)?;
    let (size, p) = varint(buf, p)?;
    Some(((offset as usize, size as usize), p))
}

/// A block's bytes, unpacked: after its contents come one byte of compression
/// (0 none, 1 Snappy) and four of checksum.
fn block(bytes: &[u8], (offset, size): (usize, usize)) -> Option<Vec<u8>> {
    let end = offset.checked_add(size)?;
    let data = bytes.get(offset..end)?;
    match *bytes.get(end)? {
        0 => Some(data.to_vec()),
        1 => snappy::decompress(data),
        _ => None,
    }
}

/// A block's key/value pairs. Keys share a prefix with the one before them;
/// the block ends in its restart points, which a full scan does not need.
fn entries(block: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    if block.len() < 4 {
        return None;
    }
    let restarts = u32::from_le_bytes(block[block.len() - 4..].try_into().ok()?) as usize;
    let end = block.len().checked_sub(4 + restarts.checked_mul(4)?)?;
    let mut out = Vec::new();
    let mut key: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos < end {
        let (shared, p) = varint(block, pos)?;
        let (unshared, p) = varint(block, p)?;
        let (vlen, p) = varint(block, p)?;
        let (shared, unshared, vlen) = (shared as usize, unshared as usize, vlen as usize);
        if shared > key.len() {
            return None;
        }
        key.truncate(shared);
        // Checked: a corrupt block can carry lengths that wrap past the end.
        let key_end = p.checked_add(unshared)?;
        let value_end = key_end.checked_add(vlen)?;
        key.extend_from_slice(block.get(p..key_end)?);
        let v = block.get(key_end..value_end)?;
        out.push((key.clone(), v.to_vec()));
        pos = value_end;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 0];
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.push(kind);
        out.extend_from_slice(data);
        out
    }

    fn batch(seq: u64, ops: &[(u8, &[u8], &[u8])]) -> Vec<u8> {
        let mut out = seq.to_le_bytes().to_vec();
        out.extend_from_slice(&(ops.len() as u32).to_le_bytes());
        for (tag, k, v) in ops {
            out.push(*tag);
            out.push(k.len() as u8);
            out.extend_from_slice(k);
            if *tag == 1 {
                out.push(v.len() as u8);
                out.extend_from_slice(v);
            }
        }
        out
    }

    #[test]
    fn a_log_keeps_the_newest_write_and_honours_deletes() {
        let mut log = chunk(1, &batch(1, &[(1, b"a", b"1"), (1, b"b", b"2")]));
        // A record split across FIRST/LAST.
        let rec = batch(3, &[(1, b"a", b"3"), (0, b"b", b"")]);
        log.extend(chunk(2, &rec[..5]));
        log.extend(chunk(4, &rec[5..]));
        // A torn write at the end: a FIRST with no LAST.
        log.extend(chunk(2, &batch(9, &[(1, b"c", b"x")])[..4]));
        let dir = std::env::temp_dir().join(format!("kryoto-import-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("000003.log"), &log).unwrap();
        let all = read_all(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(all.get(b"a".as_slice()).map(Vec::as_slice), Some(b"3".as_slice()));
        assert!(!all.contains_key(b"b".as_slice()), "deleted");
        assert!(!all.contains_key(b"c".as_slice()), "torn tail is dropped");
    }

    #[test]
    fn an_empty_folder_is_missing() {
        let dir = std::env::temp_dir().join(format!("kryoto-import-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_all(&dir), Err(ReadError::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
