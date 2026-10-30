//! rc_save: saving, loading and checkpoints.
//!
//! The interface every part fills in is in `rc_game_core::save` (see
//! [`rc_game_core::save::SavePart`]). This plugin does the rest:
//!
//! - writes and reads the save files (readable TOML, see [`file`]) in the
//!   [`SaveFolder`], never in the game's install;
//! - quicksave and quickload (`Action::QuickSave`, `Action::QuickLoad`);
//! - saves to the checkpoint slot at the start of each level and the first
//!   time each checkpoint is reached ([`CheckpointReached`]);
//! - loads a save by loading its map fresh and then handing each part its
//!   section on the first frame of play;
//! - keeps [`SaveList`] up to date for a Load Game screen;
//! - goes on to the next map when a level exit is reached
//!   ([`ChangeLevel`]): the sections listed in `data/levels/travel.toml`
//!   (the player's health, shield and weapons) are carried over, the map
//!   is loaded fresh, the carried sections are put back on its first
//!   frame, and its start is saved as the first checkpoint.
//!
//! Everything is logged with the prefix `save:`, including a short summary
//! of each part's section, so a save and load can be checked from the log.

pub mod file;

use bevy::prelude::*;
use file::{Header, SaveFile};
use rc_game_core::save::{
    CheckpointReached, LoadGame, RestoringSave, SaveAppExt, SaveGame, SaveList, SavePart,
    SaveParts, SaveResult, SaveSlot,
};
use rc_game_core::{
    playing, Action, ActionState, ArrivalPortal, ChangeLevel, CurrentLevel, GameLog, GamePaths,
    GameSet, GameState, HudMessage,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where save files go. `rc_game` inserts it from the settings file; if
/// nobody does, the system's usual folder is used ([`file::default_folder`]).
#[derive(Resource, Debug, Clone)]
pub struct SaveFolder(pub PathBuf);

/// Checkpoints reached on this map, oldest first. Saved in the
/// `[checkpoints]` section, so a checkpoint isn't saved twice after a load.
#[derive(Resource, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checkpoints {
    pub reached: Vec<String>,
}

impl Checkpoints {
    pub fn last(&self) -> Option<&str> {
        self.reached.last().map(|s| s.as_str())
    }
}

/// The name the start of a level is saved under.
pub const LEVEL_START: &str = "level start";

impl SavePart for Checkpoints {
    const KEY: &'static str = "checkpoints";
    fn save(world: &mut World) -> Option<Self> {
        world.get_resource::<Checkpoints>().cloned()
    }
    fn load(world: &mut World, data: Self) -> Result<(), String> {
        world.insert_resource(data);
        Ok(())
    }
}

/// Time spent playing this map, seconds. Counts only while not paused.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PlayTime(pub f64);

/// What goes with the player from one map to the next: the save sections
/// (by key) carried over. From `data/levels/travel.toml`
/// ([`TRAVEL_FILE`]); without it, `combat` (health, shield, weapons and
/// ammo).
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct TravelSettings {
    pub carry: Vec<String>,
}

impl Default for TravelSettings {
    fn default() -> Self {
        TravelSettings {
            carry: vec!["combat".into()],
        }
    }
}

/// The travel settings file in the data folders. Optional.
pub const TRAVEL_FILE: &str = "levels/travel.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TravelFile {
    carry: Vec<String>,
}

impl TravelSettings {
    /// Read the file's text. An unknown name is an error, so a typo in a
    /// mod is reported.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let f: TravelFile = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(TravelSettings { carry: f.carry })
    }
}

/// Sections carried from the map just left, waiting for the next one to
/// finish loading.
#[derive(Resource, Debug, Clone)]
struct PendingTravel {
    from: String,
    sections: toml::Table,
}

/// Saves and loads asked for this frame, done in order at the end of it.
#[derive(Resource, Debug, Default)]
struct Requests(Vec<Request>);

#[derive(Debug, Clone)]
enum Request {
    Save {
        slot: SaveSlot,
        checkpoint: Option<String>,
    },
    Load(SaveSlot),
    Travel(ChangeLevel),
}

/// A save read from disk, waiting for its map to finish loading.
#[derive(Resource, Debug, Clone)]
struct PendingLoad {
    slot: SaveSlot,
    file: SaveFile,
}

