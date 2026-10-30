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
