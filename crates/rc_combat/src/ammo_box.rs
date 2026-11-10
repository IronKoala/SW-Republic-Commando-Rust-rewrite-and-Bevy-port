//! The levels' ammo and grenade boxes: walk into one to refill.
//!
//! The level part reads the map's boxes (`rc_level::pickups`) and hands
//! them over as `rc_game_core::pickups::LevelAmmoBoxes`. Each becomes an
//! [`AmmoBox`] entity here. What each of the original's ammo kinds refills
//! in this port is in `data/pickups/ammo_boxes.toml`.
//!
//! The rules, in our own words, from the game's `Pickup` and `Ammunition`
//! scripts (engine.u):
//!
//! - A box is taken when the player starts touching it: their collision
//!   cylinders overlap. Standing on it doesn't keep trying; step off and
//!   back on.
//! - It only refills ammo the player already carries, and only if that
//!   ammo isn't full. A full player leaves it lying.
//! - It adds its whole amount, capped at the most the player can carry,
//!   and is used up, even if only part of it fitted.
//! - If the weapon in hand was empty, it reloads at once.
//! - In single player a taken box is gone for good, unless the map marks
//!   it to come back (`bForceRespawn`), which only geo_01a's grenade crate
//!   by the start does (back after 0.1 s: an endless supply).
//! - Taking one plays its `PickupSound` and shows its `PickupMessage`
//!   (sent as `rc_game_core::pickups::PickedUp`; see `pickup_sound` and
//!   the HUD). A full player's touch is silent.

use crate::data::Table;
use crate::health::Dead;
use crate::library::WeaponLibrary;
use crate::{Arsenal, Grenades, Hittable};
use bevy::prelude::*;
use rc_game_core::pickups::{LevelAmmoBoxes, PickedUp};
use rc_game_core::save::SavePart;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, GamePaths, GameState, Player};
use rc_level_viewer::draw::MoverPart;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The file saying what each ammo kind refills.
pub const RULES_FILE: &str = "pickups/ammo_boxes.toml";

/// `data/pickups/ammo_boxes.toml`, read once.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct AmmoBoxRules {
    /// The original's ammo class (lower case) to this port's weapon or
    /// grenade id.
    pub kinds: BTreeMap<String, String>,
    /// Box class (lower case) to the amount it gives, where the file
    /// changes the game's.
    pub amounts: BTreeMap<String, u32>,
}

impl AmmoBoxRules {
    pub fn from_text(text: &str) -> Result<AmmoBoxRules, String> {
        let mut t = Table::parse(text)?;
        let mut rules = AmmoBoxRules::default();
        for name in t.names() {
            if let Some(kind) = name.strip_prefix("ammo.") {
                let id = t.text(&name)?.unwrap_or_default();
                rules
                    .kinds
                    .insert(kind.to_ascii_lowercase(), id.to_ascii_lowercase());
            } else if let Some(class) = name.strip_prefix("amount.") {
                let n = t.number(&name)?.unwrap_or(0.0);
                if n < 0.0 {
                    return Err(format!("`{name}` can't be below 0"));
                }
                rules.amounts.insert(class.to_ascii_lowercase(), n as u32);
            }
        }
        t.finish()?;
        Ok(rules)
    }
}

/// A box in the level.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct AmmoBox {
    /// Export index in the map; the drawn model has a `MoverPart` with it.
    pub export: usize,
    /// Name in the map, used by save files.
    pub name: String,
    pub class: String,
    /// The weapon or grenade id it refills, `None` if the rules file has
    /// no entry for its ammo kind (it then can't be taken).
    pub gives: Option<String>,
    pub amount: u32,
    /// Seconds before it comes back, for the few boxes that do.
    pub respawn: Option<f32>,
    /// Touch cylinder, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// Taken (and hidden).
    pub taken: bool,
    /// Seconds until a taken box that comes back is back.
    pub back_in: f32,
    /// The player touched it last frame (a box is tried when touching
    /// starts).
    pub touching: bool,
    /// What the HUD says when it is taken (the game's `PickupMessage`).
    pub message: String,
    /// The sound it makes when taken: sound package and name.
    pub sound: Option<(String, String)>,
}

/// What happened to a box this frame, for the log and self-tests.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct AmmoBoxTaken {
    pub name: String,
    pub gives: String,
    /// How much went in (it may be less than the box held).
    pub added: u32,
    /// The player's total after.
    pub now: u32,
    pub max: u32,
}

/// Boxes for this level have been made.
#[derive(Resource)]
pub(crate) struct BoxesPlaced;

