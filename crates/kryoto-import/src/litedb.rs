//! LiteDB 4 data files (file format 7), read-only, from bytes already in
//! memory: nothing is opened for writing, no journal is replayed, no lock
//! is taken. Playnite keeps its library in these (`library/games.db`), on
//! purpose still on LiteDB 4 (`source/Playnite/Database/Collections/ItemCollection.cs`).
//!
//! The layout follows the reader LiteDB 5 ships for upgrading version 4
//! files, which Playnite also carries to rescue damaged libraries
//! (`LiteDBFileReaderV7.cs`): 4096-byte pages, each with a 25-byte header;
//! the header page names the collections; a collection page points at its
//! `_id` index; index nodes point at data blocks; a document too big for one
//! page continues in a chain of extend pages. Documents are BSON (`bson.rs`).
//!
//! Every offset is checked and every page is visited once, so a damaged file
//! gives fewer documents or an error, never a hang or a panic.

use crate::bson::{self, Bson};
use std::collections::{BTreeSet, HashSet};

pub const PAGE: usize = 4096;
const HEADER: usize = 25;
const INFO: &[u8] = b"** This is a LiteDB file **";
const NONE: u32 = u32::MAX;

const T_HEADER: u8 = 1;
const T_COLLECTION: u8 = 2;
const T_INDEX: u8 = 3;
const T_DATA: u8 = 4;
const T_EXTEND: u8 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum LiteError {
    /// Not a LiteDB 4 file (a LiteDB 5 one, or something else).
    NotLiteDb,
    /// Protected with a password.
    Encrypted,
    Damaged(&'static str),
}

struct Page<'a> {
    kind: u8,
    next: u32,
    items: u16,
    body: Reader<'a>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    /// LiteDB 4's string: an Int32 length, then UTF-8.
    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(i32::from_le_bytes(self.take(4)?.try_into().ok()?)).ok()?;
        Some(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
    fn address(&mut self) -> Option<(u32, u16)> {
        Some((self.u32()?, self.u16()?))
    }
}

/// A LiteDB 4 file, held in memory.
pub struct File<'a> {
    bytes: &'a [u8],
    collections: Vec<(String, u32)>,
}

impl<'a> File<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<File<'a>, LiteError> {
        let head = page(bytes, 0).ok_or(LiteError::NotLiteDb)?;
        if head.kind != T_HEADER {
            return Err(LiteError::NotLiteDb);
        }
        let mut r = head.body;
        if r.take(INFO.len()) != Some(INFO) || r.u8() != Some(7) {
            return Err(LiteError::NotLiteDb);
        }
        let bad = LiteError::Damaged("header");
        // ChangeID, FreeEmptyPageID, LastPageID, UserVersion, Password.
        r.skip(2 + 4 + 4 + 2 + 20).ok_or(bad.clone())?;
        let salt = r.take(16).ok_or(bad.clone())?;
        if salt.iter().any(|&b| b != 0) {
            return Err(LiteError::Encrypted);
        }
        let count = r.u8().ok_or(bad.clone())?;
        let mut collections = Vec::new();
        for _ in 0..count {
            let name = r.string().ok_or(bad.clone())?;
            let at = r.u32().ok_or(bad.clone())?;
            collections.push((name, at));
        }
        Ok(File { bytes, collections })
    }

    pub fn collections(&self) -> Vec<&str> {
        self.collections.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Every document of a collection (named without regard to case), in
    /// `_id` order. A document that cannot be read is left out.
    pub fn documents(&self, collection: &str) -> Result<Vec<Bson>, LiteError> {
        let Some(&(_, at)) = self.collections.iter().find(|(n, _)| n.eq_ignore_ascii_case(collection)) else {
            return Ok(Vec::new());
        };
        let col = page(self.bytes, at).filter(|p| p.kind == T_COLLECTION).ok_or(LiteError::Damaged("collection page"))?;
        let head = id_index_head(col).ok_or(LiteError::Damaged("collection page"))?;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (page_id, index) in self.data_blocks(head) {
            if !seen.insert((page_id, index)) {
                continue;
            }
            if let Some(doc) = self.block(page_id, index).and_then(|d| bson::parse(&d)) {
                out.push(doc);
            }
        }
        Ok(out)
    }

    /// Where every node of the index starting on `head` keeps its document,
    /// in page order. The index's pages are linked through the nodes' first
    /// level; each is visited once.
    fn data_blocks(&self, head: u32) -> Vec<(u32, u16)> {
        let mut todo = BTreeSet::from([head]);
        let mut visited = HashSet::new();
        let mut blocks = Vec::new();
        while let Some(id) = todo.pop_first() {
            if !visited.insert(id) {
                continue;
            }
            let Some(p) = page(self.bytes, id).filter(|p| p.kind == T_INDEX) else { continue };
            let mut r = p.body;
            for _ in 0..p.items {
                let Some(node) = index_node(&mut r) else { break };
                if node.data.0 != NONE {
                    blocks.push(node.data);
                }
                for link in [node.prev, node.next] {
                    if link != NONE && !visited.contains(&link) {
                        todo.insert(link);
                    }
                }
            }
        }
        blocks.sort();
        blocks
    }

    /// A document's bytes: in its data block, or along its extend pages.
    fn block(&self, page_id: u32, index: u16) -> Option<Vec<u8>> {
        let p = page(self.bytes, page_id).filter(|p| p.kind == T_DATA)?;
        let mut r = p.body;
        for _ in 0..p.items {
            let at = r.u16()?;
            let extend = r.u32()?;
            let len = r.u16()? as usize;
            let data = r.take(len)?;
            if at == index {
                return if extend == NONE { Some(data.to_vec()) } else { self.extended(extend) };
            }
        }
        None
    }

    fn extended(&self, mut id: u32) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        while id != NONE {
            if !seen.insert(id) {
                return None;
            }
            let p = page(self.bytes, id).filter(|p| p.kind == T_EXTEND)?;
            let mut r = p.body;
            out.extend_from_slice(r.take(p.items as usize)?);
            id = p.next;
        }
        Some(out)
    }
}

