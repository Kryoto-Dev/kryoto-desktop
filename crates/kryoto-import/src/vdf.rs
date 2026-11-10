//! Valve's two key-value formats.
//!
//! Text (`libraryfolders.vdf`, `appmanifest_*.acf`, `localconfig.vdf`,
//! `loginusers.vdf`) is only ever read. Binary (`shortcuts.vdf`) is read and
//! written back, so its parser keeps every byte it does not understand the
//! meaning of: [`write_binary`] of [`parse_binary`] is the same file, and a
//! caller can (and does) refuse to touch a file for which that is not so.

/* ── text ──────────────────────────────────────────────────────────── */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Str(String),
    Map(Vec<(String, Text)>),
}

impl Text {
    /// A value by key, ignoring case: Steam writes `apps`, `Apps` and `Software`,
    /// `software` depending on its age.
    pub fn get(&self, key: &str) -> Option<&Text> {
        match self {
            Text::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Text::Str(_) => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Text> {
        keys.iter().try_fold(self, |at, k| at.get(k))
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Text::Str(s) => Some(s),
            Text::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Text)] {
        match self {
            Text::Map(m) => m,
            Text::Str(_) => &[],
        }
    }
}

struct Tokens<'a> {
    rest: std::str::Chars<'a>,
}

#[derive(PartialEq)]
enum Token {
    Open,
    Close,
    Word(String),
}

impl Tokens<'_> {
    fn next(&mut self) -> Option<Token> {
        loop {
            let c = self.rest.next()?;
            match c {
                '{' => return Some(Token::Open),
                '}' => return Some(Token::Close),
                '"' => {
                    let mut out = String::new();
                    loop {
                        match self.rest.next()? {
                            '"' => return Some(Token::Word(out)),
                            '\\' => match self.rest.next()? {
                                'n' => out.push('\n'),
                                't' => out.push('\t'),
                                other => out.push(other),
                            },
                            other => out.push(other),
                        }
                    }
                }
                '/' if self.rest.as_str().starts_with('/') => {
                    // A // comment, to the end of the line.
                    for c in self.rest.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                c if c.is_whitespace() || c == '\u{feff}' => {}
                c => {
                    // An unquoted word.
                    let mut out = c.to_string();
                    while let Some(n) = self.rest.as_str().chars().next() {
                        if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                            break;
                        }
                        out.push(n);
                        self.rest.next();
                    }
                    return Some(Token::Word(out));
                }
            }
        }
    }
}

fn parse_map(t: &mut Tokens, top: bool) -> Option<Vec<(String, Text)>> {
    let mut out = Vec::new();
    loop {
        let key = match t.next() {
            Some(Token::Word(k)) => k,
            Some(Token::Close) if !top => return Some(out),
            None if top => return Some(out),
            _ => return None,
        };
        match t.next()? {
            Token::Word(v) => out.push((key, Text::Str(v))),
            Token::Open => out.push((key, Text::Map(parse_map(t, false)?))),
            Token::Close => return None,
        }
    }
}

/// A text VDF as a map of its top-level keys; None when it is not one.
pub fn parse_text(text: &str) -> Option<Text> {
    parse_map(&mut Tokens { rest: text.chars() }, true).map(Text::Map)
}

/* ── binary ────────────────────────────────────────────────────────── */

/// One value. Keys and strings stay bytes: a name need not be UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bin {
    Map(Vec<(Vec<u8>, Bin)>),
    Str(Vec<u8>),
    Int(u32),
    /// A number of another width, as it was: type byte and raw value.
    Raw(u8, Vec<u8>),
}

const MAP: u8 = 0;
const STR: u8 = 1;
const INT: u8 = 2;
const FLOAT: u8 = 3;
const U64: u8 = 7;
const END: u8 = 8;
const I64: u8 = 10;

fn cstr(buf: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let end = buf.get(*pos..)?.iter().position(|b| *b == 0)? + *pos;
    let s = buf[*pos..end].to_vec();
    *pos = end + 1;
    Some(s)
}

