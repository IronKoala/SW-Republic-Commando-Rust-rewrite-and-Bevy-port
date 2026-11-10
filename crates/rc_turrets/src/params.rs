//! A kind of turret, from its data file (`data/turrets/<id>.toml`): which
//! map classes it is, its model, how it aims, where its gunner sits and
//! what the player fires from it. A misspelt name stops the file loading
//! with an error naming it.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurretParams {
    /// Shown in the log.
    pub name: String,
    /// The map classes this file is for (`WookieeHeavyTurret`). A map
    /// turret no file claims is left out (and the log says so).
    pub classes: Vec<String>,
    pub model: Model,
    pub aim: Aim,
    pub gunner: Gunner,
    /// A character on the turret (a squad mate, an enemy).
    #[serde(default)]
    pub auto: Auto,
}

/// The turret's animated model in the game's animation packages.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// Relative to the game folder.
    #[serde(default = "turrets_package")]
    pub package: String,
    pub mesh: String,
    /// The animation whose first frame is its resting pose.
    pub pose: String,
    /// The bones that turn it: around the up axis, and up and down.
    #[serde(default = "yaw_bone")]
    pub yaw_bone: String,
    #[serde(default = "pitch_bone")]
    pub pitch_bone: String,
    /// The bones shots leave from (a character gunner's shots).
    #[serde(default)]
    pub muzzles: Vec<String>,
    /// Hidden while the player is on it: the first-person weapon shows the
    /// turret instead (the original hides it when the view is the
    /// player's own).
    #[serde(default = "yes")]
    pub hide_when_player_on: bool,
}

fn turrets_package() -> String {
    "GameData/Animations/turrets.ukx".into()
}
fn yaw_bone() -> String {
    "pivotYaw".into()
}
fn pitch_bone() -> String {
    "pivotPitch".into()
}
fn yes() -> bool {
    true
}

/// How it turns, in the game's rotation units (65536 = a full turn). A
/// map can set its own limits and speed for one turret.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aim {
    /// Fastest turn, units per second, each way (`Speed`).
    pub speed: f32,
    pub min_yaw: i32,
    pub max_yaw: i32,
    pub min_pitch: i32,
    pub max_pitch: i32,
    /// Turns all the way round (`bUnlimitedYaw`).
    #[serde(default)]
    pub unlimited_yaw: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gunner {
    /// The gunner's body is held this far above the turret's centre, uu
    /// (`TurretHeight`).
    pub height: f32,
    /// Sits rather than stands (`SitToUse`).
    #[serde(default)]
    pub sit: bool,
    /// The player may get on (`bPlayerUsable`; maps and scripts change it).
    #[serde(default = "yes")]
    pub player_usable: bool,
    /// How close the player must be to get on: uu from the edge of the
    /// turret's collision cylinder to the edge of the player's.
    pub reach: f32,
    /// The turret's collision cylinder, uu (`CollisionRadius`,
    /// `CollisionHeight`).
    pub radius: f32,
    pub half_height: f32,
    /// Where the gunner gets off when the map gives no spot for it:
    /// forward, right, up from the turret's centre, uu (`NavPtLocs`).
    pub exit: [f32; 3],
    /// What the player fires while on it (`data/weapons/<id>.toml`).
    pub weapon: Option<String>,
    /// The message offered when the player can get on. `@` stands for the
    /// USE button.
    #[serde(default = "mount_prompt")]
    pub prompt: String,
}

fn mount_prompt() -> String {
    "PRESS @ TO MOUNT TURRET".into()
}

/// A character gunner (`AutoTurret` state): the turret fires on its own
/// at the gunner's enemy.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Auto {
    /// The shot it fires (`data/weapons/<id>.toml`), from each muzzle.
    pub weapon: Option<String>,
    /// Seconds between shots (`ShotDelay`).
    #[serde(default)]
    pub shot_delay: f32,
    /// How far it looks for something to shoot, uu.
    #[serde(default)]
    pub range: f32,
    /// Each shot leaves from the next muzzle in turn, rather than one from
    /// every muzzle at once.
    #[serde(default)]
    pub muzzles_in_turn: bool,
}

impl TurretParams {
    pub fn from_text(text: &str) -> Result<TurretParams, String> {
        let p: TurretParams = toml::from_str(text).map_err(|e| e.to_string())?;
        if p.classes.is_empty() {
            return Err("`classes` is empty: name the map classes this file is for".into());
        }
        if p.aim.speed <= 0.0 {
            return Err("aim.speed must be more than 0".into());
        }
        if p.aim.min_pitch > p.aim.max_pitch || p.aim.min_yaw > p.aim.max_yaw {
            return Err("aim: a min is more than its max".into());
        }
        Ok(p)
    }
}

/// Every turret file in `folders` (later folders override earlier ones by
/// file name: mods). Returns the files by id, and errors as text.
pub fn load_folders(folders: &[&Path]) -> (BTreeMap<String, TurretParams>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| TurretParams::from_text(&t))
            {
                Ok(p) => {
                    out.insert(id, p);
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
    }
    (out, errors)
}

/// The file that claims a map class, if any: its id and its values.
pub fn for_class<'a>(
    files: &'a BTreeMap<String, TurretParams>,
    class: &str,
) -> Option<(&'a String, &'a TurretParams)> {
    files
        .iter()
        .find(|(_, p)| p.classes.iter().any(|c| c.eq_ignore_ascii_case(class)))
}
