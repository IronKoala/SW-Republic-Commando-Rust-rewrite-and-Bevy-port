//! Saving and loading: the one interface every part of the game fills in.
//!
//! A save file is a readable TOML file with a header (`[save]`: which map,
//! which checkpoint, when) and one section per part, like `[player]`,
//! `[weapons]` or `[squad]`. Each part decides what goes in its own section
//! by implementing [`SavePart`] and registering it with
//! [`SaveAppExt::add_save_part`]. The save plugin (`rc_save`) does the rest:
//! the save folder, writing the file safely, quicksave and quickload, and
//! saving at checkpoints.
//!
//! Loading always loads the save's map from scratch first, the same way the
//! menu does. Then, on the first frame of play, after every part's
//! `OnEnter(GameState::InGame)` has run, each part's [`SavePart::load`] is
//! called with its section. So a part only has to put back what changed
//! since the map started (where the player is, what is dead, which doors are
//! open), and never has to undo half a level.
//!
//! While a save is being restored, the resource [`RestoringSave`] exists
//! (from the load request until every part has loaded). Anything that
//! happens "when the level starts" (an opening script, the level-start
//! autosave) checks for it and stays quiet.
//!
//! Example, a part that keeps a kill count in a resource:
//!
//! ```ignore
//! #[derive(Serialize, Deserialize)]
//! struct KillsSave { kills: u32 }
//!
//! impl SavePart for KillsSave {
//!     const KEY: &'static str = "kills";
//!     fn save(world: &mut World) -> Option<Self> {
//!         Some(KillsSave { kills: world.get_resource::<Kills>()?.0 })
//!     }
//!     fn load(world: &mut World, data: Self) -> Result<(), String> {
//!         world.insert_resource(Kills(data.kills));
//!         Ok(())
//!     }
//! }
//!
//! app.add_save_part::<KillsSave>();
//! ```

use bevy::ecs::world::World;
use bevy::prelude::{App, Message, Resource};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// One part's section of a save file. The type itself is what's written:
/// derive `Serialize` and `Deserialize` on it and keep the fields readable
/// (names, numbers, Unreal units), since players and modders read these
/// files.
pub trait SavePart: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The section's name in the file, like `"player"`. Lower case, one
    /// word or `snake_case`. `"save"` is taken by the header.
    const KEY: &'static str;

    /// Read this part's state out of the game. `None` writes no section
    /// (nothing to save on this map).
    fn save(world: &mut World) -> Option<Self>;

    /// Put this part's state back. Called on the first frame of play after
    /// the save's map has been loaded fresh. An `Err` is logged and the
    /// rest of the save still loads.
    fn load(world: &mut World, data: Self) -> Result<(), String>;

    /// The file has no section for this part (it was saved by an older
    /// version, or this part had nothing to save). The default leaves the
    /// part as the fresh map started it.
    fn load_missing(_world: &mut World) {}
}

/// Saves a section: `Ok(None)` when the part has nothing to save.
pub type SaveFn = fn(&mut World) -> Result<Option<toml::Table>, String>;
/// Loads a section; `None` when the file has none for this part.
pub type LoadFn = fn(&mut World, Option<toml::Table>) -> Result<(), String>;

/// One registered part.
#[derive(Clone)]
pub struct RegisteredPart {
    pub key: &'static str,
    pub save: SaveFn,
    pub load: LoadFn,
}

/// Every part that saves, in the order they were registered. Loading goes in
/// the same order. Filled by [`SaveAppExt::add_save_part`].
#[derive(Resource, Clone, Default)]
pub struct SaveParts(pub Vec<RegisteredPart>);

impl SaveParts {
    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().map(|p| p.key)
    }
}

fn save_part<P: SavePart>(world: &mut World) -> Result<Option<toml::Table>, String> {
    match P::save(world) {
        None => Ok(None),
        Some(data) => toml::Table::try_from(&data)
            .map(Some)
            .map_err(|e| format!("can't write the {} section: {e}", P::KEY)),
    }
}

fn load_part<P: SavePart>(world: &mut World, table: Option<toml::Table>) -> Result<(), String> {
    match table {
        None => {
            P::load_missing(world);
            Ok(())
        }
        Some(t) => {
            let data: P = t
                .try_into()
                .map_err(|e| format!("the {} section doesn't read: {e}", P::KEY))?;
            P::load(world, data)
        }
    }
}

/// `app.add_save_part::<MySave>()`.
pub trait SaveAppExt {
    fn add_save_part<P: SavePart>(&mut self) -> &mut Self;
}