/// Box names a save marked taken, applied once the boxes exist.
#[derive(Resource, Debug, Clone)]
pub(crate) struct TakenFromSave(pub Vec<String>);

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
                format!("pickups: no {RULES_FILE}: ammo boxes give nothing"),
            );
            AmmoBoxRules::default()
        }
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| AmmoBoxRules::from_text(&t))
        {
            Ok(r) => {
                say(
                    &log,
                    format!(
                        "pickups: {} ammo kinds, {} changed amounts read from {}",
                        r.kinds.len(),
                        r.amounts.len(),
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
                AmmoBoxRules::default()
            }
        },
    };
    commands.insert_resource(rules);
}

/// Make the level's boxes, once per level.
pub(crate) fn place_boxes(
    mut commands: Commands,
    level: Res<LevelAmmoBoxes>,
    rules: Res<AmmoBoxRules>,
    lib: Res<WeaponLibrary>,
    placed: Option<Res<BoxesPlaced>>,
    log: Option<Res<GameLog>>,
) {
    if placed.is_some() {
        return;
    }
    commands.insert_resource(BoxesPlaced);
    let mut by_kind: BTreeMap<String, (usize, u32)> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    for b in &level.0 {
        let gives = rules
            .kinds
            .get(&b.ammo.to_ascii_lowercase())
            .filter(|id| lib.weapons.contains_key(*id) || lib.grenades.contains_key(*id))
            .cloned();
        let amount = rules
            .amounts
            .get(&b.class.to_ascii_lowercase())
            .copied()
            .unwrap_or(b.amount);
        match &gives {
            Some(id) => {
                let e = by_kind
                    .entry(format!("{} -> {id}", b.class))
                    .or_insert((0, amount));
                e.0 += 1;
            }
            None => {
                *unknown
                    .entry(format!("{} ({})", b.class, b.ammo))
                    .or_default() += 1
            }
        }
        commands.spawn((
            AmmoBox {
                export: b.export,
                name: b.name.clone(),
                class: b.class.clone(),
                gives,
                amount,
                respawn: b.respawn,
                radius: b.radius,
                height: b.height,
                taken: false,
                back_in: 0.0,
                touching: false,
                message: b.message.clone(),
                sound: b.sound.clone(),
            },
            Transform::from_translation(b.at),
            DespawnOnExit(GameState::InGame),
        ));
    }
    let list: Vec<String> = by_kind
        .iter()
        .map(|(k, (n, a))| format!("{n} x {k} ({a} each)"))
        .collect();
    say(
        &log,
        format!(
            "pickups: {} ammo boxes placed: {}",
            level.0.len(),
            list.join(", ")
        ),
    );
    if !unknown.is_empty() {
        say(
            &log,
            format!("pickups: boxes nobody can take (no entry in {RULES_FILE}, or no such weapon or grenade file): {unknown:?}"),
        );
    }
}

pub(crate) fn forget_boxes(mut commands: Commands) {
    commands.remove_resource::<BoxesPlaced>();
}

/// Whether a commando standing with their feet at `feet` touches a box
/// centred at `at` (both Bevy units): the two upright cylinders overlap.
pub fn touches(feet: Vec3, body: &Hittable, at: Vec3, radius: f32, height: f32) -> bool {
    let gap = (at - feet) * UU_PER_UNIT;
    let across = Vec2::new(gap.x, gap.z).length();
    // Box from gap.y - height to gap.y + height above the feet; the body
    // from 0 to twice its half-height.
    across < body.radius + radius && gap.y + height > 0.0 && gap.y - height < 2.0 * body.half_height
}

/// Fill what a box gives, if there is room. Returns (added, now, max), or
/// `None` if the player can't take it (doesn't carry that ammo, or full).
pub fn refill(
    gives: &str,
    amount: u32,
    lib: &WeaponLibrary,
    arsenal: Option<&mut Arsenal>,
    grenades: Option<&mut Grenades>,
) -> Option<(u32, u32, u32)> {
    if let Some(params) = lib.weapons.get(gives) {
        let arsenal = arsenal?;
        let in_hand = arsenal.current;
        let (i, (_, state)) = arsenal
            .weapons
            .iter_mut()
            .enumerate()
            .find(|(_, (id, _))| id == gives)?;
        if params.recharges() {
            return None;
        }
        let total = state.shots_in_clip() + state.reserve;
        if total >= params.max_ammo {
            return None;
        }
        let added = amount.min(params.max_ammo - total);
        state.reserve += added;
        // "Out of ammo" with this weapon up: reload straight away.
        if total == 0 && i == in_hand {
            state.start_reload(params);
        }
        return Some((added, total + added, params.max_ammo));
    }
    if let Some(params) = lib.grenades.get(gives) {
        let grenades = grenades?;
        let (_, count) = grenades.carried.iter_mut().find(|(id, _)| id == gives)?;
        if *count >= params.max_count {
            return None;
        }
        let added = amount.min(params.max_count - *count);
        *count += added;
        return Some((added, *count, params.max_count));
    }
    None
}

