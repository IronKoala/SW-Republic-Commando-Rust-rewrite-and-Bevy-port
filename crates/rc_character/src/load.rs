//! Everything `rc_character` reads from the game before Bevy starts: the
//! skeletal mesh, one animation sequence, and the mesh's skin texture.
//! Plain Rust, no Bevy.

use crate::RunLog;
use rc_anim::{edges, globals, skin, stretch, weights_per_point, Clip, Convention, Skeleton};
use rc_image::{PixelFormat, Rgba};
use rc_package::animation::{read_mesh_animation, MeshAnimation};
use rc_package::properties::Value;
use rc_package::skeletal::{bounds, read_skeletal_mesh, SkeletalMesh};
use rc_package::texture::{self, format_name};
use rc_package::{ObjectRef, Package};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Everything the Bevy side needs.
pub struct Character {
    pub mesh_name: String,
    pub mesh: SkeletalMesh,
    pub skeleton: Skeleton,
    pub weights: Vec<Vec<(usize, f32)>>,
    pub clip: Clip,
    pub set_name: String,
    /// The skin picture, if it could be found and decoded.
    pub skin: Option<(String, Rgba)>,
    /// Whether triangles must be flipped to face outwards in Bevy's axes.
    pub flip_winding: bool,
}

fn read_package(path: &Path) -> Result<(Vec<u8>, Package), String> {
    let data = fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((data, pkg))
}

pub struct Wanted {
    pub package: PathBuf,
    pub mesh: Option<String>,
    pub set: Option<String>,
    pub sequence: Option<String>,
    pub list: bool,
}

pub fn load(log: &RunLog, game: &Path, wanted: &Wanted) -> Result<Character, String> {
    let t = Instant::now();
    let (data, pkg) = read_package(&wanted.package)?;
    log.line(format!(
        "package read: {} bytes, {} exports, {:.1} ms",
        data.len(),
        pkg.exports.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));

    // The mesh: the one asked for, or the first in the package.
    let mesh_export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "SkeletalMesh"
                && wanted
                    .mesh
                    .as_ref()
                    .is_none_or(|m| pkg.name(e.object_name).eq_ignore_ascii_case(m))
        })
        .ok_or_else(|| match &wanted.mesh {
            Some(m) => format!("no SkeletalMesh called {m} in this package"),
            None => "no SkeletalMesh in this package".into(),
        })?;
    let mesh_name = pkg.name(mesh_export.object_name).to_string();
    let mesh = read_skeletal_mesh(&pkg, &data, mesh_export).map_err(|e| e.to_string())?;
    let (lo, hi) = mesh.points_bounds();
    log.line(format!(
        "mesh {mesh_name}: {} bones, {} points, {} wedges, {} triangles, {} influences, {} material slots",
        mesh.bones.len(),
        mesh.points.len(),
        mesh.wedges.len(),
        mesh.triangles.len(),
        mesh.influences.len(),
        mesh.materials.len()
    ));
    log.line(format!(
        "mesh {mesh_name}: reference pose from ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}) game units, scale {:?}, origin {:?}, rotation {:?}",
        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], mesh.mesh_scale, mesh.mesh_origin, mesh.rot_origin
    ));
    let problems = mesh.check();
    for p in &problems {
        log.line(format!("mesh check: {p}"));
    }
    log.line(format!("mesh check: {} problems", problems.len()));

    // The animation set: the one asked for, or the first one the mesh links to.
    let set_export = match &wanted.set {
        Some(s) => pkg.exports.iter().find(|e| {
            pkg.export_class_name(e) == "MeshAnimation"
                && pkg.name(e.object_name).eq_ignore_ascii_case(s)
        }),
        None => mesh
            .animations
            .iter()
            .find_map(|&a| match ObjectRef::from_index(a) {
                ObjectRef::Export(i) => pkg.exports.get(i),
                _ => None,
            }),
    }
    .ok_or("animation set not found in this package")?;
    let set_name = pkg.name(set_export.object_name).to_string();
    let anim: MeshAnimation =
        read_mesh_animation(&pkg, &data, set_export).map_err(|e| e.to_string())?;
    log.line(format!(
        "animation set {set_name}: {} bones, {} sequences",
        anim.bones.len(),
        anim.sequences.len()
    ));
    if wanted.list {
        for s in &anim.sequences {
            log.line(format!(
                "  sequence {}: {} frames at {} per second",
                s.name, s.num_frames, s.rate
            ));
        }
    }
    let seq = match &wanted.sequence {
        Some(n) => anim
            .sequences
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(n))
            .ok_or_else(|| {
                format!("no sequence called {n} in {set_name} (add --list to see them)")
            })?,
        None => anim.sequences.first().ok_or("the animation set is empty")?,
    };

    let skeleton = Skeleton::from_mesh(&mesh, Convention::InvertRoot);
    let clip = Clip::new(&anim, seq, &skeleton);
    log.line(format!(
        "sequence {}: {} frames at {} per second ({:.2} s), {} of {} mesh bones animated",
        clip.name,
        clip.num_frames,
        clip.rate,
        clip.duration(),
        clip.matched_bones,
        skeleton.names.len()
    ));
    let weights = weights_per_point(&mesh);
    measure_clip(log, &mesh, &skeleton, &weights, &clip);

    let skin = match load_skin(log, game, &pkg, &mesh) {
        Ok(s) => Some(s),
        Err(e) => {
            log.line(format!("skin: not loaded, drawing plain grey instead: {e}"));
            None
        }
    };

    // Which way round the triangles go. Bevy draws a triangle's front when
    // its corners go anticlockwise. Adding up the "signed volume" of every
    // triangle with the mesh's own winding gives a positive number when the
    // fronts face outwards. Our axis change mirrors the model, so measure in
    // Bevy's axes.
    let pts: Vec<[f32; 3]> = mesh
        .points
        .iter()
        .map(|&p| crate::to_bevy(&mesh, p))
        .collect();
    let mut volume = 0f64;
    for tri in &mesh.triangles {
        let [a, b, c] = tri
            .wedges
            .map(|w| pts[mesh.wedges[w as usize].point as usize]);
        let cr = rc_anim::math::cross(b, c);
        volume += (a[0] * cr[0] + a[1] * cr[1] + a[2] * cr[2]) as f64 / 6.0;
    }
    let flip_winding = volume < 0.0;
    log.line(format!(
        "winding: signed volume {volume:.0} cubic units with the stored corner order, so triangles are {}",
        if flip_winding { "flipped" } else { "kept as stored" }
    ));

    Ok(Character {
        mesh_name,
        mesh,
        skeleton,
        weights,
        clip,
        set_name,
        skin,
        flip_winding,
    })
}

