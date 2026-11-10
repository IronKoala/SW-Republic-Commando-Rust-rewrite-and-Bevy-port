//! Weapons lying on the ground: the ones the maps place (a Wookiee
//! bowcaster on a crate, a spare sniper attachment) and the ones a fallen
//! enemy leaves. What each of the original's weapons becomes in this port is
//! in `data/pickups/weapon_pickups.toml`.
//!
//! The rules, in our own words, from the game's `Pickup`, `Weapon` and
//! `PlayerController` scripts (engine.u) and `EliteBeamPickup`
//! (ctinventory.u):
//!
//! - A weapon you already carry is taken by walking into it: its shots go
//!   into your spare ammo, up to the most you can carry. If you are full it
//!   is left lying. A weapon that recharges (the DC-15s) never takes ammo.
//! - A weapon you don't carry is taken by pressing Use while touching it.
//!   The game shows "PRESS @ TO PICKUP" with the Use button in place of @.
//! - You carry one weapon picked up from the enemy at a time (the game's
//!   secondary slot). Taking another throws the one you carry down in front
//!   of you, with the shots it has left, and puts the new one in your hands.
//! - The Elite Beam is never taken for ammo: Use swaps it in, and only once
//!   the one you carry is empty.
//! - A weapon taken is gone for good.

use crate::ammo_box::touches;
use crate::data::Table;
use crate::health::Dead;
use crate::library::WeaponLibrary;
use crate::weapon::WeaponState;
use crate::{Arsenal, Hittable};
use bevy::prelude::*;
use rc_game_core::pickups::{LevelWeaponPickups, PickedUp};
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{
    Action, ActionState, GameLog, GamePaths, GameState, HudMessage, InputPrompts, Player,
};
use rc_level_viewer::draw::MoverPart;
use std::collections::BTreeMap;

/// The file saying what each of the original's weapons becomes.
pub const RULES_FILE: &str = "pickups/weapon_pickups.toml";

/// Touch size of a weapon an enemy drops, uu: radius and half-height.
/// The game's `CTPickup` collision (84 wide); the half-height is **our
/// choice** (the dropped weapon falls with physics, which we don't run).
pub const DROPPED_RADIUS: f32 = 84.0;
pub const DROPPED_HEIGHT: f32 = 15.0;

/// `data/pickups/weapon_pickups.toml`, read once.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct WeaponPickupRules {
    /// The original's weapon class (lower case) to this port's weapon id.
    pub weapons: BTreeMap<String, String>,
    /// Weapons carried one at a time (the game's secondary slot).
    pub one_at_a_time: Vec<String>,
    /// Weapons never taken for their ammo, only swapped in once the one
    /// carried is empty (the Elite Beam).
    pub swap_when_empty: Vec<String>,
    /// The prompt shown while touching a weapon you can take, `@` standing
    /// for the Use button.
    pub prompt: String,
    /// The sound of taking a weapon that doesn't say its own (one an enemy
    /// dropped): sound package and name.
    pub sound: Option<(String, String)>,
}

impl Default for WeaponPickupRules {
    fn default() -> Self {
        WeaponPickupRules {
            weapons: BTreeMap::new(),
            one_at_a_time: Vec::new(),
            swap_when_empty: Vec::new(),
            prompt: "PRESS @ TO PICKUP".into(),
            sound: Some(("GEN_Sound".into(), "int_GEN_wepPickup_01".into())),
        }
    }
}

impl WeaponPickupRules {
    pub fn from_text(text: &str) -> Result<WeaponPickupRules, String> {
        let mut t = Table::parse(text)?;
        let mut rules = WeaponPickupRules::default();
        for name in t.names() {
            if let Some(class) = name.strip_prefix("weapons.") {
                let id = t.text(&name)?.unwrap_or_default();
                rules
                    .weapons
                    .insert(class.to_ascii_lowercase(), id.to_ascii_lowercase());
            }
        }
        let lower = |v: Vec<String>| v.into_iter().map(|s| s.to_ascii_lowercase()).collect();
        rules.one_at_a_time = lower(t.texts("carry.one_at_a_time")?.unwrap_or_default());
        rules.swap_when_empty = lower(t.texts("carry.swap_when_empty")?.unwrap_or_default());
        if let Some(p) = t.text("prompt.text")? {
            rules.prompt = p;
        }
        let package = t.text("sound.package")?;
        let name = t.text("sound.name")?;
        if let (Some(package), Some(name)) = (package, name) {
            rules.sound = (!name.is_empty()).then_some((package, name));
        }
        t.finish()?;
        Ok(rules)
    }

