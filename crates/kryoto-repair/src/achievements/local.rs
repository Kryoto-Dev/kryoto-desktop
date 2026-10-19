//! Portable native-game achievements. No Steam identity or network lookup.
use super::{Achievement, Unlock};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST: &str = ".kryoto-achievements.json";
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Format {
    TrophyLog,
    GoldbergJson,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    format: Format,
    unlock_file: String,
    achievements: Vec<Achievement>,
}

#[derive(Clone)]
pub struct LocalGame {
    root: PathBuf,
    pub file: PathBuf,
    format: Format,
    pub list: Vec<Achievement>,
}

fn bounded(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

pub fn load(root: &Path) -> Option<LocalGame> {
    let root = root.canonicalize().ok()?;
    let manifest_path = root.join(MANIFEST);
    if !manifest_path.canonicalize().ok()?.starts_with(&root) {
        return None;
    }
    let m: Manifest = serde_json::from_slice(&bounded(&manifest_path)?).ok()?;
    let rel = Path::new(&m.unlock_file);
    if m.version != 1
        || m.achievements.is_empty()
        || m.achievements.len() > 512
        || m.unlock_file.contains([':', '\\'])
        || rel.components().next().is_none()
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let mut names = HashSet::new();
    for a in &m.achievements {
        if a.name.is_empty()
            || a.name.len() > 128
            || a.display_name.len() > 512
            || a.description.len() > 4096
            || !names.insert(a.name.clone())
        {
            return None;
        }
        if matches!(m.format, Format::TrophyLog) && a.name.parse::<u32>().is_err() {
            return None;
        }
        // Icons are remote web images, never file URLs or executable protocols.
        if [&a.icon, &a.icon_gray]
            .into_iter()
            .flatten()
            .any(|i| !i.starts_with("https://"))
        {
            return None;
        }
    }
    Some(LocalGame {
        file: root.join(rel),
        root,
        format: m.format,
        list: m.achievements,
    })
}

impl LocalGame {
    pub fn unlocks(&self) -> Vec<Unlock> {
        // Recheck canonical containment on every read, including symlinked parents.
        let Ok(real) = self.file.canonicalize() else {
            return Vec::new();
        };
        if !real.starts_with(&self.root) {
            return Vec::new();
        }
        let Some(bytes) = bounded(&real) else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut result: Vec<Unlock> = match self.format {
            Format::GoldbergJson => super::parse_goldberg(&text),
            Format::TrophyLog => {
                let mut found: Vec<Unlock> = Vec::new();
                for line in text.lines() {
                    let mut parts = line.split_whitespace();
                    let (Some(id), Some(time)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let (Ok(id), Ok(time)) = (id.parse::<u32>(), time.parse::<u64>()) else {
                        continue;
                    };
                    if time == 0 || parts.next().is_some() {
                        continue;
                    }
                    let name = id.to_string();
                    if let Some(old) = found.iter_mut().find(|u| u.name == name) {
                        old.time = Some(old.time.unwrap_or(time).min(time));
                    } else {
                        found.push(Unlock {
                            name,
                            time: Some(time),
                        });
                    }
                }
                found
            }
        };
        result.retain(|u| self.list.iter().any(|a| a.name == u.name));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (TempDir, serde_json::Value) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kryoto-native-ach-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let d = TempDir(path);
        let m = serde_json::json!({"version":1,"format":"trophy_log","unlockFile":"user/trophies.log",
            "achievements":[{"name":"14","displayName":"Father Gascoigne","description":"Defeat the beast.",
            "hidden":false,"icon":null,"iconGray":null,"percent":null}]});
        (d, m)
    }
    fn write(d: &Path, m: &serde_json::Value) {
        std::fs::write(d.join(MANIFEST), m.to_string()).unwrap();
    }
    #[test]
    fn native_trophies_load_offline_and_preserve_earliest_time() {
        let (d, m) = setup();
        write(d.path(), &m);
        let game = load(d.path()).unwrap();
        assert!(game.unlocks().is_empty());
        std::fs::create_dir(d.path().join("user")).unwrap();
        std::fs::write(&game.file, "14 20\n14 10\n999 5\nbad\n14 invalid\n").unwrap();
        assert_eq!(
            game.unlocks(),
            vec![Unlock {
                name: "14".into(),
                time: Some(10)
            }]
        );
        assert_eq!(game.list[0].display_name, "Father Gascoigne");
    }
    #[test]
    fn invalid_paths_versions_duplicate_ids_and_protocols_are_rejected() {
        let (d, original) = setup();
        for path in [
            "../outside",
            "/outside",
            "C:/outside",
            "user/../outside",
            "user\\outside",
            "",
        ] {
            let mut m = original.clone();
            m["unlockFile"] = path.into();
            write(d.path(), &m);
            assert!(load(d.path()).is_none(), "{path}");
        }
        let mut m = original.clone();
        m["version"] = 2.into();
        write(d.path(), &m);
        assert!(load(d.path()).is_none());
        let mut m = original.clone();
        m["achievements"]
            .as_array_mut()
            .unwrap()
            .push(original["achievements"][0].clone());
        write(d.path(), &m);
        assert!(load(d.path()).is_none());
        let mut m = original;
        m["achievements"][0]["icon"] = "file:///outside".into();
        write(d.path(), &m);
        assert!(load(d.path()).is_none());
    }
}
