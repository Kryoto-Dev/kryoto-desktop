//! Saves kept in the registry (Unity's PlayerPrefs and older games), under
//! `HKEY_CURRENT_USER\Software` only.

use base64::Engine;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::{RegKey, RegValue as WinValue};

use crate::bundle::RegValue;
use crate::rules::{Kind, Rule};

/// At most this many values, and this many bytes of them, per game.
pub const MAX_VALUES: usize = 4000;
pub const MAX_BYTES: usize = 4 << 20;

fn win_path(path: &str) -> String {
    path.replace('/', "\\")
}

fn safe(path: &str) -> bool {
    path.to_ascii_lowercase().starts_with("software/") && !path.split('/').any(|s| s.is_empty() || s == "." || s == "..")
}

fn kind_code(t: &winreg::enums::RegType) -> u32 {
    use winreg::enums::RegType::*;
    match t {
        REG_NONE => 0,
        REG_SZ => 1,
        REG_EXPAND_SZ => 2,
        REG_BINARY => 3,
        REG_DWORD => 4,
        REG_DWORD_BIG_ENDIAN => 5,
        REG_LINK => 6,
        REG_MULTI_SZ => 7,
        REG_RESOURCE_LIST => 8,
        REG_FULL_RESOURCE_DESCRIPTOR => 9,
        REG_RESOURCE_REQUIREMENTS_LIST => 10,
        REG_QWORD => 11,
    }
}

fn kind_of(code: u32) -> Option<winreg::enums::RegType> {
    use winreg::enums::RegType::*;
    Some(match code {
        0 => REG_NONE,
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        3 => REG_BINARY,
        4 => REG_DWORD,
        5 => REG_DWORD_BIG_ENDIAN,
        7 => REG_MULTI_SZ,
        11 => REG_QWORD,
        _ => return None,
    })
}

fn read_key(key: &RegKey, path: &str, out: &mut Vec<RegValue>, bytes: &mut usize, depth: usize) {
    for (name, value) in key.enum_values().flatten() {
        if out.len() >= MAX_VALUES || *bytes + value.bytes.len() > MAX_BYTES {
            return;
        }
        *bytes += value.bytes.len();
        out.push(RegValue { path: path.to_string(), name, kind: kind_code(&value.vtype), data: base64::engine::general_purpose::STANDARD.encode(&value.bytes) });
    }
    if depth == 0 {
        return;
    }
    for sub in key.enum_keys().flatten() {
        if let Ok(child) = key.open_subkey_with_flags(&sub, KEY_READ) {
            read_key(&child, &format!("{path}/{sub}"), out, bytes, depth - 1);
        }
    }
}

/// Every value under the registry rules' keys.
pub fn read(rules: &[Rule]) -> Vec<RegValue> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut out = Vec::new();
    let mut bytes = 0;
    for r in rules.iter().filter(|r| r.kind == Kind::Registry && r.problem().is_none()) {
        if let Ok(key) = hkcu.open_subkey_with_flags(win_path(&r.path), KEY_READ) {
            read_key(&key, &r.path, &mut out, &mut bytes, 8);
        }
    }
    out.sort_by_key(|v| (v.path.to_lowercase(), v.name.to_lowercase()));
    out.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path) && a.name.eq_ignore_ascii_case(&b.name));
    out
}

/// Write values back. Returns how many were written.
pub fn write(values: &[RegValue]) -> Result<usize, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut n = 0;
    for v in values.iter().take(MAX_VALUES) {
        if !safe(&v.path) {
            continue;
        }
        let Some(vtype) = kind_of(v.kind) else { continue };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&v.data) else { continue };
        let (key, _) = hkcu.create_subkey_with_flags(win_path(&v.path), KEY_WRITE).map_err(|e| format!("Could not write to the registry: {e}"))?;
        key.set_raw_value(&v.name, &WinValue { bytes, vtype }).map_err(|e| format!("Could not write to the registry: {e}"))?;
        n += 1;
    }
    Ok(n)
}

/// Last-write times (Unix seconds) of keys under `HKCU\Software`, two levels
/// down (`Software\Company\Product`), for seeing which a game touched.
pub fn key_times(depth: usize) -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(software) = hkcu.open_subkey_with_flags("Software", KEY_READ) else { return out };
    fn walk(key: &RegKey, path: &str, depth: usize, out: &mut std::collections::HashMap<String, u64>) {
        for sub in key.enum_keys().flatten().take(5000) {
            let Ok(child) = key.open_subkey_with_flags(&sub, KEY_READ) else { continue };
            let p = format!("{path}/{sub}");
            if let Ok(info) = child.query_info() {
                let secs = filetime_secs(&info);
                out.insert(p.clone(), secs);
            }
            if depth > 1 {
                walk(&child, &p, depth - 1, out);
            }
        }
    }
    walk(&software, "Software", depth, &mut out);
    out
}

fn filetime_secs(m: &winreg::RegKeyMetadata) -> u64 {
    // 100-nanosecond steps since 1601.
    let ft = ((m.last_write_time.dwHighDateTime as u64) << 32) | m.last_write_time.dwLowDateTime as u64;
    ft.saturating_sub(116_444_736_000_000_000) / 10_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Source;

    #[test]
    fn registry_values_round_trip_under_hkcu_software() {
        let path = format!("Software/KryotoSavesTest{}", std::process::id());
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(win_path(&path)).unwrap();
        key.set_value("level", &7u32).unwrap();
        key.set_value("name", &"Mira").unwrap();
        let (sub, _) = key.create_subkey("Slot1").unwrap();
        sub.set_value("gold", &1234u32).unwrap();
        let rules = vec![Rule::registry(path.clone(), Source::Engine)];
        let values = read(&rules);
        assert_eq!(values.len(), 3);
        hkcu.delete_subkey_all(win_path(&path)).unwrap();
        assert_eq!(write(&values).unwrap(), 3);
        let back: u32 = hkcu.open_subkey(win_path(&format!("{path}/Slot1"))).unwrap().get_value("gold").unwrap();
        assert_eq!(back, 1234);
        assert!(key_times(1).contains_key(&path));
        hkcu.delete_subkey_all(win_path(&path)).unwrap();
        assert_eq!(write(&[RegValue { path: "System/Evil".into(), name: "x".into(), kind: 1, data: String::new() }]).unwrap(), 0);
    }
}