    fn one_at_a_time(&self, id: &str) -> bool {
        self.one_at_a_time.iter().any(|w| w == id)
    }

    fn swap_when_empty(&self, id: &str) -> bool {
        self.swap_when_empty.iter().any(|w| w == id)
    }
}

/// A weapon on the ground. Spawn one with a `Transform` (Bevy space, at the
/// centre of its touch cylinder).
#[derive(Component, Debug, Clone, PartialEq)]
pub struct WeaponPickup {
    /// A weapon file name without `.toml`.
    pub weapon: String,
    /// Shots it holds. `None`: the weapon file's `start_ammo`.
    pub ammo: Option<u32>,
    /// For weapons the map places: its export index (the drawn model has a
    /// `MoverPart` with it) and name in the map (save files use it).
    /// `None` for dropped ones, which get the weapon's bolt look instead.
    pub placed: Option<(usize, String)>,
    /// What to show when it's taken ("Wookiee Bowcaster"). Empty: the
    /// weapon file's name.
    pub message: String,
    /// The sound of taking it (package, name). `None`: the rules file's.
    pub sound: Option<(String, String)>,
    /// Touch cylinder, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// The player touched it last frame.
    pub touching: bool,
    /// Seconds until the "press Use" prompt is shown again while touching.
    pub prompt_in: f32,
}

impl WeaponPickup {
    /// A weapon dropped on the ground (by a fallen enemy, or swapped out).
    pub fn dropped(weapon: &str, ammo: Option<u32>) -> Self {
        WeaponPickup {
            weapon: weapon.to_string(),
            ammo,
            placed: None,
            message: String::new(),
            sound: None,
            radius: DROPPED_RADIUS,
            height: DROPPED_HEIGHT,
            touching: false,
            prompt_in: 0.0,
        }
    }
}

/// A weapon was taken (or its ammo), for the log and self-tests.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct WeaponTaken {
    /// The weapon file id.
    pub weapon: String,
    /// For weapons the map placed: export index and name in the map.
    pub placed: Option<(usize, String)>,
    /// True if it went into the player's hands; false if only its ammo
    /// was taken.
    pub new_weapon: bool,
    /// Shots added.
    pub added: u32,
    /// The weapon thrown down to make room, if any.
    pub dropped: Option<String>,
}

/// Spawn a dropped weapon `at` the ground (with the bolt look of its
/// weapon, if the game draws).
pub fn spawn_pickup(commands: &mut Commands, weapon: &str, at: Vec3) -> Entity {
    spawn_dropped(commands, weapon, None, at)
}

fn spawn_dropped(commands: &mut Commands, weapon: &str, ammo: Option<u32>, at: Vec3) -> Entity {
    commands
        .spawn((
            WeaponPickup::dropped(weapon, ammo),
            Transform::from_translation(at + Vec3::Y * (DROPPED_HEIGHT / UU_PER_UNIT)),
            Visibility::default(),
            DespawnOnExit(GameState::InGame),
        ))
        .id()
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// Read the rules file once the game's folders are known.
pub(crate) fn load_rules(mut commands: Commands, paths: Res<GamePaths>, log: Option<Res<GameLog>>) {
    let rules = match paths.data_file(RULES_FILE) {
        None => {
            say(
                &log,
                format!("pickups: no {RULES_FILE}: weapons in the maps can't be taken"),
            );
            WeaponPickupRules::default()
        }
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| WeaponPickupRules::from_text(&t))
        {
            Ok(r) => {
                say(
                    &log,
                    format!(
                        "pickups: {} weapon kinds, {} carried one at a time, read from {}",
                        r.weapons.len(),
                        r.one_at_a_time.len(),
                        p.display()
                    ),
                );
                r
            }
            Err(e) => {
                say(
                    &log,
                    format!("pickups: FAILED to read {}: {e}", p.display()),
                );
                WeaponPickupRules::default()
            }
        },
    };
    commands.insert_resource(rules);
}

