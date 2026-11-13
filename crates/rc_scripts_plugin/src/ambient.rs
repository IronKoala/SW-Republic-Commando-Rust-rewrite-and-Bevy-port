//! Sounds that loop where the map puts them (`rc_level::ambient`): on
//! geo_01a the battle in the distance (`ambPS_GEO_battle_lp_01`), four
//! fires and a steam vent. Every one is the game's own sound file, at the
//! actor's spot in the map.
//!
//! When each one plays is the game's own rule, read from its
//! `UAudioSubsystem::UpdateAmbientList` and `UpdateAmbients` (Engine.dll,
//! 2026-11-13; the decompiled code stays on the PC and is never committed):
//!
//! - every frame the game works out how far the loop can be heard: its
//!   `Radius` (its `AudioParams`) times 100 (`GAudioMaxRadiusMultiplier`,
//!   Core.dll), times `0.85 ^ (rooms apart squared)` when a wall of the
//!   level stands between the loop and the listener (`rc_level::hearing`);
//! - a loop that isn't playing starts when the listener is within that
//!   distance, fading in over 0.4 s;
//! - a playing loop stops, fading out over 0.3 s, once the listener is more
//!   than that distance plus 10 units away. Started again, it plays from
//!   its beginning.
//!
//! While it plays it is a placed sound like any other (`crate::sounds`):
//! full loudness out to its reach, then `reach / distance`, the reach
//! shrinking when a wall is between. geo_01a's loops have `Radius` 10
//! (`Params_SFX.Ambience_PS_01`; the Xbox's is 1000), so they are heard
//! only within 1000 units, and loud only right next to them. A sound whose
//! settings say `NonPositional` follows the same start and stop rule but is
//! not placed. Their loudness is the scripts' `[sound] volume`.
//!
//! When there are more sounds than channels, the sound engine (`engine.rs`)
//! sorts the loops with every other sound by priority, as the game does; a
//! loop keeps the priority it started with. The game's special case for a
//! loop on the listener's own actor (always heard, never placed) doesn't
//! come up here: these loops are on the map's own actors, and the player
//! is spawned, not placed in the map.

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{ActorAmbient, GameLog, GamePaths, PlayerCamera, SoundAt, SoundBy, SoundFade};
use rc_level_plugin::{LevelAmbientSounds, LevelHearing};
use std::collections::BTreeMap;

use crate::sounds::Placed;
use crate::{LevelScript, PART};

/// Core.dll's `GAudioMaxRadiusMultiplier` (100, never changed in the
/// game's code): how many times its `Radius` away a map's loop still plays.
pub const MAX_RADIUS_MULTIPLIER: f32 = 100.0;

/// A loop stops only this many units past where it starts
/// (`UAudioSubsystem::UpdateAmbients`).
pub const STOP_MARGIN: f32 = 10.0;

/// Seconds a loop takes to fade in when it starts (`UpdateAmbients` asks
/// `PlaySound` for 0.4) and out when it stops (`StopSound` with 0.3). Both
/// fades are straight lines (`USound::GetFadeInType`/`GetFadeOutType`
/// give 0).
pub const FADE_IN: f32 = 0.4;
pub const FADE_OUT: f32 = 0.3;

/// A looping sound from the map, playing.
#[derive(Component, Debug, Clone)]
pub struct AmbientLoop {
    /// The sound's name (`ambPS_GEO_battle_lp_01`).
    pub sound: String,
    /// The actor it comes from (`AmbientSound0`).
    pub actor: String,
    /// Its place in `AmbientSources::list`.
    pub index: usize,
    /// The fade, 0 to 1.
    pub fade: Fade,
}

/// A straight-line fade, as the game's `FAbstractSource::GetFadeMultiplier`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    pub value: f32,
    /// Where it is going: 1 fading in, 0 fading out.
    pub target: f32,
    /// Change per second.
    pub rate: f32,
    /// Seconds it lasts, and seconds gone.
    pub length: f32,
    pub elapsed: f32,
    /// Still moving.
    pub moving: bool,
}

impl Fade {
    fn new(from: f32, to: f32, length: f32) -> Fade {
        Fade {
            value: from,
            target: to,
            rate: (to - from) / length,
            length,
            elapsed: 0.0,
            moving: true,
        }
    }

