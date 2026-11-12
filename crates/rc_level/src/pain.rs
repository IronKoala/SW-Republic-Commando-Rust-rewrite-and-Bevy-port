//! Pain volumes: the invisible areas that hurt or kill whoever is inside,
//! such as the bottom of a pit (`DamageFallToDeath`) or a crushing machine
//! (`Crushed`).
//!
//! In the original these are `PhysicsVolume`s (or subclasses) with
//! `bPainCausing` on and a `DamagePerSec` other than 0. Their rules are the
//! game's own UnrealScript (`Engine.PhysicsVolume`, read as text from the
//! install's `engine.u`); `rc_scripts_plugin::pain` carries them out. This
//! part only reads the volumes and their settings from the map.
//!
//! Settings the map leaves out come from the volume's class defaults,
//! which the caller looks up (`Defaults`), so a modded volume class with
//! its own defaults works too.

use crate::actors::{self, LevelInfo};
use crate::volumes::{self, BlockingVolume};
use rc_package::properties::{self, Property, Value};
use rc_package::Package;

/// One volume that hurts (or can be switched to hurt by an event).
#[derive(Debug, Clone)]
pub struct PainVolume {
    /// Export index (0-based) of the actor.
    pub export: usize,
    pub name: String,
    pub class: String,
    /// The name events reach it by (`Tag`; its class when the map sets
    /// none, as in UE2). An event with this name switches the pain on or
    /// off (`PhysicsVolume.Trigger`).
    pub tag: String,
    /// Hurting now (`bPainCausing`).
    pub pain_causing: bool,
    /// Damage each time it hurts (`DamagePerSec`, once a second).
    /// Below 0 it heals instead.
    pub damage_per_sec: f32,
    /// The damage class (`DamageType`), e.g. `DamageFallToDeath`.
    pub damage_type: String,
    /// Only characters of this class or its subclasses are hurt
    /// (`ClassProximityType`); `None`: anything that can be hurt.
    pub proximity: Option<String>,
    /// The shape, in world coordinates.
    pub shape: BlockingVolume,
    /// The shape's bounding box (lowest and highest corner).
    pub bounds: ([f32; 3], [f32; 3]),
}

/// What a volume's class sets when the map doesn't.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    pub pain_causing: bool,
    pub damage_per_sec: f32,
    pub damage_type: Option<String>,
    pub proximity: Option<String>,
}

/// Everything [`read`] found.
#[derive(Debug, Clone, Default)]
pub struct PainReport {
    pub volumes: Vec<PainVolume>,
    /// Volumes that couldn't be read, and why.
    pub failures: Vec<String>,
}

fn bool_of(pkg: &Package, props: &[Property], name: &str) -> Option<bool> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// `Some(None)` when the map sets the object reference to None.
fn object_of(pkg: &Package, props: &[Property], name: &str) -> Option<Option<String>> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Object(0)) => Some(None),
        Some(Value::Object(o)) => Some(Some(pkg.object_name(*o).to_string())),
        _ => None,
    }
}