/// Order within `GameSet::Hud`: requests are gathered, then carried out.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SaveSet {
    Gather,
    Run,
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Requests>()
            .init_resource::<Checkpoints>()
            .init_resource::<PlayTime>()
            .init_resource::<TravelSettings>()
            .add_save_part::<Checkpoints>()
            .configure_sets(
                Update,
                (SaveSet::Gather, SaveSet::Run).chain().in_set(GameSet::Hud),
            )
            .add_systems(Startup, (set_up_folder, refresh_list, read_travel).chain())
            .add_systems(OnEnter(GameState::MainMenu), (forget_pending, refresh_list))
            .add_systems(OnEnter(GameState::InGame), level_started)
            .add_systems(
                Update,
                apply_pending
                    .before(GameSet::Input)
                    .run_if(in_state(GameState::InGame))
                    .run_if(resource_exists::<PendingLoad>),
            )
            .add_systems(
                Update,
                arrive
                    .before(GameSet::Input)
                    .run_if(in_state(GameState::InGame))
                    .run_if(resource_exists::<PendingTravel>),
            )
            .add_systems(
                Update,
                count_play_time.in_set(GameSet::Simulate).run_if(playing),
            )
            .add_systems(Update, gather.in_set(SaveSet::Gather))
            .add_systems(
                Update,
                run_requests
                    .in_set(SaveSet::Run)
                    .run_if(|r: Res<Requests>| !r.0.is_empty()),
            );
    }
}

fn set_up_folder(mut commands: Commands, folder: Option<Res<SaveFolder>>, log: Res<GameLog>) {
    let dir = match folder {
        Some(f) => f.0.clone(),
        None => {
            let d = file::default_folder(file::System::this(), |k| std::env::var(k).ok());
            commands.insert_resource(SaveFolder(d.clone()));
            d
        }
    };
    log.line(format!("save: folder {}", dir.display()));
}

fn refresh_list(folder: Option<Res<SaveFolder>>, mut list: ResMut<SaveList>, log: Res<GameLog>) {
    // On the first frame, `default_folder`'s insert isn't applied yet.
    let dir = folder
        .map(|f| f.0.clone())
        .unwrap_or_else(|| file::default_folder(file::System::this(), |k| std::env::var(k).ok()));
    let (saves, bad) = file::list(&dir);
    for b in bad {
        log.line(format!("save: skipping {b}"));
    }
    log.line(format!(
        "save: {} saves found{}",
        saves.len(),
        saves
            .iter()
            .map(|s| format!(
                " [{} {} {}]",
                s.slot.label(),
                s.map,
                file::time_text(s.saved_at)
            ))
            .collect::<String>()
    ));
    list.0 = saves;
}

fn read_travel(paths: Option<Res<GamePaths>>, mut travel: ResMut<TravelSettings>, log: Res<GameLog>) {
    let Some(path) = paths.and_then(|p| p.data_file(TRAVEL_FILE)) else {
        log.line(format!(
            "save: no {TRAVEL_FILE}; carrying {:?} to the next map",
            travel.carry
        ));
        return;
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| TravelSettings::from_text(&t))
    {
        Ok(t) => {
            *travel = t;
            log.line(format!(
                "save: {} says carry {:?} to the next map",
                path.display(),
                travel.carry
            ));
        }
        Err(e) => log.line(format!(
            "save: ERROR in {}: {e}; carrying {:?}",
            path.display(),
            travel.carry
        )),
    }
}

/// Back in the menu with a load still waiting means its map didn't load.
fn forget_pending(
    mut commands: Commands,
    pending: Option<Res<PendingLoad>>,
    travel: Option<Res<PendingTravel>>,
    log: Res<GameLog>,
) {
    if travel.is_some() {
        log.line("save: gave up going on to the next map (it didn't load)");
        commands.remove_resource::<PendingTravel>();
        commands.remove_resource::<ArrivalPortal>();
    }
    if let Some(p) = pending {
        log.line(format!(
            "save: gave up loading {} (its map {} didn't load)",
            p.slot.label(),
            p.file.header.map
        ));
        commands.remove_resource::<PendingLoad>();
        commands.remove_resource::<RestoringSave>();
    }
}

/// A map was entered. Unless a save is about to be put back, it starts
/// from scratch: no checkpoints yet, no time played, and the start is saved
/// as the first checkpoint.
fn level_started(
    restoring: Option<Res<RestoringSave>>,
    mut checkpoints: ResMut<Checkpoints>,
    mut time: ResMut<PlayTime>,
    mut requests: ResMut<Requests>,
    log: Res<GameLog>,
) {
    if restoring.is_some() {
        return;
    }
    *checkpoints = Checkpoints {
        reached: vec![LEVEL_START.into()],
    };
    time.0 = 0.0;
    log.line("save: new level, saving the start as a checkpoint");
    requests.0.push(Request::Save {
        slot: SaveSlot::Checkpoint,
        checkpoint: Some(LEVEL_START.into()),
    });
}

