//! Doors and lifts in the game. `MoversPlugin` takes the map's movers that
//! the level part read (`rc_level_plugin::LevelMoverData`) and runs them
//! with `rc_movers`, the same code the walker (`rc_walk --ride`) and the
//! check tool (`rc_mover_check`) use:
//!
//! - it swaps `LevelCollision` for the level plus the movers where they are
//!   now, so doors block and lifts hold up everything that moves or shoots;
//! - [`LevelMovers::step`] moves the movers, carries the player on a lift
//!   and pushes them out of a closing door. The player's movement calls it
//!   just before its own step, because a mover moves first in the original
//!   too (UE2 ticks movers before pawns);
//! - the drawn parts of each mover (`MoverPart`) follow it;
//! - everything that happens (a door starts opening, a trigger fires, the
//!   player boards a lift) is logged with a `movers:` prefix and sent as a
//!   [`MoverHappened`] message, for sounds and level scripts.
//!
//! Other parts open doors by event name with [`LevelMovers::send`], the way
//! the original's scripts do (`TriggerEvent` with the door's `Tag`).
//!
//! Where every door and lift is goes into save files (`[movers]`, see
//! `save.rs`), so a load puts them back.
//!
//! Our own choices (how close counts as a bump, what each `InitialState`
//! does) are in the data file `movers/movers.toml`.

pub mod save;

use bevy::prelude::*;
use rc_game_core::save::SaveAppExt;
use rc_game_core::{
    units::to_bevy, GameLog, GamePaths, GameSet, GameState, LevelCollision, LoadingGate,
};
use rc_level_plugin::LevelMoverData;
use rc_level_viewer::draw::MoverPart;
use rc_movement::{Hit, Pawn, Vec3 as UVec3, World as MoveWorld};
use rc_movers::{MoverEvent, MoverSet, Settings};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

pub use rc_movers::MoverEvent as Happened;

/// The name this part uses in the loading gate and the log.
pub const PART: &str = "movers";

/// The settings file in the data folders. Optional: the built-in values
/// are the same as the file we ship.
pub const SETTINGS_FILE: &str = "movers/movers.toml";

pub struct MoversPlugin;

/// Moving the movers' drawn parts. After the player has moved, before the
/// camera is placed.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MoverSystems;

