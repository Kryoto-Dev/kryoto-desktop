//! HatchZip (hatchzip.dev, MIT/Apache-2.0): unpacking an archive in place.
//!
//! 7-Zip writes the game next to its archive, so a 180 GB game needs 360 GB
//! free for a moment. HatchZip reads the archive from its end and cuts it
//! short as each file lands, so the drive only ever has to hold the game and
//! a little extra. Its `check` says beforehand whether that will fit and how
//! much it saves; a 7z packed as one solid block saves nothing (the block
//! can only go once all of it is out), and 7-Zip stays the unpacker then.
//!
//! The pinned release is fetched once into `<app_data>/tools/`, like `7zr.exe`
//! (downloads.rs `ensure_7zr`), and its SHA-256 is checked against the hash
//! recorded here before it is ever run. Exit codes, from its README: 0 done,
//! 1 failed (progress kept), 2 bad arguments, 3 not enough space (nothing
//! changed), 4 damaged archive (nothing deleted), 5 password, 130 paused.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

/// The release this build fetches. Bump the three together.
const VERSION: &str = "v0.1.8";

struct Pinned {
    /// The asset's name under the release.
    asset: &'static str,
    /// SHA-256 of that asset, from its `.sha256` companion on the release.
    sha256: &'static str,
    /// The CLI's path inside the asset.
    inner: &'static str,
}

#[cfg(all(windows, target_arch = "x86_64"))]
const PINNED: Option<Pinned> = Some(Pinned {
    asset: "hatchzip-x86_64-windows.zip",
    sha256: "8b90782629930e25f336246024e178dbdcb6087d346d386e6a6fdccef0948764",
    inner: "hatchzip.exe",
});
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PINNED: Option<Pinned> = Some(Pinned {
    asset: "hatchzip-x86_64-linux.tar.gz",
    sha256: "9c2e734dde74ea822979e6ddb071e7271b4d5fd924e4f3cf48b778f8a70da301",
    inner: "hatchzip-x86_64-linux/hatchzip",
});
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const PINNED: Option<Pinned> = Some(Pinned {
    asset: "hatchzip-aarch64-linux.tar.gz",
    sha256: "8f1a4d18248bd35e07074890d673769c0a173d9266db26dfcb1ba6cf4fa02468",
    inner: "hatchzip-aarch64-linux/hatchzip",
});
#[cfg(not(any(all(windows, target_arch = "x86_64"), all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))))]
const PINNED: Option<Pinned> = None;

/// Free space HatchZip leaves untouched on the drive while it works.
pub const KEEP_FREE: &str = "500MB";

/// What `hatchzip check` said about an archive, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// What 7-Zip would need next to the archive: the unpacked size.
    pub standard: u64,
    /// What HatchZip needs on top of what is there now (reserve not counted).
    pub required: u64,
    pub available: u64,
}

impl Plan {
    /// Whether unpacking in place is worth anything: a solid single-block 7z
    /// needs the whole block out before a byte of the archive can go.
    pub fn gains(&self) -> bool {
        self.standard > 0 && self.required < self.standard.saturating_mul(9) / 10
    }
}

/// Why an unpack did not finish, by HatchZip's exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Exit 3: not enough room even in place. Nothing was changed.
    NoSpace,
    /// Exit 4: the archive is damaged. Nothing was deleted.
    Damaged(String),
    /// Exit 5: a password is wanted.
    Password,
    /// Stopped by Pause or Cancel.
    Stopped,
    /// Anything else, with what it printed. Progress is kept on disk and the
    /// same command carries on from there.
    Other(String),
}

static TOOL: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Whether this platform has a HatchZip build to unpack in place with.
pub fn supported() -> bool {
    PINNED.is_some()
}

