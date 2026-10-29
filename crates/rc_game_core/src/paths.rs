//! Where things are: the original game's files, our data files, and mods.

use bevy::prelude::Resource;
use std::path::{Path, PathBuf};

/// Folders, decided once at start-up from the settings file.
///
/// - `game_dir`: the original game's install (the folder holding
///   `GameData`). Only ever read. On Android this is the folder the player
///   picked.
/// - `data_dirs`: our own data files (settings for weapons, input, menus),
///   searched in order. Enabled mods come first and the base `data` folder
///   last, so a mod's file replaces the base file of the same name.
#[derive(Resource, Debug, Clone)]
pub struct GamePaths {
    pub game_dir: PathBuf,
    pub data_dirs: Vec<PathBuf>,
}

impl GamePaths {
    pub fn new(game_dir: impl Into<PathBuf>, data_dirs: Vec<PathBuf>) -> Self {
        GamePaths {
            game_dir: game_dir.into(),
            data_dirs,
        }
    }

    /// `<game>/GameData`, where all the packages are.
    pub fn game_data(&self) -> PathBuf {
        resolve(&self.game_dir, "GameData")
    }

    /// A file or folder in the game install, like `GameData/Maps/geo_01a.ctm`.
    /// Upper and lower case don't have to match, which matters on Linux and
    /// Android where file names are case-sensitive.
    pub fn game_file(&self, rel: &str) -> PathBuf {
        resolve(&self.game_dir, rel)
    }

    /// The `.ctm` file for a map name (`geo_01a`), or the path itself if it
    /// already ends in `.ctm`.
    pub fn map_path(&self, map: &str) -> PathBuf {
        if map.to_ascii_lowercase().ends_with(".ctm") {
            PathBuf::from(map)
        } else {
            self.game_file(&format!("GameData/Maps/{map}.ctm"))
        }
    }

    /// Our data file `rel` (like `weapons/dc15s.toml`): the first one found
    /// in the data folders, so mods win. `None` if no folder has it.
    pub fn data_file(&self, rel: &str) -> Option<PathBuf> {
        self.data_dirs
            .iter()
            .map(|d| resolve(d, rel))
            .find(|p| p.exists())
    }

    /// Every copy of data file `rel`, base first and mods after, for data
    /// that adds up instead of replacing (like extra levels in a list).
    pub fn data_files(&self, rel: &str) -> Vec<PathBuf> {
        self.data_dirs
            .iter()
            .rev()
            .map(|d| resolve(d, rel))
            .filter(|p| p.exists())
            .collect()
    }
}

/// `base` joined with `rel` (parts separated by `/`), matching each part
/// without caring about upper or lower case when the exact name isn't
/// there. Parts that can't be found are joined as written.
pub fn resolve(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        let exact = p.join(part);
        if exact.exists() {
            p = exact;
            continue;
        }
        let found = std::fs::read_dir(&p).ok().and_then(|entries| {
            entries
                .flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                .map(|e| e.path())
        });
        p = found.unwrap_or(exact);
    }
    p
}
