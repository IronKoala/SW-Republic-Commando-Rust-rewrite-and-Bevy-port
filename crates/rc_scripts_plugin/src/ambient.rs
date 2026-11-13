//! Sounds that loop where the map puts them (`rc_level::ambient`): on
//! geo_01a the battle in the distance (`ambPS_GEO_battle_lp_01`), four
//! fires and a steam vent. Every one is the game's own sound file, at the
//! actor's spot in the map.
//!
//! They start with the first frame of play, loop until the level ends,
//! stop while the game is paused, and are placed and fade with distance by
//! the game's own rule (`crate::sounds`, read from its sound code): full
//! loudness out to the sound's `Radius` (its `AudioParams`), then
//! `Radius / distance`. The distant battle's `Radius` is 10 uu, so the
//! game itself keeps it very quiet until the player is near its speaker.
//! Their loudness is the scripts' `[sound] volume`.

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, GamePaths, PlayerCamera, SoundAt};
use rc_level_plugin::LevelAmbientSounds;
use std::collections::BTreeMap;

use crate::sounds::{loudness, Placed};
use crate::{LevelScript, PART};

/// A looping sound from the map, playing.
#[derive(Component, Debug, Clone)]
pub struct AmbientLoop {
    /// The sound's name (`ambPS_GEO_battle_lp_01`).
    pub sound: String,
    /// The actor it comes from (`AmbientSound0`).
    pub actor: String,
}

/// Start every looping sound the map places, once per level.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start(
    mut commands: Commands,
    ambient: Option<Res<LevelAmbientSounds>>,
    script: Option<Res<LevelScript>>,
    paths: Option<Res<GamePaths>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    playing: Query<(), With<AmbientLoop>>,
    log: Res<GameLog>,
    mut done: Local<bool>,
) {
    let Some(ambient) = ambient else {
        // A new level gets its own.
        *done = false;
        return;
    };
    if *done || !playing.is_empty() {
        return;
    }
    let (Some(script), Some(paths), Some(mut audio)) = (script, paths, audio) else {
        return;
    };
    *done = true;
    if ambient.0.is_empty() {
        return;
    }
    // Read the sounds, one package at a time.
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for a in &ambient.0 {
        let name = a.sound.last().cloned().unwrap_or_default();
        let list = wanted.entry(a.sound[0].to_ascii_lowercase()).or_default();
        if !list.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            list.push(name);
        }
    }
    let mut sounds = std::collections::HashMap::new();
    let mut notes = Vec::new();
    for (package, names) in &wanted {
        let (found, n) = rc_combat::read_sounds(&paths, package, names, &mut audio);
        notes.extend(n);
        for (name, s) in found {
            sounds.insert(name.to_ascii_lowercase(), s);
        }
    }
    let mut param_packages: Vec<String> = sounds
        .values()
        .filter_map(|s| s.params.as_ref())
        .filter_map(|p| p.split_once('.').map(|(pk, _)| pk.to_string()))
        .collect();
    param_packages.sort();
    param_packages.dedup();
    let (params, mut unplaced) = rc_combat::read_audio_params(&paths, &param_packages);
    notes.append(&mut unplaced);
    let settings = &script.director.settings;
    let ear = ears.iter().next().map(|g| g.translation());
    let mut started = 0;
    for a in &ambient.0 {
        let name = a.sound.last().cloned().unwrap_or_default();
        let Some(sound) = sounds.get(&name.to_ascii_lowercase()) else {
            log.line(format!(
                "{PART}: looping sound {} of {} not found; nothing plays",
                a.sound.join("."),
                a.name
            ));
            continue;
        };
        let Some(h) = sound.pick() else { continue };
        let p = sound.params.as_ref().and_then(|p| params.get(p));
        let [x, y, z] = a.location;
        let at = to_bevy(rc_movement::Vec3::new(x, y, z));
        if p.is_some_and(|p| p.non_positional) {
            commands.spawn((
                AmbientLoop {
                    sound: name.clone(),
                    actor: a.name.clone(),
                },
                AudioPlayer(h),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(settings.sound_volume)),
            ));
            log.line(format!(
                "{PART}: {} loops {name} (heard everywhere: the game's settings say so)",
                a.name
            ));
            started += 1;
            continue;
        }
        // A Radius of 0 counts as 10 (the game's sound code).
        let radius = match p.and_then(|p| p.radius) {
            Some(r) if r > 0.0 => r,
            Some(_) => 10.0,
            None => settings.sound_radius,
        };
        let falloff = !p.is_some_and(|p| p.no_falloff);
        let distance = ear.map(|e| to_unreal(at - e).length());
        let gain = distance.map_or(1.0, |d| loudness(d, radius, falloff));
        commands.spawn((
            AmbientLoop {
                sound: name.clone(),
                actor: a.name.clone(),
            },
            AudioPlayer(h),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(settings.sound_volume * gain)),
            SoundAt::place(at),
            Placed::new(radius, falloff, None, gain),
        ));
        log.line(format!(
            "{PART}: {} loops {name} at ({:.0}, {:.0}, {:.0}), reach {radius:.0} uu{}, {} uu away, loudness {gain:.2}",
            a.name,
            a.location[0],
            a.location[1],
            a.location[2],
            if a.from_map { "" } else { " (its class's sound)" },
            distance.map_or("?".into(), |d| format!("{d:.0}")),
        ));
        started += 1;
    }
    log.line(format!(
        "{PART}: {started} of {} looping sounds from the map started",
        ambient.0.len()
    ));
    for n in notes {
        log.line(format!("{PART}:   looping sound: {n}"));
    }
}

/// Leaving the level silences its loops.
pub(crate) fn forget(mut commands: Commands, playing: Query<Entity, With<AmbientLoop>>) {
    for e in &playing {
        commands.entity(e).try_despawn();
    }
}