fn page(bytes: &[u8], id: u32) -> Option<Page<'_>> {
    let start = (id as usize).checked_mul(PAGE)?;
    let b = bytes.get(start..start.checked_add(PAGE)?)?;
    let mut r = Reader { b, at: 0 };
    let _id = r.u32()?;
    let kind = r.u8()?;
    let _prev = r.u32()?;
    let next = r.u32()?;
    let items = r.u16()?;
    // Free bytes and 8 reserved.
    r.skip(2 + 8)?;
    debug_assert_eq!(r.at, HEADER);
    Some(Page { kind, next, items, body: r })
}

/// The first page of the collection's first index, `_id`.
fn id_index_head(col: Page) -> Option<u32> {
    let mut r = col.body;
    let _name = r.string()?;
    // Document count (8) and the free data page (4).
    r.skip(12)?;
    let _field = r.string()?;
    let _unique = r.u8()?;
    let (head, _) = r.address()?;
    (head != NONE).then_some(head)
}

struct Node {
    data: (u32, u16),
    prev: u32,
    next: u32,
}

fn index_node(r: &mut Reader) -> Option<Node> {
    let _index = r.u16()?;
    let levels = r.u8()? as usize;
    if levels == 0 {
        return None;
    }
    // Slot, PrevNode, NextNode.
    r.skip(1 + 6 + 6)?;
    let key_len = r.u16()? as usize;
    // The key's type and value.
    r.skip(1 + key_len)?;
    let data = r.address()?;
    let (prev, _) = r.address()?;
    let (next, _) = r.address()?;
    r.skip((levels - 1) * 12)?;
    Some(Node { data, prev, next })
}

/// Writing LiteDB 4 files, for tests and `examples/`: the layout the reader
/// expects, one collection, documents spread over data and extend pages and
/// index nodes over several pages, so every path of the reader runs.
#[doc(hidden)]
pub mod testfile {
    use super::*;

    fn header(id: u32, kind: u8, next: u32, items: u16) -> Vec<u8> {
        let mut p = Vec::with_capacity(PAGE);
        p.extend(id.to_le_bytes());
        p.push(kind);
        p.extend(NONE.to_le_bytes());
        p.extend(next.to_le_bytes());
        p.extend(items.to_le_bytes());
        p.extend([0u8; 10]);
        p
    }

    fn string(p: &mut Vec<u8>, s: &str) {
        p.extend((s.len() as i32).to_le_bytes());
        p.extend(s.as_bytes());
    }

    fn finish(mut p: Vec<u8>) -> Vec<u8> {
        assert!(p.len() <= PAGE, "page overflow");
        p.resize(PAGE, 0);
        p
    }

