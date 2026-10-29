//! Reads `MeshAnimation` objects: a set of animations ("sequences") for a
//! skeleton, such as walking, aiming or reloading.
//!
//! Each sequence stores one "track" per bone. A track is a list of key
//! poses (position and rotation of that bone, relative to its parent) and
//! how many frames each key lasts. Playing the animation means picking the
//! two keys around the current time and blending between them.
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov): `Unreal/UnrealMesh/UnMesh2.h` (UMeshAnimation, FNamedBone,
//! FMeshAnimSeq, FMeshAnimNotify) and `UnAnim2.cpp` (`SerializeSWRCAnims`,
//! `AnalogTrack::SerializeSWRC`, `FVectorShortSWRC`). Written from those
//! descriptions; no code copied.
//!
//! Republic Commando packs its tracks small: positions and rotations are
//! three 16-bit numbers each, and key durations are one byte (in frames).
//! - Position = number / 32767 * the track's `pos_scale`.
//! - Rotation (package version 151 and later): the three numbers are three of
//!   the quaternion's four parts, each scaled to +-1/sqrt(2). The lowest bit
//!   of each number is borrowed as a flag: X and Y's bits say which part was
//!   left out, Z's bit says its sign. The left-out part is rebuilt so the
//!   quaternion has length 1. (The largest part is the one usually left out,
//!   which is why +-1/sqrt(2) is enough for the other three.)
//! - Rotation (older packages): an axis times an angle, 32767 = pi.

use crate::properties::{self, Property};
use crate::skeletal::{array, export_data};
use crate::{Export, Package, Reader, Result};

const RF_HAS_STACK: u32 = 0x0200_0000;

/// A bone the animation moves, matched to the mesh's bones by name.
#[derive(Debug, Clone)]
pub struct NamedBone {
    pub name: String,
    pub flags: u32,
    pub parent: i32,
}

/// Something that happens at a moment of an animation (a footstep sound,
/// the gun firing, ...). `time` is 0.0 to 1.0 through the sequence.
#[derive(Debug, Clone)]
pub struct Notify {
    pub time: f32,
    pub function: String,
    /// Object reference to the `AnimNotify_*` object, 0 if none.
    pub object: i32,
}

/// One bone's keys in one sequence, still in the packed form.
#[derive(Debug, Clone, Default)]
pub struct Track {
    pub pos_scale: f32,
    pub positions: Vec<[i16; 3]>,
    pub rotations: Vec<[i16; 3]>,
    /// How many frames each key lasts.
    pub durations: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: String,
    pub groups: Vec<String>,
    pub start_frame: i32,
    pub num_frames: i32,
    /// Frames per second.
    pub rate: f32,
    pub notifies: Vec<Notify>,
    /// Numbers Republic Commando adds whose meaning we don't know. Kept so
    /// the log can show them.
    pub unknown: Vec<f32>,
    /// One track per bone in `MeshAnimation::bones`.
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub struct MeshAnimation {
    pub properties: Vec<Property>,
    pub version: i32,
    pub bones: Vec<NamedBone>,
    pub sequences: Vec<Sequence>,
    /// Whether rotations use the newer packing (package version 151+).
    pub new_rotation_format: bool,
    pub leftover: usize,
}

fn name_text(pkg: &Package, r: &mut Reader) -> Result<String> {
    let i = r.compact_index()?;
    if !(0..pkg.names.len() as i32).contains(&i) {
        return r.invalid(format!("name index {i} is out of range"));
    }
    Ok(pkg.name(i).to_string())
}

fn short3(r: &mut Reader) -> Result<[i16; 3]> {
    Ok([r.i16()?, r.i16()?, r.i16()?])
}

/// Read a `MeshAnimation` export. `data` is the whole package file.
pub fn read_mesh_animation(pkg: &Package, data: &[u8], export: &Export) -> Result<MeshAnimation> {
    let (_, object) = export_data(data, export)?;
    let version = pkg.summary.version;
    let mut r = Reader::new(object);
    if export.object_flags & RF_HAS_STACK != 0 {
        return r.invalid("object has a script state frame, not supported for animations");
    }
    let properties = properties::read_properties(pkg, &mut r)?;
    let r = &mut r;
    let anim_version = r.i32()?;
    let bones = array(r, |r| {
        Ok(NamedBone {
            name: name_text(pkg, r)?,
            flags: r.u32()?,
            parent: r.i32()?,
        })
    })?;
    let sequences = array(r, |r| {
        let mut unknown = Vec::new();
        if version >= 115 {
            unknown.push(r.f32()?);
        }
        let name = name_text(pkg, r)?;
        let groups = array(r, |r| name_text(pkg, r))?;
        let start_frame = r.i32()?;
        let num_frames = r.i32()?;
        let notifies = array(r, |r| {
            Ok(Notify {
                time: r.f32()?,
                function: name_text(pkg, r)?,
                object: if version >= 112 {
                    r.compact_index()?
                } else {
                    0
                },
            })
        })?;
        let rate = r.f32()?;
        // Republic Commando additions to the sequence.
        unknown.push(r.i32()? as f32);
        unknown.push(r.f32()?);
        unknown.push(r.u8()? as f32);
        if version >= 144 {
            unknown.push(r.f32()?);
        }
        if version < 143 {
            unknown.push(r.i32()? as f32);
        }
        unknown.push(r.i32()? as f32);
        unknown.push(r.i32()? as f32);
        if version >= 143 {
            unknown.push(r.i32()? as f32);
        }
        let tracks = array(r, |r| {
            Ok(Track {
                pos_scale: r.f32()?,
                positions: array(r, short3)?,
                rotations: array(r, short3)?,
                durations: array(r, |r| r.u8())?,
            })
        })?;
        Ok(Sequence {
            name,
            groups,
            start_frame,
            num_frames,
            rate,
            notifies,
            unknown,
            tracks,
        })
    })?;
    Ok(MeshAnimation {
        properties,
        version: anim_version,
        bones,
        sequences,
        new_rotation_format: version >= 151,
        leftover: object.len() - r.pos(),
    })
}

impl Track {
    /// Positions in the mesh's units.
    pub fn key_positions(&self) -> Vec<[f32; 3]> {
        let s = self.pos_scale / 32767.0;
        self.positions
            .iter()
            .map(|p| [p[0] as f32 * s, p[1] as f32 * s, p[2] as f32 * s])
            .collect()
    }

    /// Rotations as quaternions x, y, z, w, unpacked but otherwise as stored
    /// (the same convention as `skeletal::Bone::orientation`).
    pub fn key_rotations(&self, new_format: bool) -> Vec<[f32; 4]> {
        self.rotations
            .iter()
            .map(|&k| {
                if new_format {
                    unpack_quat(k)
                } else {
                    unpack_quat_old(k)
                }
            })
            .collect()
    }

    /// The frame each key starts on (0, then the running total of durations).
    pub fn key_times(&self) -> Vec<f32> {
        let mut t = 0u32;
        self.durations
            .iter()
            .map(|&d| {
                let start = t as f32;
                t += d as u32;
                start
            })
            .collect()
    }
}

/// Newer packing: see the module notes.
pub fn unpack_quat(k: [i16; 3]) -> [f32; 4] {
    let s = std::f32::consts::FRAC_1_SQRT_2 / 32767.0;
    let a = (k[0] & !1) as f32 * s;
    let b = (k[1] & !1) as f32 * s;
    let c = (k[2] & !1) as f32 * s;
    let mut d = (1.0 - (a * a + b * b + c * c)).max(0.0).sqrt();
    if k[2] & 1 != 0 {
        d = -d;
    }
    match (k[0] & 1 != 0, k[1] & 1 != 0) {
        (false, false) => [a, b, c, d],
        (true, false) => [b, c, d, a],
        (false, true) => [c, d, a, b],
        (true, true) => [d, a, b, c],
    }
}

/// Older packing: axis * angle, 32767 = pi.
pub fn unpack_quat_old(k: [i16; 3]) -> [f32; 4] {
    let s = std::f32::consts::PI / 32767.0;
    let mut v = [k[0] as f32 * s, k[1] as f32 * s, k[2] as f32 * s];
    let angle = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if angle > 0.0 {
        // UE Viewer notes this scaling looks odd ("strange code") but it is
        // what the game does: sin(angle/2)/angle, not sin(angle/2)/|axis|.
        let scale = (angle / 2.0).sin() / angle;
        for x in &mut v {
            *x *= scale;
        }
    }
    let w = (1.0 - (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]))
        .max(0.0)
        .sqrt();
    [v[0], v[1], v[2], w]
}

impl MeshAnimation {
    /// Checks that would fail if we read the format wrongly.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let nb = self.bones.len();
        for (i, b) in self.bones.iter().enumerate() {
            let ok = if i == 0 {
                b.parent <= 0
            } else {
                b.parent >= 0 && (b.parent as usize) < i
            };
            if !ok {
                problems.push(format!("bone {i} ({}) has parent {}", b.name, b.parent));
            }
        }
        for s in &self.sequences {
            if s.tracks.len() != nb {
                problems.push(format!(
                    "sequence {} has {} tracks for {nb} bones",
                    s.name,
                    s.tracks.len()
                ));
            }
            if s.num_frames < 1 || !(s.rate > 0.0 && s.rate < 1000.0) {
                problems.push(format!(
                    "sequence {}: {} frames at {} per second",
                    s.name, s.num_frames, s.rate
                ));
            }
            for (bi, t) in s.tracks.iter().enumerate() {
                let nt = t.durations.len();
                let shape_ok = (t.positions.len() <= 1 || t.positions.len() == nt)
                    && (t.rotations.len() <= 1 || t.rotations.len() == nt);
                if !shape_ok {
                    problems.push(format!(
                        "sequence {} bone {bi}: {} positions, {} rotations, {nt} times",
                        s.name,
                        t.positions.len(),
                        t.rotations.len()
                    ));
                    break;
                }
                let total: u32 = t.durations.iter().map(|&d| d as u32).sum();
                if nt > 1 && total as i32 > s.num_frames + 1 {
                    problems.push(format!(
                        "sequence {} bone {bi}: keys last {total} frames, sequence has {}",
                        s.name, s.num_frames
                    ));
                    break;
                }
            }
        }
        if self.leftover != 0 {
            problems.push(format!("{} bytes left over", self.leftover));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(q: [f32; 4]) -> f32 {
        q.iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    #[test]
    fn packed_quaternions_have_unit_length() {
        for k in [[0, 0, 0], [1000, -2001, 3000], [23170, 0, 1], [-5, 7, -9]] {
            assert!((len(unpack_quat(k)) - 1.0).abs() < 1e-3, "{k:?}");
            assert!((len(unpack_quat_old(k)) - 1.0).abs() < 1e-3, "{k:?}");
        }
        // No flags: the left-out part is w, and it's positive.
        assert_eq!(unpack_quat([0, 0, 0]), [0.0, 0.0, 0.0, 1.0]);
        // Z's flag makes the rebuilt part negative.
        assert_eq!(unpack_quat([0, 0, 1]), [0.0, 0.0, 0.0, -1.0]);
        // X's flag alone: the parts shift, so the rebuilt one lands in z.
        assert_eq!(unpack_quat([1, 0, 0]), [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn key_times_are_running_totals() {
        let t = Track {
            durations: vec![2, 3, 1],
            ..Default::default()
        };
        assert_eq!(t.key_times(), vec![0.0, 2.0, 5.0]);
    }
}