/// Weapons for this level have been made.
#[derive(Resource)]
pub(crate) struct WeaponsPlaced;

/// Weapon names a save marked taken, applied once the weapons exist.
#[derive(Resource, Debug, Clone)]
pub(crate) struct WeaponsTakenFromSave(pub Vec<String>);

/// Make the level's weapons, once per level.
pub(crate) fn place_weapons(
    mut commands: Commands,
    level: Res<LevelWeaponPickups>,
    rules: Res<WeaponPickupRules>,
    lib: Res<WeaponLibrary>,
    placed: Option<Res<WeaponsPlaced>>,
    log: Option<Res<GameLog>>,
) {
    if placed.is_some() {
        return;
    }
    commands.insert_resource(WeaponsPlaced);
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    for w in &level.0 {
        let id = rules
            .weapons
            .get(&w.weapon.to_ascii_lowercase())
            .filter(|id| lib.weapons.contains_key(*id));
        let Some(id) = id else {
            *unknown
                .entry(format!("{} ({})", w.class, w.weapon))
                .or_default() += 1;
            continue;
        };
        *by_kind.entry(format!("{} -> {id}", w.class)).or_default() += 1;
        commands.spawn((
            WeaponPickup {
                weapon: id.clone(),
                ammo: Some(w.ammo),
                placed: Some((w.export, w.name.clone())),
                message: w.message.clone(),
                sound: w.sound.clone(),
                radius: w.radius,
                height: w.height,
                touching: false,
                prompt_in: 0.0,
            },
            Transform::from_translation(w.at),
            DespawnOnExit(GameState::InGame),
        ));
    }
    let list: Vec<String> = by_kind.iter().map(|(k, n)| format!("{n} x {k}")).collect();
    say(
        &log,
        format!(
            "pickups: {} weapons placed: {}",
            level.0.len() - unknown.values().sum::<usize>(),
            list.join(", ")
        ),
    );
    if !unknown.is_empty() {
        say(
            &log,
            format!(
                "pickups: weapons nobody can take (no entry in {RULES_FILE}, or no such weapon file), left out: {unknown:?}"
            ),
        );
    }
}

pub(crate) fn forget_weapons(mut commands: Commands) {
    commands.remove_resource::<WeaponsPlaced>();
}

/// Take a weapon's ammo into the arsenal. Returns the shots added, or
/// `None` if there was no room (or the weapon recharges, or isn't carried).
pub fn take_ammo(lib: &WeaponLibrary, arsenal: &mut Arsenal, id: &str, ammo: u32) -> Option<u32> {
    let params = lib.weapons.get(id)?;
    if params.recharges() {
        return None;
    }
    let in_hand = arsenal.current;
    let (i, (_, state)) = arsenal
        .weapons
        .iter_mut()
        .enumerate()
        .find(|(_, (w, _))| w == id)?;
    let total = state.shots_in_clip() + state.reserve;
    if total >= params.max_ammo {
        return None;
    }
    let added = ammo.min(params.max_ammo - total);
    if added == 0 {
        return None;
    }
    state.reserve += added;
    if total == 0 && i == in_hand {
        state.start_reload(params);
    }
    Some(added)
}

/// Put a new weapon in the player's hands with `ammo` shots. If it is
/// carried one at a time and another such weapon is carried, that one is
/// taken out and returned (id, shots left) to be thrown down.
pub fn take_weapon(
    lib: &WeaponLibrary,
    rules: &WeaponPickupRules,
    arsenal: &mut Arsenal,
    id: &str,
    ammo: u32,
) -> Option<Option<(String, u32)>> {
    let params = lib.weapons.get(id)?;
    let mut thrown = None;
    // A carried one of the same kind (an empty Elite Beam) goes too.
    let swap_out = arsenal
        .weapons
        .iter()
        .position(|(w, _)| w == id || (rules.one_at_a_time(id) && rules.one_at_a_time(w)));
    if let Some(i) = swap_out {
        let (old, state) = arsenal.weapons.remove(i);
        thrown = Some((old, state.shots_in_clip() + state.reserve));
        if arsenal.current > i || arsenal.current >= arsenal.weapons.len() {
            arsenal.current = arsenal.current.saturating_sub(1);
        }
    }
    let ammo = ammo.min(params.max_ammo);
    let mut state = WeaponState::new(params);
    if !params.recharges() {
        let clip = params.clip_size.min(ammo);
        state.clip = clip as f32;
        state.reserve = ammo - clip;
    }
    arsenal.weapons.push((id.to_string(), state));
    // The game puts a newly picked-up weapon in your hands.
    arsenal.current = arsenal.weapons.len() - 1;
    Some(thrown)
}

