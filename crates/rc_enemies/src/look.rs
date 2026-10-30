//! What a droid looks like: its animated mesh from the game's package, its
//! animations, and its skin picture. Read once, shared by every droid.
//!
//! Posing works as in `rc_character`: each frame `rc_anim` poses the
//! skeleton and moves the mesh's points on the CPU ("skinning"), and the
//! new positions go into that droid's Bevy mesh. The skin loading below is
//! copied from `rc_character/src/load.rs` (that crate is a program, not a
//! library), using `GamePaths` so file names don't depend on upper or
//! lower case.

use rc_ai::params::Look;
use rc_anim::math::Transform;
use rc_anim::{globals, skin, weights_per_point, Clip, Convention, Skeleton};
use rc_game_core::GamePaths;
use rc_image::{PixelFormat, Rgba};
use rc_package::animation::{read_mesh_animation, MeshAnimation};
use rc_package::properties::Value;
use rc_package::skeletal::{read_skeletal_mesh, SkeletalMesh};
use rc_package::{texture, ObjectRef, Package};
use std::collections::HashMap;

/// What a droid is doing, as far as its animation is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pose {
    Idle,
    Walk,
    Run,
    WalkBack,
    StepLeft,
    StepRight,
    Fire,
    Crouch,
    CrouchFire,
    Death,
    /// Throwing a grenade.
    Throw,
    /// A close-up hit (a force pike).
    Melee,
}

impl Pose {
    pub const ALL: [Pose; 12] = [
        Pose::Idle,
        Pose::Walk,
        Pose::Run,
        Pose::WalkBack,
        Pose::StepLeft,
        Pose::StepRight,
        Pose::Fire,
        Pose::Crouch,
        Pose::CrouchFire,
        Pose::Death,
        Pose::Throw,
        Pose::Melee,
    ];

    /// The animation `l` names for this pose (empty: none).
    pub fn sequence(self, l: &Look) -> &str {
        match self {
            Pose::Idle => &l.idle,
            Pose::Walk => &l.walk,
            Pose::Run => &l.run,
            Pose::WalkBack => &l.walk_back,
            Pose::StepLeft => &l.step_left,
            Pose::StepRight => &l.step_right,
            Pose::Fire => &l.fire,
            Pose::Crouch => &l.crouch,
            Pose::CrouchFire => &l.crouch_fire,
            Pose::Death => &l.death,
            Pose::Throw => &l.throw,
            Pose::Melee => &l.melee,
        }
    }

    /// Played once and held on the last frame, instead of looping.
    pub fn once(self) -> bool {
        self == Pose::Death
    }

    /// Seconds to blend into this animation from the one before.
    pub fn blend_time(self, l: &Look) -> f32 {
        match self {
            Pose::Fire | Pose::CrouchFire => l.blend_time_fire,
            Pose::Death => l.blend_time_death,
            _ => l.blend_time,
        }
    }
}

pub struct DroidLook {
    pub mesh: SkeletalMesh,
    pub skeleton: Skeleton,
    pub weights: Vec<Vec<(usize, f32)>>,
    pub clips: HashMap<Pose, Clip>,
    /// The mesh's whole animation sets, for animations a level script
    /// names (`ACTION_PlayAnim`).
    pub anims: Vec<MeshAnimation>,
    pub skin: Option<(String, Rgba)>,
    /// Triangle corners as wedge numbers, already in Bevy's winding.
    pub indices: Vec<u32>,
    /// Added to every point so the feet are at the entity's origin, uu.
    pub lift: f32,
}

/// A mesh point (the mesh's own units and axes) to Unreal units relative
/// to the character's feet, before the character's own turn: the mesh's
/// scale, origin and turn are applied, as in `rc_character`.
fn mesh_point(mesh: &SkeletalMesh, p: [f32; 3]) -> [f32; 3] {
    let s = mesh.mesh_scale;
    let o = mesh.mesh_origin;
    let q = [
        (p[0] - o[0]) * s[0],
        (p[1] - o[1]) * s[1],
        (p[2] - o[2]) * s[2],
    ];
    let yaw = mesh.rot_origin[1] as f32 * std::f32::consts::TAU / 65536.0;
    let (sin, cos) = yaw.sin_cos();
    [q[0] * cos - q[1] * sin, q[0] * sin + q[1] * cos, q[2]]
}

