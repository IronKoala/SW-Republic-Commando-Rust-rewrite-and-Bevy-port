//! Reads the level object (its list of actors and its BSP model) and each
//! actor's placement: where it is, how it's turned and scaled, and which
//! static mesh it shows.
//!
//! Layouts:
//! - The "state frame" at the start of actors (objects with the HasStack
//!   flag) follows the Unreal Wiki, "Package File Format / Data Details".
//! - `Level` (actor list, then the map's URL, then the model) follows the
//!   UE2 `ULevelBase`/`ULevel` field order in the public headers (read via
//!   SWRC-Modding/CT, no license, nothing copied).
//! - Republic Commando specifics were found by reading `geo_01a.ctm` and
//!   other maps: one extra byte before the actor list, and a 2-byte field in
//!   the state frame where UE2 has 4 bytes.

use crate::bsp::Vec3;
use rc_package::properties::{self, read_properties, Property, Value};
use rc_package::{Export, Package, Reader, Result};

/// Object flag: the object's data starts with a script "state frame".
pub const RF_HAS_STACK: u32 = 0x0200_0000;

#[derive(Debug, Clone)]
pub struct LevelInfo {
    /// Export indexes (0-based) of every actor slot; `None` for empty slots.
    pub actors: Vec<Option<usize>>,
    /// The map URL stored in the level, e.g. `republiccommando` / `Index.ctm`.
    pub url_protocol: String,
    pub url_map: String,
    /// Export index (0-based) of the level's BSP model.
    pub model: Option<usize>,
}

fn export_index(i: i32) -> Option<usize> {
    if i > 0 {
        Some((i - 1) as usize)
    } else {
        None
    }
}

fn object_bytes<'a>(data: &'a [u8], e: &Export) -> Result<&'a [u8]> {
    let start = e.serial_offset as usize;
    match data.get(start..start + e.serial_size as usize) {
        Some(b) => Ok(b),
        None => Reader::new(data).invalid("object data lies outside the file"),
    }
}

/// Skip the state frame if the object has one. Returns the frame's "node"
/// reference (0 if there is no frame).
fn skip_state_frame(r: &mut Reader, e: &Export) -> Result<i32> {
    if e.object_flags & RF_HAS_STACK == 0 {
        return Ok(0);
    }
    let node = r.compact_index()?;
    let _state_node = r.compact_index()?;
    let _probe_mask = r.bytes(8)?;
    // Republic Commando stores 2 bytes here where UE2 stores a 4-byte
    // "latent action" (found by the lighting thread; fits every actor read).
    let _latent_action = r.u16()?;
    if node != 0 {
        let _offset = r.compact_index()?;
    }
    Ok(node)
}

/// Find and read the map's `Level` object.
pub fn read_level(pkg: &Package, data: &[u8]) -> Result<LevelInfo> {
    let Some(e) = pkg
        .exports
        .iter()
        .find(|e| pkg.export_class_name(e) == "Level")
    else {
        return Reader::new(data).invalid("no Level object in this package");
    };
    let mut r = Reader::new(object_bytes(data, e)?);
    skip_state_frame(&mut r, e)?;
    read_properties(pkg, &mut r)?;
    // Republic Commando: one extra byte here, 0 in every map read so far.
    let extra = r.u8()?;
    if extra != 0 {
        return r.invalid(format!("byte before the actor list is {extra}, expected 0"));
    }
    let n = r.compact_index()?;
    if n < 0 || n as usize > r.len() - r.pos() {
        return r.invalid(format!("actor count {n} doesn't fit"));
    }
    let actors = (0..n)
        .map(|_| r.compact_index().map(export_index))
        .collect::<Result<Vec<_>>>()?;
    // The map's URL: protocol, host, map, portal, options list, port, valid.
    let url_protocol = r.fstring()?;
    let _host = r.fstring()?;
    let url_map = r.fstring()?;
    let _portal = r.fstring()?;
    let options = r.compact_index()?;
    for _ in 0..options.max(0) {
        r.fstring()?;
    }
    let _port = r.i32()?;
    let _valid = r.i32()?;
    let model = export_index(r.compact_index()?);
    Ok(LevelInfo {
        actors,
        url_protocol,
        url_map,
        model,
    })
}

