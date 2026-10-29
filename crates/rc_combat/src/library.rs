//! Loading: every weapon file in `data/weapons/`, the player's setup in
//! `data/combat/player.toml`, and the weapons' sounds from the game's sound
//! packages.

use crate::data::Table;
use crate::health::{Health, Shield, Team};
use crate::weapon::{WeaponParams, WeaponState};
use crate::{Arsenal, Hittable};
use bevy::audio::AudioSource;
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, Player};
use rc_package::sound::read_sound;
use rc_package::Package;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

/// Every weapon the data files describe, by id (file name without
/// `.toml`), and the sounds they use.
#[derive(Resource, Debug, Default)]
pub struct WeaponLibrary {
    pub weapons: BTreeMap<String, WeaponParams>,
    /// Files that didn't read, with the reason. They are left out.
    pub errors: Vec<String>,
    /// Sound name to Bevy sound. Empty when there is no audio (tests).
    pub sounds: HashMap<String, Handle<AudioSource>>,
    pub player: PlayerSetup,
}

/// The player's numbers, from `data/combat/player.toml`. Defaults are the
/// game's (`PlayerCommando` and `PlayerCommandoDefaults`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSetup {
    pub health: f32,
    pub shield: f32,
    pub shield_recharge_rate: f32,
    pub shield_recharge_delay: f32,
    /// Weapon ids, the first one in hand at the start.
    pub weapons: Vec<String>,
}

impl Default for PlayerSetup {
    fn default() -> Self {
        PlayerSetup {
            health: 200.0,
            shield: 100.0,
            shield_recharge_rate: 20.0,
            shield_recharge_delay: 3.0,
            weapons: vec!["dc17m_blaster".into(), "dc15s_pistol".into()],
        }
    }
}

impl PlayerSetup {
    pub fn from_text(text: &str) -> Result<PlayerSetup, String> {
        let mut t = Table::parse(text)?;
        let d = PlayerSetup::default();
        let p = PlayerSetup {
            health: t.number("player.health")?.unwrap_or(d.health),
            shield: t.number("player.shield")?.unwrap_or(d.shield),
            shield_recharge_rate: t
                .number("player.shield_recharge_rate")?
                .unwrap_or(d.shield_recharge_rate),
            shield_recharge_delay: t
                .number("player.shield_recharge_delay")?
                .unwrap_or(d.shield_recharge_delay),
            weapons: t.texts("player.weapons")?.unwrap_or(d.weapons),
        };
        t.finish()?;
        Ok(p)
    }
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// Every `weapons/*.toml` across the data folders. A mod's file replaces
/// the base file with the same name (the data folders list mods first).
fn weapon_files(paths: &GamePaths) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for dir in paths.data_dirs.iter().rev() {
        // Finds `weapons` in any letter case (for Linux and Android).
        let folder = GamePaths::new(dir.clone(), vec![]).game_file("weapons");
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
            {
                let id = p
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                out.insert(id, p);
            }
        }
    }
    out
}

/// Read the data files and sounds once `GamePaths` is known.
pub(crate) fn load(
    mut commands: Commands,
    paths: Res<GamePaths>,
    log: Option<Res<GameLog>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
) {
    let t = Instant::now();
    let mut lib = WeaponLibrary::default();
    for (id, path) in weapon_files(&paths) {
        let read = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| WeaponParams::from_text(&id, &text));
        match read {
            Ok(p) => {
                say(
                    &log,
                    format!(
                        "weapons: {id} ({}): {:?}, {} s between shots, aim error {}, magazine {}, max ammo {}, damage {} ({:?}), {:?}",
                        p.name, p.fire_mode, p.shot_delay, p.aim_error, p.clip_size,
                        p.max_ammo, p.damage, p.damage_kind, p.shot
                    ),
                );
                lib.weapons.insert(id, p);
            }
            Err(e) => {
                let e = format!("{}: {e}", path.display());
                say(&log, format!("weapons: FAILED to read {e}"));
                lib.errors.push(e);
            }
        }
    }
    match paths.data_file("combat/player.toml") {
        None => say(
            &log,
            "weapons: no combat/player.toml, using the game's player numbers".into(),
        ),
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| PlayerSetup::from_text(&t))
        {
            Ok(s) => lib.player = s,
            Err(e) => {
                let e = format!("{}: {e}", p.display());
                say(&log, format!("weapons: FAILED to read {e}"));
                lib.errors.push(e);
            }
        },
    }
    for id in &lib.player.weapons {
        if !lib.weapons.contains_key(id) {
            let e = format!("player weapon `{id}` has no file weapons/{id}.toml");
            say(&log, format!("weapons: {e}"));
            lib.errors.push(e);
        }
    }
    if let Some(mut audio) = audio {
        load_sounds(&mut lib, &paths, &mut audio, &log);
    }
    say(
        &log,
        format!(
            "weapons: {} weapon files read, {} failed, {} sounds, {:.0} ms",
            lib.weapons.len(),
            lib.errors.len(),
            lib.sounds.len(),
            t.elapsed().as_secs_f64() * 1000.0
        ),
    );
    commands.insert_resource(lib);
}

