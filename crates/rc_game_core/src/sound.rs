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

/// The game's `AudioParams.Category` a sound plays with (0 music, 1
/// voice, 2 sound effects; `rc_package::sound::AG_*`). The game turns a
/// sound up and down with the slider for its category
/// (`UAudioSubsystem::GetMasterVolumeMultiplier`: 0 the music volume, 1
/// the voice volume, anything else the sound volume); the SOUND options
/// follow this tag when a sound has it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundCategory(pub u8);

/// A sound's EAX occlusion, mB (0 or less): how much the walls between
/// it and the listener muffle it, by the game's rules (set by the sound
/// engine; `FAbstractSource` reach against its full reach,
/// `868.589 * ln(reach / full)`). The room's echo part plays it
/// (`rc_audio::room::occlusion_gains`).
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EaxOcclusion {
    pub millibels: i32,
}

/// The AUDIO menu's EAX row: with it off the game sends no sound to the
/// room's echo and muffles nothing (`ALAudio.dll` without EAX). On by
/// default.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EaxOn(pub bool);

impl Default for EaxOn {
    fn default() -> Self {
        EaxOn(true)
    }
}
