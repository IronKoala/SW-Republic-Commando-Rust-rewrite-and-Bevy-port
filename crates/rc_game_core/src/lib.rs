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
//!   [`InputPrompts`] names the button for each action on the device used
//!   last, for "Press F to use" messages.
//! - [`PlayerVitals`], [`WeaponReadout`], [`SquadReadout`]: numbers the
//!   on-screen display shows. Gameplay writes them, the display only reads.
//! - [`LevelCollision`], [`PlayerStart`], [`Player`], [`PlayerCamera`]: the
//!   level's solid surfaces, where the player starts, the player's body and
//!   the player's eyes.
//! - [`MovingFloors`]: lifts and doors that carry or push bodies other than
//!   the player (squad mates, droids).
//! - [`MusicMood`]: calm or battle music; whoever knows a fight started
//!   sets it.
//! - [`Voice`] and [`SoundAt`]: tags for a sound a part plays (speech;
//!   where it comes from), for the VOICES volume and 3D SOUND options.
//! - [`GameLog`]: one shared log file, `logs/rc_game.log`.
//! - [`events`]: the level's named events ([`ScriptEvent`] out of the level
//!   scripts, [`RaiseEvent`] into them), [`ChangeLevel`] when a level
//!   exit sends the player on to the next map, and [`ShowDebriefing`] when
//!   a campaign's last level ends.
//! - [`UseItem`]: something the player uses by holding Use (a bacta
//!   dispenser) pulls them into place, holds them there and shows its
//!   progress round the crosshair.
//! - [`save`]: the interface each part fills in so it is saved and loaded
//!   ([`save::SavePart`]), and the save, load and checkpoint messages.
//!
//! This crate has no window or renderer in it, so it builds quickly and its
//! tests run anywhere.

mod actions;
pub mod cutscene;
pub mod events;
mod hud;
mod log;
pub mod paths;
pub mod pickups;
pub mod save;
pub mod sound;
mod state;
pub mod units;
pub mod use_item;

pub use actions::{Action, ActionState, InputDevice, InputPrompts};
pub use cutscene::{Cutscene, HeldView, TeleportPlayer};
pub use events::{
    ArrivalPortal, ChangeLevel, CharacterAct, CharacterOrder, CharacterOrderDone, FactoryOrder,
    LevelScriptsRun, MarkerJob, MarkerJobStep, PropAnim, RaiseEvent, ScriptEvent, ScriptTags,
    ShowDebriefing, SquadStanceOrder, UseClaimed,
};
pub use hud::{
    HudMessage, PlayerVitals, SquadMember, SquadReadout, SquadStatus, ViewKick, WeaponReadout,
};
pub use log::GameLog;
pub use paths::GamePaths;
pub use sound::{SoundAt, Voice};
pub use state::{
    playing, CarryBodies, CurrentLevel, GameSet, GameState, LevelCollision, LoadingGate,
    MovingFloors, MusicMood, Play, Player, PlayerCamera, PlayerStart,
};
pub use use_item::{UseItem, UseLock, UsePanel};

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
            .init_resource::<InputPrompts>()
            .init_resource::<PlayerVitals>()
            .init_resource::<WeaponReadout>()
            .init_resource::<SquadReadout>()
            .init_resource::<MusicMood>()
            .add_message::<HudMessage>()
            .add_message::<pickups::PickedUp>()
            .add_message::<ViewKick>()
            .add_message::<ScriptEvent>()
            .add_message::<RaiseEvent>()
            .add_message::<FactoryOrder>()
            .add_message::<CharacterOrder>()
            .add_message::<CharacterOrderDone>()
            .add_message::<SquadStanceOrder>()
            .add_message::<MarkerJob>()
            .add_message::<PropAnim>()
            .add_message::<ChangeLevel>()
            .add_message::<ShowDebriefing>()
            .init_resource::<Cutscene>()
            .init_resource::<UseItem>()
            .add_message::<TeleportPlayer>()
            .init_resource::<save::SaveList>()
            .add_message::<save::SaveGame>()
            .add_message::<save::LoadGame>()
            .add_message::<save::DeleteSave>()
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