impl SaveAppExt for App {
    fn add_save_part<P: SavePart>(&mut self) -> &mut Self {
        assert!(P::KEY != "save", "\"save\" is the header's section name");
        let world = self.world_mut();
        let mut parts = world.get_resource_or_insert_with(SaveParts::default);
        assert!(
            parts.keys().all(|k| k != P::KEY),
            "two parts both save a section called {:?}",
            P::KEY
        );
        parts.0.push(RegisteredPart {
            key: P::KEY,
            save: save_part::<P>,
            load: load_part::<P>,
        });
        self
    }
}

/// Which save file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SaveSlot {
    /// Quicksave and quickload.
    Quick,
    /// Written automatically at each checkpoint (and the start of a level);
    /// loaded by "retry from checkpoint".
    Checkpoint,
    /// A named save from a save menu. Only letters, digits, `-` and `_` are
    /// kept in the file name.
    Named(String),
}

impl SaveSlot {
    /// The file name, without the folder.
    pub fn file_name(&self) -> String {
        match self {
            SaveSlot::Quick => "quicksave.toml".into(),
            SaveSlot::Checkpoint => "checkpoint.toml".into(),
            SaveSlot::Named(n) => {
                let clean: String = n
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .take(64)
                    .collect();
                format!(
                    "save_{}.toml",
                    if clean.is_empty() { "unnamed" } else { &clean }
                )
            }
        }
    }

    /// The slot a file name belongs to (the reverse of `file_name`).
    pub fn from_file_name(name: &str) -> Option<SaveSlot> {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "quicksave.toml" => Some(SaveSlot::Quick),
            "checkpoint.toml" => Some(SaveSlot::Checkpoint),
            _ => {
                let stem = name.strip_suffix(".toml")?.strip_prefix("save_")?;
                Some(SaveSlot::Named(stem.to_string()))
            }
        }
    }

    /// For the log and the menus.
    pub fn label(&self) -> String {
        match self {
            SaveSlot::Quick => "quicksave".into(),
            SaveSlot::Checkpoint => "checkpoint".into(),
            SaveSlot::Named(n) => n.clone(),
        }
    }
}

/// Ask for the game to be saved now. Only does anything while in a level.
#[derive(Message, Debug, Clone)]
pub struct SaveGame {
    pub slot: SaveSlot,
}

/// Ask for a save to be loaded (from the menus, or in a level).
#[derive(Message, Debug, Clone)]
pub struct LoadGame {
    pub slot: SaveSlot,
}

/// Ask for a save file to be deleted (from a Load or Save Game screen).
/// [`SaveList`] is updated afterwards.
#[derive(Message, Debug, Clone)]
pub struct DeleteSave {
    pub slot: SaveSlot,
}

/// The player reached a checkpoint. The level's scripts send this with the
/// checkpoint's name (anything unique within the map, like the trigger's
/// name). The save plugin saves to [`SaveSlot::Checkpoint`] the first time
/// each name is reached, and ignores repeats.
#[derive(Message, Debug, Clone)]
pub struct CheckpointReached {
    pub name: String,
}

/// What happened to a save or load, for the display and the menus.
#[derive(Message, Debug, Clone)]
pub struct SaveResult {
    pub slot: SaveSlot,
    /// `true` for loading, `false` for saving.
    pub loading: bool,
    /// `Err` holds a message a player can read.
    pub outcome: Result<(), String>,
}

/// Exists while a save is being restored: from the load request until every
/// part's `load` has run. Level-start things (opening scripts, the
/// level-start autosave) check for this and don't run.
#[derive(Resource, Debug, Clone)]
pub struct RestoringSave {
    pub slot: SaveSlot,
}

/// One save on disk, for a Load Game screen. Kept up to date by the save
/// plugin in [`SaveList`].
#[derive(Debug, Clone, PartialEq)]
pub struct SaveInfo {
    pub slot: SaveSlot,
    /// The map, as in `GameData/Maps` (`geo_01a`).
    pub map: String,
    /// The last checkpoint reached before saving, if any.
    pub checkpoint: Option<String>,
    /// When it was saved: seconds since 1970 (UTC).
    pub saved_at: u64,
    /// Time spent playing this map, seconds.
    pub play_seconds: f64,
}

/// Every readable save in the save folder, newest first. Refreshed when the
/// main menu opens and after each save.
#[derive(Resource, Debug, Clone, Default)]
pub struct SaveList(pub Vec<SaveInfo>);

impl SaveList {
    pub fn newest(&self) -> Option<&SaveInfo> {
        self.0.first()
    }
    pub fn get(&self, slot: &SaveSlot) -> Option<&SaveInfo> {
        self.0.iter().find(|s| &s.slot == slot)
    }
}
