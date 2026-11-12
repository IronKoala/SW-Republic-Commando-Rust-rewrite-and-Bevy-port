//! The droid dispensers in the level's world: riding what they are based
//! on, being solid, and crushing whoever they land on.
//!
//! What the original does, read from the game's files and Engine.dll
//! (read only, 2026-11-12; nothing decompiled is kept here):
//!
//! - **Riding a mover.** A dispenser's `AttachTag` bases it, when the
//!   level loads (`UGameEngine::LoadMap`), on the first actor whose tag or
//!   object name is that tag (`SetBase`). From then on the base carries it
//!   rigidly (`ULevel::MoveActor`: its place relative to the base is kept
//!   and it turns as the base turns). In the maps every such base is a
//!   mover: the transports that fly ras_04a's and ras_04b's dispensers in.
//! - **Solid.** `DroidDispenser`'s defaults: `bCollideActors`,
//!   `bBlockActors`, `bBlockPlayers`; its collision is its cylinder
//!   (`CollisionRadius`, `CollisionHeight`).
//! - **Crushing.** Its `Bump` deals a pawn its `MaxHealth` as `Crushed`
//!   damage while it plays `entranceSky`, `EntranceSky704` or
//!   `EntranceSky384`. The engine bumps both ways whenever a move is
//!   blocked by the other (`ULevel::MoveActor` calls `NotifyBumpEx` on
//!   each), so a pawn the falling machine runs into, or one that walks into
//!   it then, is crushed. `Crushed` is a plain `DamageType`: no damage
//!   table lists it (x1), shields take it first, and it kills the player
//!   rather than leaving them incapacitated (`bKillsNotIncapacitatesPlayer`).

use crate::dispensers::DispenserBody;
use crate::{Fight, PART};
use bevy::prelude::*;
use rc_ai::Carry;
use rc_combat::{Damage, DamageKind, DamageTable, Dead, Health, Hittable, WeaponLibrary};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, LevelCollision};
use rc_movement::{Hit, Vec3 as UVec3, World};
use rc_movers::{Motion, Pose};
use rc_movers_plugin::LevelMovers;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// The animations during which the machine crushes (`DroidDispenser.Bump`).
const CRUSHING: [&str; 3] = ["entranceSky", "EntranceSky704", "EntranceSky384"];

/// Move each dispenser with the mover it is based on (`AttachTag`).
pub fn ride_bases(
    mut fight: ResMut<Fight>,
    movers: Option<Res<LevelMovers>>,
    mut told: Local<HashSet<usize>>,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let wanted: Vec<(usize, String, String)> = fight
        .battle
        .spawners
        .iter()
        .enumerate()
        .filter_map(|(i, sp)| {
            let d = sp.factory.dispenser.as_ref()?;
            Some((i, sp.factory.name.clone(), d.attach_tag.clone()?))
        })
        .collect();
    if wanted.is_empty() {
        return;
    }
    let Some(movers) = movers else { return };
    let set = movers.read();
    for (i, name, tag) in wanted {
        let found = set.movers.iter().find(|m| {
            m.info.tag.eq_ignore_ascii_case(&tag) || m.info.place.name.eq_ignore_ascii_case(&tag)
        });
        let carry = found.map(|m| {
            // Where it was when the level loaded: its starting key.
            let (pos, rot) = m.info.key(m.info.key_num as usize);
            let start = Pose {
                pos: UVec3::new(pos[0], pos[1], pos[2]),
                rot: rot.map(|r| r as f32),
            };
            let (rows, shift) = Motion::new(start, m.pose).matrix();
            Carry {
                turn: rows.map(|r| UVec3::new(r[0], r[1], r[2])),
                shift,
                yaw: (m.pose.rot[1] - start.rot[1]) * std::f32::consts::TAU / 65536.0,
            }
        });
        if told.insert(i) {
            match found {
                Some(m) => log.line(format!(
                    "{PART}: dispenser {name} rides mover {} (AttachTag {tag})",
                    m.info.place.name
                )),
                None => log.line(format!(
                    "{PART}: dispenser {name}: no mover is tagged or named {tag} (its AttachTag); it stays put"
                )),
            }
        }
        fight.battle.set_dispenser_carry(i, carry);
    }
}

/// Who can be crushed: anyone that can be hit and is alive.
type Pawns<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static Hittable,
        &'static Health,
        Option<&'static DamageTable>,
        Option<&'static Name>,
    ),
    (Without<DispenserBody>, Without<Dead>),
>;