    /// A file with `collection` holding `docs` (BSON bytes). Documents larger
    /// than `inline` bytes go to extend pages; `per_index` nodes per index page.
    pub fn build(collection: &str, docs: &[Vec<u8>], inline: usize, per_index: usize) -> Vec<u8> {
        let mut pages: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
        // Data: one block per data page keeps the arithmetic plain.
        let mut addresses = Vec::new();
        for doc in docs {
            let data_id = pages.len() as u32;
            pages.push(Vec::new());
            let (extend, inline_bytes): (u32, &[u8]) = if doc.len() > inline {
                let first = pages.len() as u32;
                let chunks: Vec<&[u8]> = doc.chunks(PAGE - HEADER).collect();
                for (i, chunk) in chunks.iter().enumerate() {
                    let id = pages.len() as u32;
                    let next = if i + 1 == chunks.len() { NONE } else { id + 1 };
                    let mut p = header(id, T_EXTEND, next, chunk.len() as u16);
                    p.extend(*chunk);
                    pages.push(finish(p));
                }
                (first, &[])
            } else {
                (NONE, doc)
            };
            let mut p = header(data_id, T_DATA, NONE, 1);
            p.extend(3u16.to_le_bytes());
            p.extend(extend.to_le_bytes());
            p.extend((inline_bytes.len() as u16).to_le_bytes());
            p.extend(inline_bytes);
            pages[data_id as usize] = finish(p);
            addresses.push((data_id, 3u16));
        }
        // The _id index: a head and tail with no document, the real nodes
        // between, chained over pages through their first level.
        let mut nodes: Vec<(u32, u16)> = vec![(NONE, u16::MAX)];
        nodes.extend(addresses);
        nodes.push((NONE, u16::MAX));
        let first_index = pages.len() as u32;
        let chunks: Vec<&[(u32, u16)]> = nodes.chunks(per_index).collect();
        let count = chunks.len() as u32;
        for (i, chunk) in chunks.iter().enumerate() {
            let id = first_index + i as u32;
            let mut p = header(id, T_INDEX, NONE, chunk.len() as u16);
            for (j, data) in chunk.iter().enumerate() {
                p.extend((j as u16).to_le_bytes());
                p.push(1); // levels
                p.push(0); // slot
                p.extend([0xffu8; 12]); // PrevNode, NextNode
                p.extend(4u16.to_le_bytes());
                p.push(0x10);
                p.extend((i as i32 * 1000 + j as i32).to_le_bytes());
                p.extend(data.0.to_le_bytes());
                p.extend(data.1.to_le_bytes());
                let prev = if i == 0 { NONE } else { id - 1 };
                let next = if i as u32 + 1 == count { NONE } else { id + 1 };
                p.extend(prev.to_le_bytes());
                p.extend(0u16.to_le_bytes());
                p.extend(next.to_le_bytes());
                p.extend(0u16.to_le_bytes());
            }
            pages.push(finish(p));
        }
        // The collection page.
        let mut c = header(1, T_COLLECTION, NONE, 0);
        string(&mut c, collection);
        c.extend([0u8; 12]);
        string(&mut c, "_id");
        c.push(1);
        c.extend(first_index.to_le_bytes());
        c.extend(0u16.to_le_bytes());
        c.extend([0xffu8; 6]);
        c.extend(NONE.to_le_bytes());
        pages[1] = finish(c);
        // The header page.
        let mut h = header(0, T_HEADER, NONE, 0);
        h.extend(INFO);
        h.push(7);
        h.extend([0u8; 2 + 4 + 4 + 2 + 20 + 16]);
        h.push(1);
        string(&mut h, collection);
        h.extend(1u32.to_le_bytes());
        pages[0] = finish(h);
        pages.concat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bson::encode;

    fn docs(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| encode(&[("Name", Bson::Str(format!("Game {i}"))), ("Notes", Bson::Str("x".repeat(if i % 3 == 0 { 9_000 } else { 10 })))])).collect()
    }

    #[test]
    fn reads_every_document_inline_and_extended() {
        let file = testfile::build("Game", &docs(10), 2_000, 4);
        let db = File::open(&file).unwrap();
        assert_eq!(db.collections(), vec!["Game"]);
        let got = db.documents("game").unwrap();
        let names: Vec<_> = got.iter().map(|d| d.str("Name").unwrap().to_string()).collect();
        assert_eq!(names, (0..10).map(|i| format!("Game {i}")).collect::<Vec<_>>());
        assert_eq!(got[0].str("Notes").unwrap().len(), 9_000);
        assert!(db.documents("Platform").unwrap().is_empty());
    }

    #[test]
    fn refuses_what_is_not_a_litedb_4_file() {
        assert_eq!(File::open(b"").err(), Some(LiteError::NotLiteDb));
        let mut v5 = testfile::build("Game", &docs(1), 2_000, 4);
        v5[HEADER + INFO.len()] = 8;
        assert_eq!(File::open(&v5).err(), Some(LiteError::NotLiteDb));
        let mut locked = testfile::build("Game", &docs(1), 2_000, 4);
        locked[HEADER + INFO.len() + 1 + 32] = 1;
        assert_eq!(File::open(&locked).err(), Some(LiteError::Encrypted));
    }

    #[test]
    fn damage_costs_documents_not_the_app() {
        let file = testfile::build("Game", &docs(8), 2_000, 3);
        // Every byte of every page flipped in turn would take long; flip a
        // spread of them and truncate at many places. Nothing may panic.
        for at in (0..file.len()).step_by(97) {
            let mut bad = file.clone();
            bad[at] ^= 0xff;
            if let Ok(db) = File::open(&bad) {
                let _ = db.documents("Game");
            }
        }
        for cut in (0..file.len()).step_by(PAGE / 2) {
            if let Ok(db) = File::open(&file[..cut]) {
                let _ = db.documents("Game");
            }
        }
    }

    #[test]
    fn an_extend_chain_that_loops_is_cut() {
        let mut file = testfile::build("Game", &[encode(&[("Notes", Bson::Str("y".repeat(9_000)))])], 100, 4);
        // Page 3 is the first extend page; point the last one back at it.
        let last = 5;
        file[last * PAGE + 9..last * PAGE + 13].copy_from_slice(&3u32.to_le_bytes());
        let db = File::open(&file).unwrap();
        assert!(db.documents("Game").unwrap().is_empty());
    }
}
