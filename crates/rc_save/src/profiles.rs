//! Player profiles: each one is a folder in the save folder, holding that
//! profile's saves. The original keeps a profile in
//! `GameData/Save/Profile_<name>` (its settings) and puts the profile's
//! name in front of each save's name; this port keeps the same folder name
//! and puts the profile's saves inside it, so deleting a profile deletes
//! its saves, as the original does.
//!
//! The PROFILES pages that use these are in `rc_game`
//! (`original_profiles.rs`).

use std::path::{Path, PathBuf};

/// The original's `ProfilePrefix` (`MenuSelectProfile` and
/// `MenuPCNewProfileName` in `XInterfaceCommon.u`): profile folders are
/// called `Profile_<name>`.
pub const PREFIX: &str = "Profile";

/// The folder of profile `name` in the save folder `root`.
pub fn folder(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{PREFIX}_{name}"))
}

/// The profiles in `root`, sorted as the original's `SortProfileList`
/// sorts them (plain string order, capitals first).
pub fn list(root: &Path) -> Vec<String> {
    let start = format!("{PREFIX}_").to_ascii_lowercase();
    let mut names: Vec<String> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    let lower = n.to_lowercase();
                    lower
                        .starts_with(&start)
                        .then(|| n[start.len()..].to_string())
                })
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The profile in `names` with this name, ignoring capitals (profile
/// folders on Windows ignore them too).
pub fn find<'a>(names: &'a [String], name: &str) -> Option<&'a String> {
    names
        .iter()
        .find(|n| n.to_lowercase() == name.to_lowercase())
}

/// Make a new profile's folder.
pub fn create(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = folder(root, name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Delete a profile's folder and the saves in it.
pub fn delete(root: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("{name:?} isn't a profile name"));
    }
    let dir = folder(root, name);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("can't delete {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_folders_in_the_save_folder() {
        let root = std::env::temp_dir().join(format!("rc_profiles_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Saves and other folders aren't profiles.
        std::fs::write(root.join("quicksave.toml"), "").unwrap();
        std::fs::create_dir_all(root.join("Other")).unwrap();
        create(&root, "zed").unwrap();
        create(&root, "Matt").unwrap();
        std::fs::write(folder(&root, "Matt").join("quicksave.toml"), "").unwrap();
        assert_eq!(list(&root), ["Matt", "zed"]);
        assert_eq!(find(&list(&root), "MATT").map(String::as_str), Some("Matt"));
        delete(&root, "Matt").unwrap();
        assert_eq!(list(&root), ["zed"]);
        assert!(delete(&root, "../x").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
