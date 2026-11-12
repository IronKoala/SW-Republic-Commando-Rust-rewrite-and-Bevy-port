//! Sounds the characters' animations play (`AnimNotify_Sound` in the
//! game's animation files): on geo_01a the landing gunship's engine
//! (`GunshipAmbushSpiderdroid`: `veh_GEN_gsPeel_06`, `gsLand_01`, ...),
//! the clone troopers' ropes and punches, the droids' grunts and wings.
//!
//! When each plays is the animation file's own moment for it, sent by the
//! cast (`rc_cast`) and the enemies' level-script animations
//! (`rc_enemies`) as [`AnimSound`]. The sounds they can play are read
//! while the level loads, from the list they leave ([`AnimSoundsWanted`]).
//!
//! How the game plays one (`UAnimNotify_Sound::Notify` in its
//! `Engine.dll`, read by disassembly, nothing copied): it asks the audio
//! code to play the sound from the actor whose animation it is, so the
//! sound comes from that character and carries by the same rule as every
//! placed sound (`crate::sounds`): full loudness out to its
//! `AudioParams.Radius` (0 counts as 10), then `Radius / distance`;
//! `NonPositional` ones are heard everywhere. Here it follows the
//! character as it moves, pauses with the game and stops when the level
//! ends.

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_combat::{AudioParams, GameSound};
use rc_game_core::units::to_unreal;
use rc_game_core::{AnimSound, AnimSoundsWanted, GameLog, GamePaths, PlayerCamera, SoundAt};
use std::collections::{BTreeMap, HashMap};

use crate::sounds::{loudness, Placed};
use crate::{LevelScript, PART};

/// An animation's sound, playing.
#[derive(Component, Debug, Clone)]
pub struct AnimSoundPlaying {
    pub sound: String,
    pub by: String,
}

/// The cast's animation sounds, read from the game's files.
#[derive(Resource, Default)]
pub struct AnimSounds {
    /// By dotted path in lower case.
    pub sounds: HashMap<String, GameSound>,
    pub params: HashMap<String, AudioParams>,
    /// Every path tried, found or not, so a missing one isn't read again.
    tried: std::collections::HashSet<String>,
    /// For the self-tests: what played, in order (`by: sound`).
    pub played: Vec<String>,
}

fn key(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// Package and the sound's own name from a dotted path
/// (`GEO_Sound.Vehicles.veh_GEN_gsPeel_06`).
fn split(path: &str) -> Option<(String, String)> {
    let (package, rest) = path.split_once('.')?;
    let name = rest.rsplit('.').next().unwrap_or(rest);
    Some((package.to_ascii_lowercase(), name.to_string()))
}

/// Read the sounds the characters' animations can play that aren't read
/// yet, when the list grows (the cast and the enemies add to it as a
/// level loads).
pub(crate) fn load(
    wanted: Res<AnimSoundsWanted>,
    mut sounds: ResMut<AnimSounds>,
    paths: Option<Res<GamePaths>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    log: Res<GameLog>,
) {
    if !wanted.is_changed() {
        return;
    }
    let (Some(paths), Some(mut audio)) = (paths, audio) else {
        return;
    };
    let mut by_package: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut new = 0;
    for path in &wanted.0 {
        let k = key(path);
        if sounds.sounds.contains_key(&k) || !sounds.tried.insert(k) {
            continue;
        }
        if let Some((package, name)) = split(path) {
            new += 1;
            let names = by_package.entry(package).or_default();
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
    }
    if new == 0 {
        return;
    }
    let mut notes = Vec::new();
    let mut read = 0;
    for (package, names) in &by_package {
        let (found, n) = rc_combat::read_sounds(&paths, package, names, &mut audio);
        notes.extend(n);
        for (name, s) in found {
            read += 1;
            sounds.sounds.insert(key(&format!("{package}.{name}")), s);
        }
    }
    let mut param_packages: Vec<String> = sounds
        .sounds
        .values()
        .filter_map(|s| s.params.as_ref())
        .filter_map(|p| p.split_once('.').map(|(pk, _)| pk.to_string()))
        .collect();
    param_packages.sort();
    param_packages.dedup();
    let (params, n) = rc_combat::read_audio_params(&paths, &param_packages);
    notes.extend(n);
    sounds.params = params;
    log.line(format!(
        "{PART}: {read} of {new} new animation sounds read ({} in all)",
        sounds.sounds.len()
    ));
    for n in notes {
        log.line(format!("{PART}:   animation sound: {n}"));
    }
}

/// Play the sounds the cast's animations reach.
pub(crate) fn play(
    mut commands: Commands,
    mut heard: MessageReader<AnimSound>,
    mut sounds: ResMut<AnimSounds>,
    script: Option<Res<LevelScript>>,
    bodies: Query<&GlobalTransform>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    log: Res<GameLog>,
) {
    let (volume, default_radius) = script.as_ref().map_or((1.0, 600.0), |s| {
        (
            s.director.settings.sound_volume,
            s.director.settings.sound_radius,
        )
    });
    let ear = ears.iter().next().map(|g| g.translation());
    for a in heard.read() {
        let Some((package, name)) = split(&a.sound) else {
            continue;
        };
        let Some(sound) = sounds.sounds.get(&key(&format!("{package}.{name}"))) else {
            log.line(format!("{PART}: {} plays {} (not read)", a.by, a.sound));
            continue;
        };
        let Some(h) = sound.pick() else { continue };
        let p = sound.params.as_ref().and_then(|p| sounds.params.get(p));
        let tag = AnimSoundPlaying {
            sound: name.clone(),
            by: a.by.clone(),
        };
        let at = bodies.get(a.from).map(|g| g.translation()).ok();
        match (p.is_some_and(|p| p.non_positional), at) {
            (false, Some(at)) => {
                // A Radius of 0 counts as 10 (the game's sound code).
                let radius = match p.and_then(|p| p.radius) {
                    Some(r) if r > 0.0 => r,
                    Some(_) => 10.0,
                    None => default_radius,
                };
                let falloff = !p.is_some_and(|p| p.no_falloff);
                let gain = ear.map_or(1.0, |e| {
                    loudness(to_unreal(at - e).length(), radius, falloff)
                });
                commands.spawn((
                    tag,
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume * gain)),
                    SoundAt::place(at),
                    Placed::new(radius, falloff, Some(a.from), gain),
                ));
                log.line(format!(
                    "{PART}: {} plays {name}, reach {radius:.0} uu, loudness {gain:.2}",
                    a.by
                ));
            }
            _ => {
                commands.spawn((
                    tag,
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
                ));
                log.line(format!("{PART}: {} plays {name} (heard everywhere)", a.by));
            }
        }
        sounds.played.push(format!("{}: {name}", a.by));
    }
}

/// Leaving the level silences them.
pub(crate) fn forget(mut commands: Commands, playing: Query<Entity, With<AnimSoundPlaying>>) {
    for e in &playing {
        commands.entity(e).try_despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_dotted_paths() {
        assert_eq!(
            split("GEO_Sound.Vehicles.veh_GEN_gsPeel_06"),
            Some(("geo_sound".into(), "veh_GEN_gsPeel_06".into()))
        );
        assert_eq!(
            split("GEN_Sound.foo"),
            Some(("gen_sound".into(), "foo".into()))
        );
        assert_eq!(split("nothing"), None);
    }
}