/// The player walks into weapons (ammo), or presses Use on them (new
/// weapons).
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_pickups(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    rules: Option<Res<WeaponPickupRules>>,
    actions: Option<Res<ActionState>>,
    prompts: Option<Res<InputPrompts>>,
    mut players: Query<
        (
            &Transform,
            &GlobalTransform,
            Option<&Hittable>,
            &mut Arsenal,
        ),
        (With<Player>, Without<Dead>),
    >,
    mut pickups: Query<(Entity, &Transform, &mut WeaponPickup), Without<Player>>,
    looks: Option<Res<crate::view::BoltLooks>>,
    shown: Query<(), With<Mesh3d>>,
    mut taken: MessageWriter<WeaponTaken>,
    mut hud: MessageWriter<HudMessage>,
    mut picked: MessageWriter<PickedUp>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, me_global, body, mut arsenal)) = players.single_mut() else {
        return;
    };
    let default_rules = WeaponPickupRules::default();
    let rules = rules.as_deref().unwrap_or(&default_rules);
    let body = body.copied().unwrap_or(Hittable::COMMANDO);
    let use_pressed = actions
        .as_ref()
        .is_some_and(|a| a.just_pressed(Action::Use));
    let mut used = false;
    let dt = time.delta_secs();
    for (e, t, mut p) in &mut pickups {
        if p.placed.is_none() {
            if let (Some(looks), false) = (&looks, shown.contains(e)) {
                looks.attach(&mut commands.entity(e), &p.weapon);
            }
        }
        let touching = touches(me.translation, &body, t.translation, p.radius, p.height);
        let started = touching && !p.touching;
        if p.touching != touching {
            p.touching = touching;
        }
        if !touching {
            continue;
        }
        let Some(params) = lib.weapons.get(&p.weapon) else {
            say(
                &log,
                format!("weapons: pickup of unknown weapon `{}`", p.weapon),
            );
            commands.entity(e).despawn();
            continue;
        };
        let ammo = p.ammo.unwrap_or(params.start_ammo);
        let label = if p.message.is_empty() {
            params.name.clone()
        } else {
            p.message.clone()
        };
        let sound = p.sound.clone().or_else(|| rules.sound.clone());
        let carried = arsenal.weapons.iter().find(|(w, _)| *w == p.weapon);
        let swap = rules.swap_when_empty(&p.weapon);
        // Walking into a weapon you carry takes its ammo.
        if carried.is_some() && !swap {
            if !started {
                continue;
            }
            match take_ammo(&lib, &mut arsenal, &p.weapon, ammo) {
                Some(added) => {
                    say(
                        &log,
                        format!(
                            "pickups: took {added} shots for {} from {}",
                            p.weapon,
                            p.placed.as_ref().map_or("a dropped weapon", |x| &x.1)
                        ),
                    );
                    picked.write(PickedUp {
                        text: label.clone(),
                        sound: sound.clone(),
                    });
                    taken.write(WeaponTaken {
                        weapon: p.weapon.clone(),
                        placed: p.placed.clone(),
                        new_weapon: false,
                        added,
                        dropped: None,
                    });
                    commands.entity(e).despawn();
                }
                None => say(
                    &log,
                    format!(
                        "pickups: touched {} but {} is full (or recharges): left lying",
                        p.placed.as_ref().map_or("a dropped weapon", |x| &x.1),
                        p.weapon
                    ),
                ),
            }
            continue;
        }
        // An Elite Beam is only swapped for an empty one.
        if swap && carried.is_some_and(|(_, s)| s.shots_in_clip() + s.reserve > 0) {
            continue;
        }
        // Otherwise: press Use.
        if !use_pressed || used {
            p.prompt_in -= dt;
            if started || p.prompt_in <= 0.0 {
                let key = prompts
                    .as_ref()
                    .and_then(|pr| pr.label(Action::Use))
                    .unwrap_or("USE")
                    .to_uppercase();
                hud.write(HudMessage {
                    text: format!(
                        "{} {}",
                        rules.prompt.replace('@', &key),
                        label.to_uppercase()
                    ),
                    seconds: 1.0,
                });
                p.prompt_in = 0.9;
            }
            continue;
        }
        used = true;
        let Some(thrown) = take_weapon(&lib, rules, &mut arsenal, &p.weapon, ammo) else {
            continue;
        };
        say(
            &log,
            format!(
                "pickups: picked up {} ({label}) with {} shots{}{}",
                p.weapon,
                ammo.min(params.max_ammo),
                p.placed
                    .as_ref()
                    .map_or(String::new(), |x| format!(" from {}", x.1)),
                thrown.as_ref().map_or(String::new(), |(w, n)| format!(
                    "; threw down {w} with {n} shots"
                )),
            ),
        );
        // Thrown down in front of the player, as the game does (0.8 of
        // the player's width ahead, half of it to the left).
        if let Some((w, n)) = &thrown {
            let fwd = me_global.forward().as_vec3() * Vec3::new(1.0, 0.0, 1.0);
            let fwd = fwd.normalize_or_zero();
            let left = Vec3::new(fwd.z, 0.0, -fwd.x);
            let at =
                me.translation + (fwd * 0.8 * body.radius + left * 0.5 * body.radius) / UU_PER_UNIT;
            spawn_dropped(&mut commands, w, Some(*n), at);
        }
        picked.write(PickedUp {
            text: label.clone(),
            sound,
        });
        taken.write(WeaponTaken {
            weapon: p.weapon.clone(),
            placed: p.placed.clone(),
            new_weapon: true,
            added: ammo.min(params.max_ammo),
            dropped: thrown.map(|t| t.0),
        });
        commands.entity(e).despawn();
    }
}

