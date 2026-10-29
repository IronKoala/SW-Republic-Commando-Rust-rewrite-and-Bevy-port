//! rc_combat: weapons, shots, health and damage, with Republic Commando's
//! own numbers.
//!
//! Two layers:
//!
//! - Plain rules with no Bevy systems, tested on their own: [`data`] (our
//!   settings files), [`weapon`] (magazine, reload, recharge, fire rate),
//!   [`shot`] (aim error, bolt flight, hitting a character's cylinder) and
//!   [`health`] (shield first, then health, shield recharge).
//! - Bevy plugins that use them: [`HealthPlugin`] applies [`Damage`] to
//!   [`Health`] and [`Shield`] and sends [`Died`]; [`WeaponsPlugin`] loads
//!   the weapon files, lets the player fire from the input actions, turns
//!   every [`FireShot`] into bolts or instant hits, and plays the sounds.
//!   [`CombatPlugin`] adds both.
//!
//! The plugins follow `docs/PLUGINS.md`: they read input only from
//! `ActionState`, the player from `Player` and `PlayerCamera`, the level
//! from `LevelCollision`, and write `PlayerVitals` and `WeaponReadout`.
//! Nothing here assumes a flat screen or a mouse: a shot is a start point
//! and a direction, so a VR controller can be the gun later.

mod damage;
pub mod data;
mod fire;
pub mod health;
mod library;
pub mod shot;
mod view;
pub mod weapon;

pub use health::{Damage, DamageKind, Dead, Died, Difficulty, Health, Shield, Team};
pub use library::{PlayerSetup, WeaponLibrary};
pub use weapon::{WeaponParams, WeaponState};

use bevy::prelude::*;
use rc_game_core::{playing, GamePaths, GameSet};

/// Something shots can hit: an upright cylinder, as Unreal Engine 2 uses
/// for characters. Sizes are Unreal units. `center_up` is how far the
/// cylinder's middle is above the entity's `Transform` (characters stand
/// on their feet, so it is usually `half_height`).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Hittable {
    pub radius: f32,
    pub half_height: f32,
    pub center_up: f32,
}

impl Hittable {
    /// The game's commando size: 40 wide each way, 168 tall.
    pub const COMMANDO: Hittable = Hittable {
        radius: 40.0,
        half_height: 84.0,
        center_up: 84.0,
    };
}

/// "Fire this weapon from here, this way." Anyone can send it (droids,
/// squad mates, turrets); the player's own trigger sends it too. In Bevy
/// space. The weapon's aim error is added on top of `direction`.
#[derive(Message, Debug, Clone)]
pub struct FireShot {
    pub shooter: Entity,
    pub origin: Vec3,
    pub direction: Vec3,
    /// A weapon file name without `.toml`, like `e5_blaster`.
    pub weapon: String,
}

/// Where a shot ended: on a character (`target`), on the level, or nowhere
/// (`target` none and `on_level` false: it ran out of range or time).
/// For effects, sounds and tests. In Bevy space.
#[derive(Message, Debug, Clone)]
pub struct ShotLanded {
    pub shooter: Entity,
    pub weapon: String,
    pub point: Vec3,
    pub target: Option<Entity>,
    pub on_level: bool,
    /// Unreal units from where the shot started.
    pub distance: f32,
}

/// The view kick from one shot, for whoever turns the player's view (the
/// player plugin, or a VR mod that chooses to ignore it). Radians: `pitch`
/// up, `yaw` right.
#[derive(Message, Debug, Clone, Copy)]
pub struct ViewKick {
    pub pitch: f32,
    pub yaw: f32,
}

/// The weapons a character carries, and which one is in hand. The player
/// gets one from `data/combat/player.toml`; give one to anyone else whose
/// weapons should keep ammo (droids firing `FireShot` don't need one).
#[derive(Component, Debug, Clone, Default)]
pub struct Arsenal {
    pub weapons: Vec<(String, WeaponState)>,
    pub current: usize,
}

impl Arsenal {
    pub fn current(&self) -> Option<&(String, WeaponState)> {
        self.weapons.get(self.current)
    }
}

/// The order of combat's work inside `GameSet::Simulate`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum CombatSet {
    /// Triggers pulled, weapons switched and reloaded, shots launched.
    Fire,
    /// Bolts fly and hit things.
    Flight,
    /// `Damage` applied to health and shields; `Died` sent.
    ApplyDamage,
    /// Readouts for the on-screen display.
    Readouts,
}

/// Health, shields and damage. Usable without weapons (for falling or
/// explosions).
pub struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Damage>()
            .add_message::<Died>()
            .init_resource::<Difficulty>()
            .configure_sets(
                Update,
                (
                    CombatSet::Fire,
                    CombatSet::Flight,
                    CombatSet::ApplyDamage,
                    CombatSet::Readouts,
                )
                    .chain()
                    .in_set(GameSet::Simulate),
            )
            .add_systems(
                Update,
                (damage::apply_damage, damage::recharge_shields)
                    .chain()
                    .in_set(CombatSet::ApplyDamage)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                damage::player_vitals
                    .in_set(CombatSet::Readouts)
                    .run_if(playing),
            );
    }
}

/// Weapons and shots: the player's guns, `FireShot` from anyone, bolts,
/// hits, sounds and the gun in the player's view.
pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<FireShot>()
            .add_message::<ShotLanded>()
            .add_message::<ViewKick>()
            .insert_resource(fire::CombatRng(shot::Rng::new(0x5EED_D17)))
            .add_systems(
                Update,
                (
                    library::load
                        .run_if(not(resource_exists::<WeaponLibrary>))
                        .run_if(resource_exists::<GamePaths>),
                    library::equip_player.run_if(resource_exists::<WeaponLibrary>),
                )
                    .chain()
                    .before(CombatSet::Fire)
                    .in_set(GameSet::Simulate),
            )
            .add_systems(
                Update,
                (fire::player_trigger, fire::launch_shots)
                    .chain()
                    .in_set(CombatSet::Fire)
                    .run_if(playing)
                    .run_if(resource_exists::<WeaponLibrary>),
            )
            .add_systems(
                Update,
                fire::fly_bolts.in_set(CombatSet::Flight).run_if(playing),
            )
            .add_systems(
                Update,
                fire::weapon_readout
                    .in_set(CombatSet::Readouts)
                    .run_if(playing)
                    .run_if(resource_exists::<WeaponLibrary>),
            )
            .add_systems(
                Update,
                (view::show_weapon_model, view::make_bolt_looks)
                    .in_set(GameSet::View)
                    .run_if(resource_exists::<WeaponLibrary>),
            );
    }
}

/// Everything in this crate: add this one to the game.
pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((HealthPlugin, WeaponsPlugin));
    }
}
