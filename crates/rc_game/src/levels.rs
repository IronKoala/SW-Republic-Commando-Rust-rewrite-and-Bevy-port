//! The list of levels for the level select screen.
//!
//! Names, order and loading-screen text are read at start-up from the
//! original game's `GameData/System/LevelLoadingInfo.int`. Our
//! `data/levels/campaigns.toml` (and any mod's copy) groups them into
//! campaigns and can add more maps.

use bevy::prelude::Resource;
use rc_game_core::GamePaths;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct LevelEntry {
    /// Map name as given to `GamePaths::map_path` (`geo_01a`).
    pub map: String,
    pub title: String,
    /// Shown on the loading screen. May be empty.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Campaign {
    pub name: String,
    pub levels: Vec<LevelEntry>,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct LevelList {
    pub campaigns: Vec<Campaign>,
}

impl LevelList {
    pub fn find(&self, map: &str) -> Option<&LevelEntry> {
        self.campaigns
            .iter()
            .flat_map(|c| &c.levels)
            .find(|l| l.map.eq_ignore_ascii_case(map))
    }
    pub fn count(&self) -> usize {
        self.campaigns.iter().map(|c| c.levels.len()).sum()
    }
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct File {
    skip_prefixes: Vec<String>,
    show_other_maps: Option<bool>,
    campaign: Vec<CampaignDef>,
    extra: Vec<Extra>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignDef {
    name: String,
    prefix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Extra {
    campaign: String,
    map: String,
    title: String,
    #[serde(default)]
    text: String,
}

/// Text of an Unreal `.int` file: UTF-16 if it starts with a byte order
/// mark, otherwise Windows-1252 (where 0x92 is a curly apostrophe).
pub fn decode_int(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x91 | 0x92 => '\'',
            0x93 | 0x94 => '"',
            0x96 | 0x97 => '-',
            0x85 => '…',
            _ => b as char,
        })
        .collect()
}

/// `(map, title, text)` in the game file's order, from lines like
/// `MapName[3]="GEO_03A"` and `LoadingTitle[3]="..."`.
pub fn parse_loading_info(text: &str) -> Vec<(String, String, String)> {
    let mut rows: Vec<(usize, String, String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((field, rest)) = key.split_once('[') else {
            continue;
        };
        let Ok(i) = rest.trim_end_matches(']').parse::<usize>() else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        let row = match rows.iter_mut().find(|r| r.0 == i) {
            Some(r) => r,
            None => {
                rows.push((i, String::new(), String::new(), String::new()));
                rows.last_mut().unwrap()
            }
        };
        match field {
            "MapName" => row.1 = value,
            "LoadingTitle" => row.2 = value,
            "LoadingText" => row.3 = value,
            _ => {}
        }
    }
    rows.sort_by_key(|r| r.0);
    rows.into_iter()
        .filter(|r| !r.1.is_empty())
        .map(|r| (r.1, r.2, r.3))
        .collect()
}

/// Title case for the game's all-capitals titles ("INTO THE HIVE" to
/// "Into The Hive").
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the level list. Returns it and notes for the log.
pub fn load(paths: &GamePaths) -> (LevelList, Vec<String>) {
    let mut notes = Vec::new();
    let mut def = File::default();
    for p in paths.data_files("levels/campaigns.toml") {
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<File>(&t).map_err(|e| e.to_string()))
        {
            Ok(f) => {
                def.skip_prefixes.extend(f.skip_prefixes);
                if f.show_other_maps.is_some() {
                    def.show_other_maps = f.show_other_maps;
                }
                for c in f.campaign {
                    if !def.campaign.iter().any(|d| d.name == c.name) {
                        def.campaign.push(c);
                    }
                }
                def.extra.extend(f.extra);
                notes.push(format!("levels: groups from {}", p.display()));
            }
            Err(e) => notes.push(format!("levels: ERROR in {}: {e}", p.display())),
        }
    }
    let info_path = paths.game_file("GameData/System/LevelLoadingInfo.int");
    let game_list = match std::fs::read(&info_path) {
        Ok(b) => parse_loading_info(&decode_int(&b)),
        Err(e) => {
            notes.push(format!("levels: can't read {} ({e})", info_path.display()));
            vec![]
        }
    };
    let skip = |m: &str| {
        let m = m.to_ascii_lowercase();
        def.skip_prefixes
            .iter()
            .any(|p| m.starts_with(&p.to_ascii_lowercase()))
    };
    let mut list = LevelList::default();
    let mut used = HashSet::new();
    for c in &def.campaign {
        let prefix = c.prefix.to_ascii_lowercase();
        let levels: Vec<LevelEntry> = game_list
            .iter()
            .filter(|(m, _, _)| m.to_ascii_lowercase().starts_with(&prefix) && !skip(m))
            .filter(|(m, _, _)| paths.map_path(&m.to_ascii_lowercase()).exists())
            .map(|(m, t, x)| LevelEntry {
                map: m.to_ascii_lowercase(),
                title: title_case(t),
                text: x.clone(),
            })
            .collect();
        for l in &levels {
            used.insert(l.map.clone());
        }
        if !levels.is_empty() {
            list.campaigns.push(Campaign {
                name: c.name.clone(),
                levels,
            });
        }
    }
    for e in def.extra {
        used.insert(e.map.to_ascii_lowercase());
        let entry = LevelEntry {
            map: e.map,
            title: e.title,
            text: e.text,
        };
        match list.campaigns.iter_mut().find(|c| c.name == e.campaign) {
            Some(c) => c.levels.push(entry),
            None => list.campaigns.push(Campaign {
                name: e.campaign,
                levels: vec![entry],
            }),
        }
    }
    if def.show_other_maps.unwrap_or(false) {
        let mut other: Vec<String> = std::fs::read_dir(paths.game_file("GameData/Maps"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.strip_suffix(".ctm").map(|s| s.to_string())
            })
            .filter(|m| !used.contains(m))
            .collect();
        other.sort();
        if !other.is_empty() {
            list.campaigns.push(Campaign {
                name: "Other maps".into(),
                levels: other
                    .into_iter()
                    .map(|m| LevelEntry {
                        title: m.clone(),
                        map: m,
                        text: String::new(),
                    })
                    .collect(),
            });
        }
    }
    notes.push(format!(
        "levels: {} in the game's list, {} campaigns, {} levels shown",
        game_list.len(),
        list.campaigns.len(),
        list.count()
    ));
    (list, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_game_list_format() {
        let text = "[LevelLoadingInfo]\r\n\r\nMapName[0]=\"GEO_01A\"\r\nLoadingTitle[0]=\"EXTREME PREJUDICE\"\r\nLoadingText[0]=\"Go.\"\r\nMapName[1]=\"GEO_01B\"\r\nLoadingTitle[1]=\"INTO THE HIVE\"\r\n";
        let rows = parse_loading_info(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            ("GEO_01A".into(), "EXTREME PREJUDICE".into(), "Go.".into())
        );
        assert_eq!(title_case(&rows[1].1), "Into The Hive");
        assert_eq!(decode_int(b"FAC\x92S"), "FAC'S");
    }

    #[test]
    fn shipped_campaigns_file_reads() {
        let f: File = toml::from_str(include_str!("../../../data/levels/campaigns.toml")).unwrap();
        assert_eq!(f.campaign.len(), 5);
        assert!(f.skip_prefixes.contains(&"dm_".to_string()));
    }
}