/// While a dispenser plays a crushing animation, whoever it touches takes
/// their full health as damage, every frame they touch.
#[allow(clippy::too_many_arguments)]
pub fn crush(
    fight: Res<Fight>,
    bodies: Query<&DispenserBody>,
    pawns: Pawns,
    library: Option<Res<WeaponLibrary>>,
    mut hurt: MessageWriter<Damage>,
    mut last: Local<HashMap<usize, UVec3>>,
    mut told: Local<HashSet<(usize, Entity)>>,
    log: Res<GameLog>,
) {
    for b in &bodies {
        let centre = fight.battle.dispenser_location(b.spawner);
        let before = last.insert(b.spawner, centre).unwrap_or(centre);
        let crushing = b
            .playing
            .as_ref()
            .is_some_and(|(a, _, _)| CRUSHING.iter().any(|c| c.eq_ignore_ascii_case(a)));
        if !crushing {
            continue;
        }
        let Some(d) = fight.battle.spawners[b.spawner].factory.dispenser.as_ref() else {
            continue;
        };
        // The space the machine went through this frame.
        let (low, high) = (
            before.z.min(centre.z) - d.collision_height,
            before.z.max(centre.z) + d.collision_height,
        );
        for (e, t, h, health, table, name) in &pawns {
            let c = to_unreal(t.translation) + UVec3::new(0.0, 0.0, h.center_up);
            let flat = ((c.x - centre.x).powi(2) + (c.y - centre.y).powi(2)).sqrt();
            let touches = flat <= d.collision_radius + h.radius
                && c.z + h.half_height >= low
                && c.z - h.half_height <= high;
            if !touches {
                continue;
            }
            let mult = library.as_ref().map_or(1.0, |l| {
                l.damage_types
                    .multiplier(table.map(|t| t.0.as_str()), "Crushed")
            });
            let amount = health.max * mult;
            if told.insert((b.spawner, e)) {
                log.line(format!(
                    "{PART}: dispenser {} crushes {} ({amount} Crushed damage while it touches)",
                    b.name,
                    name.map_or_else(|| format!("{e:?}"), |n| n.to_string())
                ));
            }
            hurt.write(Damage {
                target: e,
                amount,
                kind: DamageKind::Other,
                instigator: None,
                point: to_bevy(c),
                direction: Vec3::NEG_Y,
            });
        }
    }
}

/// One dispenser's collision cylinder (Unreal units).
#[derive(Debug, Clone, Copy)]
struct Solid {
    centre: UVec3,
    radius: f32,
    half: f32,
}

/// The dispensers' cylinders, shared with the level's collision.
#[derive(Resource, Default)]
pub struct DispenserSolids {
    solids: Arc<RwLock<Vec<Solid>>>,
    /// The collision we made, so we know when someone replaced it.
    made: Option<Arc<dyn World + Send + Sync>>,
}

/// Keep the dispensers' cylinders where the machines are, and make them
/// part of the level's collision (wrapping whatever collision is there).
pub fn be_solid(
    mut commands: Commands,
    fight: Res<Fight>,
    bodies: Query<&DispenserBody>,
    collision: Option<Res<LevelCollision>>,
    mut solids: ResMut<DispenserSolids>,
) {
    let now: Vec<Solid> = bodies
        .iter()
        .filter_map(|b| {
            let d = fight
                .battle
                .spawners
                .get(b.spawner)?
                .factory
                .dispenser
                .as_ref()?;
            Some(Solid {
                centre: fight.battle.dispenser_location(b.spawner),
                radius: d.collision_radius,
                half: d.collision_height,
            })
        })
        .collect();
    *solids.solids.write().unwrap_or_else(|e| e.into_inner()) = now;
    let Some(collision) = collision else { return };
    let ours = solids
        .made
        .as_ref()
        .is_some_and(|m| std::ptr::addr_eq(Arc::as_ptr(m), Arc::as_ptr(&collision.0)));
    if ours {
        return;
    }
    let made: Arc<dyn World + Send + Sync> = Arc::new(WithDispensers {
        inner: collision.0.clone(),
        solids: solids.solids.clone(),
    });
    solids.made = Some(made.clone());
    commands.insert_resource(LevelCollision(made));
}

/// The level's collision plus the dispensers' cylinders.
struct WithDispensers {
    inner: Arc<dyn World + Send + Sync>,
    solids: Arc<RwLock<Vec<Solid>>>,
}