/// Unreal (Z up) to Bevy (Y up), the same rule as `rc_game_core::units`.
fn to_bevy(p: [f32; 3]) -> [f32; 3] {
    [p[0] / 50.0, p[2] / 50.0, p[1] / 50.0]
}

impl DroidLook {
    /// Read the mesh and animations named in `look`. Returns the look and
    /// lines for the log.
    pub fn load(paths: &GamePaths, look: &Look) -> Result<(DroidLook, Vec<String>), String> {
        let mut notes = Vec::new();
        let file = paths.game_file(&look.package);
        let data =
            std::fs::read(&file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
        let pkg = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        let export = pkg
            .exports
            .iter()
            .find(|e| {
                pkg.export_class_name(e) == "SkeletalMesh"
                    && pkg.name(e.object_name).eq_ignore_ascii_case(&look.mesh)
            })
            .ok_or_else(|| format!("no SkeletalMesh called {} in {}", look.mesh, file.display()))?;
        let mesh = read_skeletal_mesh(&pkg, &data, export).map_err(|e| e.to_string())?;
        // Every animation set the mesh links to in its package, in the
        // mesh's order; each animation comes from the first set that has
        // it. A Trandoshan mercenary's mesh lists its cutscene sets first
        // and borrows the commandos' everyday moves (CloneCommandoSet).
        let sets: Vec<&rc_package::Export> = mesh
            .animations
            .iter()
            .filter_map(|&a| match ObjectRef::from_index(a) {
                ObjectRef::Export(i) => pkg.exports.get(i),
                _ => None,
            })
            .collect();
        if sets.is_empty() {
            return Err("the mesh has no animation set in its package".into());
        }
        let mut anims = Vec::with_capacity(sets.len());
        for set in &sets {
            match read_mesh_animation(&pkg, &data, set) {
                Ok(a) => anims.push((pkg.name(set.object_name).to_string(), a)),
                Err(e) => notes.push(format!(
                    "animation set {} not read: {e}",
                    pkg.name(set.object_name)
                )),
            }
        }
        let skeleton = Skeleton::from_mesh(&mesh, Convention::InvertRoot);
        let mut clips = HashMap::new();
        let mut used_sets: Vec<&str> = Vec::new();
        for pose in Pose::ALL {
            let name = pose.sequence(look);
            if name.is_empty() {
                continue; // this character has none: it plays the idle one
            }
            let found = anims.iter().find_map(|(set_name, anim)| {
                anim.sequences
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
                    .map(|seq| (set_name, anim, seq))
            });
            match found {
                Some((set_name, anim, seq)) => {
                    clips.insert(pose, Clip::new(anim, seq, &skeleton));
                    if !used_sets.contains(&set_name.as_str()) {
                        used_sets.push(set_name);
                    }
                }
                None => notes.push(format!(
                    "no animation called {name} in the mesh's {} animation sets; {pose:?} uses the idle one",
                    anims.len()
                )),
            }
        }
        if !clips.contains_key(&Pose::Idle) {
            return Err(format!("no idle animation {} in the set", look.idle));
        }
        let weights = weights_per_point(&mesh);

        // Which way round the triangles go once in Bevy's axes (see
        // rc_character/src/load.rs): flip them if the stored order gives a
        // negative volume.
        let pts: Vec<[f32; 3]> = mesh
            .points
            .iter()
            .map(|&p| to_bevy(mesh_point(&mesh, p)))
            .collect();
        let mut volume = 0f64;
        for tri in &mesh.triangles {
            let [a, b, c] = tri
                .wedges
                .map(|w| pts[mesh.wedges[w as usize].point as usize]);
            let cr = rc_anim::math::cross(b, c);
            volume += (a[0] * cr[0] + a[1] * cr[1] + a[2] * cr[2]) as f64 / 6.0;
        }
        let flip = volume < 0.0;
        let mut indices = Vec::with_capacity(mesh.triangles.len() * 3);
        for t in &mesh.triangles {
            let [a, b, c] = t.wedges.map(|w| w as u32);
            indices.extend(if flip { [a, c, b] } else { [a, b, c] });
        }

        let skin = match load_skin(paths, &pkg, &mesh, &look.skin) {
            Ok(s) => Some(s),
            Err(e) => {
                notes.push(format!("skin not loaded, droids are plain grey: {e}"));
                None
            }
        };
        let mut look = DroidLook {
            mesh,
            skeleton,
            weights,
            clips,
            anims: anims.iter().map(|(_, a)| a.clone()).collect(),
            skin,
            indices,
            lift: 0.0,
        };
        // Stand the idle pose's lowest point on the floor.
        let idle = look.pose_unreal(Pose::Idle, 0.0);
        let (lo, hi) = idle
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[2]), hi.max(p[2]))
            });
        look.lift = -lo;
        notes.push(format!(
            "mesh {}: {} bones, {} points, {} triangles ({}), {} of {} animations found (from {}), idle pose {:.0} uu tall (lowest point {lo:.0} uu, lifted to the floor), skin {}",
            pkg.name(export.object_name),
            look.mesh.bones.len(),
            look.mesh.points.len(),
            look.mesh.triangles.len(),
            if flip { "flipped" } else { "kept as stored" },
            look.clips.len(),
            Pose::ALL.len(),
            used_sets.join(", "),
            hi - lo,
            look.skin.as_ref().map_or("none".to_string(), |(n, i)| {
                let [r, g, b, _] = i.mean();
                format!("{n} {}x{}, average colour R {r:.0} G {g:.0} B {b:.0}", i.width, i.height)
            })
        ));
        Ok((look, notes))
    }

    pub fn clip(&self, pose: Pose) -> &Clip {
        self.clips.get(&pose).unwrap_or(&self.clips[&Pose::Idle])
    }

    /// An animation from the set by name (any case), for a level script.
    pub fn named_clip(&self, name: &str) -> Option<Clip> {
        self.anims.iter().find_map(|anim| {
            anim.sequences
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .map(|seq| Clip::new(anim, seq, &self.skeleton))
        })
    }

    /// Each bone's local transform at `frame` of `clip`. The root bone's
    /// sideways travel is removed: the droid's body moves the droid, not
    /// the animation.
    pub fn local_clip(&self, clip: &Clip, frame: f32) -> Vec<Transform> {
        let mut local = clip.sample(&self.skeleton, frame);
        if let (Some(l), Some(rest)) = (local.first_mut(), self.skeleton.local.first()) {
            l.pos[0] = rest.pos[0];
            l.pos[1] = rest.pos[1];
        }
        local
    }

    /// Each bone's local transform at `frame` of `pose`.
    pub fn local_pose(&self, pose: Pose, frame: f32) -> Vec<Transform> {
        self.local_clip(self.clip(pose), frame)
    }

    /// Points for bones placed at `local`, Unreal units relative to the
    /// mesh origin (feet not yet lifted).
    fn unreal_points(&self, local: &[Transform]) -> Vec<[f32; 3]> {
        let pose = globals(&self.skeleton.parents, local);
        skin(&self.mesh.points, &self.weights, &self.skeleton, &pose)
            .into_iter()
            .map(|p| mesh_point(&self.mesh, p))
            .collect()
    }

    fn pose_unreal(&self, pose: Pose, frame: f32) -> Vec<[f32; 3]> {
        self.unreal_points(&self.local_pose(pose, frame))
    }

    /// Points for bones placed at `local` (from `local_pose`, perhaps
    /// blended), in Bevy units relative to the droid's feet, before its
    /// facing is applied.
    pub fn posed_local(&self, local: &[Transform]) -> Vec<[f32; 3]> {
        self.unreal_points(local)
            .into_iter()
            .map(|p| to_bevy([p[0], p[1], p[2] + self.lift]))
            .collect()
    }

    /// Points posed at `frame` of `pose`, in Bevy units relative to the
    /// droid's feet, before its facing is applied.
    pub fn posed(&self, pose: Pose, frame: f32) -> Vec<[f32; 3]> {
        self.posed_local(&self.local_pose(pose, frame))
    }

    /// Like [`Self::posed`], for a named animation (see
    /// [`Self::named_clip`]).
    pub fn posed_clip(&self, clip: &Clip, frame: f32) -> Vec<[f32; 3]> {
        self.posed_local(&self.local_clip(clip, frame))
    }

    /// Per-wedge positions and smooth normals for the Bevy mesh.
    pub fn wedge_attributes(&self, points: &[[f32; 3]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        use rc_anim::math::{cross, length, sub};
        let point_of = |w: u32| self.mesh.wedges[w as usize].point as usize;
        let mut n = vec![[0f32; 3]; points.len()];
        for tri in self.indices.chunks_exact(3) {
            let [a, b, d] = [point_of(tri[0]), point_of(tri[1]), point_of(tri[2])];
            let f = cross(sub(points[b], points[a]), sub(points[d], points[a]));
            for p in [a, b, d] {
                for k in 0..3 {
                    n[p][k] += f[k];
                }
            }
        }
        for v in &mut n {
            let l = length(*v);
            *v = if l > 0.0 {
                v.map(|x| x / l)
            } else {
                [0.0, 1.0, 0.0]
            };
        }
        let pos = self
            .mesh
            .wedges
            .iter()
            .map(|w| points[w.point as usize])
            .collect();
        let normals = self
            .mesh
            .wedges
            .iter()
            .map(|w| n[w.point as usize])
            .collect();
        (pos, normals)
    }

    pub fn uvs(&self) -> Vec<[f32; 2]> {
        self.mesh.wedges.iter().map(|w| [w.u, w.v]).collect()
    }
}

