//! Game screens, loading, and the player's place in the world.

use bevy::prelude::*;
use rc_movement::{Pawn, World};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Which screen the game is on.
///
/// `Boot` reads the settings and checks the game folder, `MainMenu` shows the
/// menus, `Loading` builds the chosen map, `InGame` is play. Anything a
/// plugin spawns for one map should carry `DespawnOnExit(GameState::InGame)`
/// so it is cleared when the player quits to the menu or loads another map.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default]
    Boot,
    MainMenu,
    Loading,
    InGame,
}

/// Whether play is running or paused. Only exists while in `InGame`, so
/// pausing does not count as leaving the game (nothing is despawned).
#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(GameState = GameState::InGame)]
pub enum Play {
    #[default]
    Running,
    Paused,
}

/// The level has loaded but waits for START on its briefing screen (the
/// original doesn't run the level before START). Set by the game's
/// loading screen. Play is `Running` for the first frame in the level
/// before the pause takes hold; this keeps that frame from running the
/// level, so nothing a level script sends in it is lost while paused.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StartHeld(pub bool);

/// Run condition for gameplay systems: in a level, not paused and not
/// waiting for START.
///
/// `app.add_systems(Update, my_system.in_set(GameSet::Simulate).run_if(playing))`
pub fn playing(play: Option<Res<State<Play>>>, held: Option<Res<StartHeld>>) -> bool {
    play.is_some_and(|p| *p.get() == Play::Running) && !held.is_some_and(|h| h.0)
}

/// The order of a frame in `Update`. Put each gameplay system in one of
/// these so input is read before anything moves, and the display shows the
/// frame's final numbers.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameSet {
    /// Devices to `ActionState` (done by the shell's input plugin).
    Input,
    /// Movement, weapons, enemies, squad: everything that changes the game.
    Simulate,
    /// Cameras and anything that only follows what moved.
    View,
    /// The on-screen display reads the readouts.
    Hud,
}

/// The map to load, by name as in `GameData/Maps` (like `geo_01a`), or a
/// full path to a `.ctm` file. Set by the menu (or `--map`) before entering
/// `GameState::Loading`.
#[derive(Resource, Debug, Clone, Default)]
pub struct CurrentLevel {
    pub map: Option<String>,
}

/// How plugins tell the shell that loading is finished.
///
/// On `OnEnter(GameState::Loading)` a plugin that has work to do calls
/// `gate.wait("my part")`, and later `gate.done("my part")` (the same frame
/// is fine). The shell moves to `InGame` once nothing is waiting, at the
/// earliest one frame after loading began, so every plugin's `OnEnter`
/// has run. A plugin with nothing slow to do doesn't need to touch this.
///
/// If something fails, call `gate.fail("my part", reason)`; the shell goes
/// back to the menu and shows the reason.
#[derive(Resource, Debug, Default)]
pub struct LoadingGate {
    waiting: BTreeSet<String>,
    failed: Option<String>,
}

impl LoadingGate {
    pub fn wait(&mut self, part: &str) {
        self.waiting.insert(part.to_string());
    }
    pub fn done(&mut self, part: &str) {
        self.waiting.remove(part);
    }
    pub fn fail(&mut self, part: &str, reason: impl Into<String>) {
        self.waiting.remove(part);
        self.failed = Some(format!("{part}: {}", reason.into()));
    }
    /// The parts still loading, in name order.
    pub fn waiting(&self) -> impl Iterator<Item = &str> {
        self.waiting.iter().map(|s| s.as_str())
    }
    pub fn is_clear(&self) -> bool {
        self.waiting.is_empty()
    }
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }
    /// Start over (the shell does this when loading begins).
    pub fn reset(&mut self) {
        self.waiting.clear();
        self.failed = None;
    }
}

/// The current map's solid surfaces, in Unreal units, for anything that
/// moves or traces a line (the player, shots, droids). Inserted by the
/// level plugin while loading; removed when leaving `InGame`.
#[derive(Resource, Clone)]
pub struct LevelCollision(pub Arc<dyn World + Send + Sync>);

/// Something in the level that moves and carries whoever stands on it
/// (lifts) or pushes whoever is in its way (doors). The doors and lifts
/// part implements it; see [`MovingFloors`].
pub trait CarryBodies: Send + Sync {
    /// Carry or push `pawn` by what the level's movers did this frame, and
    /// say what it stands on (the mover's name in the map), if anything.
    /// Call it once per body per frame, before the body's own move.
    fn carry(&self, pawn: &mut Pawn) -> Option<String>;
}

/// The level's lifts and doors, for every body other than the player to
/// ride and be pushed by (squad mates, droids). Inserted by the doors and
/// lifts part while a level is played; absent on a map without one.
///
/// The player's movement moves the movers (and the player) first each
/// frame; parts that move other bodies run after it
/// (`.after(PlayerMoveSet)` in `rc_game`) and call
/// [`CarryBodies::carry`] for each body before stepping it, against
/// [`LevelCollision`].
#[derive(Resource, Clone)]
pub struct MovingFloors(pub Arc<dyn CarryBodies>);

/// The player's body. Its `Transform` is in Bevy space (see `units`), at the
/// feet, turned to face where the player faces (yaw only).
#[derive(Component, Debug, Default)]
pub struct Player;

/// The player's body as the rules see it, copied from the player part
/// after it moves each frame: the centre of its collision box (the
/// original's `Pawn.Location`, which drops when crouching) in Unreal
/// units, and where it looks (radians, Unreal's yaw and pitch). Parts that
/// need the player's own numbers (the objective tracker's distance) read
/// this instead of the feet in the player's `Transform`.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerPawn {
    pub centre: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// The camera that shows the player's view. Weapons in first person and the
/// on-screen display attach to this. A VR mod replaces this camera, not the
/// gameplay around it.
#[derive(Component, Debug, Default)]
pub struct PlayerCamera;

/// On a camera whose distance fog belongs to something else for now, like
/// the visor's low light mode (the original's vision modes override the
/// zone's fog). The level's zone fog leaves such a camera's fog alone, and
/// takes over again once this is removed (with the fog).
#[derive(Component, Debug, Default)]
pub struct OwnFog;

/// Where the player starts on the current map: feet position in Unreal
/// units, and facing (radians, Unreal's yaw: 0 along +X, a quarter turn
/// along +Y). Inserted by the level plugin while loading, from the map's
/// first PlayerStart. Without it the player starts at the origin.
#[derive(Resource, Debug, Clone, Copy)]
pub struct PlayerStart {
    pub feet: rc_movement::Vec3,
    pub yaw: f32,
}

/// Whether the enemies are fighting (`Battle`) or not (`Calm`). The enemies
/// part sets it. The music no longer reads it: it follows the original's own
/// battle rules from [`crate::MusicThreats`] (where the enemies are).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MusicMood {
    #[default]
    Calm,
    Battle,
}
