//! Known save locations from the Ludusavi manifest
//! (github.com/mtkennerly/ludusavi-manifest), which is compiled from
//! PCGamingWiki: about twenty thousand games and where each one saves.
//!
//! Kryoto Desktop downloads the manifest itself, the way Ludusavi does, and
//! keeps a small index of it ([`KnownIndex`]): by Steam AppID, and by title for
//! games without one. Only Windows save paths and `HKEY_CURRENT_USER` keys are
//! kept, with the manifest's placeholders turned into [`Root`]s.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::rules::{squash, Root, Rule, Source};

pub const MANIFEST_URL: &str = "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

#[derive(Deserialize, Default)]
#[serde(default)]
struct Game {
    files: BTreeMap<String, Constraint>,
    registry: BTreeMap<String, Constraint>,
    steam: Steam,
    #[serde(rename = "installDir")]
    install_dir: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Steam {
    id: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Constraint {
    tags: Vec<String>,
    when: Vec<When>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct When {
    os: Option<String>,
}

impl Constraint {
    /// A save (or untagged) entry that applies on Windows.
    fn wanted(&self) -> bool {
        let save = self.tags.is_empty() || self.tags.iter().any(|t| t == "save");
        let windows = self.when.is_empty() || self.when.iter().any(|w| w.os.as_deref().is_none_or(|os| os == "windows" || os == "dos"));
        save && windows
    }
}

/// The index Kryoto Desktop keeps of the manifest.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct KnownIndex {
    /// When it was built (Unix seconds) and the manifest's ETag, for refreshing.
    pub built_at: u64,
    pub etag: Option<String>,
    pub by_appid: HashMap<String, Vec<Rule>>,
    pub by_title: HashMap<String, Vec<Rule>>,
}

impl KnownIndex {
    /// The known locations for a game, by AppID first, then by its title.
    pub fn rules_for(&self, appid: Option<&str>, title: &str) -> Vec<Rule> {
        if let Some(r) = appid.and_then(|a| self.by_appid.get(a)) {
            return r.clone();
        }
        self.by_title.get(&squash(title)).cloned().unwrap_or_default()
    }

    /// Build the index from the manifest's YAML.
    pub fn build(yaml: &str, etag: Option<String>, now: u64) -> Result<KnownIndex, String> {
        let games: BTreeMap<String, Game> = serde_yaml::from_str(yaml).map_err(|e| format!("The save database could not be read: {e}"))?;
        let mut index = KnownIndex { built_at: now, etag, ..Default::default() };
        for (name, game) in games {
            let install = game.install_dir.keys().next().cloned();
            let appid = game.steam.id.map(|i| i.to_string());
            let mut rules = Vec::new();
            for (path, c) in &game.files {
                if !c.wanted() {
                    continue;
                }
                if let Some((root, rel)) = map_path(path, install.as_deref(), appid.as_deref()) {
                    let rule = Rule::files(root, rel, Source::Known);
                    if rule.problem().is_none() && !rules.contains(&rule) {
                        rules.push(rule);
                    }
                }
            }
            for (key, c) in &game.registry {
                if !c.wanted() {
                    continue;
                }
                let key = key.replace('\\', "/");
                let Some(rest) = key.strip_prefix("HKEY_CURRENT_USER/") else { continue };
                if rest.contains('<') || rest.contains('*') {
                    continue;
                }
                let rule = Rule::registry(rest, Source::Known);
                if rule.problem().is_none() && !rules.contains(&rule) {
                    rules.push(rule);
                }
            }
            if rules.is_empty() {
                continue;
            }
            match appid {
                Some(id) => {
                    index.by_appid.entry(id).or_default().extend(rules);
                }
                None => {
                    index.by_title.entry(squash(&name)).or_default().extend(rules);
                }
            }
        }
        Ok(index)
    }
}

/// A manifest path as a root and a relative path, or None when it is not a
/// Windows location this can resolve.
pub fn map_path(path: &str, install_dir: Option<&str>, appid: Option<&str>) -> Option<(Root, String)> {
    let p = path.replace('\\', "/");
    let strip = |prefix: &str| p.strip_prefix(prefix).map(|r| r.trim_start_matches('/').to_string());
    let (root, rest) = if let Some(r) = strip("<base>") {
        (Root::GameDir, r)
    } else if let Some(r) = strip("<root>/<game>") {
        (Root::GameDir, r)
    } else if let Some(r) = strip("<winLocalAppData>/VirtualStore") {
        (Root::VirtualStore, r)
    } else if let Some(r) = strip("<winLocalAppDataLow>") {
        (Root::LocalLow, r)
    } else if let Some(r) = strip("<winLocalAppData>") {
        (Root::LocalAppData, r)
    } else if let Some(r) = strip("<winAppData>") {
        (Root::AppData, r)
    } else if let Some(r) = strip("<winDocuments>") {
        (Root::Documents, r)
    } else if let Some(r) = strip("<winPublic>/Documents") {
        (Root::PublicDocuments, r)
    } else if let Some(r) = strip("<winProgramData>") {
        (Root::ProgramData, r)
    } else if let Some(r) = strip("<home>/AppData/Roaming") {
        (Root::AppData, r)
    } else if let Some(r) = strip("<home>/AppData/LocalLow") {
        (Root::LocalLow, r)
    } else if let Some(r) = strip("<home>/AppData/Local/VirtualStore") {
        (Root::VirtualStore, r)
    } else if let Some(r) = strip("<home>/AppData/Local") {
        (Root::LocalAppData, r)
    } else if let Some(r) = strip("<home>/Documents") {
        (Root::Documents, r)
    } else if let Some(r) = strip("<home>/Saved Games") {
        (Root::SavedGames, r)
    } else if let Some(r) = strip("<home>") {
        (Root::Home, r)
    } else {
        return None;
    };
    let mut rest = rest
        .replace("<storeUserId>", "*")
        .replace("<osUserName>", "*")
        .replace("<storeGameId>", appid.unwrap_or("*"))
        .replace("<game>", install_dir.unwrap_or("*"));
    // A whole tree under a folder is the folder.
    while let Some(r) = rest.strip_suffix("/**").or_else(|| rest.strip_suffix("/*")).map(str::to_string) {
        rest = r;
    }
    if rest.contains('<') || rest.contains('[') || rest.contains('{') || rest.contains("**") {
        return None;
    }
    let rest = crate::rules::normalize(&rest);
    // The Microsoft Store / Xbox app's copy of a game keeps its saves in its
    // package; the builds Kryoto installs never do.
    if rest.to_lowercase().starts_with("packages/") || rest.to_lowercase().contains("/systemappdata/wgs") {
        return None;
    }
    // Only wildcards left, or nothing: that is the whole root.
    if rest.split('/').all(|s| s.chars().all(|c| c == '*')) {
        return None;
    }
    Some((root, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Hollow Knight:
  files:
    <winLocalAppDataLow>/Team Cherry/Hollow Knight/*.dat:
      tags:
        - save
      when:
        - os: windows
    <home>/Library/Application Support/unity.Team Cherry.Hollow Knight:
      when:
        - os: mac
    <winLocalAppDataLow>/Team Cherry/Hollow Knight/*.log:
      tags:
        - config
  installDir:
    Hollow Knight: {}
  registry:
    HKEY_CURRENT_USER/Software/Team Cherry/Hollow Knight:
      tags:
        - config
  steam:
    id: 367520
Some Old Game:
  files:
    <base>/SAVES:
      tags:
        - save
    <root>/<game>/Profiles/<storeUserId>:
      tags:
        - save
    <winDir>/game.ini: {}
  installDir:
    OldGame: {}
  registry:
    HKEY_CURRENT_USER/Software/Old Co/Old Game:
      tags:
        - save
"#;

    #[test]
    fn indexes_windows_saves_by_appid_and_title() {
        let idx = KnownIndex::build(SAMPLE, None, 1).unwrap();
        assert_eq!(idx.rules_for(Some("367520"), ""), vec![Rule::files(Root::LocalLow, "Team Cherry/Hollow Knight/*.dat", Source::Known)]);
        let old = idx.rules_for(None, "Some old game!");
        assert_eq!(
            old,
            vec![
                Rule::files(Root::GameDir, "SAVES", Source::Known),
                // Every player's folder under it, not one id's.
                Rule::files(Root::GameDir, "Profiles", Source::Known),
                Rule::registry("Software/Old Co/Old Game", Source::Known),
            ]
        );
        assert!(idx.rules_for(Some("1"), "Nothing").is_empty());
    }

    /// The real manifest. Opt-in: `KRYOTO_LUDUSAVI=<manifest.yaml> cargo test
    /// real_manifest -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_manifest() {
        let path = std::env::var("KRYOTO_LUDUSAVI").expect("KRYOTO_LUDUSAVI");
        let yaml = std::fs::read_to_string(path).unwrap();
        let t = std::time::Instant::now();
        let idx = KnownIndex::build(&yaml, None, 0).unwrap();
        println!("{} by appid, {} by title, in {:?}", idx.by_appid.len(), idx.by_title.len(), t.elapsed());
        println!("Hollow Knight: {:?}", idx.rules_for(Some("367520"), ""));
        println!("Skyrim SE: {:?}", idx.rules_for(Some("489830"), ""));
        println!("Celeste: {:?}", idx.rules_for(Some("504230"), ""));
        println!("index json {} bytes", serde_json::to_vec(&idx).unwrap().len());
        assert!(idx.by_appid.len() > 5000);
    }

    #[test]
    fn maps_placeholders_to_roots() {
        assert_eq!(map_path("<winDocuments>/My Games/Skyrim/Saves", None, None), Some((Root::Documents, "My Games/Skyrim/Saves".into())));
        assert_eq!(map_path("<home>/Saved Games/Crytek/**", None, None), Some((Root::SavedGames, "Crytek".into())));
        assert_eq!(map_path("<winPublic>/Documents/Steam/CODEX/<storeGameId>", None, Some("70")), Some((Root::PublicDocuments, "Steam/CODEX/70".into())));
        assert_eq!(map_path("<winLocalAppData>/VirtualStore/Program Files/X", None, None), Some((Root::VirtualStore, "Program Files/X".into())));
        assert_eq!(map_path("<base>", None, None), None);
        assert_eq!(map_path("<winDocuments>/*", None, None), None);
        assert_eq!(map_path("<winDir>/x.ini", None, None), None);
        assert_eq!(map_path("<xdgConfig>/x", None, None), None);
        assert_eq!(map_path("<winAppData>/a/**/b", None, None), None);
        assert_eq!(map_path("<winLocalAppData>/Packages/Studio.Game_x/SystemAppData/wgs", None, None), None);
    }
}
