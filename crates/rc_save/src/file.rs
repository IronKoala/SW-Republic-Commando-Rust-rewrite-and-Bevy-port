//! Save files on disk: the header, reading and writing, the save folder and
//! the list of saves. No Bevy systems in here, so it's tested on its own.

use rc_game_core::save::{SaveInfo, SaveSlot};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The newest file layout this version writes and reads. A file with a
/// bigger number was written by a newer version and isn't loaded.
pub const FORMAT: u32 = 1;

/// The `[save]` section at the top of every save file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub format: u32,
    /// Which version of the game wrote it (for bug reports).
    pub game_version: String,
    /// The map, as in `GameData/Maps` (`geo_01a`).
    pub map: String,
    /// The last checkpoint reached, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    /// Seconds since 1970 (UTC).
    pub saved_at: u64,
    /// The same, for people reading the file. Not read back.
    #[serde(default)]
    pub saved_at_text: String,
    /// Time spent playing this map, seconds (paused time doesn't count).
    pub play_seconds: f64,
}

/// A whole save: the header and each part's section, by section name.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveFile {
    pub header: Header,
    pub parts: toml::Table,
}

impl SaveFile {
    /// The file's text: a comment, the header, then each part's section in
    /// `order` (the order parts were registered), then any others.
    pub fn to_text(&self, order: &[&str]) -> Result<String, String> {
        let mut out = String::from(
            "# Star Wars Republic Commando (Rust and Bevy) save file.\n\
             # Plain text: safe to read, and to edit if you know what you're changing.\n\
             # Each [section] belongs to one part of the game. Positions are in\n\
             # Unreal units, angles in radians.\n\n",
        );
        let mut header = toml::Table::new();
        header.insert(
            "save".into(),
            toml::Value::try_from(&self.header).map_err(|e| e.to_string())?,
        );
        out.push_str(&toml::to_string(&header).map_err(|e| e.to_string())?);
        let mut keys: Vec<&str> = order
            .iter()
            .copied()
            .filter(|k| self.parts.contains_key(*k))
            .collect();
        for k in self.parts.keys() {
            if !keys.contains(&k.as_str()) {
                keys.push(k);
            }
        }
        for k in keys {
            let mut one = toml::Table::new();
            one.insert(k.to_string(), self.parts[k].clone());
            out.push('\n');
            out.push_str(&toml::to_string(&one).map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn parse(text: &str) -> Result<SaveFile, String> {
        let mut root: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let header = root
            .remove("save")
            .ok_or("no [save] section; this isn't a save file")?;
        // Check the format number before the rest, so a newer file gets a
        // clear message rather than a complaint about a field.
        let format = header.get("format").and_then(|v| v.as_integer());
        match format {
            Some(f) if f > FORMAT as i64 => {
                return Err(format!(
                    "saved by a newer version of the game (save format {f}, this version reads up to {FORMAT})"
                ))
            }
            None => return Err("the [save] section has no format number".into()),
            _ => {}
        }
        let header: Header = header
            .try_into()
            .map_err(|e| format!("in the [save] section: {e}"))?;
        let mut parts = toml::Table::new();
        for (k, v) in root {
            match v {
                toml::Value::Table(_) => {
                    parts.insert(k, v);
                }
                _ => return Err(format!("{k} should be a [section]")),
            }
        }
        Ok(SaveFile { header, parts })
    }

    /// One part's section.
    pub fn part(&self, key: &str) -> Option<toml::Table> {
        self.parts.get(key).and_then(|v| v.as_table()).cloned()
    }

    pub fn info(&self, slot: SaveSlot) -> SaveInfo {
        SaveInfo {
            slot,
            map: self.header.map.clone(),
            checkpoint: self.header.checkpoint.clone(),
            saved_at: self.header.saved_at,
            play_seconds: self.header.play_seconds,
        }
    }
}

/// Write `text` to `path` without ever leaving half a file: write a
/// temporary file next to it, then swap it in. A crash or a full disk
/// leaves the old save as it was.
pub fn write_safely(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.writing");
    std::fs::write(&tmp, text).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't replace {}: {e}", path.display())
    })
}

pub fn read(path: &Path) -> Result<SaveFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read it: {e}"))?;
    SaveFile::parse(&text)
}

/// Every readable save in `dir`, newest first, and a note for each file
/// that looked like a save but didn't read.
pub fn list(dir: &Path) -> (Vec<SaveInfo>, Vec<String>) {
    let mut saves = Vec::new();
    let mut bad = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (saves, bad);
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(slot) = SaveSlot::from_file_name(&name) else {
            continue;
        };
        match read(&e.path()) {
            Ok(f) => saves.push(f.info(slot)),
            Err(err) => bad.push(format!("{name}: {err}")),
        }
    }
    saves.sort_by(|a: &SaveInfo, b: &SaveInfo| {
        b.saved_at
            .cmp(&a.saved_at)
            .then_with(|| a.slot.cmp(&b.slot))
    });
    (saves, bad)
}

/// Which system we're building for, for [`default_folder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    Windows,
    Linux,
    Android,
    Other,
}

impl System {
    pub fn this() -> System {
        if cfg!(target_os = "android") {
            System::Android
        } else if cfg!(windows) {
            System::Windows
        } else if cfg!(target_os = "linux") {
            System::Linux
        } else {
            System::Other
        }
    }
}

/// The usual save folder on each system, used when the settings file
/// doesn't name one. `env` looks up an environment variable.
///
/// - Windows: `%APPDATA%\rc_game\saves`
/// - Linux: `$XDG_DATA_HOME/rc_game/saves`, or `~/.local/share/rc_game/saves`
/// - Android (and anything else): `saves` in the folder the game runs
///   from. The Android app sets its own folder in the settings file (see
///   docs/ANDROID.md).
///
/// Never inside the game's install.
pub fn default_folder(system: System, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    match system {
        System::Windows => set("APPDATA").map(|d| d.join("rc_game").join("saves")),
        System::Linux => set("XDG_DATA_HOME")
            .or_else(|| set("HOME").map(|h| h.join(".local").join("share")))
            .map(|d| d.join("rc_game").join("saves")),
        System::Android | System::Other => None,
    }
    .unwrap_or_else(|| PathBuf::from("saves"))
}

/// A date and time for people, `2026-10-29 18:40 UTC`, from seconds since
/// 1970. (Days to a calendar date by Howard Hinnant's method, which is in
/// the public domain.)
pub fn time_text(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The path of a slot's file in `dir`.
pub fn slot_path(dir: &Path, slot: &SaveSlot) -> PathBuf {
    dir.join(slot.file_name())
}

/// How far apart two sections are: the biggest difference between matching
/// numbers anywhere inside them. `None` if anything other than a number
/// differs (a name, a yes/no, a missing value). For checking that a save,
/// load and second save give back the same state.
pub fn largest_difference(a: &toml::Value, b: &toml::Value) -> Option<f64> {
    use toml::Value::*;
    let num = |v: &toml::Value| match v {
        Integer(i) => Some(*i as f64),
        Float(f) => Some(*f),
        _ => None,
    };
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return Some((x - y).abs());
    }
    match (a, b) {
        (Array(x), Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .try_fold(0.0f64, |m, (p, q)| Some(m.max(largest_difference(p, q)?))),
        (Table(x), Table(y)) if x.len() == y.len() => x.iter().try_fold(0.0f64, |m, (k, p)| {
            Some(m.max(largest_difference(p, y.get(k)?)?))
        }),
        _ => (a == b).then_some(0.0),
    }
}