    /// One frame of `dt` seconds.
    fn step(&mut self, dt: f32) -> f32 {
        if !self.moving || dt >= 1.0 {
            return self.value;
        }
        self.elapsed += dt;
        self.value += self.rate * dt;
        if self.rate > 0.0 {
            self.value = self.value.min(self.target);
        } else {
            self.value = self.value.max(self.target);
        }
        if self.elapsed >= self.length {
            self.value = self.target;
            self.moving = false;
        }
        self.value
    }

    /// Fading out.
    pub fn stopping(&self) -> bool {
        self.target == 0.0
    }
}

/// One of the map's looping sounds, ready to play.
#[derive(Debug, Clone)]
pub struct Source {
    pub sound: String,
    pub actor: String,
    /// Unreal units.
    pub at: rc_movement::Vec3,
    pub handle: Handle<AudioSource>,
    /// Its settings' `Radius` as stored (for the start and stop distance).
    pub radius: f32,
    pub falloff: bool,
    pub non_positional: bool,
    /// The level's walls cut how far it is heard (its actor's
    /// `SoundOcclusion` isn't `OCCLUSION_None`).
    pub walls: bool,
    /// Playing now.
    pub playing: Option<Entity>,
}

/// The map's looping sounds, read once the level runs.
#[derive(Resource, Debug, Default)]
pub struct AmbientSources {
    pub list: Vec<Source>,
    /// How many times each has started and stopped, for the log and checks.
    pub starts: usize,
    pub stops: usize,
}

/// Read every looping sound the map places, once per level.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start(
    mut commands: Commands,
    ambient: Option<Res<LevelAmbientSounds>>,
    sources: Option<Res<AmbientSources>>,
    script: Option<Res<LevelScript>>,
    paths: Option<Res<GamePaths>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    log: Res<GameLog>,
) {
    let Some(ambient) = ambient else { return };
    if sources.is_some() {
        return;
    }
    let (Some(script), Some(paths), Some(mut audio)) = (script, paths, audio) else {
        return;
    };
    let mut ready = AmbientSources::default();
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
        let Some(handle) = sound.pick() else { continue };
        let p = sound.params.as_ref().and_then(|p| params.get(p));
        let radius = p.and_then(|p| p.radius).unwrap_or(settings.sound_radius);
        let walls = a.sound_occlusion != 1;
        log.line(format!(
            "{PART}: {} loops {name} at ({:.0}, {:.0}, {:.0}), radius {radius:.0} uu{}{}{}: heard within {:.0} uu",
            a.name,
            a.location[0],
            a.location[1],
            a.location[2],
            if a.from_map { "" } else { " (its class's sound)" },
            if p.is_some_and(|p| p.non_positional) { ", not placed" } else { "" },
            if walls { "" } else { ", walls don't muffle it" },
            radius * MAX_RADIUS_MULTIPLIER,
        ));
        ready.list.push(Source {
            sound: name.clone(),
            actor: a.name.clone(),
            at: rc_movement::Vec3::new(a.location[0], a.location[1], a.location[2]),
            handle,
            radius,
            falloff: !p.is_some_and(|p| p.no_falloff),
            non_positional: p.is_some_and(|p| p.non_positional),
            walls,
            playing: None,
        });
    }
    log.line(format!(
        "{PART}: {} of {} looping sounds from the map ready",
        ready.list.len(),
        ambient.0.len()
    ));
    for n in notes {
        log.line(format!("{PART}:   looping sound: {n}"));
    }
    commands.insert_resource(ready);
}