/// Plays the whole clip on the CPU once and logs what happens to the mesh:
/// its size on every frame and how much its surface stretches.
fn measure_clip(
    log: &RunLog,
    mesh: &SkeletalMesh,
    skel: &Skeleton,
    weights: &[Vec<(usize, f32)>],
    clip: &Clip,
) {
    let edge_list = edges(mesh);
    let frames = clip.num_frames.ceil() as usize;
    let (mut min_h, mut max_h) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut worst_stretch = 0f32;
    let mut root_start = [0f32; 3];
    let mut root_end = [0f32; 3];
    for f in 0..frames {
        let local = clip.sample(skel, f as f32);
        let pose = globals(&skel.parents, &local);
        let posed = skin(&mesh.points, weights, skel, &pose);
        let (lo, hi) = bounds(&posed);
        min_h = min_h.min(hi[2] - lo[2]);
        max_h = max_h.max(hi[2] - lo[2]);
        worst_stretch = worst_stretch.max(stretch(&mesh.points, &posed, &edge_list).mean);
        if f == 0 {
            root_start = pose[0].pos;
        }
        root_end = pose[0].pos;
    }
    let travel = rc_anim::math::length(rc_anim::math::sub(root_end, root_start));
    log.line(format!(
        "whole sequence on the CPU: {frames} frames, height between {min_h:.1} and {max_h:.1} game units, worst average edge stretch {worst_stretch:.4}, root bone travels {travel:.1} units"
    ));
}

