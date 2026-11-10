//! The sound a pickup makes when the player takes it.
//!
//! The original plays each pickup's own `PickupSound` for the player only,
//! from where they stand (`Pickup.AnnouncePickup` in engine.u, through
//! `ClientPlaySoundLocally`): nearly every ammo box uses
//! `GEN_Sound.Weapons.GenericWeaponPickup`, the EMP grenade box
//! `GEN_Sound.Interface.int_GEN_wepPickup_01`. So it isn't placed in the
//! level here either; it plays at full volume (scaled by the EFFECTS
//! VOLUME setting like every effect).
//!
//! The sounds a level's boxes name are read when the level loads, so
//! taking one doesn't wait for the disk. A sound nobody named before is
//! read the first time it is needed.

use crate::library::{read_sounds, GameSound};
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::pickups::{LevelAmmoBoxes, PickedUp};
use rc_game_core::{GameLog, GamePaths};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Pickup sounds read so far, by `package.name` in lower case.
#[derive(Resource, Default)]
pub struct PickupSounds {
    pub sounds: HashMap<String, GameSound>,
    /// Every sound tried, found or not, so a missing one is tried once.
    tried: HashSet<String>,
    /// The last few sounds played (`package.name`), for the log and
    /// self-tests.
    pub played: Vec<String>,
}

/// The key a pickup sound is kept under.
pub fn key(package: &str, name: &str) -> String {
    format!("{package}.{name}").to_ascii_lowercase()
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

impl PickupSounds {
    /// Read the sounds in `wanted` that weren't tried yet.
    fn read(
        &mut self,
        wanted: impl IntoIterator<Item = (String, String)>,
        paths: &GamePaths,
        audio: &mut Assets<AudioSource>,
        log: &Option<Res<GameLog>>,
    ) {
        let mut by_package: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (package, name) in wanted {
            if self.tried.insert(key(&package, &name)) {
                by_package
                    .entry(package.to_ascii_lowercase())
                    .or_default()
                    .push(name);
            }
        }
        for (package, names) in by_package {
            let (found, notes) = read_sounds(paths, &package, &names, audio);
            for n in notes {
                say(log, format!("pickups: {n}"));
            }
            say(
                log,
                format!(
                    "pickups: {} of {} pickup sounds read from {package}: {}",
                    found.len(),
                    names.len(),
                    names.join(", ")
                ),
            );
            for (name, s) in found {
                self.sounds.insert(key(&package, &name), s);
            }
        }
    }
}

/// Read the sounds this level's boxes make, when a level's boxes arrive.
pub(crate) fn read_level_sounds(
    mut sounds: ResMut<PickupSounds>,
    level: Res<LevelAmmoBoxes>,
    paths: Res<GamePaths>,
    mut audio: ResMut<Assets<AudioSource>>,
    log: Option<Res<GameLog>>,
) {
    if !level.is_changed() {
        return;
    }
    let wanted: Vec<(String, String)> = level.0.iter().filter_map(|b| b.sound.clone()).collect();
    sounds.read(wanted, &paths, &mut audio, &log);
}

/// Play the sound of each thing just picked up.
pub(crate) fn play_pickup_sounds(
    mut commands: Commands,
    mut picked: MessageReader<PickedUp>,
    mut sounds: ResMut<PickupSounds>,
    paths: Option<Res<GamePaths>>,
    mut audio: ResMut<Assets<AudioSource>>,
    log: Option<Res<GameLog>>,
) {
    for p in picked.read() {
        let Some((package, name)) = &p.sound else {
            continue;
        };
        let k = key(package, name);
        if !sounds.sounds.contains_key(&k) {
            if let Some(paths) = &paths {
                sounds.read([(package.clone(), name.clone())], paths, &mut audio, &log);
            }
        }
        match sounds.sounds.get(&k).and_then(|s| s.pick()) {
            Some(h) => {
                commands.spawn((
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(1.0)),
                ));
                say(&log, format!("pickups: played sound {package}.{name}"));
                sounds.played.push(format!("{package}.{name}"));
                let extra = sounds.played.len().saturating_sub(8);
                sounds.played.drain(..extra);
            }
            None => say(
                &log,
                format!("pickups: no sound {package}.{name} to play (not in the game files?)"),
            ),
        }
    }
}
