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

/// The vision mode the player's view is in now, as the game's object path
/// (`FrameFX.VisionModes.VisionNight`), `None` for the plain view. Set by
/// the visor part. The game uses a mode's own echo (`EAXEffect`) in place
/// of the room's while the view is in it.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct VisionModeNow(pub Option<String>);

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
