//! The level's events, shared by the parts that send and hear them.
//!
//! Unreal Engine 2 levels are tied together by named events: an actor
//! "triggers" a name and every actor whose `Tag` is that name hears it (see
//! `rc_scripts`). The level scripts part (`rc_scripts_plugin`) keeps the
//! list: it sends every event as a [`ScriptEvent`], and any other part
//! that has its own actors sending events (droids dying, spawners running
//! out) writes a [`RaiseEvent`], which the scripts pass on to everyone.
//! Script actions aimed at enemy spawners come out as [`FactoryOrder`]s.

use bevy::prelude::*;

/// An event sent in the level, by name (`levelstart`,
/// `SpawnBarricadeDroids`, `BactaUsed`, ...). Read it with a
/// `MessageReader` to hook a part of the game to the map's events.
#[derive(Message, Debug, Clone)]
pub struct ScriptEvent {
    pub name: String,
    /// What sent it (a script, a trigger, `level start`).
    pub by: String,
}

/// Send an event into the level from outside the scripts: a droid's death
/// event, a spawner's "all done" event. The scripts hear it and pass it on
/// as a [`ScriptEvent`] to every part.
#[derive(Message, Debug, Clone)]
pub struct RaiseEvent {
    pub name: String,
    /// What sent it, for the log.
    pub by: String,
}

/// Present while the level's scripts run: parts that would otherwise
/// start their actors themselves (droid spawners) wait for the scripts'
/// events instead.
#[derive(Resource, Debug, Default)]
pub struct LevelScriptsRun;

/// A script's order to the enemy spawners tagged `tag` (`rc_enemies`
/// carries it out); sent by the level scripts part.
#[derive(Message, Debug, Clone, PartialEq)]
pub enum FactoryOrder {
    /// `ACTION_StopFactorySpawning`: switch off; for good (events no
    /// longer switch it on) when `for_good`; remove what it made (no death
    /// events) when `remove_offspring`.
    Stop {
        tag: String,
        for_good: bool,
        remove_offspring: bool,
    },
    /// `ACTION_ResetFactory`: its whole capacity is available again.
    Reset { tag: String },
}

/// Leave this map for another one: a level exit was reached (a
/// `Teleporter` with a `URL`, `ACTION_ChangeLevel`, or the end-of-campaign
/// debriefing's `NewLevel`). Sent by the level scripts part; the save part
/// (`rc_save`) carries the player's state over, loads `map` and saves the
/// new map's start as a checkpoint.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ChangeLevel {
    /// The map, as in `GameData/Maps` (`geo_01b`).
    pub map: String,
    /// Where on the new map to arrive: the `Tag` of a `Teleporter` there
    /// (`FromA`). `rc_scripts::level_url` reads both out of the map's
    /// address (`RAS_03B.ctm#FromA`). `None`, or a name no teleporter has (`playerstart`),
    /// means the map's player start.
    pub portal: Option<String>,
    /// What sent it, for the log.
    pub by: String,
}

/// Where the player should arrive on the map being loaded: a `Teleporter`'s
/// `Tag` (see [`ChangeLevel::portal`]). Put in by the save part before
/// loading; the level part reads it, uses that teleporter's place as the
/// player start if there is one, and removes it.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ArrivalPortal(pub String);