/// Read every pain volume in the level. `physics_volume` says whether a
/// class is a `PhysicsVolume` (or a subclass); `defaults` gives a class's
/// own settings. Volumes whose `DamagePerSec` is 0 are left out: the
/// original never hurts with them, and an event can't switch them on.
pub fn read(
    pkg: &Package,
    data: &[u8],
    level: &LevelInfo,
    physics_volume: &dyn Fn(&str) -> bool,
    defaults: &dyn Fn(&str) -> Defaults,
) -> PainReport {
    let mut out = PainReport::default();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]).to_string();
        if !physics_volume(&class) {
            continue;
        }
        let name = pkg.name(pkg.exports[a].object_name).to_string();
        let props = match actors::read_actor_properties(pkg, data, a) {
            Ok(p) => p,
            Err(e) => {
                out.failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let d = defaults(&class);
        let damage_per_sec = match properties::find(pkg, &props, "DamagePerSec").map(|p| &p.value) {
            Some(Value::Float(f)) => *f,
            _ => d.damage_per_sec,
        };
        if damage_per_sec == 0.0 {
            continue;
        }
        let tag = match properties::find(pkg, &props, "Tag").map(|p| &p.value) {
            Some(Value::Name(n)) => pkg.name(*n).to_string(),
            _ => class.clone(),
        };
        let shape = match volumes::read_volume(pkg, data, a) {
            Ok((v, _)) => v,
            Err(e) => {
                out.failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in shape.polygons.iter().flatten() {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        out.volumes.push(PainVolume {
            export: a,
            name,
            class,
            tag,
            pain_causing: bool_of(pkg, &props, "bPainCausing").unwrap_or(d.pain_causing),
            damage_per_sec,
            damage_type: object_of(pkg, &props, "DamageType")
                .unwrap_or(d.damage_type)
                .unwrap_or_else(|| "DamageType".into()),
            proximity: object_of(pkg, &props, "ClassProximityType").unwrap_or(d.proximity),
            shape,
            bounds: (lo, hi),
        });
    }
    out
}

impl PainVolume {
    /// Does a character's collision cylinder (`centre`, `radius`,
    /// `half_height`, Unreal units) touch the volume?
    ///
    /// The original tests the cylinder against the brush exactly, in its
    /// compiled engine. This tests 11 points of the cylinder instead: the
    /// middle, the top and bottom middles, and four points around the edge
    /// at the bottom and at the top. A character reaching into a volume by
    /// less than that (a sliver of its edge) isn't counted.
    pub fn touches(&self, centre: [f32; 3], radius: f32, half_height: f32) -> bool {
        let (lo, hi) = self.bounds;
        let outside = (0..2).any(|i| centre[i] + radius < lo[i] || centre[i] - radius > hi[i])
            || centre[2] + half_height < lo[2]
            || centre[2] - half_height > hi[2];
        if outside {
            return false;
        }
        let [x, y, z] = centre;
        let mut points = vec![[x, y, z], [x, y, z - half_height], [x, y, z + half_height]];
        for dz in [-half_height, half_height] {
            for (dx, dy) in [(radius, 0.0), (-radius, 0.0), (0.0, radius), (0.0, -radius)] {
                points.push([x + dx, y + dy, z + dz]);
            }
        }
        points.into_iter().any(|p| self.shape.contains(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(lo: [f32; 3], hi: [f32; 3]) -> PainVolume {
        let c = |x: f32, y: f32, z: f32| [x, y, z];
        let (a, b) = (lo, hi);
        let faces = vec![
            vec![
                c(a[0], a[1], a[2]),
                c(b[0], a[1], a[2]),
                c(b[0], b[1], a[2]),
                c(a[0], b[1], a[2]),
            ],
            vec![
                c(a[0], a[1], b[2]),
                c(b[0], a[1], b[2]),
                c(b[0], b[1], b[2]),
                c(a[0], b[1], b[2]),
            ],
            vec![
                c(a[0], a[1], a[2]),
                c(b[0], a[1], a[2]),
                c(b[0], a[1], b[2]),
                c(a[0], a[1], b[2]),
            ],
            vec![
                c(a[0], b[1], a[2]),
                c(b[0], b[1], a[2]),
                c(b[0], b[1], b[2]),
                c(a[0], b[1], b[2]),
            ],
            vec![
                c(a[0], a[1], a[2]),
                c(a[0], b[1], a[2]),
                c(a[0], b[1], b[2]),
                c(a[0], a[1], b[2]),
            ],
            vec![
                c(b[0], a[1], a[2]),
                c(b[0], b[1], a[2]),
                c(b[0], b[1], b[2]),
                c(b[0], a[1], b[2]),
            ],
        ];
        PainVolume {
            export: 0,
            name: "pit".into(),
            class: "PhysicsVolume".into(),
            tag: "PhysicsVolume".into(),
            pain_causing: true,
            damage_per_sec: 500.0,
            damage_type: "DamageFallToDeath".into(),
            proximity: None,
            shape: BlockingVolume {
                export: 0,
                name: "pit".into(),
                location: [0.0; 3],
                block_players: false,
                block_actors: false,
                block_traces: false,
                polygons: faces,
            },
            bounds: (lo, hi),
        }
    }

    #[test]
    fn a_character_touches_when_any_part_is_inside() {
        // A pit from z -1000 to -200; a commando is 34 wide and 84 half tall.
        let v = boxed([-500.0, -500.0, -1000.0], [500.0, 500.0, -200.0]);
        // Standing on the ground above it: feet at 0.
        assert!(!v.touches([0.0, 0.0, 84.0], 34.0, 84.0));
        // Fallen in: the middle is inside.
        assert!(v.touches([0.0, 0.0, -400.0], 34.0, 84.0));
        // Only the feet reach the top of the pit.
        assert!(v.touches([0.0, 0.0, -150.0], 34.0, 84.0));
        // Beside it, an edge point reaching in.
        assert!(v.touches([520.0, 0.0, -400.0], 34.0, 84.0));
        assert!(!v.touches([600.0, 0.0, -400.0], 34.0, 84.0));
    }
}