/// Read an actor's property list (skipping its state frame). `index` is the
/// 0-based export index.
pub fn read_actor_properties(pkg: &Package, data: &[u8], index: usize) -> Result<Vec<Property>> {
    let e = &pkg.exports[index];
    let mut r = Reader::new(object_bytes(data, e)?);
    skip_state_frame(&mut r, e)?;
    read_properties(pkg, &mut r)
}

/// Where an actor is and how it's drawn, in Unreal units and angles.
#[derive(Debug, Clone)]
pub struct Placement {
    pub export: usize,
    pub class: String,
    pub name: String,
    pub location: Vec3,
    /// Pitch, yaw, roll in Unreal angle units (65536 = one full turn).
    pub rotation: [i32; 3],
    pub draw_scale: f32,
    pub draw_scale_3d: Vec3,
    pub pre_pivot: Vec3,
    /// Object reference to the static mesh, 0 if none.
    pub static_mesh: i32,
    /// Material overrides by section (`Skins` property), 0 where not set.
    pub skins: Vec<i32>,
    pub hidden: bool,
    /// Object reference to the actor's `StaticMeshInstance` (its baked
    /// per-vertex light), 0 if none.
    pub static_mesh_instance: i32,
}

fn raw_vec3(p: &Property) -> Option<Vec3> {
    match &p.value {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some([f(0), f(1), f(2)])
        }
        _ => None,
    }
}

fn raw_ints3(p: &Property) -> Option<[i32; 3]> {
    match &p.value {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some([f(0), f(1), f(2)])
        }
        _ => None,
    }
}

/// Read one actor's placement. Properties left out of the file keep the
/// engine's defaults (scale 1, no rotation), which is how Unreal stores them.
pub fn read_placement(pkg: &Package, data: &[u8], index: usize) -> Result<Placement> {
    let e = &pkg.exports[index];
    let class = pkg.export_class_name(e).to_string();
    let props = read_actor_properties(pkg, data, index)?;
    let find = |n: &str| properties::find(pkg, &props, n);
    let mut p = Placement {
        export: index,
        class,
        name: pkg.name(e.object_name).to_string(),
        location: [0.0; 3],
        rotation: [0; 3],
        draw_scale: 1.0,
        draw_scale_3d: [1.0; 3],
        pre_pivot: [0.0; 3],
        static_mesh: 0,
        skins: Vec::new(),
        hidden: false,
        static_mesh_instance: 0,
    };
    if let Some(v) = find("Location").and_then(raw_vec3) {
        p.location = v;
    }
    if let Some(v) = find("Rotation").and_then(raw_ints3) {
        p.rotation = v;
    }
    if let Some(Value::Float(s)) = find("DrawScale").map(|p| &p.value) {
        p.draw_scale = *s;
    }
    if let Some(v) = find("DrawScale3D").and_then(raw_vec3) {
        p.draw_scale_3d = v;
    }
    if let Some(v) = find("PrePivot").and_then(raw_vec3) {
        p.pre_pivot = v;
    }
    if let Some(Value::Object(o)) = find("StaticMesh").map(|p| &p.value) {
        p.static_mesh = *o;
    }
    if let Some(Value::Bool(b)) = find("bHidden").map(|p| &p.value) {
        p.hidden = *b;
    }
    if let Some(Value::Object(o)) = find("StaticMeshInstance").map(|p| &p.value) {
        p.static_mesh_instance = *o;
    }
    for prop in props
        .iter()
        .filter(|q| pkg.name(q.name).eq_ignore_ascii_case("Skins"))
    {
        if let Value::Object(o) = prop.value {
            let i = prop.array_index as usize;
            if p.skins.len() <= i {
                p.skins.resize(i + 1, 0);
            }
            p.skins[i] = o;
        }
    }
    Ok(p)
}