impl World for WithDispensers {
    fn sweep(&self, extent: UVec3, start: UVec3, end: UVec3) -> Option<Hit> {
        let mut best = self.inner.sweep(extent, start, end);
        // Bodies only: shots already hit the machine as a target, and a
        // body that starts inside one may move out.
        if extent.x <= 0.0 && extent.z <= 0.0 {
            return best;
        }
        let solids = self.solids.read().unwrap_or_else(|e| e.into_inner());
        for s in solids.iter() {
            if let Some(h) = sweep_cylinder(s, extent, start, end) {
                if best.is_none_or(|b| h.time < b.time) {
                    best = Some(h);
                }
            }
        }
        best
    }
}

/// When a box of half-size `extent` (taken as a cylinder of its X
/// half-width, as the engine's pawns are) moving from `start` to `end`
/// first touches cylinder `s`. `None` if it doesn't, or if it starts inside.
fn sweep_cylinder(s: &Solid, extent: UVec3, start: UVec3, end: UVec3) -> Option<Hit> {
    let r = s.radius + extent.x;
    let h = s.half + extent.z;
    let d = end - start;
    let p = start - s.centre;
    let inside = |q: UVec3| q.x * q.x + q.y * q.y <= r * r && q.z.abs() <= h;
    if inside(p) {
        return None;
    }
    // When it is within the circle (flat).
    let a = d.x * d.x + d.y * d.y;
    let b = 2.0 * (p.x * d.x + p.y * d.y);
    let c = p.x * p.x + p.y * p.y - r * r;
    let (t0, t1) = if a < 1e-9 {
        if c > 0.0 {
            return None;
        }
        (f32::NEG_INFINITY, f32::INFINITY)
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let q = disc.sqrt();
        ((-b - q) / (2.0 * a), (-b + q) / (2.0 * a))
    };
    // When it is between the top and the bottom.
    let (z0, z1) = if d.z.abs() < 1e-9 {
        if p.z.abs() > h {
            return None;
        }
        (f32::NEG_INFINITY, f32::INFINITY)
    } else {
        let ta = (-h - p.z) / d.z;
        let tb = (h - p.z) / d.z;
        (ta.min(tb), ta.max(tb))
    };
    let enter = t0.max(z0);
    let leave = t1.min(z1);
    if enter > leave || !(0.0..=1.0).contains(&enter) {
        return None;
    }
    let at = p + d * enter;
    let normal = if t0 >= z0 {
        let n = UVec3::new(at.x, at.y, 0.0);
        let l = n.length();
        if l > 1e-6 {
            n * (1.0 / l)
        } else {
            UVec3::new(1.0, 0.0, 0.0)
        }
    } else if d.z < 0.0 {
        UVec3::new(0.0, 0.0, 1.0)
    } else {
        UVec3::new(0.0, 0.0, -1.0)
    };
    // Stop a hair short of touching.
    let len = d.length().max(1e-6);
    Some(Hit {
        time: (enter - 0.1 / len).max(0.0),
        normal,
        start_solid: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_walking_into_the_machine_stops_at_its_side() {
        let s = Solid {
            centre: UVec3::new(0.0, 0.0, 125.0),
            radius: 170.0,
            half: 125.0,
        };
        let e = UVec3::new(40.0, 40.0, 84.0);
        let h = sweep_cylinder(
            &s,
            e,
            UVec3::new(-500.0, 0.0, 84.0),
            UVec3::new(0.0, 0.0, 84.0),
        )
        .expect("blocked");
        // Touches when its centre is 210 from the machine's: at x -210.
        assert!((-500.0 + 500.0 * h.time - -210.0).abs() < 0.5, "{h:?}");
        assert!((h.normal.x + 1.0).abs() < 1e-4);
        // Passing beside it misses.
        assert!(sweep_cylinder(
            &s,
            e,
            UVec3::new(-500.0, 300.0, 84.0),
            UVec3::new(500.0, 300.0, 84.0)
        )
        .is_none());
        // Falling onto it lands on its top (250 up, plus 84).
        let h = sweep_cylinder(
            &s,
            e,
            UVec3::new(0.0, 0.0, 600.0),
            UVec3::new(0.0, 0.0, 0.0),
        )
        .unwrap();
        assert!((600.0 - 600.0 * h.time - 334.0).abs() < 0.5, "{h:?}");
        assert_eq!(h.normal, UVec3::new(0.0, 0.0, 1.0));
        // Starting inside: free to move out.
        assert!(sweep_cylinder(
            &s,
            e,
            UVec3::new(0.0, 0.0, 84.0),
            UVec3::new(500.0, 0.0, 84.0)
        )
        .is_none());
    }
}
