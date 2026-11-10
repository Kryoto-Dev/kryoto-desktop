//! The Discord client's local socket: a named pipe on Windows, a Unix socket
//! elsewhere, numbered 0 to 9. Frames are an opcode and a length, little
//! endian, then JSON. Nothing here goes over the network.

use serde_json::Value;
#[cfg(any(unix, test))]
use std::path::{Path, PathBuf};

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
const MAX_FRAME: usize = 1 << 20;

#[cfg(windows)]
pub type Pipe = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
pub type Pipe = tokio::net::UnixStream;

pub fn frame(op: u32, body: &Value) -> Vec<u8> {
    let json = body.to_string().into_bytes();
    let mut out = Vec::with_capacity(json.len() + 8);
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out
}

/// Bytes as they arrive, frames as they complete.
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame. `Err` is a stream that makes no sense.
    pub fn next(&mut self) -> Result<Option<(u32, Value)>, ()> {
        if self.buf.len() < 8 {
            return Ok(None);
        }
        let op = u32::from_le_bytes(self.buf[0..4].try_into().map_err(|_| ())?);
        let len = u32::from_le_bytes(self.buf[4..8].try_into().map_err(|_| ())?) as usize;
        if len > MAX_FRAME {
            return Err(());
        }
        if self.buf.len() < 8 + len {
            return Ok(None);
        }
        let body = serde_json::from_slice(&self.buf[8..8 + len]).unwrap_or(Value::Null);
        self.buf.drain(..8 + len);
        Ok(Some((op, body)))
    }
}

/// Where a Unix Discord may have its socket: the plain client, Flatpak and
/// Snap, and the common other clients. Pure, for the tests.
#[cfg(any(unix, test))]
pub fn unix_dirs(runtime: Option<&Path>, tmp: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(run) = runtime {
        out.push(run.to_path_buf());
        for app in ["com.discordapp.Discord", "com.discordapp.DiscordCanary", "com.discordapp.DiscordPTB", "dev.vencord.Vesktop"] {
            out.push(run.join("app").join(app));
        }
        out.push(run.join(".flatpak/dev.vencord.Vesktop/xdg-run"));
        for snap in ["snap.discord", "snap.discord-canary"] {
            out.push(run.join(snap));
        }
    }
    out.extend(tmp.iter().cloned());
    out.push(PathBuf::from("/tmp"));
    let mut seen = Vec::new();
    out.retain(|p| {
        let new = !seen.contains(p);
        seen.push(p.clone());
        new
    });
    out
}

#[cfg(unix)]
pub async fn open() -> Option<Pipe> {
    let var = |n: &str| std::env::var_os(n).filter(|v| !v.is_empty()).map(PathBuf::from);
    let tmp: Vec<PathBuf> = ["TMPDIR", "TMP", "TEMP"].into_iter().filter_map(var).collect();
    for dir in unix_dirs(var("XDG_RUNTIME_DIR").as_deref(), &tmp) {
        for n in 0..10 {
            if let Ok(pipe) = tokio::net::UnixStream::connect(dir.join(format!("discord-ipc-{n}"))).await {
                return Some(pipe);
            }
        }
    }
    None
}

#[cfg(windows)]
pub async fn open() -> Option<Pipe> {
    // Debug builds only: a stand-in, so a test never touches the Discord on this PC.
    #[cfg(debug_assertions)]
    let name = std::env::var("KRYOTO_DEV_DISCORD_PIPE").unwrap_or_else(|_| "discord-ipc".into());
    #[cfg(not(debug_assertions))]
    let name = "discord-ipc";
    (0..10).find_map(|n| tokio::net::windows::named_pipe::ClientOptions::new().open(format!(r"\\.\pipe\{name}-{n}")).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_survive_arriving_in_pieces() {
        let mut f = Frames::default();
        let a = frame(OP_FRAME, &json!({ "evt": "READY" }));
        let b = frame(OP_CLOSE, &json!({ "code": 4000 }));
        let bytes = [a, b].concat();
        f.push(&bytes[..5]);
        assert_eq!(f.next(), Ok(None));
        f.push(&bytes[5..]);
        assert_eq!(f.next(), Ok(Some((OP_FRAME, json!({ "evt": "READY" })))));
        assert_eq!(f.next(), Ok(Some((OP_CLOSE, json!({ "code": 4000 })))));
        assert_eq!(f.next(), Ok(None));
    }

    #[test]
    fn a_length_that_makes_no_sense_ends_it() {
        let mut f = Frames::default();
        f.push(&[1, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(f.next(), Err(()));
    }

    #[test]
    fn flatpak_and_snap_discord_are_looked_for() {
        let dirs = unix_dirs(Some(Path::new("/run/user/1000")), &[PathBuf::from("/tmp")]);
        for want in ["/run/user/1000", "/run/user/1000/app/com.discordapp.Discord", "/run/user/1000/snap.discord", "/tmp"] {
            assert!(dirs.contains(&PathBuf::from(want)), "{want}");
        }
        assert_eq!(dirs.iter().filter(|d| d.as_path() == Path::new("/tmp")).count(), 1);
        assert_eq!(unix_dirs(None, &[]), vec![PathBuf::from("/tmp")]);
    }
}
