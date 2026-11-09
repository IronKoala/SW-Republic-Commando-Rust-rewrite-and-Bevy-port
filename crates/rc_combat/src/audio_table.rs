//! A character's voice table, read from the game at run time.
//!
//! Every character class names an "audio table" class in
//! `GameData/System/CTAudio.u` (`Delta07Audio` for Sev, `BattleDroidAudio`
//! for a battle droid). Its one default value, `Table`, is a list of sound
//! references: entry *i* is the sound for event *i* of the enum
//! `EPawnAudioEvent` in `GameData/System/Engine.u`. Entry 0 is event 0,
//! `PAE_Invalid`, and is always empty. Checked against the group names:
//! Sev's entry 105 is `D07_FormUp` and event 105 is `PAE_FormUp`.
//! An empty entry means the character has nothing to say for that event.
//! See docs/RESEARCH.md, "Grenade explosion sounds and enemy voices".

use rc_game_core::GamePaths;
use rc_package::class::{find_export, read_class_defaults, read_enum};
use rc_package::class_defaults::{find_class, parent_name};
use rc_package::properties::Value;
use rc_package::{Package, Reader};

/// The events' names in order, without the `PAE_` start, read from
/// Engine.u.
pub fn read_audio_events(paths: &GamePaths) -> Result<Vec<String>, String> {
    let path = paths.game_file("GameData/System/Engine.u");
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let e = find_export(&pkg, "EPawnAudioEvent", "Enum")
        .ok_or("Engine.u has no enum EPawnAudioEvent")?;
    let names = read_enum(&pkg, &data, e).map_err(|e| e.to_string())?;
    Ok(names
        .into_iter()
        .map(|n| n.strip_prefix("PAE_").map(str::to_string).unwrap_or(n))
        .collect())
}

/// One audio table: for each event that has a sound, (event name, sound
/// group name). `events` comes from `read_audio_events`.
pub fn read_audio_table(
    paths: &GamePaths,
    class: &str,
    events: &[String],
) -> Result<Vec<(String, String)>, String> {
    let path = paths.game_file("GameData/System/CTAudio.u");
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    // A table that doesn't set `Table` itself takes its parent's.
    let mut name = class.to_string();
    for _ in 0..8 {
        let e = find_class(&pkg, &name).ok_or_else(|| format!("CTAudio.u has no class {name}"))?;
        let defaults = read_class_defaults(&pkg, &data, e).map_err(|e| e.to_string())?;
        let table = defaults
            .properties
            .iter()
            .find(|p| pkg.name(p.name).eq_ignore_ascii_case("Table"));
        if let Some(p) = table {
            let Value::Raw(bytes) = &p.value else {
                return Err(format!("{name}.Table is not a list"));
            };
            return table_entries(&pkg, bytes, events);
        }
        match parent_name(&pkg, e) {
            Some(parent) => name = parent.to_string(),
            None => break,
        }
    }
    Err(format!("{class} has no Table"))
}

/// The list's bytes: a count, then one object reference per event,
/// starting with event 0 (`Invalid`).
fn table_entries(
    pkg: &Package,
    bytes: &[u8],
    events: &[String],
) -> Result<Vec<(String, String)>, String> {
    let mut r = Reader::new(bytes);
    let count = r.compact_index().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..count.max(0) as usize {
        let obj = r.compact_index().map_err(|e| e.to_string())?;
        if obj == 0 {
            continue;
        }
        let event = events
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("event{i}"));
        out.push((event, pkg.object_name(obj).to_string()));
    }
    Ok(out)
}