/// Where the CLI lives once fetched, or `None` on a platform with no pinned
/// build, or while it cannot be fetched (the unpack then goes to 7-Zip).
pub async fn ensure<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    if let Some(p) = TOOL.get() {
        return Some(p.clone());
    }
    let pinned = PINNED?;
    let dir = app.path().app_data_dir().ok()?.join("tools");
    let file = dir.join(if cfg!(windows) { "hatchzip.exe" } else { "hatchzip" });
    let stamp = dir.join("hatchzip.version");
    if file.is_file() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(VERSION) {
        let _ = TOOL.set(file.clone());
        return Some(file);
    }
    let _ = std::fs::create_dir_all(&dir);
    match fetch(&pinned, &dir, &file).await {
        Ok(()) => {
            let _ = std::fs::write(&stamp, VERSION);
            let _ = TOOL.set(file.clone());
            crate::logging::info("hatchzip", &format!("{VERSION} ready at {}", file.display()));
            Some(file)
        }
        Err(e) => {
            crate::logging::warn("hatchzip", &format!("could not fetch {VERSION}: {e}; unpacking with 7-Zip"));
            None
        }
    }
}

async fn fetch(pinned: &Pinned, dir: &Path, file: &Path) -> Result<(), String> {
    let url = format!("https://github.com/xevrion/hatchzip/releases/download/{VERSION}/{}", pinned.asset);
    let res = crate::net::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("GitHub answered {}", res.status().as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let actual = sha256_hex(&bytes);
    if actual != pinned.sha256 {
        return Err(format!("{} is not the pinned build (sha256 {actual})", pinned.asset));
    }
    let archive = dir.join(pinned.asset);
    std::fs::write(&archive, &bytes).map_err(|e| e.to_string())?;
    let out = unpack_tool(&archive, dir, pinned.inner).inspect_err(|_| {
        let _ = std::fs::remove_file(&archive);
    })?;
    let _ = std::fs::remove_file(&archive);
    let tmp = file.with_extension("part");
    std::fs::rename(&out, &tmp).or_else(|_| std::fs::copy(&out, &tmp).map(|_| ())).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())?;
    // The folder the tarball unpacked into, if any.
    if let Some(top) = Path::new(pinned.inner).components().next() {
        let top = dir.join(top.as_os_str());
        if top.is_dir() {
            let _ = std::fs::remove_dir_all(top);
        }
    }
    Ok(())
}

