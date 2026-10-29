//! rc_game_core: the shared pieces every part of the game plugs into.
//!
//! The game program (`rc_game`) is built from Bevy "plugins": bundles of
//! systems that each add one part of the game (the level, weapons, enemies,
//! the squad, the menus). They don't call each other. They talk through the
//! small set of shared types in this crate:
//!
//! - [`GameState`] and [`Play`]: which screen the game is on, and whether
//!   play is paused. Gameplay systems run only in `Play::Running`.
//! - [`GamePaths`]: where the original game's files are, and where our own
//!   data files and mods are. Nobody reads `RC_GAME_DIR` or a Steam path
//!   directly.
//! - [`CurrentLevel`] and [`LoadingGate`]: which map to load, and how plugins
//!   tell the menus when they've finished loading it.
//! - [`ActionState`]: what the player asked for this frame (move, look,
//!   fire, ...), whatever the device. Gameplay never reads keys or buttons.
//! - [`PlayerVitals`], [`WeaponReadout`], [`SquadReadout`]: numbers the
//!   on-screen display shows. Gameplay writes them, the display only reads.
//! - [`LevelCollision`], [`PlayerStart`], [`Player`], [`PlayerCamera`]: the
//!   level's solid surfaces, where the player starts, the player's body and
//!   the player's eyes.
//! - [`MusicMood`]: calm or battle music; whoever knows a fight started
//!   sets it.
//! - [`GameLog`]: one shared log file, `logs/rc_game.log`.
//! - [`save`]: the interface each part fills in so it is saved and loaded
//!   ([`save::SavePart`]), and the save, load and checkpoint messages.
//!
//! This crate has no window or renderer in it, so it builds quickly and its
//! tests run anywhere.

mod actions;
mod hud;
mod log;
pub mod paths;
pub mod save;
mod state;
pub mod units;

pub use actions::{Action, ActionState, InputDevice};
pub use hud::{
    HudMessage, PlayerVitals, SquadMember, SquadReadout, SquadStatus, ViewKick, WeaponReadout,
};
pub use log::GameLog;
pub use paths::GamePaths;
pub use state::{
    playing, CurrentLevel, GameSet, GameState, LevelCollision, LoadingGate, MusicMood, Play,
    Player, PlayerCamera, PlayerStart,
};

use bevy::prelude::*;

/// Adds the shared states, resources and messages. `rc_game` adds this
/// once; plugins must not add it again (check with
/// `app.is_plugin_added::<CorePlugin>()` if unsure).
pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .add_sub_state::<Play>()
            .init_resource::<CurrentLevel>()
            .init_resource::<LoadingGate>()
            .init_resource::<ActionState>()
            .init_resource::<PlayerVitals>()
            .init_resource::<WeaponReadout>()
            .init_resource::<SquadReadout>()
            .init_resource::<MusicMood>()
            .add_message::<HudMessage>()
            .add_message::<ViewKick>()
            .init_resource::<save::SaveList>()
            .add_message::<save::SaveGame>()
            .add_message::<save::LoadGame>()
            .add_message::<save::CheckpointReached>()
            .add_message::<save::SaveResult>()
            .configure_sets(
                Update,
                (
                    GameSet::Input,
                    GameSet::Simulate,
                    GameSet::View,
                    GameSet::Hud,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests;
