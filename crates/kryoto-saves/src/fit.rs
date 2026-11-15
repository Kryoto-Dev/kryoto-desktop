//! Whether a backup can be taken, decided before anything is packed.
//!
//! A place caps one backup two ways: the packed file ([`Limits::packed`]) and
//! the save data it holds ([`Limits::raw`], larger, so saves that pack small
//! fit). The save data's size is the files' sizes from the scan (metadata
//! only). Only when that is over the packed cap is anything read: a few
//! megabytes from the biggest files ([`estimate`]) say about how small the
//! whole lot packs with the most compression ([`crate::bundle::Packing::Small`]),
//! so the player can be asked before minutes of packing, and nothing is packed
//! that would be refused anyway.

use std::cmp::Reverse;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::scan::Found;

/// What one backup may be, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The backup file, packed.
    pub packed: u64,
    /// The save data it holds, before packing (at least `packed`).
    pub raw: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Under the cap as they are: packed the usual way.
    Fits,
    /// Over the cap as they are, but about `estimate` packed small: fits.
    Compress { estimate: u64 },
    /// Will not fit however they are packed: more save data than one backup
    /// holds (`estimate` None, nothing was read), or about `estimate` packed
    /// small, still over the cap.
    TooBig { estimate: Option<u64> },
}

/// Decide from the save data's size (`raw`). `estimate` (about the packed
/// size, packed small) is only asked when it could change the answer.
pub fn judge(raw: u64, limits: Limits, estimate: impl FnOnce() -> u64) -> Fit {
    if raw <= limits.packed {
        return Fit::Fits;
    }
    if raw > limits.raw.max(limits.packed) {
        return Fit::TooBig { estimate: None };
    }
    let estimate = estimate();
    if estimate <= limits.packed {
        Fit::Compress { estimate }
    } else {
        Fit::TooBig { estimate: Some(estimate) }
    }
}

/// How many of the biggest files are sampled.
pub const SAMPLE_FILES: usize = 24;
/// Bytes read at each of a sampled file's start, middle and end.
pub const SAMPLE_CHUNK: u64 = 64 * 1024;
/// The deflate level of [`crate::bundle::Packing::Small`]. Above 9 the zip
/// crate switches to Zopfli, far too slow for a backup.
pub const SMALL_LEVEL: u32 = 9;
/// Sampled ratios are taken this much worse than measured (never worse than
/// not packing at all): a sample is a guess, and a guess that is too kind
/// costs the player a pack for nothing.
const MARGIN: f64 = 1.05;

/// One sampled file: its whole size, and what of it was read and packed into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sampled {
    pub size: u64,
    pub read: u64,
    pub packed: u64,
}

/// About how big `files` pack with [`crate::bundle::Packing::Small`], zip
/// headers and manifest included. Reads at most [`SAMPLE_FILES`] files,
/// three [`SAMPLE_CHUNK`]s each (about 4.5 MB); blocking, so never on the UI
/// thread. A file that cannot be read counts as not packing at all.
pub fn estimate(files: &[Found]) -> u64 {
    let mut biggest: Vec<&Found> = files.iter().filter(|f| f.size > 0).collect();
    biggest.sort_by_key(|f| Reverse(f.size));
    let samples: Vec<Sampled> = biggest.iter().take(SAMPLE_FILES).filter_map(|f| sample(&f.abs, f.size)).collect();
    let total: u64 = files.iter().map(|f| f.size).sum();
    extrapolate(&samples, total) + overhead(files)
}

/// The packed size of `total` bytes of files from samples of some of them:
/// each sampled file at its own ratio, the rest at the ratio of every sample
/// together (or not packing at all, with no samples). Pure.
pub fn extrapolate(samples: &[Sampled], total: u64) -> u64 {
    let (mut read, mut packed, mut covered, mut est) = (0u64, 0u64, 0u64, 0f64);
    for s in samples.iter().filter(|s| s.read > 0) {
        read += s.read;
        packed += s.packed;
        covered += s.size;
        est += s.size as f64 * ratio(s.packed, s.read);
    }
    let rest = total.saturating_sub(covered);
    let rest_ratio = if read > 0 { ratio(packed, read) } else { 1.0 };
    (est + rest as f64 * rest_ratio).round() as u64
}

fn ratio(packed: u64, read: u64) -> f64 {
    (packed as f64 / read as f64 * MARGIN).min(1.0)
}

/// Zip headers (local and central, Zip64 extras, a data descriptor, the entry
/// name twice) and the manifest's line for each file, plus a little fixed.
pub fn overhead(files: &[Found]) -> u64 {
    4096 + files.iter().map(|f| 400 + 3 * f.path.len() as u64).sum::<u64>()
}

/// Pack a file's start, middle and end (or all of it, when that is less)
/// into one deflate stream at [`SMALL_LEVEL`], counting what came out.
fn sample(path: &Path, size: u64) -> Option<Sampled> {
    let mut file = std::fs::File::open(path).ok()?;
    let offsets: Vec<u64> = if size <= 3 * SAMPLE_CHUNK {
        vec![0]
    } else {
        vec![0, size / 2 - SAMPLE_CHUNK / 2, size - SAMPLE_CHUNK]
    };
    let mut enc = flate2::write::DeflateEncoder::new(Counter(0), flate2::Compression::new(SMALL_LEVEL));
    let mut read = 0u64;
    let mut buf = Vec::with_capacity(3 * SAMPLE_CHUNK as usize);
    for at in offsets {
        file.seek(SeekFrom::Start(at)).ok()?;
        buf.clear();
        let want = if size <= 3 * SAMPLE_CHUNK { 3 * SAMPLE_CHUNK } else { SAMPLE_CHUNK };
        (&mut file).take(want).read_to_end(&mut buf).ok()?;
        enc.write_all(&buf).ok()?;
        read += buf.len() as u64;
    }
    let packed = enc.finish().ok()?.0;
    Some(Sampled { size, read, packed })
}