/// The baked light of one placed static mesh: a `StaticMeshInstance`
/// object, which is an (empty) property list, then a count and one colour
/// per mesh vertex, stored blue, green, red, alpha. Returned as red, green,
/// blue, alpha. A few bytes after the colours are not understood yet and
/// are ignored. Layout measured on the game's maps (no reference describes
/// it); `index` is the object reference from `Placement`.
pub fn read_vertex_colours(pkg: &Package, data: &[u8], index: i32) -> Result<Vec<[u8; 4]>> {
    let Some(e) = usize::try_from(index - 1)
        .ok()
        .and_then(|i| pkg.exports.get(i))
    else {
        return Reader::new(&[]).invalid(format!(
            "vertex colours: {index} isn't an object in this map"
        ));
    };
    let mut r = Reader::new(object_bytes(data, e)?);
    read_properties(pkg, &mut r)?;
    let n = r.compact_index()?;
    if n < 0 || n as usize * 4 > r.len() - r.pos() {
        return r.invalid(format!("vertex colour count {n} doesn't fit"));
    }
    Ok(r.bytes(n as usize * 4)?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| [c[2], c[1], c[0], c[3]])
        .collect())
}

/// Unreal's rotation as three axis directions (forward X, right Y, up Z) in
/// world space, from pitch, yaw and roll. Standard UE2 convention: yaw turns
/// around Z, pitch around Y, roll around X; 65536 units = 360 degrees.
pub fn rotation_axes(rot: [i32; 3]) -> [Vec3; 3] {
    let to_rad = std::f32::consts::TAU / 65536.0;
    let (sp, cp) = (rot[0] as f32 * to_rad).sin_cos();
    let (sy, cy) = (rot[1] as f32 * to_rad).sin_cos();
    let (sr, cr) = (rot[2] as f32 * to_rad).sin_cos();
    [
        [cp * cy, cp * sy, sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp],
        [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp],
    ]
}

impl Placement {
    /// Move a point from the mesh's own coordinates into the level:
    /// subtract the pivot, scale, rotate, then move to the location.
    pub fn transform_point(&self, v: Vec3) -> Vec3 {
        let axes = rotation_axes(self.rotation);
        let s = self.draw_scale;
        let local = [
            (v[0] - self.pre_pivot[0]) * self.draw_scale_3d[0] * s,
            (v[1] - self.pre_pivot[1]) * self.draw_scale_3d[1] * s,
            (v[2] - self.pre_pivot[2]) * self.draw_scale_3d[2] * s,
        ];
        let mut out = self.location;
        for (k, axis) in axes.iter().enumerate() {
            for c in 0..3 {
                out[c] += local[k] * axis[c];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn quarter_turn_of_yaw_points_forward_along_y() {
        // 16384 = 90 degrees. Yaw turns forward (X) towards right (Y).
        let axes = rotation_axes([0, 16384, 0]);
        assert!(close(axes[0], [0.0, 1.0, 0.0]));
        assert!(close(axes[2], [0.0, 0.0, 1.0]));
    }

    #[test]
    fn quarter_turn_of_pitch_points_forward_up() {
        let axes = rotation_axes([16384, 0, 0]);
        assert!(close(axes[0], [0.0, 0.0, 1.0]));
    }

    #[test]
    fn transform_scales_turns_then_moves() {
        let p = Placement {
            export: 0,
            class: "StaticMeshActor".into(),
            name: "test".into(),
            location: [100.0, 0.0, 0.0],
            rotation: [0, 16384, 0],
            draw_scale: 2.0,
            draw_scale_3d: [1.0, 1.0, 1.0],
            pre_pivot: [0.0; 3],
            static_mesh: 0,
            skins: vec![],
            hidden: false,
            static_mesh_instance: 0,
        };
        // One unit forward, doubled, turned to +Y, then moved.
        assert!(close(p.transform_point([1.0, 0.0, 0.0]), [100.0, 2.0, 0.0]));
    }
}