fn parse_body(buf: &[u8], pos: &mut usize, depth: usize) -> Option<Vec<(Vec<u8>, Bin)>> {
    if depth > 16 {
        return None;
    }
    let mut out = Vec::new();
    loop {
        let kind = *buf.get(*pos)?;
        *pos += 1;
        if kind == END {
            return Some(out);
        }
        let key = cstr(buf, pos)?;
        let value = match kind {
            MAP => Bin::Map(parse_body(buf, pos, depth + 1)?),
            STR => Bin::Str(cstr(buf, pos)?),
            INT => {
                let b = buf.get(*pos..*pos + 4)?;
                *pos += 4;
                Bin::Int(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            FLOAT | U64 | I64 => {
                let n = if kind == FLOAT { 4 } else { 8 };
                let b = buf.get(*pos..*pos + n)?.to_vec();
                *pos += n;
                Bin::Raw(kind, b)
            }
            _ => return None,
        };
        out.push((key, value));
    }
}

/// A binary VDF's top level; None for anything that is not exactly one.
pub fn parse_binary(buf: &[u8]) -> Option<Vec<(Vec<u8>, Bin)>> {
    let mut pos = 0;
    let out = parse_body(buf, &mut pos, 0)?;
    (pos == buf.len()).then_some(out)
}

fn write_body(map: &[(Vec<u8>, Bin)], out: &mut Vec<u8>) {
    for (k, v) in map {
        let kind = match v {
            Bin::Map(_) => MAP,
            Bin::Str(_) => STR,
            Bin::Int(_) => INT,
            Bin::Raw(t, _) => *t,
        };
        out.push(kind);
        out.extend_from_slice(k);
        out.push(0);
        match v {
            Bin::Map(m) => write_body(m, out),
            Bin::Str(s) => {
                out.extend_from_slice(s);
                out.push(0);
            }
            Bin::Int(n) => out.extend_from_slice(&n.to_le_bytes()),
            Bin::Raw(_, b) => out.extend_from_slice(b),
        }
    }
    out.push(END);
}

pub fn write_binary(map: &[(Vec<u8>, Bin)]) -> Vec<u8> {
    let mut out = Vec::new();
    write_body(map, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_as_steam_writes_it() {
        let t = parse_text("\u{feff}\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"228980\"\t\t\"12\"\n\t\t}\n\t}\n\t// a comment\n\t\"contentstatsid\"\t\t\"123\"\n}\n").unwrap();
        assert_eq!(t.path(&["LibraryFolders", "0", "PATH"]).and_then(Text::str), Some(r"C:\Program Files (x86)\Steam"));
        assert_eq!(t.path(&["libraryfolders", "0", "apps"]).map(|a| a.entries().len()), Some(1));
        assert_eq!(t.path(&["libraryfolders", "contentstatsid"]).and_then(Text::str), Some("123"));
    }

    #[test]
    fn text_refuses_what_is_cut_short() {
        assert!(parse_text("\"a\"\n{\n\"b\" \"c\"\n").is_none());
        assert!(parse_text("\"a\" \"unterminated").is_none());
        assert!(parse_text("}").is_none());
    }

    #[test]
    fn binary_comes_back_byte_for_byte_even_what_it_does_not_understand() {
        let mut raw = vec![MAP];
        raw.extend_from_slice(b"shortcuts\0");
        raw.push(MAP);
        raw.extend_from_slice(b"0\0");
        raw.push(INT);
        raw.extend_from_slice(b"appid\0");
        raw.extend_from_slice(&0x8123_4567u32.to_le_bytes());
        raw.push(STR);
        raw.extend_from_slice(b"AppName\0caf\xe9 not utf8\0");
        raw.push(U64);
        raw.extend_from_slice(b"Big\0");
        raw.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        raw.extend_from_slice(&[END, END, END]);
        let parsed = parse_binary(&raw).unwrap();
        assert_eq!(write_binary(&parsed), raw);
    }

    #[test]
    fn binary_refuses_damage() {
        assert!(parse_binary(b"").is_none());
        assert!(parse_binary(&[MAP, b'a']).is_none());
        // An unknown type: not guessed at.
        assert!(parse_binary(&[9, b'a', 0, END]).is_none());
        // Bytes after the end.
        assert!(parse_binary(&[END, 1]).is_none());
    }
}
