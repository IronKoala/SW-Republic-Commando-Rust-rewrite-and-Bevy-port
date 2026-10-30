//! Loading: every weapon file in `data/weapons/`, the player's setup in
//! `data/combat/player.toml`, and the weapons' sounds from the game's sound
//! packages.

use crate::damage_types::{DamageTable, DamageTypes};
use crate::data::Table;
use crate::grenade::{GrenadeParams, Grenades};
use crate::health::{Health, Shield, Team};
use crate::weapon::{WeaponParams, WeaponState};
use crate::{Arsenal, Hittable};
use bevy::audio::AudioSource;
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, Player};
use rc_package::sound::{read_multiple, read_sound};
use rc_package::{ObjectRef, Package};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

/// Every weapon the data files describe, by id (file name without
/// `.toml`), and the sounds they use.
#[derive(Resource, Debug, Default)]
pub struct WeaponLibrary {
    pub weapons: BTreeMap<String, WeaponParams>,
    /// Every grenade the data files describe (`data/grenades/*.toml`).
    pub grenades: BTreeMap<String, GrenadeParams>,
    /// Damage types and every character's damage table
    /// (`data/combat/damage_types.toml`).
    pub damage_types: DamageTypes,
    /// Files that didn't read, with the reason. They are left out.
    pub errors: Vec<String>,
    /// Sound name to Bevy sound. Empty when there is no audio (tests).
    pub sounds: HashMap<String, GameSound>,
    pub player: PlayerSetup,
}

/// One sound name from a game package, ready to play. A plain `Sound` has
/// one variant; a `SoundMultiple` (a group, such as `Uzi_Fire` or a droid's
/// `BD1_SpottedEnemy`) has several, and each playing picks the next one.
#[derive(Debug, Clone, Default)]
pub struct GameSound {
    pub variants: Vec<Handle<AudioSource>>,
}

impl GameSound {
    /// The variant to play this time. **Our choice:** the game picks from a
    /// group at random in compiled code we don't read; we step through the
    /// group in a shuffled order that is the same on every run, so logs and
    /// tests repeat.
    pub fn pick(&self) -> Option<Handle<AudioSource>> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static PICKS: AtomicU32 = AtomicU32::new(0);
        if self.variants.is_empty() {
            return None;
        }
        let n = PICKS.fetch_add(1, Ordering::Relaxed);
        // A fixed odd multiplier walks the group in a scrambled order.
        let i = (n.wrapping_mul(2_654_435_761) >> 16) as usize % self.variants.len();
        Some(self.variants[i].clone())
    }
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
    /// Grenade ids, in the order "next grenade" cycles; the first is
    /// selected at the start. Each starts with its file's `start_count`.
    pub grenades: Vec<String>,
    pub melee: MeleeParams,
    /// The player's damage table (in `damage_types.toml`).
    pub damage_table: Option<String>,
}

/// The player's close-range hit when the weapon in hand has none of its
/// own, and the time between hits, from `data/combat/player.toml`
/// `[melee]`.
#[derive(Debug, Clone, PartialEq)]
pub struct MeleeParams {
    pub damage: f32,
    /// How far in front of the eye it reaches, uu.
    pub range: f32,
    /// Seconds between hits.
    pub delay: f32,
    pub sound: Option<String>,
}

impl Default for MeleeParams {
    /// The DC-17m's melee (`DC17mBlasterDefaults`), and our own delay.
    fn default() -> Self {
        MeleeParams {
            damage: 220.0,
            range: 150.0,
            delay: 0.8,
            sound: None,
        }
    }
}