impl Plugin for MoversPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MoverHappened>()
            .add_save_part::<save::MoversSave>()
            .add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(Update, build.run_if(in_state(GameState::Loading)))
            .add_systems(
                Update,
                (report, place_parts)
                    .chain()
                    .in_set(MoverSystems)
                    .in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

/// Insert this (in the app, before the game starts) when another part runs
/// the map's triggers and sends their events with [`LevelMovers::send`] and
/// [`LevelMovers::release`]. The level scripts part does. Without it, the
/// movers run the triggers that only open doors themselves.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct TriggersRunElsewhere;

/// Something a door, lift or trigger did, with the movers' clock (seconds
/// since the level started). Read it with a `MessageReader`.
#[derive(Message, Debug, Clone)]
pub struct MoverHappened {
    pub time: f32,
    pub event: MoverEvent,
    /// The mover's (or trigger's) name in the map, e.g. `Mover37`.
    pub name: String,
}

struct Shared {
    set: RwLock<MoverSet>,
    /// The level without the movers.
    fixed: Arc<dyn MoveWorld + Send + Sync>,
    /// Events since `report` last ran.
    pending: Mutex<Vec<(f32, MoverEvent)>>,
}

/// The map's doors and lifts while a level is played. Cheap to clone.
#[derive(Resource, Clone)]
pub struct LevelMovers {
    shared: Arc<Shared>,
    /// Which mover each map export index is (for the drawn parts).
    by_export: Arc<HashMap<usize, usize>>,
}

impl LevelMovers {
    pub fn new(set: MoverSet, fixed: Arc<dyn MoveWorld + Send + Sync>) -> Self {
        let by_export = set
            .movers
            .iter()
            .enumerate()
            .map(|(i, m)| (m.info.place.export, i))
            .collect();
        LevelMovers {
            shared: Arc::new(Shared {
                set: RwLock::new(set),
                fixed,
                pending: Mutex::new(Vec::new()),
            }),
            by_export: Arc::new(by_export),
        }
    }

    /// Move every mover by `dt` seconds. A lift carries `player`, a closing
    /// door pushes it, and a trigger it stands in goes off. Call this before
    /// the player's own move, which then moves in [`Self::world`].
    pub fn step(&self, dt: f32, player: &mut Pawn) {
        let mut set = self.shared.set.write().unwrap_or_else(|e| e.into_inner());
        let events = set.step(dt, self.shared.fixed.as_ref(), player);
        let t = set.time;
        self.queue(t, events);
    }

    /// Send an event to every mover whose `Tag` is that name, as the
    /// original's `TriggerEvent` does. Returns how many movers got it.
    pub fn send(&self, event: &str) -> usize {
        let mut set = self.shared.set.write().unwrap_or_else(|e| e.into_inner());
        let mut events = Vec::new();
        let n = set.send(event, &mut events);
        let t = set.time;
        self.queue(t, events);
        n
    }

    /// The event is over (the player left the trigger that sent it), as
    /// the original's `UntriggerEvent`: movers that open only while set off
    /// close again.
    pub fn release(&self, event: &str) {
        let mut set = self.shared.set.write().unwrap_or_else(|e| e.into_inner());
        let mut events = Vec::new();
        set.release(event, &mut events);
        let t = set.time;
        self.queue(t, events);
    }

    fn queue(&self, t: f32, events: Vec<MoverEvent>) {
        if events.is_empty() {
            return;
        }
        let mut p = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        p.extend(events.into_iter().map(|e| (t, e)));
    }

    /// Put the movers back as a saved game had them (see
    /// [`MoverSet::restore`]).
    pub fn restore(&self, snap: &rc_movers::SetSnapshot) -> rc_movers::RestoreReport {
        let mut set = self.shared.set.write().unwrap_or_else(|e| e.into_inner());
        set.restore(snap)
    }

    /// The movers as they are now (for reading; don't hold on to it).
    pub fn read(&self) -> RwLockReadGuard<'_, MoverSet> {
        self.shared.set.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The level plus the movers where they are now: what `LevelCollision`
    /// holds while a level is played.
    pub fn world(&self) -> Arc<dyn MoveWorld + Send + Sync> {
        Arc::new(WithMovers(self.shared.clone()))
    }
}

/// The level and the movers together, for sweeps.
struct WithMovers(Arc<Shared>);

impl MoveWorld for WithMovers {
    fn sweep(&self, extent: UVec3, start: UVec3, end: UVec3) -> Option<Hit> {
        let set = self.0.set.read().unwrap_or_else(|e| e.into_inner());
        set.world(self.0.fixed.as_ref()).sweep(extent, start, end)
    }
}

fn begin(mut gate: ResMut<LoadingGate>) {
    gate.wait(PART);
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelMovers>();
}

fn load_settings(paths: &GamePaths, log: &GameLog) -> Settings {
    let Some(path) = paths.data_file(SETTINGS_FILE) else {
        log.line(format!(
            "{PART}: no {SETTINGS_FILE}; built-in door and lift settings"
        ));
        return Settings::default();
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| Settings::default().with_overrides(&t))
    {
        Ok(s) => {
            log.line(format!("{PART}: settings from {}", path.display()));
            s
        }
        Err(e) => {
            log.line(format!(
                "{PART}: ERROR in {}: {e}; built-in door and lift settings",
                path.display()
            ));
            Settings::default()
        }
    }
}

/// Once the level part has loaded the map, build its movers and make them
/// part of the level's collision.
fn build(
    mut commands: Commands,
    data: Option<Res<LevelMoverData>>,
    collision: Option<Res<LevelCollision>>,
    paths: Res<GamePaths>,
    elsewhere: Option<Res<TriggersRunElsewhere>>,
    mut gate: ResMut<LoadingGate>,
    log: Res<GameLog>,
) {
    let (Some(data), Some(collision)) = (data, collision) else {
        return;
    };
    if !gate.waiting().any(|p| p == PART) {
        return;
    }
    let settings = load_settings(&paths, &log);
    let v = |p: [f32; 3]| UVec3::new(p[0], p[1], p[2]);
    let mut map = data.movers.clone();
    if elsewhere.is_some() {
        log.line(format!(
            "{PART}: the level scripts run the map's {} triggers",
            map.triggers.len()
        ));
        map.triggers.clear();
    }
    let set = MoverSet::new(map, settings, |e| {
        data.triangles
            .get(&e)
            .map(|ts| ts.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect())
            .unwrap_or_default()
    });
    let waiting_for_scripts = set
        .movers
        .iter()
        .filter(|m| m.behaviour == rc_movers::Behaviour::Static && !m.info.tag.is_empty())
        .count();
    log.line(format!(
        "{PART}: {} movers ({} solid), {} triggers that run, {} that don't yet, {} unreadable; {} wait for an event by tag",
        set.movers.len(),
        set.movers.iter().filter(|m| m.collision.is_some()).count(),
        set.active_triggers().count(),
        set.unhandled_triggers.len(),
        data.movers.failures.len(),
        waiting_for_scripts
    ));
    for (i, m) in set.movers.iter().enumerate() {
        log.line(format!(
            "{PART}:   {i} {} ({}): {} keys, opens in {:.2} s, state {:?} -> {}, tag {:?}, at ({:.0}, {:.0}, {:.0})",
            m.info.place.name,
            m.info.chain.first().map_or("?", |s| s.as_str()),
            m.info.num_keys,
            m.info.open_time(),
            m.info.state,
            m.behaviour.name(),
            m.info.tag,
            m.pose.pos.x,
            m.pose.pos.y,
            m.pose.pos.z
        ));
    }
    for t in &set.unhandled_triggers {
        log.line(format!("{PART}:   trigger not run yet: {t}"));
    }
    let movers = LevelMovers::new(set, collision.0.clone());
    commands.insert_resource(LevelCollision(movers.world()));
    commands.insert_resource(movers);
    gate.done(PART);
}

/// Log what the movers did since last frame and pass it on.
fn report(
    movers: Option<Res<LevelMovers>>,
    mut out: MessageWriter<MoverHappened>,
    log: Res<GameLog>,
) {
    let Some(movers) = movers else { return };
    let events: Vec<(f32, MoverEvent)> = {
        let mut p = movers
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *p)
    };
    if events.is_empty() {
        return;
    }
    let set = movers.read();
    let name = |i: usize| {
        set.movers
            .get(i)
            .map_or("?".to_string(), |m| m.info.place.name.clone())
    };
    for (t, e) in events {
        let (who, text) = match &e {
            MoverEvent::StartedOpening { mover } => (name(*mover), "starts opening".to_string()),
            MoverEvent::Opened { mover, secs } => (name(*mover), format!("open after {secs:.2} s")),
            MoverEvent::StartedClosing { mover } => (name(*mover), "starts closing".to_string()),
            MoverEvent::Closed { mover, secs } => {
                (name(*mover), format!("closed after {secs:.2} s"))
            }
            MoverEvent::Blocked { mover, what } => {
                (name(*mover), format!("blocked by the player ({what:?})"))
            }
            MoverEvent::Sound { mover, sound } => {
                (name(*mover), format!("sound {sound} (not played yet)"))
            }
            MoverEvent::Triggered {
                trigger,
                event,
                movers,
            } => (
                format!("trigger {trigger}"),
                format!("sends {event:?} to {movers} movers"),
            ),
            MoverEvent::Event {
                mover,
                event,
                movers,
            } => (name(*mover), format!("sends {event:?} to {movers} movers")),
            MoverEvent::Boarded { mover } => (name(*mover), "the player stands on it".to_string()),
            MoverEvent::LeftBoard { mover } => (name(*mover), "the player steps off".to_string()),
        };
        log.line(format!("{PART}: t={t:.2} {who} {text}"));
        out.write(MoverHappened {
            time: t,
            event: e,
            name: who,
        });
    }
}

