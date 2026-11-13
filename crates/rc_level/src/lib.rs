//! Reads Republic Commando maps (`.ctm` packages): the level's own geometry
//! (the BSP) and the actors placed in it.
//!
//! Plain Rust with no Bevy, like `rc_package`. Coordinates stay in Unreal
//! units and axes (X forward, Y right, Z up); the viewer converts them.

pub mod actors;
pub mod ambient;
pub mod breakables;
pub mod bsp;
pub mod checkpoints;
pub mod class_look;
pub mod collision;
pub mod hearing;
pub mod lightmap;
pub mod map_collision;
pub mod movers;
pub mod music;
pub mod pickups;
pub mod scripts;
pub mod settle;
pub mod terrain;
pub mod volumes;
