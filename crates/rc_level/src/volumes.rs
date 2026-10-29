//! Blocking volumes: the invisible walls level designers placed to keep the
//! player (and sometimes the AI) out of places, such as the edge of a
//! cliff or a gap between props.
//!
//! A `BlockingVolume` is a brush actor: its shape is a small BSP `Model`
//! (the `Brush` property) in the actor's own coordinates. To place it:
//! subtract `PrePivot`, turn by `Rotation`, then move to `Location`, the
//! same as a static mesh. Brushes also have `MainScale` and `PostScale`;
//! in all 79 maps of the game both are 1, so they are not applied (a scale
//! other than 1 is reported).
//!
//! Which ones block what: UE2's `bBlockPlayers` (default true) decides
//! whether players are stopped, `bBlockActors` (default true) whether other
//! characters (the AI) are. Shots go through unless `bBlockZeroExtentTraces`
//! is set. Names follow the UE2 `Actor`/`Volume` script classes in the
//! game's own `Engine.u`.

use crate::actors::{self, LevelInfo, Placement};
use crate::bsp::{self, Vec3};
use rc_package::properties::{self, Property, Value};
use rc_package::Package;

#[derive(Debug, Clone)]
pub struct BlockingVolume {
    /// Export index (0-based) of the actor.
    pub export: usize,
    pub name: String,
    pub location: Vec3,
    /// Stops players (`bBlockPlayers`).
    pub block_players: bool,
    /// Stops other characters (`bBlockActors`).
    pub block_actors: bool,
    /// Stops shots and other line checks (`bBlockZeroExtentTraces`).
    pub block_traces: bool,
    /// The faces, in world coordinates (Unreal units).
    pub polygons: Vec<Vec<Vec3>>,
}

#[derive(Debug, Clone, Default)]
pub struct VolumeReport {
    pub volumes: Vec<BlockingVolume>,
    /// Volumes that couldn't be read, and why.
    pub failures: Vec<String>,
    /// Volumes whose MainScale or PostScale isn't 1 (drawn unscaled).
    pub scaled: usize,
}

fn bool_prop(pkg: &Package, props: &[Property], name: &str, default: bool) -> bool {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Bool(b)) => *b,
        _ => default,
    }
}

/// The `Scale` vector inside a stored `Scale` struct (`MainScale`,
/// `PostScale`). Republic Commando stores these structs as their own small
/// tagged property list: name, a type byte (0x3A = a 12-byte struct), the
/// struct's type name, then the three floats. Returns `None` if the bytes
/// don't look like that.
fn scale_vector(raw: &[u8]) -> Option<Vec3> {
    let mut r = rc_package::Reader::new(raw);
    r.compact_index().ok()?;
    if r.u8().ok()? != 0x3A {
        return None;
    }
    r.compact_index().ok()?;
    let b = r.bytes(12).ok()?;
    let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
    Some([f(0), f(1), f(2)])
}

fn is_unit_scale(pkg: &Package, props: &[Property], name: &str) -> bool {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Raw(b)) => {
            scale_vector(b).is_none_or(|s| s.iter().all(|c| (c - 1.0).abs() < 1e-4))
        }
        _ => true,
    }
}

/// Read every `BlockingVolume` in the level.
pub fn blocking_volumes(pkg: &Package, data: &[u8], level: &LevelInfo) -> VolumeReport {
    let mut out = VolumeReport::default();
    for &a in level.actors.iter().flatten() {
        if pkg.export_class_name(&pkg.exports[a]) != "BlockingVolume" {
            continue;
        }
        match read_volume(pkg, data, a) {
            Ok((v, unit)) => {
                out.scaled += !unit as usize;
                out.volumes.push(v);
            }
            Err(e) => out
                .failures
                .push(format!("{}: {e}", pkg.name(pkg.exports[a].object_name))),
        }
    }
    out
}