/// Start and stop the map's loops as the listener moves, and run their
/// fades (see the module notes).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut commands: Commands,
    sources: Option<ResMut<AmbientSources>>,
    script: Option<Res<LevelScript>>,
    hearing: Option<Res<LevelHearing>>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    mut loops: Query<(&mut AmbientLoop, Has<AudioPlayer>)>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let (Some(mut sources), Some(script)) = (sources, script) else {
        return;
    };
    let Some(ear) = ears.iter().next().map(|g| to_unreal(g.translation())) else {
        return;
    };
    let hearing = hearing.as_ref().map(|h| &*h.0);
    let volume = script.director.settings.sound_volume;
    let dt = time.delta_secs();
    let sources = &mut *sources;
    for (index, s) in sources.list.iter_mut().enumerate() {
        let e3 = [ear.x, ear.y, ear.z];
        let a3 = [s.at.x, s.at.y, s.at.z];
        let share = match hearing {
            Some(h) if s.walls && !h.line_clear(a3, e3) => {
                h.radius_multiplier(h.zone_at(a3), h.zone_at(e3))
            }
            _ => 1.0,
        };
        let reach = s.radius * MAX_RADIUS_MULTIPLIER * share;
        let d = s.at - ear;
        let d2 = d.dot(d);
        match s.playing {
            None => {
                if d2 > reach * reach {
                    continue;
                }
                let tag = AmbientLoop {
                    sound: s.sound.clone(),
                    actor: s.actor.clone(),
                    index,
                    fade: Fade::new(0.0, 1.0, FADE_IN),
                };
                // `UpdateAmbients` plays it straight (no hearing check), as
                // the actor's ambient sound, fading in.
                let by = SoundBy {
                    actor_index: Some(index as u32),
                    ambient: true,
                    skip_hear_check: true,
                    ..default()
                };
                let mut spawned = commands.spawn((
                    tag,
                    by,
                    SoundFade(0.0),
                    AudioPlayer(s.handle.clone()),
                    PlaybackSettings::LOOP.with_volume(Volume::Linear(volume)),
                ));
                if !s.non_positional {
                    let mut placed = Placed::new(s.radius, s.falloff, None);
                    if !s.walls {
                        placed = placed.through_walls();
                    }
                    spawned.insert((SoundAt::place(to_bevy(s.at)), placed));
                }
                let e = spawned.id();
                s.playing = Some(e);
                sources.starts += 1;
                log.line(format!(
                    "{PART}: {} starts looping {} ({:.0} uu away, heard within {reach:.0})",
                    s.actor,
                    s.sound,
                    d2.sqrt()
                ));
            }
            Some(e) => {
                // Gone (the level is ending) or turned away by the sound
                // engine (no channel): it is tried again next frame.
                let Ok((mut l, has_player)) = loops.get_mut(e) else {
                    s.playing = None;
                    continue;
                };
                if !has_player {
                    commands.entity(e).try_despawn();
                    s.playing = None;
                    continue;
                }
                let far = reach + STOP_MARGIN;
                if !l.fade.stopping() && d2 > far * far {
                    l.fade = Fade::new(l.fade.value, 0.0, FADE_OUT);
                    log.line(format!(
                        "{PART}: {} stops looping {} ({:.0} uu away, heard within {reach:.0})",
                        s.actor,
                        s.sound,
                        d2.sqrt()
                    ));
                }
                let fade = l.fade.step(dt);
                commands.entity(e).insert(SoundFade(fade));
                // The game stops a sound once its fade is below 0.0001 and
                // it isn't fading in.
                if l.fade.stopping() && fade < 1e-4 {
                    commands.entity(e).try_despawn();
                    s.playing = None;
                    sources.stops += 1;
                }
            }
        }
    }
}

/// A loop of an actor's own (`rc_game_core::ActorAmbient`, such as a
/// spawned effect's hum): which actor it belongs to.
#[derive(Component, Debug, Clone, Copy)]
pub struct ActorLoop {
    pub actor: Entity,
    pub fade: Fade,
}