/// Move each mover's drawn triangles to where the mover is now. Unreal to
/// Bevy swaps Y and Z and scales, so the turn's rows and columns 1 and 2
/// swap too (as in `rc_walk`).
fn place_parts(movers: Option<Res<LevelMovers>>, mut parts: Query<(&MoverPart, &mut Transform)>) {
    let Some(movers) = movers else { return };
    let set = movers.read();
    for (part, mut t) in &mut parts {
        let Some(&i) = movers.by_export.get(&part.0) else {
            continue;
        };
        let (m, shift) = set.movers[i].motion().matrix();
        let s = [0usize, 2, 1];
        let col = |c: usize| Vec3::new(m[s[0]][s[c]], m[s[1]][s[c]], m[s[2]][s[c]]);
        let rot = Quat::from_mat3(&Mat3::from_cols(col(0), col(1), col(2))).normalize();
        let pos = to_bevy(shift);
        if t.rotation != rot || t.translation != pos {
            t.rotation = rot;
            t.translation = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_movement::{MoveParams, TriangleWorld};

    /// With no movers, the combined world is the level itself.
    #[test]
    fn empty_set_is_just_the_level() {
        let floor = TriangleWorld::new(vec![[
            UVec3::new(-1000.0, -1000.0, 0.0),
            UVec3::new(1000.0, -1000.0, 0.0),
            UVec3::new(0.0, 1000.0, 0.0),
        ]]);
        let set = MoverSet::new(Default::default(), Settings::default(), |_| Vec::new());
        let movers = LevelMovers::new(set, Arc::new(floor));
        let w = movers.world();
        let ext = UVec3::new(10.0, 10.0, 10.0);
        let hit = w.sweep(
            ext,
            UVec3::new(0.0, 0.0, 100.0),
            UVec3::new(0.0, 0.0, -100.0),
        );
        assert!(hit.is_some_and(|h| h.time > 0.4 && h.time < 0.5), "{hit:?}");
        let mut pawn = Pawn::new(MoveParams::player_commando(), UVec3::new(0.0, 0.0, 0.0));
        movers.step(0.1, &mut pawn);
        assert_eq!(movers.send("Nothing"), 0);
        assert!((movers.read().time - 0.1).abs() < 1e-6);
    }
}
