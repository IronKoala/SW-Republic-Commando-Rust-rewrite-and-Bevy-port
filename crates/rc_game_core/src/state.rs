//! Game screens, loading, and the player's place in the world.

use bevy::prelude::*;
use rc_movement::World;
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

/// Run condition for gameplay systems: in a level and not paused.
///
/// `app.add_systems(Update, my_system.in_set(GameSet::Simulate).run_if(playing))`
pub fn playing(play: Option<Res<State<Play>>>) -> bool {
    play.is_some_and(|p| *p.get() == Play::Running)
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

/// The player's body. Its `Transform` is in Bevy space (see `units`), at the
/// feet, turned to face where the player faces (yaw only).
#[derive(Component, Debug, Default)]
pub struct Player;

/// The camera that shows the player's view. Weapons in first person and the
/// on-screen display attach to this. A VR mod replaces this camera, not the
/// gameplay around it.
#[derive(Component, Debug, Default)]
pub struct PlayerCamera;
