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

/// How many cues `UPawnAudioTable::Reset` clears (`EPawnAudioEvent` and
/// spare room).
pub const CUE_SLOTS: usize = 193;

/// When each character cue was last said, by cue number, for everyone at
/// once (one table in Engine.dll, `APawn::GetSoundFromCue`), and the
/// level's clock it is kept by (`Level.TimeSeconds`: stops while paused).
/// Cleared when a map loads (`UGameEngine::LoadMap` calls
/// `UPawnAudioTable::Reset`, which sets every entry to -10000).
#[derive(Resource, Debug, Clone)]
pub struct CueClock {
    pub now: f64,
    last: Vec<f64>,
}

impl Default for CueClock {
    fn default() -> Self {
        CueClock {
            now: 0.0,
            last: vec![-10000.0; CUE_SLOTS],
        }
    }
}

impl CueClock {
    /// A new map: the clock and the table start again.
    pub fn reset(&mut self) {
        *self = CueClock::default();
    }

    /// `APawn::GetSoundFromCue`'s checks before a character says cue
    /// `cue`: not within `min_interval` seconds of the last time anyone
    /// said it, and, from cue 8 on, not while a sound that disables voice
    /// cues plays. `true`: it may be said.
    pub fn may_say(&self, cue: usize, min_interval: f32, voice_cues_blocked: bool) -> bool {
        let last = self.last.get(cue).copied().unwrap_or(-10000.0);
        if last + min_interval as f64 > self.now {
            return false;
        }
        !(voice_cues_blocked && cue >= 8)
    }

    /// The cue was said (it had a sound): remember when.
    pub fn said(&mut self, cue: usize) {
        if let Some(t) = self.last.get_mut(cue) {
            *t = self.now;
        }
    }
}

/// An actor's `AmbientSound` (a spawned effect's hum): the map loops'
/// part (`rc_scripts_plugin::ambient`) starts and stops it as the game's
/// `UAudioSubsystem::UpdateAmbientList` does, within its `Radius` × 100
/// of the listener, fading in and out. Put it on an entity standing for
/// the actor (with a `Transform` at its place, Bevy units); despawning
/// that entity stops the loop.
#[derive(Component, Debug, Clone)]
pub struct ActorAmbient {
    pub sound: Handle<bevy::audio::AudioSource>,
    /// The sound's name, for the log.
    pub name: String,
    /// The level's walls cut how far it is heard (the actor's
    /// `SoundOcclusion` isn't `OCCLUSION_None`).
    pub walls: bool,
    /// The loop playing now, if any.
    pub playing: Option<Entity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table for everyone: a cue said is too soon for anyone within
    /// its interval; from cue 8 on, nothing while radio holds cues back.
    #[test]
    fn cue_table() {
        let mut c = CueClock::default();
        assert!(c.may_say(20, 5.0, false), "never said: -10000 long ago");
        c.now = 3.0;
        c.said(20);
        c.now = 7.0;
        assert!(!c.may_say(20, 5.0, false));
        c.now = 8.0;
        assert!(c.may_say(20, 5.0, false));
        assert!(!c.may_say(20, 0.0, true));
        assert!(c.may_say(3, 0.0, true), "death cries still play");
        c.reset();
        assert_eq!(c.now, 0.0);
        assert!(c.may_say(20, 5.0, false));
    }
}
