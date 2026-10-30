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

/// What a character answers to in the level's scripts. Put it on every
/// character's entity (squad mates, enemies; the player is found by
/// `Player`). `tag` is the name scripts aim actions at (`PawnTag`, or a
/// squad mate's class like `Commando62Delta`); `script` is the
/// `ScriptedSequence` it follows (`AIScriptTag` / `AIScriptFromFactory`):
/// that sequence's own actions act on it.
#[derive(Component, Debug, Clone, Default)]
pub struct ScriptTags {
    pub tag: Option<String>,
    pub script: Option<String>,
    /// Its part carries out [`CharacterOrder`]s (walks it, plays its
    /// animations). Characters without it still take what needs no part
    /// of their own (damage, can't-be-hurt).
    pub follows_orders: bool,
}

/// A level script's order to one character (from the level scripts part;
/// the part that owns the character carries it out). Positions are Unreal
/// units, angles radians (Unreal's yaw convention).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CharacterOrder {
    pub target: Entity,
    pub act: CharacterAct,
    /// The script and action it comes from. When `waits`, the script waits
    /// for a [`CharacterOrderDone`] naming them.
    pub script: String,
    pub index: usize,
    pub waits: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CharacterAct {
    /// `ACTION_MoveToPoint`: walk (or run) to `to`; turn to `face` there
    /// when given (`OrientToPoint`).
    MoveTo {
        to: [f32; 3],
        walk: bool,
        face: Option<f32>,
    },
    /// `ACTION_MoveToPlayer`: go until within `tolerance` uu of the player.
    MoveToPlayer { tolerance: f32 },
    /// `ACTION_TurnTowardPlayer`.
    TurnToPlayer,
    /// `ACTION_TeleportToPoint`: be at `to` at once, facing `yaw`.
    Teleport { to: [f32; 3], yaw: f32 },
    /// `ACTION_PlayAnim`: play the animation `name` from the character's
    /// own set, looping or once.
    PlayAnim { name: String, looping: bool },
    /// `ACTION_WaitForAnimEnd`: done when the animation playing ends.
    WaitForAnimEnd,
    /// `ACTION_WaitForPlayer`: done when the player is within `distance`.
    WaitForPlayer { distance: f32 },
    /// `ACTION_DestroyPawn` / `ACTION_DestroyActor`: removed from the level
    /// (not killed: no death event).
    Remove,
    /// `ACTION_SetHidden`.
    Hidden(bool),
}

/// A character finished a [`CharacterOrder`] (arrived, turned, the
/// animation ended). The level scripts carry on.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CharacterOrderDone {
    pub script: String,
    pub index: usize,
}

/// A level script's squad order (`ACTION_ChangeSquadStance`), carried out
/// by the squad part like the player's own. Unreal units.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub enum SquadStanceOrder {
    FormUp,
    SearchAndDestroy,
    SecurePosition([f32; 3]),
}
