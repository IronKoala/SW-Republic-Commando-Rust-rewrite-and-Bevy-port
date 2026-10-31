//! The SOUND options for every sound but the music (which `music.rs` turns
//! up and down itself): the master, SOUND FX and VOICES volumes, 3D SOUND,
//! AUDIO CHANNELS and SOUND QUALITY.
//!
//! The parts that play sounds (weapons, enemies, level scripts, menus) play
//! them at their own loudness and tag them: `rc_game_core::Voice` for
//! speech, `rc_game_core::SoundAt` for a place in the level. This file
//! adjusts each sound as it starts, before Bevy hands it to the sound card
//! (Bevy's audio starts sounds after the transform update in `PostUpdate`).

use crate::music::Track;
use crate::settings::Settings;
use bevy::audio::{PlaybackSettings, SpatialListener, Volume};
use bevy::prelude::*;
use rc_game_core::{GameLog, PlayerCamera, SoundAt, Voice};
use std::collections::VecDeque;

pub struct SoundOptionsPlugin;

impl Plugin for SoundOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply_quality, ears)).add_systems(
            PostUpdate,
            (adjust_new_sounds, limit_channels)
                .chain()
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// The distance between the listener's ears, in Bevy units (a head is
/// about 0.2 m; a Bevy unit is 50 Unreal units, about a metre).
const EAR_GAP: f32 = 0.2;

/// How loud a new sound plays: its own loudness times the master volume
/// and the VOICES or SOUND FX volume.
pub fn volume_scale(settings: &Settings, voice: bool) -> f32 {
    let a = &settings.audio;
    let kind = if voice {
        a.voices_volume
    } else {
        a.effects_volume
    };
    (a.master_volume * kind).clamp(0.0, 1.0)
}

#[allow(clippy::type_complexity)]
fn adjust_new_sounds(
    settings: Res<Settings>,
    mut new: Query<
        (&mut PlaybackSettings, Has<Voice>, Has<SoundAt>),
        (Added<PlaybackSettings>, Without<Track>),
    >,
) {
    for (mut p, voice, placed) in &mut new {
        let scale = volume_scale(&settings, voice);
        if scale < 1.0 {
            p.volume = Volume::Linear(p.volume.to_linear() * scale);
        }
        if placed && settings.audio.sound_3d {
            p.spatial = true;
        }
    }
}

/// AUDIO CHANNELS: when more sounds play than the setting allows, the
/// oldest effects stop. Speech and music are never stopped (they still
/// count).
#[allow(clippy::type_complexity)]
fn limit_channels(
    mut commands: Commands,
    settings: Res<Settings>,
    playing: Query<(Entity, Has<Voice>), (With<PlaybackSettings>, Without<Track>)>,
    new: Query<Entity, (Added<PlaybackSettings>, Without<Voice>, Without<Track>)>,
    mut effects: Local<VecDeque<Entity>>,
    mut stopped: Local<u64>,
    log: Res<GameLog>,
) {
    effects.retain(|e| playing.contains(*e));
    effects.extend(new.iter());
    let limit = settings.audio.channels.max(1) as usize;
    let mut count = playing.iter().count();
    while count > limit {
        let Some(old) = effects.pop_front() else {
            break;
        };
        commands.entity(old).try_despawn();
        count -= 1;
        *stopped += 1;
        // The first time and then now and again, so the log shows the limit
        // working without a line per gunshot.
        if *stopped == 1 || stopped.is_multiple_of(100) {
            log.line(format!(
                "sound: {} sounds over the {limit}-channel limit stopped so far",
                *stopped
            ));
        }
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
            commands.entity(e).insert(SpatialListener::new(EAR_GAP));
        } else if !settings.audio.sound_3d && has {
            commands.entity(e).remove::<SpatialListener>();
        }
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
    fn voices_follow_their_own_volume() {
        let mut s = Settings::default();
        s.audio.master_volume = 0.5;
        s.audio.voices_volume = 0.2;
        s.audio.effects_volume = 1.0;
        assert!((volume_scale(&s, true) - 0.1).abs() < 1e-6);
        assert!((volume_scale(&s, false) - 0.5).abs() < 1e-6);
    }

    fn app(channels: u32) -> App {
        let mut app = App::new();
        let mut s = Settings::default();
        s.audio.channels = channels;
        app.insert_resource(s)
            .insert_resource(GameLog::terminal())
            .add_systems(Update, (adjust_new_sounds, limit_channels).chain());
        app
    }

    #[test]
    fn over_the_limit_the_oldest_effect_stops_never_speech() {
        let mut app = app(3);
        let line = app.world_mut().spawn((PlaybackSettings::ONCE, Voice)).id();
        app.update();
        let first = app.world_mut().spawn(PlaybackSettings::ONCE).id();
        app.update();
        let second = app.world_mut().spawn(PlaybackSettings::ONCE).id();
        app.update();
        assert!(app.world().get_entity(first).is_ok());
        let third = app.world_mut().spawn(PlaybackSettings::ONCE).id();
        app.update();
        let w = app.world();
        assert!(w.get_entity(line).is_ok(), "speech is never stopped");
        assert!(w.get_entity(first).is_err(), "the oldest effect stops");
        assert!(w.get_entity(second).is_ok() && w.get_entity(third).is_ok());
    }

    #[test]
    fn placed_sounds_are_3d_only_with_the_option_on() {
        let mut app = app(32);
        let e = app
            .world_mut()
            .spawn((PlaybackSettings::ONCE, SoundAt::place(Vec3::X)))
            .id();
        app.update();
        assert!(app.world().get::<PlaybackSettings>(e).unwrap().spatial);
        app.world_mut().resource_mut::<Settings>().audio.sound_3d = false;
        let e = app
            .world_mut()
            .spawn((PlaybackSettings::ONCE, SoundAt::place(Vec3::X)))
            .id();
        app.update();
        assert!(!app.world().get::<PlaybackSettings>(e).unwrap().spatial);
    }
}
