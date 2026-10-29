//! rc_nav: how computer-controlled characters (enemies, the squad) find
//! their way around a level.
//!
//! The level designers placed "path nodes" by hand, and the editor worked
//! out which pairs of nodes a character can walk between ("reach specs").
//! Both are saved in the map. This crate reads that network
//! ([`NavGraph::from_map`]), finds the shortest route along it
//! ([`NavGraph::find_path`]), and turns a route into the same move input the
//! player's movement uses ([`steer`], [`PathFollower`]), so computer
//! characters walk with exactly the player's physics (`rc_movement`).
//!
//! Plain Rust with no Bevy, so it can be tested on its own. Positions are
//! Unreal units with Z up, like the game's data (`rc_movement::Vec3`).

pub mod graph;
pub mod path;
pub mod steer;

pub use graph::{reach, Link, NavGraph, NavNode, NodeKind};
pub use path::Route;
pub use steer::{steer, PathFollower};

#[cfg(test)]
mod tests;