/// The one file wanted out of the release asset, with the system's tar
/// (Windows' tar.exe reads zip; GNU and bsd tar read .tar.gz).
fn unpack_tool(archive: &Path, dir: &Path, inner: &str) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let tar = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\tar.exe");
    #[cfg(not(windows))]
    let tar = PathBuf::from("tar");
    let mut cmd = std::process::Command::new(tar);
    cmd.arg("-xf").arg(archive).arg("-C").arg(dir).arg(inner);
    crate::launch::clean_appimage_env(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("tar: {}", crate::logging::why(&e)))?;
    if !out.status.success() {
        return Err(format!("tar: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let path = dir.join(inner);
    path.is_file().then_some(path).ok_or_else(|| format!("{inner} was not in the asset"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn command(tool: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(tool);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    // Never the AppImage's own library paths for HatchZip (KB-0311).
    crate::launch::clean_appimage_env(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// `hatchzip check`: whether unpacking in place fits, and what it saves.
pub fn check(tool: &Path, archive: &Path, keep_free: &str) -> Result<Plan, Failure> {
    let out = command(tool).arg("check").arg("--keep-free").arg(keep_free).arg(archive).output().map_err(|e| Failure::Other(e.to_string()))?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    match out.status.code() {
        Some(0) => parse_plan(&text).ok_or_else(|| Failure::Other(format!("check said: {}", text.trim()))),
        Some(3) => Err(Failure::NoSpace),
        Some(4) => Err(Failure::Damaged(text.trim().to_string())),
        Some(5) => Err(Failure::Password),
        _ => Err(Failure::Other(text.trim().to_string())),
    }
}

/// The three numbers out of `check`'s report:
///
/// ```text
///   available                    30.6 GB
///   standard extraction needs    10.3 MB
///   required                     6.3 MB (+500.0 MB reserved)
/// ```
pub fn parse_plan(text: &str) -> Option<Plan> {
    let mut standard = None;
    let mut required = None;
    let mut available = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("standard extraction needs") {
            standard = parse_size(rest);
        } else if let Some(rest) = line.strip_prefix("required") {
            required = parse_size(rest);
        } else if let Some(rest) = line.strip_prefix("available after") {
            let _ = rest;
        } else if let Some(rest) = line.strip_prefix("available") {
            available = parse_size(rest);
        }
    }
    Some(Plan { standard: standard?, required: required?, available: available? })
}

/// `"6.3 MB (+500.0 MB reserved)"` → bytes of the first number. Decimal
/// units, as HatchZip prints them (10,292,302 bytes is "10.3 MB").
fn parse_size(text: &str) -> Option<u64> {
    let mut words = text.split_whitespace();
    let number: f64 = words.next()?.replace(',', "").parse().ok()?;
    let unit = words.next().unwrap_or("B").trim_end_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_uppercase();
    let scale: f64 = match unit.as_str() {
        "B" | "BYTES" => 1.0,
        "KB" | "KIB" => 1e3,
        "MB" | "MIB" => 1e6,
        "GB" | "GIB" => 1e9,
        "TB" | "TIB" => 1e12,
        _ => return None,
    };
    Some((number * scale).round() as u64)
}

/// HatchZip could not allocate what the archive needs: a block's LZMA
/// dictionary ("dictionary allocation too large", KB-0264), or a memory
/// limit. 7-Zip, one block at a time, takes over then.
pub fn out_of_memory(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["dictionary allocation too large", "needed memory too big", "not enough memory", "memory allocation", "maxmemlimited"].iter().any(|m| text.contains(m))
}

/* ── Memory ──────────────────────────────────────────────────────────────
 *
 * HatchZip decodes many solid blocks at once, one per core (its engine.rs
 * hands the blocks to rayon; v0.1.8 to v0.1.10 have no switch for it), and
 * every block being decoded holds its own LZMA dictionary. An archive packed
 * with a big dictionary on a PC with many cores and little free RAM then asks
 * for cores x dictionary at once, and the allocation fails part way through:
 * "dictionary allocation too large" (KB-0264 Baldur's Gate 3, KB-0295
 * Pragmata on Linux, Red Dead Redemption 2 twice on one PC on 2026-11-20,
 * KB-0329). 7-Zip decodes one block at a time, so it went on, but by then the
 * archive was partly eaten and the whole game had to be downloaded again.
 *
 * So the number of blocks decoded at once is set from the archive's biggest
 * dictionary and the memory free now (`RAYON_NUM_THREADS`, which rayon's
 * shared pool reads), and an archive whose one block will not fit is left to
 * 7-Zip before anything is eaten.
 */

/// The dictionary an LZMA or LZMA2 coder of a 7z needs, from its properties.
pub fn dictionary_of(method: &[u8], props: &[u8]) -> Option<u64> {
    match method {
        // LZMA2: one byte, 40 meaning 4 GiB - 1.
        [0x21] => match *props.first()? {
            b @ 0..=39 => Some(u64::from(2 | (b & 1)) << (b / 2 + 11)),
            40 => Some(0xFFFF_FFFF),
            _ => None,
        },
        // LZMA: lc/lp/pb, then the dictionary as four little-endian bytes.
        [3, 1, 1] => Some(u64::from(u32::from_le_bytes(props.get(1..5)?.try_into().ok()?))),
        _ => None,
    }
}

/// The biggest dictionary any block of `archive` (a .7z) needs.
pub fn largest_dictionary(archive: &Path) -> Option<u64> {
    let is_7z = archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z"));
    if !is_7z {
        return None;
    }
    let a = sevenz_rust::Archive::open(archive).ok()?;
    a.folders.iter().flat_map(|f| f.coders.iter()).filter_map(|c| dictionary_of(c.decompression_method_id(), &c.properties)).max()
}

/// What one block being decoded costs beyond its dictionary: the decoder's
/// own tables and buffers, and the file being written.
const PER_BLOCK: u64 = 64 << 20;

/// How many blocks may be decoded at once with `available` bytes of memory
/// free: what fits in 60% of it, at most `cores`. `None` when not even one
/// fits, and the unpack should be 7-Zip's from the start.
pub fn blocks_at_once(dictionary: u64, available: u64, cores: usize) -> Option<usize> {
    let fit = (available / 10 * 6) / (dictionary + PER_BLOCK);
    (fit >= 1).then(|| (fit as usize).min(cores.max(1)))
}

/// Memory free now, in bytes.
fn available_memory() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.available_memory()
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Whether unpacking `archive` in place fits this PC's free memory at all.
pub fn fits_in_memory(archive: &Path) -> bool {
    match largest_dictionary(archive) {
        Some(dict) => {
            let available = available_memory();
            let fits = blocks_at_once(dict, available, cores()).is_some();
            if !fits {
                crate::logging::info(
                    "hatchzip",
                    &format!("a block needs a {} dictionary and {} of memory is free; 7-Zip instead", crate::storage::human_size(dict), crate::storage::human_size(available)),
                );
            }
            fits
        }
        None => true,
    }
}

/// `RAYON_NUM_THREADS` for an unpack of `archive`, when fewer blocks than
/// cores fit in memory at once.
fn threads_for(archive: &Path) -> Option<usize> {
    let dict = largest_dictionary(archive)?;
    let cores = cores();
    let n = blocks_at_once(dict, available_memory(), cores).unwrap_or(1);
    (n < cores).then_some(n)
}

/// The two files HatchZip keeps beside an archive while an unpack is
/// unfinished (`game.7z.hatchzip`, `game.7z.hatchzip-tail`): the same
/// command carries on from them, so a stopped unpack is never started over.
pub fn resumes(archive: &Path) -> bool {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    archive.with_file_name(format!("{name}.hatchzip")).is_file()
}

/// Forget an unpack: its two files go. Left behind, the next download of the
/// same name read as one to carry on, and HatchZip then answers "done" with
/// nothing unpacked.
pub fn forget(archive: &Path) {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for side in ["hatchzip", "hatchzip-tail"] {
        let _ = std::fs::remove_file(archive.with_file_name(format!("{name}.{side}")));
    }
}

/// `hatchzip extract -y -q -o <dest>`: the archive is gone when this returns
/// Ok. The tool prints no progress in quiet mode, so it is read from disk:
/// every half second the archive's size (one `metadata` call; in place it is
/// cut short as it goes, so what is gone is done; a stored zip only shrinks
/// at the very end), and the size of `dest`, which walks every file written
/// so far and is what keeps the bar moving. The walk is throttled by how long
/// the last one took: a big game on a hard disk takes seconds to walk, and
/// walked twice a second it fought the unpack for the disk. Pause or Cancel
/// kills the tool; its own files keep where it got to.
///
/// A run that stops for space with part of the archive eaten is carried on at
/// once, as often as each run gets further (`stopped_for_space`).
///
/// `one_at_a_time` decodes one block at a time, as 7-Zip does: for a run
/// carrying on after one that ran out of memory. Otherwise as many blocks at
/// once as fit in the memory free (`threads_for`).
#[allow(clippy::too_many_arguments)]
pub fn extract(tool: &Path, archive: &Path, dest: &Path, keep_free: &str, total: Option<u64>, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8, one_at_a_time: bool) -> Result<(), Failure> {
    let threads = if one_at_a_time { Some(1) } else { threads_for(archive) };
    if let Some(n) = threads {
        crate::logging::info("hatchzip", &format!("decoding {n} block(s) at a time to stay within this PC's free memory"));
    }
    let size = || std::fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    // What the archive was when this unpack began, for the bar across runs.
    let whole = size();
    // The bar never goes back when a run carries on from the last one.
    let high = std::cell::Cell::new(0u64);
    let forward = |done: u64, total: Option<u64>| {
        if done >= high.get() {
            high.set(done);
            progress(done, total);
        }
    };
    let mut runs = 1u32;
    let mut busy = 0u32;
    loop {
        let before = size();
        match extract_once(tool, archive, dest, keep_free, total, whole, &forward, stop, threads) {
            Err(Failure::Other(text)) if stopped_for_space(&text) && resumes(archive) && size() < before => {
                crate::logging::info("hatchzip", &format!("run {runs} stopped for space with {} of the archive freed; carrying on ({})", crate::storage::human_size(before - size()), first_line(&text)));
                runs += 1;
            }
            // Held open by something else for a moment: an antivirus reading
            // the archive that has just landed, or Windows' search indexer.
            // Each ended the unpack with "Press Retry", which was all it took
            // (KB-0338); now it waits and tries again by itself, for a minute.
            Err(Failure::Other(text)) if held_open(&text) && busy < 12 => {
                busy += 1;
                if busy == 1 {
                    crate::logging::info("hatchzip", &format!("the archive is held open by another program; trying again for a minute ({})", first_line(&text)));
                }
                for _ in 0..50 {
                    if stop.load(Ordering::SeqCst) != crate::downloads::RUN {
                        return Err(Failure::Stopped);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            done => return done,
        }
    }
}

/// The archive is open in another program, or another HatchZip.
pub fn held_open(text: &str) -> bool {
    text.to_ascii_lowercase().contains("is in use by another program")
}

/// HatchZip will not unpack this archive at all, and said so before changing
/// anything: an entry whose name it refuses on this system, such as an NTFS
/// stream (`forzahorizon6.exe:4`, Forza Horizon 6), which it reads as a path
/// leaving the folder (KB-0338). 7-Zip unpacks those, so it takes over.
pub fn refused(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("would be written outside the destination folder") || t.contains("extraction refused") || t.contains("refused before anything changes")
}

/// The archive cannot be read as it is: damaged on the way, or cut short by an
/// unpack in place whose progress files are gone. HatchZip says it plainly;
/// only a fresh copy helps, and "Press Retry to carry on" sent players round
/// the same failure (KB-0338).
pub fn unreadable(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("not a readable") || t.contains("checksumverificationfailed") || t.contains("next header offset out of range") || t.contains("corrupted input data")
}

/// HatchZip stopped mid-unpack saying the drive is full, with its progress
/// saved. Through 0.1.10 that is usually not true on Windows: NTFS keeps the
/// part of the archive it cut off allocated until HatchZip's handle on it
/// closes, so its own look at the free space between batches misses what it
/// just freed, and it stops on a drive with room (xevrion/hatchzip#79; fixed
/// there after 0.1.10, in no release yet). The same command then carries on,
/// which is all Retry did: ten PCs in Desktop 0.3.10's first day stopped this
/// way, some three times in one game (KB-0329). A run that freed nothing is a
/// drive that really is full, and is not tried again.
pub fn stopped_for_space(text: &str) -> bool {
    text.to_ascii_lowercase().contains("not enough disk space to continue")
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no details").chars().take(300).collect()
}

/// One run of `hatchzip extract`. `whole` is the archive's size when the
/// unpack began, which a run carrying on from another no longer sees.
#[allow(clippy::too_many_arguments)]
fn extract_once(tool: &Path, archive: &Path, dest: &Path, keep_free: &str, total: Option<u64>, whole: u64, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8, threads: Option<usize>) -> Result<(), Failure> {
    use std::io::Read;
    let mut cmd = command(tool);
    if let Some(n) = threads {
        cmd.env("RAYON_NUM_THREADS", n.to_string());
    }
    let mut child = cmd
        .arg("extract")
        .arg("-y")
        .arg("-q")
        .arg("--keep-free")
        .arg(keep_free)
        .arg("-o")
        .arg(dest)
        .arg(archive)
        .spawn()
        .map_err(|e| Failure::Other(e.to_string()))?;
    // Full speed whichever window is in front (KB-0306).
    crate::downloads::full_speed_child(&child);
    let mut stderr = child.stderr.take();
    let mut stdout = child.stdout.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut text);
        }
        if let Some(o) = stdout.as_mut() {
            let mut out = String::new();
            let _ = o.read_to_string(&mut out);
            if !out.trim().is_empty() {
                text.push('\n');
                text.push_str(&out);
            }
        }
        text
    });
    progress(0, total);
    let archive_size = whole;
    let mut last = Instant::now();
    // The last walk of `dest`: when it ended and how long it took. The next
    // one waits ten times that (at least the half second of the tick), so a
    // walk that takes seconds on a big game never has more than a tenth of
    // the time, while a quick one keeps the bar moving twice a second.
    let mut walk: Option<(Instant, Duration)> = None;
    // Never reported lower than before: the two readings measure different
    // things (compressed bytes gone, bytes written) and would otherwise jitter.
    let mut shown = 0u64;
    let mut report = |done: u64, total: Option<u64>| {
        let done = match total {
            Some(t) => done.min(t),
            None => done,
        };
        if done > shown {
            shown = done;
            progress(done, total);
        }
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(e),
        }
        if stop.load(Ordering::SeqCst) != crate::downloads::RUN {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::Stopped);
        }
        if last.elapsed() >= Duration::from_millis(500) {
            last = Instant::now();
            // What the archive has given up so far, scaled to the unpacked total.
            if archive_size > 0 {
                let left = std::fs::metadata(archive).map(|m| m.len()).unwrap_or(archive_size);
                let consumed = archive_size.saturating_sub(left);
                match total {
                    Some(t) => report(((consumed as u128 * t as u128) / archive_size as u128) as u64, Some(t)),
                    None => report(consumed, None),
                }
            }
            let due = walk.is_none_or(|(at, took)| at.elapsed() >= (took * 10).max(Duration::from_millis(500)));
            if due {
                let started = Instant::now();
                let size = crate::library::dir_size(dest);
                walk = Some((Instant::now(), started.elapsed()));
                report(size, total);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let text = errors.join().unwrap_or_default().trim().to_string();
    match status.map(|s| s.code()) {
        Ok(Some(0)) => {
            if let Some(t) = total {
                progress(t, Some(t));
            }
            Ok(())
        }
        Ok(Some(3)) => Err(Failure::NoSpace),
        Ok(Some(4)) => Err(Failure::Damaged(text)),
        Ok(Some(5)) => Err(Failure::Password),
        Ok(Some(130)) => Err(Failure::Stopped),
        Ok(_) => Err(Failure::Other(text)),
        Err(e) => Err(Failure::Other(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECK: &str = "test-blocks.7z  10.3 MB 7z, 10.3 MB uncompressed in 4 entries
  available                    30.6 GB
  standard extraction needs    10.3 MB
  required                     6.3 MB (+500.0 MB reserved)
  available after              30.6 GB
  note: there is also enough space to keep the archive (--keep-archive)";

    /// KB-0264: these hand the unpack to 7-Zip; a damaged archive does not.
    #[test]
    fn running_out_of_memory_is_told_apart() {
        assert!(out_of_memory("error: extracting Baldur's Gate 3\\Baldur's Gate 3\\Data\\Assets.pak: dictionary allocation too large"));
        assert!(out_of_memory("needed memory too big for mem_limit_kb"));
        assert!(!out_of_memory("error: extracting Game.pck: corrupted input data (LZMA2:3)"));
        assert!(!out_of_memory(""));
    }

    /// KB-0329: HatchZip's own words when it stops for space mid-unpack.
    #[test]
    fn a_stop_for_space_is_told_apart() {
        assert!(stopped_for_space(
            "error: not enough disk space to continue: 'paks/chunk1.pak' requires 419.4 MB and 390.3 MB is available (1.3 TB reserved).\n29.1 MB more is required. Progress is saved; free up space and resume the extraction."
        ));
        assert!(!stopped_for_space("error: extracting Game.pck: corrupted input data (LZMA2:3)"));
        assert!(!stopped_for_space("error: An Application Control policy has blocked this file. (os error 4551)"));
    }

    /// KB-0338: what HatchZip printed for each of these in Desktop 0.3.10's reports.
    #[test]
    fn its_other_failures_are_told_apart() {
        let refusal = "'FH6/Forza Horizon 6/forzahorizon6.exe:4' would be written outside the destination folder; extraction refused";
        assert!(refused(refusal) && !unreadable(refusal) && !held_open(refusal));
        for damaged in [
            "not a readable 7z archive: Other(\"next header offset out of range\")",
            "extracting 'End/Content/Movie/MV_TITLB_0010.emov': ChecksumVerificationFailed",
        ] {
            assert!(unreadable(damaged) && !refused(damaged), "{damaged}");
        }
        let busy = "~/Downloads/_downloads/Project Exhibitionist - Kryoto.7z is in use by another program or HatchZip process";
        assert!(held_open(busy) && !refused(busy) && !unreadable(busy));
        assert!(!refused("dictionary allocation too large") && !unreadable("not enough disk space to continue"));
    }

    /// KB-0329/KB-0295: a dictionary read from the 7z's own coder properties,
    /// and as many blocks at once as fit in the memory free.
    #[test]
    fn blocks_decoded_at_once_fit_the_memory_free() {
        // LZMA2 property bytes as 7-Zip writes them: 16 = 1 MiB, 24 = 16 MiB, 40 = 4 GiB - 1.
        assert_eq!(dictionary_of(&[0x21], &[16]), Some(1 << 20));
        assert_eq!(dictionary_of(&[0x21], &[17]), Some(3 << 19));
        assert_eq!(dictionary_of(&[0x21], &[24]), Some(16 << 20));
        assert_eq!(dictionary_of(&[0x21], &[40]), Some(0xFFFF_FFFF));
        assert_eq!(dictionary_of(&[0x21], &[41]), None);
        // LZMA: 64 MiB as four little-endian bytes after lc/lp/pb.
        assert_eq!(dictionary_of(&[3, 1, 1], &[0x5d, 0, 0, 0, 4]), Some(64 << 20));
        assert_eq!(dictionary_of(&[4, 1, 8], &[]), None);
        let gb = 1u64 << 30;
        // A 1.5 GB dictionary on 16 cores with 8 GB free: three at a time, not sixteen.
        assert_eq!(blocks_at_once(3 * gb / 2, 8 * gb, 16), Some(3));
        // Forge's default level (a small dictionary): every core.
        assert_eq!(blocks_at_once(1 << 20, 8 * gb, 16), Some(16));
        // Not even one block fits: 7-Zip from the start, before anything is eaten.
        assert_eq!(blocks_at_once(4 * gb, 2 * gb, 8), None);
        assert_eq!(blocks_at_once(gb, 4 * gb, 0), Some(1));
    }

    #[test]
    fn reads_the_space_plan_out_of_check() {
        let plan = parse_plan(CHECK).unwrap();
        assert_eq!(plan, Plan { standard: 10_300_000, required: 6_300_000, available: 30_600_000_000 });
        assert!(plan.gains());
        // A solid single-block 7z: nothing can go until everything is out.
        let solid = parse_plan(&CHECK.replace("required                     6.3 MB", "required                     10.3 MB")).unwrap();
        assert!(!solid.gains());
        assert_eq!(parse_plan("error: reading x: no such file"), None);
    }

    #[test]
    fn sizes_are_decimal_as_printed() {
        assert_eq!(parse_size("500.0 MB reserved"), Some(500_000_000));
        assert_eq!(parse_size("1.5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("12 B"), Some(12));
        assert_eq!(parse_size("ten MB"), None);
    }

    /// The pinned build for this platform, fetched and checked, then a real
    /// archive unpacked in place with it. Network and a 7z/zip tool needed:
    /// `KRYOTO_HZ_ARCHIVE=<a .7z or .zip to copy and unpack> cargo test --lib hatchzip_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn hatchzip_live() {
        let Some(pinned) = PINNED else { return };
        let dir = std::env::temp_dir().join(format!("kryoto-hz-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join(if cfg!(windows) { "hatchzip.exe" } else { "hatchzip" });
        tauri::async_runtime::block_on(fetch(&pinned, &dir, &tool)).expect("pinned build fetched and its hash matched");
        let source = std::path::PathBuf::from(std::env::var("KRYOTO_HZ_ARCHIVE").expect("KRYOTO_HZ_ARCHIVE"));
        let archive = dir.join(source.file_name().unwrap());
        std::fs::copy(&source, &archive).unwrap();
        let plan = check(&tool, &archive, "100MB").expect("check");
        println!("plan: {plan:?} gains={}", plan.gains());
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let stop = AtomicU8::new(crate::downloads::RUN);
        extract(&tool, &archive, &out, "100MB", Some(plan.standard), &|d, t| println!("progress {d}/{t:?}"), &stop, false).expect("extract");
        assert!(!archive.exists(), "the archive is eaten as it unpacks");
        let size = crate::library::dir_size(&out);
        println!("unpacked {size} bytes into {}", out.display());
        assert!(size > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_files_sit_beside_the_archive() {
        let dir = std::env::temp_dir().join(format!("kryoto-hz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("Game - kryoto.7z");
        assert!(!resumes(&archive));
        std::fs::write(dir.join("Game - kryoto.7z.hatchzip"), b"").unwrap();
        assert!(resumes(&archive));
        let _ = std::fs::remove_dir_all(dir);
    }
}
