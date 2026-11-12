//! Particle effects: the game's `Emitter` classes (explosions, sparks,
//! smoke), read from its script packages (`read`) and run (`sim`). Plain
//! Rust; `rc_effects_plugin` draws them in Bevy.

pub mod def;
pub mod factory;
pub mod projectile;
pub mod read;
pub mod sim;
pub mod tagged;

pub use def::{CoordinateSystem, DrawStyle, EffectDef, EmitterDef, EmitterKind, Facing};
pub use sim::{EffectState, Sprite};