/// The player walks into boxes.
pub(crate) fn take_boxes(
    lib: Res<WeaponLibrary>,
    mut players: Query<
        (
            &Transform,
            Option<&Hittable>,
            Option<&mut Arsenal>,
            Option<&mut Grenades>,
        ),
        (With<Player>, Without<Dead>),
    >,
    mut boxes: Query<(&Transform, &mut AmmoBox), Without<Player>>,
    mut taken: MessageWriter<AmmoBoxTaken>,
    mut picked: MessageWriter<PickedUp>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, body, mut arsenal, mut grenades)) = players.single_mut() else {
        return;
    };
    let body = body.copied().unwrap_or(Hittable::COMMANDO);
    for (t, mut b) in &mut boxes {
        if b.taken {
            continue;
        }
        let touching = touches(me.translation, &body, t.translation, b.radius, b.height);
        let started = touching && !b.touching;
        if b.touching != touching {
            b.touching = touching;
        }
        if !started {
            continue;
        }
        let Some(gives) = b.gives.clone() else {
            continue;
        };
        let Some((added, now, max)) = refill(
            &gives,
            b.amount,
            &lib,
            arsenal.as_deref_mut(),
            grenades.as_deref_mut(),
        ) else {
            say(
                &log,
                format!(
                    "pickups: touched {} but {gives} is full (or not carried): left lying",
                    b.name
                ),
            );
            continue;
        };
        b.taken = true;
        b.touching = false;
        b.back_in = b.respawn.unwrap_or(0.0);
        say(
            &log,
            format!(
                "pickups: took {} ({}): +{added} for {gives}, now {now} of {max}{}",
                b.name,
                b.class,
                if b.respawn.is_some() {
                    "; it comes back"
                } else {
                    ""
                }
            ),
        );
        taken.write(AmmoBoxTaken {
            name: b.name.clone(),
            gives,
            added,
            now,
            max,
        });
        picked.write(PickedUp {
            text: b.message.clone(),
            sound: b.sound.clone(),
        });
    }
}

/// Boxes that come back count down.
pub(crate) fn bring_back(time: Res<Time>, mut boxes: Query<&mut AmmoBox>) {
    for mut b in &mut boxes {
        if b.taken && b.respawn.is_some() {
            b.back_in -= time.delta_secs();
            if b.back_in <= 0.0 {
                b.taken = false;
                b.touching = false;
            }
        }
    }
}

/// Hide the model of a taken box; show it again when it comes back.
pub(crate) fn show_boxes(
    boxes: Query<&AmmoBox, Changed<AmmoBox>>,
    mut parts: Query<(&MoverPart, &mut Visibility)>,
) {
    if boxes.is_empty() {
        return;
    }
    let changed: BTreeMap<usize, bool> = boxes.iter().map(|b| (b.export, b.taken)).collect();
    for (p, mut v) in &mut parts {
        let Some(&taken) = changed.get(&p.0) else {
            continue;
        };
        let want = if taken {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want && (taken || *v == Visibility::Hidden) {
            *v = want;
        }
    }
}

/// Mark the boxes a save said were taken.
pub(crate) fn apply_saved(
    mut commands: Commands,
    pending: Res<TakenFromSave>,
    mut boxes: Query<&mut AmmoBox>,
    log: Option<Res<GameLog>>,
) {
    if boxes.is_empty() {
        return;
    }
    let mut n = 0;
    for mut b in &mut boxes {
        if pending.0.iter().any(|x| x.eq_ignore_ascii_case(&b.name)) {
            b.taken = true;
            b.touching = false;
            n += 1;
        }
    }
    say(
        &log,
        format!(
            "pickups: {n} of {} saved boxes marked taken",
            pending.0.len()
        ),
    );
    commands.remove_resource::<TakenFromSave>();
}

/// The `[pickups]` section of a save file: the boxes and weapons already
/// taken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickupsSave {
    /// Names of the taken boxes (`DC17mBlasterBox14`). Boxes that come back
    /// by themselves aren't listed.
    #[serde(default)]
    pub taken: Vec<String>,
    /// Names of the weapons the map placed that were taken
    /// (`BowcasterPickup0`). **Not saved:** weapons thrown down by the
    /// player or dropped by enemies.
    #[serde(default)]
    pub weapons_taken: Vec<String>,
}

impl SavePart for PickupsSave {
    const KEY: &'static str = "pickups";

    fn save(world: &mut World) -> Option<Self> {
        let mut q = world.query::<&AmmoBox>();
        let taken: Vec<String> = q
            .iter(world)
            .filter(|b| b.taken && b.respawn.is_none())
            .map(|b| b.name.clone())
            .collect();
        let weapons_taken = world
            .get_resource::<crate::pickup::WeaponsGone>()
            .map(|g| g.0.iter().map(|(_, n)| n.clone()).collect())
            .unwrap_or_default();
        Some(PickupsSave {
            taken,
            weapons_taken,
        })
    }

