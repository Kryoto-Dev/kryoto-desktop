//! Just enough BSON to read LiteDB documents (`litedb.rs`). Every read is
//! bounds-checked: a damaged or hostile file gives `None`, never a panic.

/// One BSON value. Numbers keep their stored kind; `as_u64` reads any of them.
#[derive(Debug, Clone, PartialEq)]
pub enum Bson {
    Double(f64),
    Str(String),
    Doc(Vec<(String, Bson)>),
    Array(Vec<Bson>),
    /// Subtype and bytes. Subtype 4 is a .NET `Guid`.
    Binary(u8, Vec<u8>),
    ObjectId([u8; 12]),
    Bool(bool),
    /// Milliseconds since 1970, UTC.
    DateTime(i64),
    Null,
    Int32(i32),
    Int64(i64),
    /// .NET `decimal`: lo, mid, hi, flags (scale in bits 16-23, sign in bit 31).
    Decimal([u8; 16]),
    MinKey,
    MaxKey,
}

impl Bson {
    /// A field of a document.
    pub fn get(&self, key: &str) -> Option<&Bson> {
        match self {
            Bson::Doc(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A non-empty string.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Bson::Str(s) if !s.trim().is_empty() => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Bson::Bool(b) => Some(*b),
            Bson::Int32(n) => Some(*n != 0),
            _ => None,
        }
    }

    pub fn array(&self, key: &str) -> &[Bson] {
        match self.get(key) {
            Some(Bson::Array(items)) => items,
            _ => &[],
        }
    }

    /// Any number as a whole, non-negative count. LiteDB 4 stores a .NET
    /// `ulong` as `Int64` (unchecked), older mappers as a double or decimal.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Bson::Int32(n) => u64::try_from(*n).ok(),
            Bson::Int64(n) => u64::try_from(*n).ok(),
            Bson::Double(f) if f.is_finite() && *f >= 0.0 => Some(f.round() as u64),
            Bson::Decimal(d) => decimal_u64(d),
            Bson::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// A date as Unix seconds; None for .NET's MinValue and other dates before 1970.
    pub fn as_unix_seconds(&self) -> Option<u64> {
        match self {
            Bson::DateTime(ms) if *ms > 0 => Some((*ms / 1000) as u64),
            _ => None,
        }
    }

    /// A .NET `Guid` the way .NET writes it as text, lower case.
    pub fn as_guid(&self) -> Option<String> {
        match self {
            Bson::Binary(_, b) if b.len() == 16 => Some(guid_text(b)),
            Bson::Str(s) if s.len() == 36 => Some(s.to_ascii_lowercase()),
            _ => None,
        }
    }
}

/// .NET stores a Guid's first three groups little-endian.
pub fn guid_text(b: &[u8]) -> String {
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = String::with_capacity(36);
    for (i, &at) in order.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{:02x}", b[at]));
    }
    out
}

fn decimal_u64(d: &[u8; 16]) -> Option<u64> {
    let word = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as u128;
    let (lo, mid, hi, flags) = (word(0), word(4), word(8), word(12) as u32);
    if flags & 0x8000_0000 != 0 {
        return None;
    }
    let scale = (flags >> 16) & 0xff;
    let mut value = lo | (mid << 32) | (hi << 64);
    for _ in 0..scale.min(28) {
        value /= 10;
    }
    u64::try_from(value).ok()
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn cstring(&mut self) -> Option<String> {
        let rest = self.b.get(self.at..)?;
        let len = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.at += len + 1;
        Some(s)
    }
}

/// One whole BSON document.
pub fn parse(bytes: &[u8]) -> Option<Bson> {
    let mut c = Cursor { b: bytes, at: 0 };
    let doc = document(&mut c, 0)?;
    Some(Bson::Doc(doc))
}

/// Deep nesting is refused rather than followed.
const MAX_DEPTH: usize = 64;

fn document(c: &mut Cursor, depth: usize) -> Option<Vec<(String, Bson)>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let start = c.at;
    let len = usize::try_from(c.i32()?).ok()?;
    if len < 5 {
        return None;
    }
    let end = start.checked_add(len)?;
    if end > c.b.len() {
        return None;
    }
    let mut out = Vec::new();
    while c.at < end - 1 {
        let kind = c.u8()?;
        let name = c.cstring()?;
        out.push((name, value(c, kind, depth)?));
    }
    // The closing zero.
    (c.u8()? == 0 && c.at == end).then_some(out)
}

