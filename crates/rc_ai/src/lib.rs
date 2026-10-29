//! rc_ai: what computer-controlled enemies decide to do.
//!
//! - [`params`]: a character's settings from its data file
//!   (`data/enemies/battle_droid.toml`), defaulting to the original's numbers.
//! - [`spawns`]: the map's spawners, their spawn points and walk-in
//!   triggers.
//! - [`brain`]: one fighter: see the enemy, take cover, keep its distance,
//!   shoot in bursts. It walks with the player's physics (`rc_movement`)
//!   along the level's paths (`rc_nav`).
//! - [`battle`]: a whole fight on one map: spawners switching on, droids
//!   made, thinking, and hearing each other's shots.
//!
//! Plain Rust with no Bevy: the Bevy plugin (`rc_enemies`) and the headless
//! test (`rc_droidsim`) both drive it. Unreal units, Z up.

pub mod battle;
pub mod brain;
pub mod params;
pub mod spawns;

pub use battle::{Battle, BattleEvent, Spawner};
pub use brain::{AiEvent, Fighter, Maneuver, Mode, Target, View};
pub use params::{AiParams, EnemyType, Sounds};
pub use spawns::{read_factories, Factory, SpawnSpot, StartState};

#[cfg(test)]
mod tests;