fn load_sounds(
    lib: &mut WeaponLibrary,
    paths: &GamePaths,
    audio: &mut Assets<AudioSource>,
    log: &Option<Res<GameLog>>,
) {
    // Which sounds each package must provide.
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in lib.weapons.values() {
        let names = p
            .fire_sounds
            .iter()
            .chain(&p.reload_sound)
            .chain(&p.empty_sound);
        let list = wanted
            .entry(p.sound_package.to_ascii_lowercase())
            .or_default();
        for n in names {
            if !list.contains(n) {
                list.push(n.clone());
            }
        }
    }
    for (package, names) in wanted {
        let path = paths.game_file(&format!("GameData/Sounds/{package}.uax"));
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| {
                Package::parse(&data)
                    .map(|p| (p, data))
                    .map_err(|e| e.to_string())
            });
        let (pkg, data) = match read {
            Ok(v) => v,
            Err(e) => {
                say(
                    log,
                    format!("weapons: no sounds from {}: {e}", path.display()),
                );
                continue;
            }
        };
        let (pkg, data) = (&pkg, &data);
        for name in names {
            let export = pkg.exports.iter().find(|e| {
                pkg.export_class_name(e) == "Sound"
                    && pkg.name(e.object_name).eq_ignore_ascii_case(&name)
            });
            let Some(export) = export else {
                say(log, format!("weapons: sound {name} not in {package}"));
                continue;
            };
            match read_sound(pkg, data, export) {
                Ok(s) => {
                    let bytes: Vec<u8> = s.bytes(data).to_vec();
                    let h = audio.add(AudioSource {
                        bytes: bytes.into(),
                    });
                    lib.sounds.insert(name, h);
                }
                Err(e) => say(log, format!("weapons: sound {name} didn't read: {e}")),
            }
        }
    }
}

/// Give the player (any new `Player` entity) the game's health, shield,
/// team, size and weapons.
pub(crate) fn equip_player(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    players: Query<Entity, (With<Player>, Without<Arsenal>)>,
    log: Option<Res<GameLog>>,
) {
    for e in &players {
        let s = &lib.player;
        let weapons: Vec<(String, WeaponState)> = s
            .weapons
            .iter()
            .filter_map(|id| {
                lib.weapons
                    .get(id)
                    .map(|p| (id.clone(), WeaponState::new(p)))
            })
            .collect();
        say(
            &log,
            format!(
                "weapons: player equipped: health {}, shield {} (+{}/s after {} s), weapons {:?}",
                s.health,
                s.shield,
                s.shield_recharge_rate,
                s.shield_recharge_delay,
                weapons.iter().map(|w| &w.0).collect::<Vec<_>>()
            ),
        );
        commands.entity(e).insert((
            Health::new(s.health),
            Shield::new(s.shield, s.shield_recharge_rate, s.shield_recharge_delay),
            Team::Republic,
            Hittable::COMMANDO,
            Arsenal {
                weapons,
                current: 0,
            },
        ));
    }
}