fn value(c: &mut Cursor, kind: u8, depth: usize) -> Option<Bson> {
    Some(match kind {
        0x01 => Bson::Double(f64::from_le_bytes(c.take(8)?.try_into().ok()?)),
        0x02 => {
            let len = usize::try_from(c.i32()?).ok()?;
            let raw = c.take(len)?;
            let text = raw.strip_suffix(&[0]).unwrap_or(raw);
            Bson::Str(String::from_utf8_lossy(text).into_owned())
        }
        0x03 => Bson::Doc(document(c, depth + 1)?),
        0x04 => Bson::Array(document(c, depth + 1)?.into_iter().map(|(_, v)| v).collect()),
        0x05 => {
            let len = usize::try_from(c.i32()?).ok()?;
            let sub = c.u8()?;
            Bson::Binary(sub, c.take(len)?.to_vec())
        }
        0x07 => Bson::ObjectId(c.take(12)?.try_into().ok()?),
        0x08 => Bson::Bool(c.u8()? != 0),
        0x09 => Bson::DateTime(c.i64()?),
        0x0A => Bson::Null,
        0x10 => Bson::Int32(c.i32()?),
        0x12 => Bson::Int64(c.i64()?),
        0x13 => Bson::Decimal(c.take(16)?.try_into().ok()?),
        0xFF => Bson::MinKey,
        0x7F => Bson::MaxKey,
        _ => return None,
    })
}

/// Writing BSON, for synthetic LiteDB files (tests, `examples/`).
#[doc(hidden)]
pub fn encode(doc: &[(&str, Bson)]) -> Vec<u8> {
    fn body(fields: &mut dyn Iterator<Item = (String, &Bson)>) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 0];
        for (name, v) in fields {
            let (kind, bytes) = one(v);
            out.push(kind);
            out.extend(name.as_bytes());
            out.push(0);
            out.extend(bytes);
        }
        out.push(0);
        let len = out.len() as i32;
        out[..4].copy_from_slice(&len.to_le_bytes());
        out
    }
    fn one(v: &Bson) -> (u8, Vec<u8>) {
        match v {
            Bson::Double(f) => (0x01, f.to_le_bytes().to_vec()),
            Bson::Str(s) => {
                let mut b = ((s.len() + 1) as i32).to_le_bytes().to_vec();
                b.extend(s.as_bytes());
                b.push(0);
                (0x02, b)
            }
            Bson::Doc(f) => (0x03, body(&mut f.iter().map(|(k, v)| (k.clone(), v)))),
            Bson::Array(items) => (0x04, body(&mut items.iter().enumerate().map(|(i, v)| (i.to_string(), v)))),
            Bson::Binary(sub, b) => {
                let mut out = (b.len() as i32).to_le_bytes().to_vec();
                out.push(*sub);
                out.extend(b);
                (0x05, out)
            }
            Bson::ObjectId(b) => (0x07, b.to_vec()),
            Bson::Bool(b) => (0x08, vec![*b as u8]),
            Bson::DateTime(ms) => (0x09, ms.to_le_bytes().to_vec()),
            Bson::Null => (0x0A, Vec::new()),
            Bson::Int32(n) => (0x10, n.to_le_bytes().to_vec()),
            Bson::Int64(n) => (0x12, n.to_le_bytes().to_vec()),
            Bson::Decimal(d) => (0x13, d.to_vec()),
            Bson::MinKey => (0xFF, Vec::new()),
            Bson::MaxKey => (0x7F, Vec::new()),
        }
    }
    body(&mut doc.iter().map(|(k, v)| (k.to_string(), v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_it_wrote() {
        let guid = vec![0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];
        let bytes = encode(&[
            ("_id", Bson::Binary(4, guid)),
            ("Name", Bson::Str("Ōkami HD 大神".into())),
            ("Playtime", Bson::Int64(7_200)),
            ("LastActivity", Bson::DateTime(1_700_000_000_123)),
            ("GameActions", Bson::Array(vec![Bson::Doc(vec![("Path".into(), Bson::Str("a.exe".into()))])])),
        ]);
        let doc = parse(&bytes).unwrap();
        assert_eq!(doc.get("_id").unwrap().as_guid().unwrap(), "12345678-1234-1234-1234-56789abcdef0");
        assert_eq!(doc.str("Name"), Some("Ōkami HD 大神"));
        assert_eq!(doc.get("Playtime").unwrap().as_u64(), Some(7_200));
        assert_eq!(doc.get("LastActivity").unwrap().as_unix_seconds(), Some(1_700_000_000));
        assert_eq!(doc.array("GameActions")[0].str("Path"), Some("a.exe"));
    }

    #[test]
    fn numbers_of_every_kind_read_as_counts() {
        // 12345.0 as a .NET decimal with scale 1: 123450 / 10.
        let mut d = [0u8; 16];
        d[..4].copy_from_slice(&123_450u32.to_le_bytes());
        d[12..].copy_from_slice(&(1u32 << 16).to_le_bytes());
        assert_eq!(Bson::Decimal(d).as_u64(), Some(12_345));
        assert_eq!(Bson::Double(3.6).as_u64(), Some(4));
        assert_eq!(Bson::Int64(-1).as_u64(), None);
        assert_eq!(Bson::DateTime(-62_135_596_800_000).as_unix_seconds(), None);
    }

    #[test]
    fn damage_is_refused_not_followed() {
        let good = encode(&[("Name", Bson::Str("x".into()))]);
        for cut in 0..good.len() {
            assert!(parse(&good[..cut]).is_none(), "cut at {cut}");
        }
        let mut lying = good.clone();
        lying[0] = 200;
        assert!(parse(&lying).is_none());
        // A string claiming more bytes than there are.
        let mut long = good;
        long[10] = 120;
        assert!(parse(&long).is_none());
    }
}
