//! Tags a part puts on a sound it plays, so the SOUND options can treat it
//! right: the VOICES volume for speech, and 3D SOUND for sounds with a
//! place in the level. A sound without a tag is an effect heard from
//! nowhere in particular (the player's own gun, menu clicks).

use bevy::prelude::*;

/// Someone speaking: a squad mate, an enemy, a level script's line. Turned
/// up and down by VOICES VOLUME instead of SOUND FX VOLUME, and never cut
/// short to make room for other sounds.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Voice;

/// Where in the level the sound comes from, in Bevy units (convert with
/// `units::to_bevy`). With 3D SOUND on it is heard from that side. Spawn
/// it with [`SoundAt::place`], which adds the `Transform` Bevy's 3D sound
/// needs.
#[derive(Component, Debug, Clone, Copy)]
pub struct SoundAt(pub Vec3);

impl SoundAt {
    pub fn place(at: Vec3) -> (SoundAt, Transform) {
        (SoundAt(at), Transform::from_translation(at))
    }
}

/// A sound the room's echo leaves alone: menu clicks. The music is left
/// alone too (it has its own tag in the game program). **Our choice**: the
/// game's sound code sends every sound to the echo unless the sound's
/// flags say otherwise; which flags the menus and music use was not
/// checked.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Dry;

/// The echo of the room the player is in now, as the game would hand it
/// to EAX (`rc_audio::room`). Set by the level's zone sounds, played by
/// the game program. `None`: no echo (outside a level, or a room whose
/// audio class has no echo).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct RoomEcho {
    pub values: Option<rc_audio::room::EaxReverb>,
    /// Which effect it is (`EFFECT_CASTLE_LARGEROOM1`), for the log.
    pub name: String,
}