/// Counts the bytes written to it, and keeps none.
struct Counter(u64);

impl Write for Counter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0 += b.len() as u64;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{write_packed, Manifest, Packing};
    use crate::rules::{Root, Rule, SaveRoots, Source};
    use crate::scan::{collect, MAX_FILES};
    use std::path::PathBuf;

    const MB: u64 = 1 << 20;
    const LIMITS: Limits = Limits { packed: 100 * MB, raw: 400 * MB };

    fn never() -> u64 {
        panic!("nothing should be read")
    }

    #[test]
    fn under_the_cap_is_packed_as_always_without_reading_anything() {
        assert_eq!(judge(0, LIMITS, never), Fit::Fits);
        assert_eq!(judge(100 * MB, LIMITS, never), Fit::Fits);
    }

    #[test]
    fn more_save_data_than_a_backup_holds_is_refused_without_reading_anything() {
        assert_eq!(judge(400 * MB + 1, LIMITS, never), Fit::TooBig { estimate: None });
        // A server that does not say holds the save data to the packed cap.
        let old = Limits { packed: 100 * MB, raw: 0 };
        assert_eq!(judge(100 * MB + 1, old, never), Fit::TooBig { estimate: None });
    }

    #[test]
    fn over_the_cap_asks_only_when_packing_small_fits() {
        assert_eq!(judge(300 * MB, LIMITS, || 80 * MB), Fit::Compress { estimate: 80 * MB });
        assert_eq!(judge(300 * MB, LIMITS, || 100 * MB), Fit::Compress { estimate: 100 * MB });
        assert_eq!(judge(300 * MB, LIMITS, || 100 * MB + 1), Fit::TooBig { estimate: Some(100 * MB + 1) });
        assert_eq!(judge(400 * MB, LIMITS, || 390 * MB), Fit::TooBig { estimate: Some(390 * MB) });
    }

    #[test]
    fn extrapolates_each_sample_at_its_own_ratio_and_the_rest_at_theirs() {
        // Nothing read: nothing is assumed to pack.
        assert_eq!(extrapolate(&[], 1000), 1000);
        // A 1000-byte file that packs to a tenth, and 1000 bytes unsampled:
        // both at a tenth, taken a little worse.
        let s = [Sampled { size: 1000, read: 100, packed: 10 }];
        assert_eq!(extrapolate(&s, 2000), 210);
        // Never more than not packing at all (deflate adds a few bytes to
        // random data; the headers are counted apart).
        let random = [Sampled { size: 1000, read: 100, packed: 101 }];
        assert_eq!(extrapolate(&random, 1000), 1000);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryo-saves-fit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Bytes that do not pack (xorshift).
    fn noise(len: usize, mut x: u64) -> Vec<u8> {
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn the_estimate_is_close_to_the_real_pack_and_not_below_it() {
        let d = tmp("real");
        let saves = d.join("docs/Game/Saves");
        std::fs::create_dir_all(&saves).unwrap();
        // Text that packs well, a world file that is half noise, and a
        // screenshot-like file that does not pack, as games mix them.
        let text: Vec<u8> = (0..3_000_000u32).flat_map(|i| format!("slot {} hp={} pos=({}, {})\n", i % 7, i % 100, i % 640, i % 480).into_bytes()).take(3_000_000).collect();
        std::fs::write(saves.join("profile.json"), &text).unwrap();
        let mut world = noise(1_000_000, 7);
        world.extend(std::iter::repeat_n(0u8, 1_000_000));
        std::fs::write(saves.join("world.sav"), &world).unwrap();
        std::fs::write(saves.join("thumb.bin"), noise(500_000, 99)).unwrap();
        for i in 0..40 {
            std::fs::write(saves.join(format!("small{i}.dat")), format!("small save {i}").repeat(50)).unwrap();
        }
        let roots = SaveRoots { documents: Some(d.join("docs")), ..Default::default() };
        let rules = vec![Rule::files(Root::Documents, "Game/Saves", Source::Known)];
        let found = collect(&rules, &roots, MAX_FILES);
        let guess = estimate(&found.files);
        let template = Manifest { game_key: "steam:1".into(), rules, created_at: 1, machine: "PC".into(), ..Default::default() };
        let (_, small) = write_packed(&template, &found.files, &d.join("small.ksave"), Packing::Small, &mut |_, _| {}).unwrap();
        let (_, normal) = write_packed(&template, &found.files, &d.join("normal.ksave"), Packing::Normal, &mut |_, _| {}).unwrap();
        assert!(small.size <= normal.size, "small {} normal {}", small.size, normal.size);
        assert!(guess >= small.size, "estimate {guess} under the real {}", small.size);
        assert!(guess <= small.size * 13 / 10, "estimate {guess} far over the real {}", small.size);
        assert!(guess < found.bytes / 2, "these saves pack to under half");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saves_that_do_not_pack_are_estimated_at_their_size() {
        let d = tmp("noise");
        std::fs::create_dir_all(d.join("docs/G")).unwrap();
        std::fs::write(d.join("docs/G/a.sav"), noise(2_000_000, 3)).unwrap();
        let roots = SaveRoots { documents: Some(d.join("docs")), ..Default::default() };
        let found = collect(&[Rule::files(Root::Documents, "G", Source::Known)], &roots, MAX_FILES);
        let guess = estimate(&found.files);
        assert!((2_000_000..2_200_000).contains(&guess), "{guess}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