    fn load(world: &mut World, d: Self) -> Result<(), String> {
        world.insert_resource(TakenFromSave(d.taken));
        world.insert_resource(crate::pickup::WeaponsTakenFromSave(d.weapons_taken));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapon::WeaponState;

    fn lib() -> WeaponLibrary {
        let mut lib = WeaponLibrary::default();
        // The real DC-17m file: magazine 60, carries 300.
        let rifle = crate::weapon::WeaponParams::from_text(
            "rifle",
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../data/weapons/dc17m_blaster.toml"
            ))
            .unwrap(),
        )
        .unwrap();
        lib.weapons.insert("rifle".into(), rifle);
        let g = crate::GrenadeParams::from_text(
            "td",
            "name = \"TD\"\n[ammo]\nmax_count = 5\nstart_count = 2\n[throw]\nspeed = 1000\ntoss_up = 200\n[fuse]\ntime = 3\nafter_impact = 1\n[flight]\nbounce = 0.5\nactor_bounce = 0.1\ngravity = 1000\n[blast]\ndamage = 100\nradius = 500\n",
        )
        .unwrap();
        lib.grenades.insert("td".into(), g);
        lib
    }

    fn arsenal(lib: &WeaponLibrary, clip: f32, reserve: u32) -> Arsenal {
        let mut s = WeaponState::new(&lib.weapons["rifle"]);
        s.clip = clip;
        s.reserve = reserve;
        Arsenal {
            weapons: vec![("rifle".into(), s)],
            current: 0,
        }
    }

    #[test]
    fn reads_the_rules_file() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/pickups/ammo_boxes.toml"
        ))
        .unwrap();
        let r = AmmoBoxRules::from_text(&text).unwrap();
        assert_eq!(r.kinds["dc17mblasterammo"], "dc17m_blaster");
        assert_eq!(r.kinds.len(), 7);
        let r = AmmoBoxRules::from_text("[ammo]\nX = \"y\"\n[amount]\nBox = 12\n").unwrap();
        assert_eq!(r.amounts["box"], 12);
        assert!(AmmoBoxRules::from_text("[amount]\nBox = -1\n").is_err());
    }

    #[test]
    fn a_box_tops_up_to_the_most_carried() {
        let lib = lib();
        let mut a = arsenal(&lib, 60.0, 100);
        assert_eq!(
            refill("rifle", 300, &lib, Some(&mut a), None),
            Some((140, 300, 300))
        );
        assert_eq!(a.weapons[0].1.reserve, 240);
        // Full: left lying.
        assert_eq!(refill("rifle", 300, &lib, Some(&mut a), None), None);
    }

    #[test]
    fn an_empty_weapon_in_hand_reloads() {
        let lib = lib();
        let mut a = arsenal(&lib, 0.0, 0);
        assert_eq!(
            refill("rifle", 300, &lib, Some(&mut a), None),
            Some((300, 300, 300))
        );
        assert!(a.weapons[0].1.reloading.is_some());
    }

    #[test]
    fn ammo_not_carried_cant_be_taken() {
        let lib = lib();
        let mut a = Arsenal {
            weapons: vec![],
            current: 0,
        };
        assert_eq!(refill("rifle", 300, &lib, Some(&mut a), None), None);
        assert_eq!(refill("nothing", 300, &lib, Some(&mut a), None), None);
    }

    #[test]
    fn grenades_fill_up_to_five() {
        let lib = lib();
        let mut g = Grenades {
            carried: vec![("td".into(), 2)],
            ..Default::default()
        };
        assert_eq!(refill("td", 5, &lib, None, Some(&mut g)), Some((3, 5, 5)));
        assert_eq!(refill("td", 5, &lib, None, Some(&mut g)), None);
    }

    #[test]
    fn touching_is_the_cylinders_overlapping() {
        let body = Hittable::COMMANDO;
        let feet = Vec3::ZERO;
        let u = 1.0 / UU_PER_UNIT;
        // A DC-17m box (40 wide, 15 half-high) on the floor, 70 uu away.
        let on_floor = |x: f32| Vec3::new(x * u, 15.0 * u, 0.0);
        assert!(touches(feet, &body, on_floor(70.0), 40.0, 15.0));
        assert!(!touches(feet, &body, on_floor(90.0), 40.0, 15.0));
        // On a ledge above the head, or a floor below.
        assert!(!touches(
            feet,
            &body,
            Vec3::new(0.0, 200.0 * u, 0.0),
            40.0,
            15.0
        ));
        assert!(!touches(
            feet,
            &body,
            Vec3::new(0.0, -20.0 * u, 0.0),
            40.0,
            15.0
        ));
    }
}
