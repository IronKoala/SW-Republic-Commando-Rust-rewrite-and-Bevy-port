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

/// True while the player stands at something the Use button works on
/// (a bacta dispenser that is switched on). The Use button and the squad's
/// context order share a key (F, as in the original), so the squad leaves
/// that press alone: it uses the thing, not orders the squad.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct UseClaimed(pub bool);

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

/// A script switched the collision of the actors tagged `tag` on or off
/// (`ACTION_SetCollision`, `bCollideActors`). In Unreal an actor that
/// doesn't collide with actors is passed through by shots and traces, so
/// a character switched off can't be hit (and isn't aimed at). Sent by the
/// level scripts part for every such action that isn't a level exit; the
/// parts that own characters apply it to theirs with that tag.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CharacterCollision {
    pub tag: String,
    pub on: bool,
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
/// A level script's `ACTION_ChangeSquadMember`: put the squad mate
/// `target` in the player's squad (`join`) or take them out of it. Out of
/// the squad they stop following the player and do what the level's
/// scripts tell them ([`CharacterOrder`]s); the squad part
/// (`rc_squad_plugin`) carries it out.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SquadMembership {
    pub target: Entity,
    pub join: bool,
    /// The script and action, for the log.
    pub by: String,
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

/// Show the end-of-campaign debriefing screen, then leave for the next
/// map. Sent by the level scripts part for `ACTION_ShowDebriefingScreen`
/// with a `NewLevel`; the game (`rc_game`) pauses play, shows the screen
/// drawn from the original's own menu class, and sends `then` when the
/// player chooses CONTINUE. A program with no screen to show it should
/// send `then` at once.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ShowDebriefing {
    /// The map's menu class (`XInterfaceCTMenus.CTDebriefingInfoGEO`).
    pub menu_class: String,
    pub title: String,
    pub text: String,
    /// The picture's object path (`GUIContent.GEO_Debrief`).
    pub picture: String,
    /// Where to go on CONTINUE.
    pub then: ChangeLevel,
}

/// Where the player should arrive on the map being loaded: a `Teleporter`'s
/// `Tag` (see [`ChangeLevel::portal`]). Put in by the save part before
/// loading; the level part reads it, uses that teleporter's place as the
/// player start if there is one, and removes it.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ArrivalPortal(pub String);

/// How far a job at a squad marker has got (see [`MarkerJob`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerJobStep {
    /// Someone started on it: placing a charge, slicing a console or a
    /// door, or taking up a sniper post. The anchor's `EventUnderway`.
    Started,
    /// A proximity mine is set on its barrel (not armed yet): the squad
    /// marker's `PlaceEvent`.
    Placed,
    /// A charge is set and its fuse is burning (a demolition charge's
    /// `ChargeArmed`), or a proximity mine is armed (its marker's
    /// `ArmEvent`).
    Armed,
    /// Finished: the charge went off, the console or door is sliced, the
    /// door is breached.
    Done,
}

/// A job at one of the map's squad markers moved on. `anchor` is the map
/// actor the job is for (`DemolitionCharge2`, `HackTerminalGeo0`,
/// `BreachDoorGeo2`). The squad part sends it; the level scripts part
/// sends the anchor's own map events (`ChargeExplode.Event`,
/// `HackedState.Event`, ...), opens the doors it locks and removes what
/// it blows up. Anything else that does such a job (the player, later)
/// sends it the same way.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct MarkerJob {
    pub anchor: String,
    pub step: MarkerJobStep,
    /// Who did it, for the log.
    pub by: String,
}

/// The level took away a squad marker's anchor (`anchor`, a map actor
/// name such as `ProximityMine16`): its markers are no longer offered and
/// a squad mate on the way to one stops. Sent by the level scripts part,
/// read by the squad part.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct AnchorGone {
    pub anchor: String,
    /// Why, for the log.
    pub why: String,
}

/// Play animation `anim` (the model's own sequence name, such as `Load`)
/// on the map actor called `actor` (`BactaDispenserGEO0`), for props drawn
/// with an animated model (`rc_level_plugin::anim_props`). The animation's
/// own sounds play with it. Sent by whatever uses the prop (the bacta
/// dispenser when the player starts and stops healing).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct PropAnim {
    pub actor: String,
    pub anim: String,
}
