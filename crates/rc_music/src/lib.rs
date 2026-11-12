//! The game's music rules, without Bevy: [`library`] reads what the game's
//! packages say about each piece of music and where rooms have music;
//! [`imuse`] decides what plays when, as commands for the game to carry out.
//! `rc_game`'s `music.rs` plays them.

pub mod imuse;
pub mod library;

pub use imuse::{
    engine, BattleSettings, Command, Criteria, FadeOverride, FightEnd, Imuse, Layer, Threat,
    Tuning, Zones,
};
pub use library::{Entry, Fade, FadeShape, Group, MusicLibrary, Piece, Rng, RoomMusic};

#[cfg(test)]
mod tests;
