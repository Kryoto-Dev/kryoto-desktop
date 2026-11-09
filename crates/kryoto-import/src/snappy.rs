//! Snappy decompression, the raw format LevelDB compresses its table blocks
//! with. Only decoding: nothing here ever writes a launcher's files.

/// The decoded bytes, or `None` for input that is not valid Snappy.
pub fn decompress(input: &[u8]) -> Option<Vec<u8>> {
    let (len, mut pos) = varint(input, 0)?;
    // LevelDB blocks are a few KiB; refuse anything absurd before allocating.
    if len > 64 << 20 {
        return None;
    }
    let len = len as usize;
    let mut out = Vec::with_capacity(len);
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        match tag & 3 {
            0 => {
                let mut n = (tag >> 2) as usize;
                if n >= 60 {
                    let extra = n - 59;
                    let bytes = input.get(pos..pos + extra)?;
                    n = bytes.iter().rev().fold(0usize, |acc, b| (acc << 8) | *b as usize);
                    pos += extra;
                }
                let n = n + 1;
                out.extend_from_slice(input.get(pos..pos + n)?);
                pos += n;
            }
            kind => {
                let (n, offset) = match kind {
                    1 => {
                        let b = *input.get(pos)? as usize;
                        pos += 1;
                        (4 + ((tag >> 2) & 7) as usize, ((tag as usize >> 5) << 8) | b)
                    }
                    2 => {
                        let b = input.get(pos..pos + 2)?;
                        pos += 2;
                        ((tag >> 2) as usize + 1, u16::from_le_bytes([b[0], b[1]]) as usize)
                    }
                    _ => {
                        let b = input.get(pos..pos + 4)?;
                        pos += 4;
                        ((tag >> 2) as usize + 1, u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                    }
                };
                if offset == 0 || offset > out.len() {
                    return None;
                }
                // Byte by byte: a copy may overlap what it is writing (a run).
                let start = out.len() - offset;
                for i in 0..n {
                    let b = out[start + i];
                    out.push(b);
                }
            }
        }
        if out.len() > len {
            return None;
        }
    }
    (out.len() == len).then_some(out)
}

pub(crate) fn varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let b = *buf.get(pos)?;
        pos += 1;
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((value, pos));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decompress;

    #[test]
    fn literals_and_overlapping_copies() {
        // "abcabcabcabc": a literal "abc", then a 9-byte copy at offset 3.
        let data = [12, 0b0000_1000, b'a', b'b', b'c', 0b0001_0101, 3];
        assert_eq!(decompress(&data).unwrap(), b"abcabcabcabc");
        // A two-byte-offset copy.
        let data = [8, 0b0000_1100, b'w', b'x', b'y', b'z', 0b0000_1110, 4, 0];
        assert_eq!(decompress(&data).unwrap(), b"wxyzwxyz");
    }

    #[test]
    fn rejects_damage() {
        assert!(decompress(&[]).is_none());
        // Claims 5 bytes, holds 3.
        assert!(decompress(&[5, 0b0000_1000, b'a', b'b', b'c']).is_none());
        // A copy reaching before the start.
        assert!(decompress(&[4, 0b0000_0001, 9]).is_none());
    }
}
