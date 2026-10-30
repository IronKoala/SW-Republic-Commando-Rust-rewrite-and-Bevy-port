//! The original game's menus, read from the player's install at run time.
//!
//! Republic Commando's menus are UnrealScript classes whose layouts are
//! plain data: every picture, text and button is a default value of the
//! class (see `docs/ORIGINAL_MENUS.md`). This crate reads those values,
//! fills them in the way the original menu code does, and turns them into
//! textured rectangles to draw. It knows nothing about Bevy; `rc_game`
//! draws the rectangles.
//!
//! Nothing from the game is stored in this repository. Behaviour that the
//! original keeps in script (what a button does) is rewritten by hand in
//! `screens.rs`.

pub mod assets;
pub mod gpu;
pub mod layout;
pub mod library;
pub mod pages;
pub mod quads;
pub mod render;
pub mod screens;
pub mod value;

pub use assets::Assets;
pub use layout::{build, MenuLayout};
pub use library::Library;
