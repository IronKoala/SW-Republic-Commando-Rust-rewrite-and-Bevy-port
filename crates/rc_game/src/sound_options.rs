//! The SOUND options for every sound: the volumes, 3D SOUND, EAX, AUDIO
//! CHANNELS and SOUND QUALITY.
//!
//! The sound engine (`rc_scripts_plugin::engine`, the original's
//! `UAudioSubsystem`) plays every sound; this file hands it the options
//! each frame (`rc_game_core::SoundMix`, `rc_game_core::EaxOn`) and puts
//! the listener's ears on the player's eyes. The music part turns its own
//! tracks up and down (`music.rs`).

use crate::settings::Settings;
use bevy::audio::SpatialListener;
use bevy::prelude::*;
use rc_game_core::{EaxOn, GameLog, PlayerCamera, SoundMix};

pub struct SoundOptionsPlugin;

impl Plugin for SoundOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EaxOn>()
            .add_systems(Update, (apply_quality, ears))
            .add_systems(
                PostUpdate,
                mix.before(rc_scripts_plugin::engine::schedule_sounds),
            );
    }
}

/// The distance between the listener's ears, in Bevy units (a head is
/// about 0.2 m; a Bevy unit is 50 Unreal units, about a metre).
const EAR_GAP: f32 = 0.2;

/// The options as the sound engine reads them.
pub fn sound_mix(settings: &Settings) -> SoundMix {
    let a = &settings.audio;
    SoundMix {
        channels: a.channels,
        master: a.master_volume.clamp(0.0, 1.0),
        music: a.music_volume.clamp(0.0, 1.0),
        voice: a.voices_volume.clamp(0.0, 1.0),
        sfx: a.effects_volume.clamp(0.0, 1.0),
        sound_3d: a.sound_3d,
    }
}

/// Hand the options to the sound engine when they change.
fn mix(settings: Res<Settings>, mut mix: ResMut<SoundMix>, mut eax: ResMut<EaxOn>) {
    let want = sound_mix(&settings);
    if *mix != want {
        *mix = want;
    }
    if eax.0 != settings.audio.eax {
        eax.0 = settings.audio.eax;
    }
}

/// 3D SOUND hears from the player's eyes.
fn ears(
    mut commands: Commands,
    settings: Res<Settings>,
    cameras: Query<(Entity, Has<SpatialListener>), With<PlayerCamera>>,
) {
    for (e, has) in &cameras {
        if settings.audio.sound_3d && !has {
            commands.entity(e).insert(ears_swapped());
        } else if !settings.audio.sound_3d && has {
            commands.entity(e).remove::<SpatialListener>();
        }
    }
}

/// The listener's ears, swapped on purpose. Bevy's 3D sound (rodio 0.22's
/// `Spatial`) makes the ear *further* from a sound the louder one: a
/// sound on the left plays at 0.5 in the left speaker and 1.0 in the
/// right (measured in the cloud with rodio's own `Spatial` on a test tone,
/// 2026-11-09). Telling it the left ear is on the right puts every sound
/// back on its own side.
fn ears_swapped() -> SpatialListener {
    let plain = SpatialListener::new(EAR_GAP);
    SpatialListener {
        left_ear_offset: plain.right_ear_offset,
        right_ear_offset: plain.left_ear_offset,
    }
}

fn apply_quality(settings: Res<Settings>, mut last: Local<Option<bool>>, log: Res<GameLog>) {
    let low = settings.audio.low_quality;
    if *last != Some(low) {
        rc_audio::set_low_quality(low);
        log.line(format!(
            "sound: quality {} (from the next level load)",
            if low {
                "low (22050 per second at most)"
            } else {
                "high (as stored)"
            }
        ));
        *last = Some(low);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_reach_the_engine() {
        let mut s = Settings::default();
        s.audio.voices_volume = 0.2;
        s.audio.channels = 16;
        let m = sound_mix(&s);
        assert_eq!((m.voice, m.channels), (0.2, 16));
        // The original's defaults.
        let d = sound_mix(&Settings::default());
        assert_eq!((d.music, d.sfx, d.voice, d.channels), (0.7, 0.5, 1.0, 32));
        assert!(Settings::default().audio.eax);
    }

    /// The left ear is on the camera's right (+X), to undo rodio's swap.
    #[test]
    fn ears_are_swapped_for_rodio() {
        let l = ears_swapped();
        assert!(l.left_ear_offset.x > 0.0 && l.right_ear_offset.x < 0.0);
        assert!((l.left_ear_offset.x - l.right_ear_offset.x - EAR_GAP).abs() < 1e-6);
    }
}
