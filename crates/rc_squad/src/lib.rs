//! Delta Squad: Sev, Fixer and Scorch following the player and obeying the
//! squad orders (form up, secure position, search and destroy, cancel, and
//! the marked jobs like using a bacta dispenser).
//!
//! Plain Rust with no Bevy, so it can be tested on its own. Everything is in
//! Unreal units with Z up, like `rc_movement`. The Bevy side is
//! `rc_squad_plugin`.
//!
//! What comes from the game and what is our own is written down in
//! docs/DESIGN.md ("Delta Squad") and docs/RESEARCH.md. In short: the
//! orders, their keys and every distance each order uses were read from the
//! game's script packages (class defaults of the `Stance...` classes in
//! CTGame.u and of the commandos in CTCharacters.u) and its `.ini` files.
//! How the squad picks its spots and walks there lives in the game's
//! compiled code, which we don't read, so that part is our own. No game
//! code is copied.

pub mod map;
pub mod nav;
pub mod settings;
pub mod squad;

pub use map::{MapSquadInfo, MarkerSpot, SquadFactory};
pub use nav::{Navigator, StraightLine};
pub use settings::{Spawn, SquadSettings, StanceSettings, Tether};
pub use squad::{
    aimed_spot, Leader, MarkerState, MemberState, MemberStatus, Order, Squad, SquadEvent, Stance,
};

#[cfg(test)]
mod tests;