fn read_volume(pkg: &Package, data: &[u8], a: usize) -> Result<(BlockingVolume, bool), String> {
    let props = actors::read_actor_properties(pkg, data, a).map_err(|e| e.to_string())?;
    let place: Placement = actors::read_placement(pkg, data, a).map_err(|e| e.to_string())?;
    let brush = match properties::find(pkg, &props, "Brush").map(|p| &p.value) {
        Some(Value::Object(o)) if *o > 0 => (*o - 1) as usize,
        _ => return Err("no Brush model".into()),
    };
    let model = pkg
        .exports
        .get(brush)
        .ok_or("Brush points outside the map")?;
    let bsp = bsp::read_model(pkg, data, model).map_err(|e| format!("brush model: {e}"))?;
    let polygons: Vec<Vec<Vec3>> = bsp
        .polygons()
        .into_iter()
        .map(|p| p.points.iter().map(|v| place.transform_point(*v)).collect())
        .collect();
    if polygons.is_empty() {
        return Err("brush model has no polygons".into());
    }
    let unit = is_unit_scale(pkg, &props, "MainScale") && is_unit_scale(pkg, &props, "PostScale");
    Ok((
        BlockingVolume {
            export: a,
            name: place.name.clone(),
            location: place.location,
            block_players: bool_prop(pkg, &props, "bBlockPlayers", true),
            block_actors: bool_prop(pkg, &props, "bBlockActors", true),
            block_traces: bool_prop(pkg, &props, "bBlockZeroExtentTraces", false),
            polygons,
        },
        unit,
    ))
}

impl BlockingVolume {
    /// The faces as triangles (a fan from each face's first corner).
    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.polygons
            .iter()
            .flat_map(|p| (1..p.len().saturating_sub(1)).map(move |k| [p[0], p[k], p[k + 1]]))
    }

    /// Is the point inside the volume? Counts how many faces a line going
    /// straight up from the point crosses: an odd number means inside. Works
    /// for any closed shape, not only boxes.
    pub fn contains(&self, p: Vec3) -> bool {
        let mut crossings = 0;
        for [a, b, c] in self.triangles() {
            // Where the vertical line through p meets the triangle's plane,
            // if it passes inside the triangle seen from above.
            let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
            if d.abs() < 1e-9 {
                continue;
            }
            let l1 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
            let l2 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
            let l3 = 1.0 - l1 - l2;
            // Half-open on one side so a line through a shared edge counts once.
            if l1 < 0.0 || l2 < 0.0 || l3 <= 0.0 {
                continue;
            }
            let z = l1 * a[2] + l2 * b[2] + l3 * c[2];
            if z > p[2] {
                crossings += 1;
            }
        }
        crossings % 2 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> BlockingVolume {
        // A 2x2x2 box around the origin, as six square faces.
        let c = |x: f32, y: f32, z: f32| [x, y, z];
        let faces = vec![
            vec![
                c(-1., -1., -1.),
                c(1., -1., -1.),
                c(1., 1., -1.),
                c(-1., 1., -1.),
            ],
            vec![
                c(-1., -1., 1.),
                c(1., -1., 1.),
                c(1., 1., 1.),
                c(-1., 1., 1.),
            ],
            vec![
                c(-1., -1., -1.),
                c(1., -1., -1.),
                c(1., -1., 1.),
                c(-1., -1., 1.),
            ],
            vec![
                c(-1., 1., -1.),
                c(1., 1., -1.),
                c(1., 1., 1.),
                c(-1., 1., 1.),
            ],
            vec![
                c(-1., -1., -1.),
                c(-1., 1., -1.),
                c(-1., 1., 1.),
                c(-1., -1., 1.),
            ],
            vec![
                c(1., -1., -1.),
                c(1., 1., -1.),
                c(1., 1., 1.),
                c(1., -1., 1.),
            ],
        ];
        BlockingVolume {
            export: 0,
            name: "cube".into(),
            location: [0.0; 3],
            block_players: true,
            block_actors: true,
            block_traces: false,
            polygons: faces,
        }
    }

    #[test]
    fn inside_and_outside_a_box() {
        let v = cube();
        assert!(v.contains([0.0, 0.0, 0.0]));
        assert!(v.contains([0.5, -0.3, 0.9]));
        // Exactly below a shared edge of two top triangles.
        assert!(v.contains([0.0, 0.0, -0.5]));
        assert!(!v.contains([0.0, 0.0, 2.0]));
        assert!(!v.contains([0.0, 0.0, -2.0]));
        assert!(!v.contains([1.5, 0.0, 0.0]));
        assert_eq!(v.triangles().count(), 12);
    }

    #[test]
    fn reads_a_stored_scale() {
        // Name 5, struct type 0x3A, struct name 4, then 1.0, 2.0, 3.0.
        let mut raw = vec![5, 0x3A, 4];
        for f in [1.0f32, 2.0, 3.0] {
            raw.extend(f.to_le_bytes());
        }
        assert_eq!(scale_vector(&raw), Some([1.0, 2.0, 3.0]));
        assert_eq!(scale_vector(&[1, 2, 3]), None);
    }
}