/// Finds the mesh's first material, follows it to a picture and decodes it.
/// The material is usually in another package, e.g. the clone trooper uses
/// `CloneTextures.CloneTrooperTextures.CloneTrooper_Shader`, a "Shader"
/// whose `Diffuse` setting is the picture.
fn load_skin(
    log: &RunLog,
    game: &Path,
    pkg: &Package,
    mesh: &SkeletalMesh,
) -> Result<(String, Rgba), String> {
    let first = *mesh.textures.first().ok_or("the mesh lists no materials")?;
    let ObjectRef::Import(i) = ObjectRef::from_index(first) else {
        return Err("material stored inside the mesh package is not supported yet".into());
    };
    let material = pkg.name(pkg.imports[i].object_name).to_string();
    // The outermost package of the import is the file name.
    let mut outer = pkg.imports[i].package_index;
    let mut top = String::new();
    while let ObjectRef::Import(j) = ObjectRef::from_index(outer) {
        top = pkg.name(pkg.imports[j].object_name).to_string();
        outer = pkg.imports[j].package_index;
    }
    let path = game
        .join("GameData")
        .join("Textures")
        .join(format!("{}.utx", top.to_lowercase()));
    log.line(format!(
        "skin: material {material} from package {top} ({})",
        path.display()
    ));
    let (data, tpkg) = read_package(&path)?;
    // Find the material by name and class: names alone can clash (the
    // battle droid's texture group and its texture are both "BattleDroid").
    let class_wanted = pkg.name(pkg.imports[i].class_name).to_string();
    let mut index = tpkg
        .exports
        .iter()
        .position(|e| {
            tpkg.name(e.object_name).eq_ignore_ascii_case(&material)
                && tpkg
                    .export_class_name(e)
                    .eq_ignore_ascii_case(&class_wanted)
        })
        .ok_or_else(|| format!("no {class_wanted} called {material} in {top}"))?;
    for _ in 0..6 {
        let export = &tpkg.exports[index];
        let name = tpkg.name(export.object_name).to_string();
        let class = tpkg.export_class_name(export).to_string();
        if class == "Texture" {
            let image = decode_texture(&tpkg, &data, export)?;
            let [r, g, b, a] = image.mean();
            log.line(format!(
                "skin: texture {name} decoded, {}x{}, average colour R {r:.0} G {g:.0} B {b:.0} A {a:.0}",
                image.width, image.height
            ));
            return Ok((name, image));
        }
        // Materials that wrap a picture: follow the setting that holds it.
        let setting = match class.as_str() {
            "Shader" => "Diffuse",
            "Combiner" => "Material1",
            "FinalBlend" | "TexModifier" | "TexPanner" | "TexScaler" | "TexRotator"
            | "TexOscillator" => "Material",
            other => return Err(format!("{name} is a {other}, which we can't follow yet")),
        };
        let start = export.serial_offset as usize;
        let mut r = rc_package::Reader::new(&data[start..start + export.serial_size as usize]);
        let props =
            rc_package::properties::read_properties(&tpkg, &mut r).map_err(|e| e.to_string())?;
        let next = match rc_package::properties::find(&tpkg, &props, setting).map(|p| &p.value) {
            Some(Value::Object(o)) if *o != 0 => *o,
            _ => return Err(format!("{class} {name} has no {setting}")),
        };
        log.line(format!(
            "skin: {class} {name} -> {setting} = {}",
            tpkg.object_name(next)
        ));
        index = match ObjectRef::from_index(next) {
            ObjectRef::Export(e) if e < tpkg.exports.len() => e,
            _ => {
                return Err(format!(
                    "{setting} of {name} is in another package, not supported yet"
                ))
            }
        };
    }
    Err("materials nested too deeply".into())
}

fn decode_texture(pkg: &Package, data: &[u8], export: &rc_package::Export) -> Result<Rgba, String> {
    let tex = texture::read_texture(pkg, data, export).map_err(|e| e.to_string())?;
    let format = match tex.format {
        0 => PixelFormat::P8,
        3 => PixelFormat::Dxt1,
        5 => PixelFormat::Bgra8,
        7 => PixelFormat::Dxt3,
        8 => PixelFormat::Dxt5,
        10 => PixelFormat::G16,
        f => {
            return Err(format!(
                "texture format {} can't be decoded yet",
                format_name(f)
            ))
        }
    };
    let palette = match tex.palette_ref(pkg).map(ObjectRef::from_index) {
        Some(ObjectRef::Export(i)) => {
            let mut colors =
                texture::read_palette(pkg, data, &pkg.exports[i]).map_err(|e| e.to_string())?;
            if tex.masked(pkg) {
                colors[0][3] = 0;
            }
            Some(colors)
        }
        Some(_) => return Err("palette is in another package (not supported yet)".into()),
        None => None,
    };
    let mip = tex
        .mip_for(texture::Detail::Characters)
        .ok_or("texture has no mipmaps")?;
    rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        &data[mip.data_offset..mip.data_offset + mip.data_len],
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())
}