/// Follows the mesh's first material, or `over` when it isn't empty
/// (`Package.Group.Object`), to a picture and decodes it. Copied from
/// `rc_character/src/load.rs`.
fn load_skin(
    paths: &GamePaths,
    pkg: &Package,
    mesh: &SkeletalMesh,
    over: &str,
) -> Result<(String, Rgba), String> {
    let (top, material, class_wanted) = if over.is_empty() {
        let first = *mesh.textures.first().ok_or("the mesh lists no materials")?;
        let ObjectRef::Import(i) = ObjectRef::from_index(first) else {
            return Err("material stored inside the mesh package is not supported yet".into());
        };
        let material = pkg.name(pkg.imports[i].object_name).to_string();
        let mut outer = pkg.imports[i].package_index;
        let mut top = String::new();
        while let ObjectRef::Import(j) = ObjectRef::from_index(outer) {
            top = pkg.name(pkg.imports[j].object_name).to_string();
            outer = pkg.imports[j].package_index;
        }
        let class = pkg.name(pkg.imports[i].class_name).to_string();
        (top, material, Some(class))
    } else {
        let top = over.split('.').next().unwrap_or(over).to_string();
        let material = over.rsplit('.').next().unwrap_or(over).to_string();
        (top, material, None)
    };
    let path = paths.game_file(&format!("GameData/Textures/{top}.utx"));
    let data = std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let tpkg = Package::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    // By name and class: a group and a texture can share a name. A skin
    // named in the settings file has no class, so any material matches.
    let mut index = tpkg
        .exports
        .iter()
        .position(|e| {
            tpkg.name(e.object_name).eq_ignore_ascii_case(&material)
                && class_wanted
                    .as_ref()
                    .map_or(tpkg.export_class_name(e) != "Package", |c| {
                        tpkg.export_class_name(e).eq_ignore_ascii_case(c)
                    })
        })
        .ok_or_else(|| {
            format!(
                "no {} called {material} in {top}",
                class_wanted.as_deref().unwrap_or("material")
            )
        })?;
    for _ in 0..6 {
        let export = &tpkg.exports[index];
        let name = tpkg.name(export.object_name).to_string();
        let class = tpkg.export_class_name(export).to_string();
        if class == "Texture" {
            return Ok((name, decode_texture(&tpkg, &data, export)?));
        }
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
                texture::format_name(f)
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
    let mip = tex.mips.first().ok_or("texture has no mipmaps")?;
    rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        &data[mip.data_offset..mip.data_offset + mip.data_len],
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())
}