impl Default for PlayerSetup {
    fn default() -> Self {
        PlayerSetup {
            health: 200.0,
            shield: 100.0,
            shield_recharge_rate: 20.0,
            shield_recharge_delay: 3.0,
            weapons: vec!["dc17m_blaster".into(), "dc15s_pistol".into()],
            grenades: vec![],
            melee: MeleeParams::default(),
            damage_table: Some("DV_OrganicCTPlayerDamage".into()),
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
            grenades: t.texts("player.grenades")?.unwrap_or(d.grenades),
            melee: MeleeParams {
                damage: t.number("melee.damage")?.unwrap_or(d.melee.damage),
                range: t.number("melee.range")?.unwrap_or(d.melee.range),
                delay: t.number("melee.delay")?.unwrap_or(d.melee.delay),
                sound: t.text("melee.sound")?.or(d.melee.sound),
            },
            damage_table: t.text("player.damage_table")?.or(d.damage_table),
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

/// Every `<folder>/*.toml` across the data folders (`weapons`,
/// `grenades`). A mod's file replaces the base file with the same name (the
/// data folders list mods first).
fn data_files(paths: &GamePaths, folder: &str) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for dir in paths.data_dirs.iter().rev() {
        // Finds the folder in any letter case (for Linux and Android).
        let folder = GamePaths::new(dir.clone(), vec![]).game_file(folder);
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
    for (id, path) in data_files(&paths, "weapons") {
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
    for (id, path) in data_files(&paths, "grenades") {
        let read = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| GrenadeParams::from_text(&id, &text));
        match read {
            Ok(p) => {
                say(
                    &log,
                    format!(
                        "grenades: {id} ({}): carry {} (start {}), thrown at {} uu/s +{} up, fuse {} s (then {} s after a touch), explodes on actor {} / world {}, sticks world {} / actors {}, proximity {} uu every {} s, bounce {} / {}, damage {} ({}) within {} uu, status {:?}",
                        p.name, p.max_count, p.start_count, p.speed, p.toss_up, p.fuse_time,
                        p.after_impact, p.explode_on_actor, p.explode_on_world,
                        p.stick_to_world, p.stick_to_actors, p.proximity, p.proximity_interval,
                        p.bounce, p.actor_bounce, p.damage, p.damage_type, p.radius, p.status
                    ),
                );
                lib.grenades.insert(id, p);
            }
            Err(e) => {
                let e = format!("{}: {e}", path.display());
                say(&log, format!("grenades: FAILED to read {e}"));
                lib.errors.push(e);
            }
        }
    }
    match paths.data_file("combat/damage_types.toml") {
        None => say(
            &log,
            "weapons: no combat/damage_types.toml: every hit at full strength".into(),
        ),
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| DamageTypes::from_text(&t))
        {
            Ok(d) => {
                say(
                    &log,
                    format!(
                        "weapons: {} damage types and {} damage tables read",
                        d.parents.len(),
                        d.tables.len()
                    ),
                );
                lib.damage_types = d;
            }
            Err(e) => {
                let e = format!("{}: {e}", p.display());
                say(&log, format!("weapons: FAILED to read {e}"));
                lib.errors.push(e);
            }
        },
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
    for id in &lib.player.grenades {
        if !lib.grenades.contains_key(id) {
            let e = format!("player grenade `{id}` has no file grenades/{id}.toml");
            say(&log, format!("grenades: {e}"));
            lib.errors.push(e);
        }
    }
    if let Some(mut audio) = audio {
        load_sounds(&mut lib, &paths, &mut audio, &log);
    }
    say(
        &log,
        format!(
            "weapons: {} weapon files and {} grenade files read, {} failed, {} sounds, {:.0} ms",
            lib.weapons.len(),
            lib.grenades.len(),
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
    if let Some(n) = &lib.player.melee.sound {
        wanted
            .entry("gen_sound".into())
            .or_default()
            .push(n.clone());
    }
    for p in lib.grenades.values() {
        let list = wanted
            .entry(p.sound_package.to_ascii_lowercase())
            .or_default();
        for n in p
            .throw_sound
            .iter()
            .chain(&p.bounce_sound)
            .chain(&p.blast_sound)
        {
            if !list.contains(n) {
                list.push(n.clone());
            }
        }
    }
    for (package, names) in wanted {
        let (found, notes) = read_sounds(paths, &package, &names, audio);
        for n in notes {
            say(log, format!("weapons: {n}"));
        }
        lib.sounds.extend(found);
    }
}

/// Read sounds by name from one of the game's sound packages
/// (`GameData/Sounds/<package>.uax`) into Bevy sounds. A name can be a
/// `Sound` or a `SoundMultiple` group (every sound in the group is read).
/// Returns what was found and a note for each problem. Any part can use it
/// (enemies do).
pub fn read_sounds(
    paths: &GamePaths,
    package: &str,
    names: &[String],
    audio: &mut Assets<AudioSource>,
) -> (HashMap<String, GameSound>, Vec<String>) {
    let mut found = HashMap::new();
    let mut notes = Vec::new();
    if names.is_empty() {
        return (found, notes);
    }
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
            notes.push(format!("no sounds from {}: {e}", path.display()));
            return (found, notes);
        }
    };
    // Each export's sound, read at most once even when groups share it.
    let mut cache: HashMap<usize, Handle<AudioSource>> = HashMap::new();
    let mut one = |i: usize, notes: &mut Vec<String>| -> Option<Handle<AudioSource>> {
        if let Some(h) = cache.get(&i) {
            return Some(h.clone());
        }
        let export = &pkg.exports[i];
        match read_sound(&pkg, &data, export) {
            Ok(snd) => {
                let h = audio.add(AudioSource {
                    bytes: snd.bytes(&data).to_vec().into(),
                });
                cache.insert(i, h.clone());
                Some(h)
            }
            Err(e) => {
                notes.push(format!(
                    "sound {} didn't read: {e}",
                    pkg.name(export.object_name)
                ));
                None
            }
        }
    };
    for name in names {
        let export = pkg.exports.iter().position(|e| {
            matches!(pkg.export_class_name(e), "Sound" | "SoundMultiple")
                && pkg.name(e.object_name).eq_ignore_ascii_case(name)
        });
        let Some(i) = export else {
            notes.push(format!("sound {name} not in {package}"));
            continue;
        };
        let mut sound = GameSound::default();
        if pkg.export_class_name(&pkg.exports[i]) == "Sound" {
            sound.variants.extend(one(i, &mut notes));
        } else {
            match read_multiple(&pkg, &data, &pkg.exports[i]) {
                Ok(group) => {
                    for r in group.sounds {
                        match ObjectRef::from_index(r) {
                            ObjectRef::Export(j)
                                if j < pkg.exports.len()
                                    && pkg.export_class_name(&pkg.exports[j]) == "Sound" =>
                            {
                                sound.variants.extend(one(j, &mut notes));
                            }
                            _ => notes.push(format!(
                                "group {name}: {} is not a sound in {package}",
                                pkg.object_name(r)
                            )),
                        }
                    }
                }
                Err(e) => notes.push(format!("group {name} didn't read: {e}")),
            }
        }
        if sound.variants.is_empty() {
            notes.push(format!("sound {name} in {package} has nothing to play"));
            continue;
        }
        found.insert(name.clone(), sound);
    }
    (found, notes)
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
        if let Some(t) = &s.damage_table {
            commands.entity(e).insert(DamageTable(t.clone()));
        }
    }
}

/// Give the player (any `Player` without them) the starting grenades. On
/// its own, so a loaded game that restores the arsenal still gets them
/// (or restores its own `Grenades` first).
pub(crate) fn equip_grenades(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    players: Query<Entity, (With<Player>, Without<Grenades>)>,
    log: Option<Res<GameLog>>,
) {
    for e in &players {
        let s = &lib.player;
        let grenades = Grenades {
            carried: s
                .grenades
                .iter()
                .filter_map(|id| lib.grenades.get(id).map(|p| (id.clone(), p.start_count)))
                .collect(),
            ..Default::default()
        };
        say(
            &log,
            format!("grenades: player equipped {:?}", grenades.carried),
        );
        commands.entity(e).insert(grenades);
    }
}