/// Start and stop the loops of actors spawned during play (effects) by the
/// same rule as the map's: `UpdateAmbientList` treats every actor with an
/// `AmbientSound` alike. The radius and the rest come from the sound's own
/// `AudioParams` (`rc_combat::sound_meta`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_actor_loops(
    mut commands: Commands,
    mut actors: Query<(Entity, &mut ActorAmbient, &GlobalTransform)>,
    mut loops: Query<(Entity, &mut ActorLoop, Has<AudioPlayer>)>,
    script: Option<Res<LevelScript>>,
    hearing: Option<Res<LevelHearing>>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs();
    // A loop whose actor is gone stops with it.
    for (e, l, _) in &loops {
        if actors.get(l.actor).is_err() {
            commands.entity(e).try_despawn();
        }
    }
    let Some(ear) = ears.iter().next().map(|g| to_unreal(g.translation())) else {
        return;
    };
    let hearing = hearing.as_ref().map(|h| &*h.0);
    let volume = script
        .as_ref()
        .map_or(1.0, |s| s.director.settings.sound_volume);
    let default_radius = script
        .as_ref()
        .map_or(2000.0, |s| s.director.settings.sound_radius);
    for (actor, mut a, g) in &mut actors {
        let meta = rc_combat::sound_meta::meta(a.sound.id());
        let radius = meta.as_ref().map_or(default_radius, |m| m.radius);
        let at = to_unreal(g.translation());
        let e3 = [ear.x, ear.y, ear.z];
        let a3 = [at.x, at.y, at.z];
        let share = match hearing {
            Some(h) if a.walls && !h.line_clear(a3, e3) => {
                h.radius_multiplier(h.zone_at(a3), h.zone_at(e3))
            }
            _ => 1.0,
        };
        let reach = radius * MAX_RADIUS_MULTIPLIER * share;
        let d = at - ear;
        let d2 = d.dot(d);
        match a.playing.and_then(|e| loops.get_mut(e).ok()) {
            None => {
                a.playing = None;
                if d2 > reach * reach {
                    continue;
                }
                let non_positional = meta.as_ref().is_some_and(|m| m.non_positional);
                let falloff = !meta.as_ref().is_some_and(|m| m.no_falloff);
                let mut spawned = commands.spawn((
                    ActorLoop {
                        actor,
                        fade: Fade::new(0.0, 1.0, FADE_IN),
                    },
                    SoundBy {
                        actor: Some(actor),
                        ambient: true,
                        skip_hear_check: true,
                        ..default()
                    },
                    SoundFade(0.0),
                    AudioPlayer(a.sound.clone()),
                    PlaybackSettings::LOOP.with_volume(Volume::Linear(volume)),
                ));
                if !non_positional {
                    let mut placed = Placed::new(radius, falloff, Some(actor));
                    if !a.walls {
                        placed = placed.through_walls();
                    }
                    spawned.insert((SoundAt::place(g.translation()), placed));
                }
                a.playing = Some(spawned.id());
                log.line(format!(
                    "{PART}: an effect starts looping {} ({:.0} uu away, heard within {reach:.0})",
                    a.name,
                    d2.sqrt()
                ));
            }
            Some((e, mut l, has_player)) => {
                if !has_player {
                    commands.entity(e).try_despawn();
                    a.playing = None;
                    continue;
                }
                let far = reach + STOP_MARGIN;
                if !l.fade.stopping() && d2 > far * far {
                    l.fade = Fade::new(l.fade.value, 0.0, FADE_OUT);
                    log.line(format!(
                        "{PART}: an effect stops looping {} ({:.0} uu away)",
                        a.name,
                        d2.sqrt()
                    ));
                }
                let fade = l.fade.step(dt);
                commands.entity(e).insert(SoundFade(fade));
                if l.fade.stopping() && fade < 1e-4 {
                    commands.entity(e).try_despawn();
                    a.playing = None;
                }
            }
        }
    }
}

/// Leaving the level silences its loops.
pub(crate) fn forget(
    mut commands: Commands,
    playing: Query<Entity, Or<(With<AmbientLoop>, With<ActorLoop>)>>,
) {
    for e in &playing {
        commands.entity(e).try_despawn();
    }
    commands.remove_resource::<AmbientSources>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_are_straight_lines() {
        let mut f = Fade::new(0.0, 1.0, FADE_IN);
        assert!((f.step(0.1) - 0.25).abs() < 1e-6);
        assert!((f.step(0.1) - 0.5).abs() < 1e-6);
        f.step(0.25);
        assert_eq!(f.value, 1.0);
        assert!(!f.moving);
        let mut f = Fade::new(1.0, 0.0, FADE_OUT);
        assert!((f.step(0.15) - 0.5).abs() < 1e-6);
        f.step(0.2);
        assert_eq!(f.value, 0.0);
        assert!(f.stopping());
    }
}