fn count_play_time(real: Res<Time>, mut time: ResMut<PlayTime>) {
    time.0 += real.delta_secs_f64();
}

#[allow(clippy::too_many_arguments)]
fn gather(
    actions: Res<ActionState>,
    state: Res<State<GameState>>,
    restoring: Option<Res<RestoringSave>>,
    mut saves: MessageReader<SaveGame>,
    mut loads: MessageReader<LoadGame>,
    mut reached: MessageReader<CheckpointReached>,
    mut travel: MessageReader<ChangeLevel>,
    mut checkpoints: ResMut<Checkpoints>,
    mut requests: ResMut<Requests>,
    log: Res<GameLog>,
) {
    let in_game = *state.get() == GameState::InGame;
    if in_game && actions.just_pressed(Action::QuickSave) {
        requests.0.push(Request::Save {
            slot: SaveSlot::Quick,
            checkpoint: None,
        });
    }
    if in_game && actions.just_pressed(Action::QuickLoad) {
        requests.0.push(Request::Load(SaveSlot::Quick));
    }
    for s in saves.read() {
        requests.0.push(Request::Save {
            slot: s.slot.clone(),
            checkpoint: None,
        });
    }
    for c in reached.read() {
        if !in_game || restoring.is_some() {
            log.line(format!(
                "save: checkpoint {:?} ignored (not playing)",
                c.name
            ));
        } else if checkpoints.reached.contains(&c.name) {
            log.line(format!("save: checkpoint {:?} already reached", c.name));
        } else {
            log.line(format!("save: checkpoint {:?} reached", c.name));
            checkpoints.reached.push(c.name.clone());
            requests.0.push(Request::Save {
                slot: SaveSlot::Checkpoint,
                checkpoint: Some(c.name.clone()),
            });
        }
    }
    for l in loads.read() {
        requests.0.push(Request::Load(l.slot.clone()));
    }
    for c in travel.read() {
        if in_game && restoring.is_none() {
            requests.0.push(Request::Travel(c.clone()));
        } else {
            log.line(format!("save: leaving for {} ignored (not playing)", c.map));
        }
    }
}

/// Saves and loads need the whole game at once, so this has the world to
/// itself.
fn run_requests(world: &mut World) {
    let requests = std::mem::take(&mut world.resource_mut::<Requests>().0);
    for r in requests {
        match r {
            Request::Save { slot, checkpoint } => {
                let outcome = save_now(world, &slot, checkpoint);
                finish(world, slot, false, outcome);
            }
            Request::Load(slot) => {
                let outcome = start_load(world, &slot);
                let failed = outcome.is_err();
                finish(world, slot, true, outcome);
                // Once a load starts, the rest of this frame's requests
                // belong to the game being left.
                if !failed {
                    break;
                }
            }
            Request::Travel(c) => match start_travel(world, &c) {
                Ok(()) => break,
                Err(e) => {
                    world
                        .resource::<GameLog>()
                        .line(format!("save: FAILED to go on to {}: {e}", c.map));
                    world.write_message(HudMessage {
                        text: format!("Couldn't go on to {}: {e}", c.map),
                        seconds: 3.0,
                    });
                }
            },
        }
    }
}

fn finish(world: &mut World, slot: SaveSlot, loading: bool, outcome: Result<(), String>) {
    let text = match (&outcome, loading) {
        (Ok(()), false) if slot == SaveSlot::Checkpoint => Some("Checkpoint".to_string()),
        (Ok(()), false) => Some("Game saved".to_string()),
        (Ok(()), true) => None, // said once the load has finished
        (Err(e), false) => Some(format!("Couldn't save: {e}")),
        (Err(e), true) => Some(format!("Couldn't load: {e}")),
    };
    if let Err(e) = &outcome {
        world.resource::<GameLog>().line(format!(
            "save: FAILED to {} {}: {e}",
            if loading { "load" } else { "save" },
            slot.label()
        ));
    }
    if let Some(text) = text {
        world.write_message(HudMessage { text, seconds: 3.0 });
    }
    world.write_message(SaveResult {
        slot,
        loading,
        outcome,
    });
}

fn folder(world: &World) -> PathBuf {
    world
        .get_resource::<SaveFolder>()
        .map(|f| f.0.clone())
        .unwrap_or_else(|| file::default_folder(file::System::this(), |k| std::env::var(k).ok()))
}