/// Weapons the map placed that are gone: what save files list.
#[derive(Resource, Debug, Clone, Default)]
pub struct WeaponsGone(pub Vec<(usize, String)>);

impl WeaponsGone {
    /// Names in the map of the weapons taken, in the order taken.
    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|(_, n)| n.clone()).collect()
    }
}

/// Note placed weapons as they are taken, and hide their models.
pub(crate) fn hide_taken(
    mut taken: MessageReader<WeaponTaken>,
    mut gone: ResMut<WeaponsGone>,
    mut parts: Query<(&MoverPart, &mut Visibility)>,
) {
    for t in taken.read() {
        if let Some((x, name)) = &t.placed {
            if !gone.0.iter().any(|(_, n)| n == name) {
                gone.0.push((*x, name.clone()));
            }
        }
    }
    if !gone.is_changed() {
        return;
    }
    for (p, mut v) in &mut parts {
        if gone.0.iter().any(|(x, _)| *x == p.0) && *v != Visibility::Hidden {
            *v = Visibility::Hidden;
        }
    }
}

pub(crate) fn forget_gone(mut gone: ResMut<WeaponsGone>) {
    gone.0.clear();
}

/// Take away the weapons a save said were taken.
pub(crate) fn apply_saved(
    mut commands: Commands,
    pending: Res<WeaponsTakenFromSave>,
    pickups: Query<(Entity, &WeaponPickup)>,
    mut gone: ResMut<WeaponsGone>,
    placed: Option<Res<WeaponsPlaced>>,
    log: Option<Res<GameLog>>,
) {
    // Wait for this level's weapons to be made.
    if placed.is_none() {
        return;
    }
    let mut n = 0;
    for (e, p) in &pickups {
        let Some((x, name)) = &p.placed else { continue };
        if pending.0.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            commands.entity(e).despawn();
            gone.0.push((*x, name.clone()));
            n += 1;
        }
    }
    say(
        &log,
        format!(
            "pickups: {n} of {} saved weapons marked taken",
            pending.0.len()
        ),
    );
    commands.remove_resource::<WeaponsTakenFromSave>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> WeaponLibrary {
        let mut lib = WeaponLibrary::default();
        for id in [
            "dc17m_blaster",
            "trandoshan_smg",
            "bowcaster",
            "elite_beam",
            "dc15s_pistol",
        ] {
            let text = std::fs::read_to_string(format!(
                "{}/../../data/weapons/{id}.toml",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            let p = crate::weapon::WeaponParams::from_text(id, &text).unwrap();
            lib.weapons.insert(id.into(), p);
        }
        lib
    }

    fn rules() -> WeaponPickupRules {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/pickups/weapon_pickups.toml"
        ))
        .unwrap();
        WeaponPickupRules::from_text(&text).unwrap()
    }

    fn kit(lib: &WeaponLibrary) -> Arsenal {
        Arsenal {
            weapons: ["dc15s_pistol", "dc17m_blaster"]
                .iter()
                .map(|id| (id.to_string(), WeaponState::new(&lib.weapons[*id])))
                .collect(),
            current: 1,
        }
    }

    #[test]
    fn reads_the_rules_file() {
        let r = rules();
        assert_eq!(r.weapons["bowcaster"], "bowcaster");
        assert_eq!(r.weapons["smg"], "trandoshan_smg");
        assert!(r.one_at_a_time("bowcaster"));
        assert!(!r.one_at_a_time("dc17m_blaster"));
        assert!(r.swap_when_empty("elite_beam"));
        assert_eq!(r.prompt, "PRESS @ TO PICKUP");
        // Every weapon it names has a file.
        for id in r.weapons.values().chain(&r.one_at_a_time) {
            let f = format!(
                "{}/../../data/weapons/{id}.toml",
                env!("CARGO_MANIFEST_DIR")
            );
            assert!(std::path::Path::new(&f).exists(), "{f}");
        }
    }

    #[test]
    fn ammo_tops_up_a_carried_weapon_and_a_full_one_is_left() {
        let lib = lib();
        let mut a = kit(&lib);
        let rifle = &mut a.weapons[1].1;
        rifle.reserve = 100;
        let before = rifle.shots_in_clip() + rifle.reserve;
        let max = lib.weapons["dc17m_blaster"].max_ammo;
        let got = take_ammo(&lib, &mut a, "dc17m_blaster", 120).unwrap();
        assert_eq!(got, 120.min(max - before));
        // Topped up to the most carried, then full: left lying.
        while take_ammo(&lib, &mut a, "dc17m_blaster", 120).is_some() {}
        let r = &a.weapons[1].1;
        assert_eq!(r.shots_in_clip() + r.reserve, max);
        // The recharging pistol never takes ammo.
        assert_eq!(take_ammo(&lib, &mut a, "dc15s_pistol", 20), None);
    }

    #[test]
    fn a_second_enemy_weapon_swaps_out_the_first() {
        let lib = lib();
        let r = rules();
        let mut a = kit(&lib);
        let thrown = take_weapon(&lib, &r, &mut a, "trandoshan_smg", 160).unwrap();
        assert_eq!(thrown, None);
        assert_eq!(a.weapons.len(), 3);
        assert_eq!(a.current().unwrap().0, "trandoshan_smg");
        // Fire some, then take a bowcaster: the repeater goes down with
        // what it has left.
        a.weapons[2].1.clip -= 10.0;
        let thrown = take_weapon(&lib, &r, &mut a, "bowcaster", 24).unwrap();
        assert_eq!(thrown, Some(("trandoshan_smg".to_string(), 150)));
        let ids: Vec<&str> = a.weapons.iter().map(|w| w.0.as_str()).collect();
        assert_eq!(ids, ["dc15s_pistol", "dc17m_blaster", "bowcaster"]);
        assert_eq!(a.current().unwrap().0, "bowcaster");
        let b = &a.weapons[2].1;
        assert_eq!(b.shots_in_clip() + b.reserve, 24);
    }
}
