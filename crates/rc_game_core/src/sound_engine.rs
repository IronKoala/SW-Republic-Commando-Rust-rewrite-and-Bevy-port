//! What the parts that play sounds tell the sound engine
//! (`rc_scripts_plugin::engine`, the original's `UAudioSubsystem`), and
//! what the engine tells them back.
//!
//! A part plays a sound by spawning Bevy's `AudioPlayer` as before. The
//! engine finds the game's own facts about it (`rc_combat::sound_meta`),
//! and then plays it the original's way: a random volume and pitch from
//! its `AudioParams`, its category's volume slider, the hearing range
//! check, priorities when too many sounds want a channel, and turning
//! other sounds down while a radio line plays.

use bevy::prelude::*;

/// Who plays a sound and how, as the game's callers tell its
/// `PlayUSound`. A sound without it is played as by a script's
/// `PlaySound` from nothing in particular.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SoundBy {
    /// The character or thing playing it. A placed sound follows it, and
    /// two lines from the same speaker share an id (the newer replaces the
    /// older when no channel is free).
    pub actor: Option<Entity>,
    /// The actor's number in the level, when the speaker has no entity
    /// (a level script's own spot): two sounds with the same number and
    /// kind share an id. Defaults to `actor`'s.
    pub actor_index: Option<u32>,
    /// `PlayOwnedSound` (a character's own cue, flag 0x2000): may be cut
    /// for a more important sound. Script `PlaySound` lines aren't.
    pub owned: bool,
    /// The actor's `AmbientSound` (a map loop): keeps the priority it
    /// started with.
    pub ambient: bool,
    /// The caller asks for it to be heard everywhere (flag 0x100), whatever
    /// its `AudioParams` say: `ACTION_PlayLocalSound`, `PlaySound` with
    /// `Attenuate=false`, the player's own sounds.
    pub everywhere: bool,
    /// Don't check whether the listener is in range before starting it
    /// (the game's map loops, emitters and helmet sounds call `PlayUSound`
    /// straight, without `CheckHearSound`).
    pub skip_hear_check: bool,
}

/// The SOUND options as the engine uses them, copied from the settings
/// each frame by the game.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct SoundMix {
    /// AUDIO CHANNELS: how many sounds play at once (the original caps
    /// it at 32).
    pub channels: u32,
    /// The port's master volume (not in the original; 1 leaves the
    /// original's loudness).
    pub master: f32,
    /// MUSIC VOLUME, VOICE VOLUME, SOUND FX VOLUME (`MusicVolume`,
    /// `VoiceVolume`, `SoundVolume` in the original's ini).
    pub music: f32,
    pub voice: f32,
    pub sfx: f32,
    /// 3D SOUND: placed sounds come from their side.
    pub sound_3d: bool,
}

impl Default for SoundMix {
    /// The original's defaults (`ALAudio.int` / `system.ini`).
    fn default() -> Self {
        SoundMix {
            channels: 32,
            master: 1.0,
            music: 0.7,
            voice: 1.0,
            sfx: 0.5,
            sound_3d: true,
        }
    }
}

/// How far the engine has turned the music, voices and effects down for a
/// radio line, and whether battle chatter is held back. Written by the
/// engine each frame.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Ducking {
    pub music: f32,
    pub voice: f32,
    pub sfx: f32,
    /// A sound whose `AudioParams` disable voice cues is playing: the
    /// characters' battle chatter (cues 8 and up) is held back.
    pub voice_cues_blocked: bool,
    /// The game is paused (sounds are not lowered then).
    pub paused: bool,
}

impl Default for Ducking {
    fn default() -> Self {
        Ducking {
            music: 1.0,
            voice: 1.0,
            sfx: 1.0,
            voice_cues_blocked: false,
            paused: false,
        }
    }
}

impl Ducking {
    /// What the music plays at right now (1 while paused).
    pub fn music_now(&self) -> f32 {
        if self.paused {
            1.0
        } else {
            self.music
        }
    }
}

/// A music track (played by the music part, not the engine). The engine
/// counts it as a sound using a channel; the music part turns it down
/// with [`Ducking::music_now`] itself.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct MusicTrack;

/// A sound's own fade, 0 to 1 (a map loop fading in or out). The engine
/// multiplies its volume by it (`FAbstractSource::UpdateVolume`).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct SoundFade(pub f32);