/// Gather every part's section and write the file.
pub fn collect(world: &mut World, checkpoint: Option<String>) -> Result<SaveFile, String> {
    let map = world
        .resource::<CurrentLevel>()
        .map
        .clone()
        .ok_or("no map is loaded")?;
    let parts = world
        .get_resource::<SaveParts>()
        .cloned()
        .unwrap_or_default();
    let mut sections = toml::Table::new();
    for p in &parts.0 {
        match (p.save)(world) {
            Ok(Some(t)) => {
                sections.insert(p.key.to_string(), toml::Value::Table(t));
            }
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }
    let saved_at = file::now();
    let checkpoint = checkpoint.or_else(|| {
        world
            .get_resource::<Checkpoints>()
            .and_then(|c| c.last().map(String::from))
    });
    Ok(SaveFile {
        header: Header {
            format: file::FORMAT,
            game_version: env!("CARGO_PKG_VERSION").into(),
            map,
            checkpoint,
            saved_at,
            saved_at_text: file::time_text(saved_at),
            play_seconds: world.get_resource::<PlayTime>().map_or(0.0, |t| t.0),
        },
        parts: sections,
    })
}

fn save_now(world: &mut World, slot: &SaveSlot, checkpoint: Option<String>) -> Result<(), String> {
    if *world.resource::<State<GameState>>().get() != GameState::InGame {
        return Err("you can only save during a level".into());
    }
    if world.contains_resource::<PendingLoad>() {
        return Err("a save is still being loaded".into());
    }
    let started = std::time::Instant::now();
    let save = collect(world, checkpoint)?;
    let order: Vec<&str> = world
        .get_resource::<SaveParts>()
        .map(|p| p.keys().collect())
        .unwrap_or_default();
    let text = save.to_text(&order)?;
    let path = file::slot_path(&folder(world), slot);
    file::write_safely(&path, &text)?;
    let log = world.resource::<GameLog>();
    log.line(format!(
        "save: saved {} to {} ({} bytes, {:.1} ms): map {}, checkpoint {:?}, {:.1} s played",
        slot.label(),
        path.display(),
        text.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        save.header.map,
        save.header.checkpoint,
        save.header.play_seconds
    ));
    for line in summary(&save) {
        log.line(format!("save:   {line}"));
    }
    let info = save.info(slot.clone());
    let mut list = world.resource_mut::<SaveList>();
    list.0.retain(|s| &s.slot != slot);
    list.0.insert(0, info);
    Ok(())
}

/// Read a save and start loading its map.
fn start_load(world: &mut World, slot: &SaveSlot) -> Result<(), String> {
    match world.resource::<State<GameState>>().get() {
        GameState::MainMenu | GameState::InGame => {}
        _ => return Err("the game is busy loading".into()),
    }
    let path = file::slot_path(&folder(world), slot);
    if !path.exists() {
        return Err(format!("there is no {}", slot.label()));
    }
    let save = file::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    world.resource::<GameLog>().line(format!(
        "save: loading {} from {}: map {}, checkpoint {:?}, saved {}",
        slot.label(),
        path.display(),
        save.header.map,
        save.header.checkpoint,
        file::time_text(save.header.saved_at)
    ));
    world.resource_mut::<CurrentLevel>().map = Some(save.header.map.clone());
    world.insert_resource(RestoringSave { slot: slot.clone() });
    world.insert_resource(PendingLoad {
        slot: slot.clone(),
        file: save,
    });
    world
        .resource_mut::<NextState<GameState>>()
        .set(GameState::Loading);
    Ok(())
}

/// Leave this map for `c.map`: keep the sections that travel, then load
/// the map fresh, as the menu would.
fn start_travel(world: &mut World, c: &ChangeLevel) -> Result<(), String> {
    if *world.resource::<State<GameState>>().get() != GameState::InGame {
        return Err("not in a level".into());
    }
    let from = world
        .resource::<CurrentLevel>()
        .map
        .clone()
        .unwrap_or_default();
    let carry = world.resource::<TravelSettings>().carry.clone();
    let parts = world
        .get_resource::<SaveParts>()
        .cloned()
        .unwrap_or_default();
    let mut sections = toml::Table::new();
    for key in &carry {
        let Some(p) = parts.0.iter().find(|p| p.key == key) else {
            world.resource::<GameLog>().line(format!(
                "save: {TRAVEL_FILE} carries [{key}], but no part saves that; skipped"
            ));
            continue;
        };
        if let Some(t) = (p.save)(world)? {
            sections.insert(key.clone(), toml::Value::Table(t));
        }
    }
    let carried = SaveFile {
        header: Header {
            format: file::FORMAT,
            game_version: env!("CARGO_PKG_VERSION").into(),
            map: from.clone(),
            checkpoint: None,
            saved_at: 0,
            saved_at_text: String::new(),
            play_seconds: 0.0,
        },
        parts: sections.clone(),
    };
    let log = world.resource::<GameLog>();
    log.line(format!(
        "save: leaving {from} for {}{} (sent by {}), carrying {:?}",
        c.map,
        c.portal
            .as_ref()
            .map(|p| format!(", arriving at {p}"))
            .unwrap_or_default(),
        c.by,
        sections.keys().collect::<Vec<_>>()
    ));
    for line in summary(&carried) {
        log.line(format!("save:   {line}"));
    }
    world.insert_resource(PendingTravel { from, sections });
    match &c.portal {
        Some(p) => world.insert_resource(ArrivalPortal(p.clone())),
        None => {
            world.remove_resource::<ArrivalPortal>();
        }
    }
    world.resource_mut::<CurrentLevel>().map = Some(c.map.clone());
    world
        .resource_mut::<NextState<GameState>>()
        .set(GameState::Loading);
    Ok(())
}

/// The next map is loaded: put back what the player carried. Runs on its
/// first frame, before the level-start checkpoint is written, so that
/// checkpoint has it too.
fn arrive(world: &mut World) {
    let Some(travel) = world.remove_resource::<PendingTravel>() else {
        return;
    };
    let parts = world
        .get_resource::<SaveParts>()
        .cloned()
        .unwrap_or_default();
    let map = world
        .resource::<CurrentLevel>()
        .map
        .clone()
        .unwrap_or_default();
    for (key, value) in &travel.sections {
        let Some(p) = parts.0.iter().find(|p| p.key == key) else {
            continue;
        };
        let table = value.as_table().cloned();
        match (p.load)(world, table) {
            Ok(()) => world
                .resource::<GameLog>()
                .line(format!("save: [{key}] carried from {} to {map}", travel.from)),
            Err(e) => world.resource::<GameLog>().line(format!(
                "save: ERROR carrying [{key}] from {} to {map}: {e}",
                travel.from
            )),
        }
    }
}

/// The save's map is loaded: hand every part its section.
fn apply_pending(world: &mut World) {
    let Some(pending) = world.remove_resource::<PendingLoad>() else {
        return;
    };
    let parts = world
        .get_resource::<SaveParts>()
        .cloned()
        .unwrap_or_default();
    let mut problems = Vec::new();
    for p in &parts.0 {
        let section = pending.file.part(p.key);
        let had = section.is_some();
        match (p.load)(world, section) {
            Ok(()) => world.resource::<GameLog>().line(format!(
                "save: loaded [{}]{}",
                p.key,
                if had {
                    ""
                } else {
                    " (not in the file; left as the level starts)"
                }
            )),
            Err(e) => {
                world
                    .resource::<GameLog>()
                    .line(format!("save: ERROR loading [{}]: {e}", p.key));
                problems.push(p.key);
            }
        }
    }
    for k in pending.file.parts.keys() {
        if !parts.keys().any(|p| p == k) {
            world.resource::<GameLog>().line(format!(
                "save: [{k}] in the file belongs to no part in this game; ignored"
            ));
        }
    }
    world.resource_mut::<PlayTime>().0 = pending.file.header.play_seconds;
    world.remove_resource::<RestoringSave>();
    let log = world.resource::<GameLog>();
    log.line(format!(
        "save: loaded {} ({} parts, {} problems)",
        pending.slot.label(),
        parts.0.len(),
        problems.len()
    ));
    for line in summary(&pending.file) {
        log.line(format!("save:   {line}"));
    }
    let text = if problems.is_empty() {
        format!("Loaded {}", pending.slot.label())
    } else {
        format!(
            "Loaded {} (some parts didn't: {})",
            pending.slot.label(),
            problems.join(", ")
        )
    };
    world.write_message(HudMessage { text, seconds: 3.0 });
}

/// One line per section: its name and its values, flattened and shortened,
/// for the log.
pub fn summary(save: &SaveFile) -> Vec<String> {
    save.parts
        .iter()
        .map(|(k, v)| {
            let mut s = toml::to_string(v)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            if s.len() > 300 {
                let mut cut = 300;
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                s.truncate(cut);
                s.push_str(" ...");
            }
            format!("[{k}] {s}")
        })
        .collect()
}

#[cfg(test)]
mod tests;
