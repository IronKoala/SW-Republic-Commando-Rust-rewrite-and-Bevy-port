//! An animated character model read from the game's packages: the
//! skeletal mesh, the animations asked for, and a picture for every
//! material the mesh uses. Plain Rust, no Bevy.
//!
//! The droids (`rc_enemies/src/look.rs`) and the clone trooper viewer
//! (`rc_character`) each read one material only. Delta Squad's armour is
//! split over several materials (body, helmet, visor and so on), so this
//! reader keeps the mesh's triangles in one "section" per material, each
//! with its own picture. A program draws one mesh per section.
//!
//! Posing works as in `rc_character`: `rc_anim` poses the skeleton and
//! moves the mesh's points on the CPU ("skinning"), and the caller copies
//! the new positions into its meshes.

use rc_anim::math::Transform;
use rc_anim::{globals, skin, weights_per_point, Clip, Convention, Skeleton};
use rc_image::{PixelFormat, Rgba};
use rc_package::animation::read_mesh_animation;
use rc_package::properties::{self, Value};
use rc_package::skeletal::{read_skeletal_mesh, SkeletalMesh};
use rc_package::{texture, Export, ObjectRef, Package};
use std::collections::HashMap;
use std::path::PathBuf;

/// What to read. Names are as stored in the game's packages.
#[derive(Debug, Clone, Default)]
pub struct ModelSpec {
    /// The animation package, relative to the game folder
    /// (`GameData/Animations/clone.ukx`).
    pub package: String,
    pub mesh: String,
    /// The animation set; `None` uses the one the mesh links to.
    pub set: Option<String>,
    /// Our name for an animation, and the sequence it plays.
    pub sequences: Vec<(String, String)>,
    /// Replacement materials, by material slot (`None` keeps the mesh's
    /// own). Written `Package.Object` or `Package.Group.Object`, as
    /// UnrealEd shows them. The original swaps skins this way per class.
    pub skins: Vec<Option<String>>,
}

/// A material's picture.
#[derive(Debug, Clone)]
pub struct Skin {
    pub texture: String,
    pub image: Rgba,
    /// See-through where the picture's alpha is low (a masked texture, an
    /// alpha-tested blend, or a shader with an opacity picture), rather
    /// than alpha holding something else such as shininess.
    pub cutout: bool,
    /// How the material led to the picture, for the log.
    pub chain: Vec<String>,
}

/// The triangles that use one material.
#[derive(Debug, Clone)]
pub struct Section {
    /// Index into the mesh's material list (`SkeletalMesh::textures`).
    pub slot: usize,
    /// The material's full name, or `slot N` when it has none.
    pub material: String,
    pub skin: Option<Skin>,
    /// The mesh's wedges this section uses; vertex `i` of the section is
    /// wedge `wedges[i]`.
    pub wedges: Vec<u32>,
    /// Triangle corners as section vertex numbers, in Bevy's winding.
    pub indices: Vec<u32>,
}

pub struct Model {
    pub name: String,
    pub mesh: SkeletalMesh,
    pub skeleton: Skeleton,
    pub weights: Vec<Vec<(usize, f32)>>,
    /// By our name (`ModelSpec::sequences`).
    pub clips: HashMap<String, Clip>,
    pub sections: Vec<Section>,
    /// Added to every point so the feet are at the origin, Unreal units.
    pub lift: f32,
    /// Whether the stored triangle order had to be reversed.
    pub flipped: bool,
}

/// A mesh point (the mesh's own units and axes) to Unreal units relative
/// to the character's origin, before the character's own turn: the mesh's
/// scale, origin and turn are applied, as in `rc_character`.
pub fn mesh_point(mesh: &SkeletalMesh, p: [f32; 3]) -> [f32; 3] {
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

/// Unreal (Z up) to Bevy (Y up), the same rule as `rc_game_core::units`
/// (50 Unreal units to one Bevy unit).
pub fn to_bevy(p: [f32; 3]) -> [f32; 3] {
    [p[0] / 50.0, p[2] / 50.0, p[1] / 50.0]
}

/// Packages read so far, by lower-case name (`None`: not found).
struct Packages<'a> {
    resolve: &'a dyn Fn(&str) -> PathBuf,
    read: HashMap<String, Option<(Package, Vec<u8>)>>,
}

impl Packages<'_> {
    /// Find a package by its name, in the folders materials live in.
    fn get(&mut self, name: &str) -> Option<&(Package, Vec<u8>)> {
        let key = name.to_ascii_lowercase();
        if !self.read.contains_key(&key) {
            let found = [
                format!("GameData/Textures/{key}.utx"),
                format!("GameData/Animations/{key}.ukx"),
                format!("GameData/StaticMeshes/{key}.usx"),
            ]
            .iter()
            .find_map(|rel| {
                let data = std::fs::read((self.resolve)(rel)).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some((pkg, data))
            });
            self.read.insert(key.clone(), found);
        }
        self.read.get(&key).and_then(|p| p.as_ref())
    }
}

/// The outermost name of an import (the package file it lives in) and
/// its own name and class.
fn import_location(pkg: &Package, i: usize) -> (String, String, String) {
    let name = pkg.name(pkg.imports[i].object_name).to_string();
    let class = pkg.name(pkg.imports[i].class_name).to_string();
    let mut outer = pkg.imports[i].package_index;
    let mut top = String::new();
    while let ObjectRef::Import(j) = ObjectRef::from_index(outer) {
        top = pkg.name(pkg.imports[j].object_name).to_string();
        outer = pkg.imports[j].package_index;
    }
    (top, name, class)
}

/// Where a material is: which package, and which export in it.
#[derive(Clone)]
struct Place {
    package: String,
    export: usize,
}

/// Find an object by name (and class, when known: names can clash, like
/// the battle droid's texture group and its texture, both "BattleDroid").
/// `path` is `Object`, `Group.Object` or longer; only the last part and
/// the class are compared.
fn find_in(pkg: &Package, path: &str, class: Option<&str>) -> Option<usize> {
    let name = path.rsplit('.').next().unwrap_or(path);
    let materials = [
        "Texture",
        "Shader",
        "Combiner",
        "FinalBlend",
        "TexModifier",
        "TexPanner",
        "TexPanner2D",
        "TexScaler",
        "TexRotator",
        "TexOscillator",
        "TexEnvMap",
        "TexCoordSource",
    ];
    pkg.exports.iter().position(|e| {
        let c = pkg.export_class_name(e);
        pkg.name(e.object_name).eq_ignore_ascii_case(name)
            && match class {
                Some(want) => c.eq_ignore_ascii_case(want),
                None => materials.iter().any(|m| c.eq_ignore_ascii_case(m)),
            }
    })
}

fn export_props(
    pkg: &Package,
    data: &[u8],
    e: &Export,
) -> Result<Vec<properties::Property>, String> {
    let start = e.serial_offset as usize;
    let end = start + e.serial_size as usize;
    let bytes = data.get(start..end).ok_or("object outside its file")?;
    let mut r = rc_package::Reader::new(bytes);
    properties::read_properties(pkg, &mut r).map_err(|e| e.to_string())
}

/// Follow a material down to its picture: a Shader's `Diffuse`, a
/// Combiner's `Material1`, a FinalBlend's or modifier's `Material`. Steps
/// may cross into other packages.
fn follow(packs: &mut Packages, start: Place) -> Result<Skin, String> {
    let mut at = start;
    let mut chain = Vec::new();
    let mut cutout = false;
    for _ in 0..8 {
        let (pkg, data) = packs
            .get(&at.package)
            .ok_or_else(|| format!("package {} not found", at.package))?;
        let export = &pkg.exports[at.export];
        let name = pkg.name(export.object_name).to_string();
        let class = pkg.export_class_name(export).to_string();
        chain.push(format!("{class} {}.{name}", at.package));
        if class == "Texture" {
            let tex = texture::read_texture(pkg, data, export).map_err(|e| e.to_string())?;
            cutout |= tex.masked(pkg);
            let image = decode_texture(pkg, data, export)?;
            return Ok(Skin {
                texture: name,
                image,
                cutout,
                chain,
            });
        }
        let props = export_props(pkg, data, export)?;
        let object = |setting: &str| match properties::find(pkg, &props, setting).map(|p| &p.value)
        {
            Some(Value::Object(o)) if *o != 0 => Some(*o),
            _ => None,
        };
        let setting = match class.as_str() {
            "Shader" => {
                cutout |= object("Opacity").is_some();
                "Diffuse"
            }
            "Combiner" => "Material1",
            "FinalBlend" => {
                cutout |= matches!(
                    properties::find(pkg, &props, "bAlphaTest").map(|p| &p.value),
                    Some(Value::Bool(true))
                );
                "Material"
            }
            // TexPanner2D is Republic Commando's own (the ammo counter's
            // digits on the first-person rifle): followed to its picture
            // here; rc_combat's arms_view slides it by the ammo count.
            "TexModifier" | "TexPanner" | "TexPanner2D" | "TexScaler" | "TexRotator"
            | "TexOscillator" | "TexEnvMap" | "TexCoordSource" => "Material",
            other => return Err(format!("{name} is a {other}, which we can't follow yet")),
        };
        let next = object(setting).ok_or_else(|| format!("{class} {name} has no {setting}"))?;
        at = match ObjectRef::from_index(next) {
            ObjectRef::Export(e) if e < pkg.exports.len() => Place {
                package: at.package.clone(),
                export: e,
            },
            ObjectRef::Import(i) if i < pkg.imports.len() => {
                let (top, obj, cls) = import_location(pkg, i);
                let (other, _) = packs
                    .get(&top)
                    .ok_or_else(|| format!("{setting} of {name} is in {top}, which isn't found"))?;
                let e = find_in(other, &obj, Some(&cls))
                    .ok_or_else(|| format!("no {cls} called {obj} in {top}"))?;
                Place {
                    package: top,
                    export: e,
                }
            }
            _ => return Err(format!("{setting} of {name} points nowhere")),
        };
    }
    Err("materials nested too deeply".into())
}

fn decode_texture(pkg: &Package, data: &[u8], export: &Export) -> Result<Rgba, String> {
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
    // Models are the squad, the arms and weapons: CHARACTER DETAIL.
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

/// The package name of a game path (`GameData/Animations/clone.ukx` ->
/// `clone`).
fn package_name(rel: &str) -> String {
    let file = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    file.split('.').next().unwrap_or(file).to_string()
}

impl Model {
    /// Read what `spec` names. `resolve` turns a path relative to the game
    /// folder into a real file path (any upper or lower case).
    pub fn load(
        resolve: &dyn Fn(&str) -> PathBuf,
        spec: &ModelSpec,
    ) -> Result<(Model, Vec<String>), String> {
        let mut notes = Vec::new();
        let file = resolve(&spec.package);
        let data =
            std::fs::read(&file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
        let pkg = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        let export = pkg
            .exports
            .iter()
            .find(|e| {
                pkg.export_class_name(e) == "SkeletalMesh"
                    && pkg.name(e.object_name).eq_ignore_ascii_case(&spec.mesh)
            })
            .ok_or_else(|| format!("no SkeletalMesh called {} in {}", spec.mesh, file.display()))?;
        let name = pkg.name(export.object_name).to_string();
        let mesh = read_skeletal_mesh(&pkg, &data, export).map_err(|e| e.to_string())?;
        // Animation sets: the one asked for (or the first the mesh links
        // to), then every other set the mesh links to in its package.
        // Commandos keep their everyday moves in one set and the
        // single-player ones (wounded, healing) in another.
        let linked: Vec<&Export> = mesh
            .animations
            .iter()
            .filter_map(|&a| match ObjectRef::from_index(a) {
                ObjectRef::Export(i) => pkg.exports.get(i),
                _ => None,
            })
            .collect();
        let first = match &spec.set {
            Some(s) => pkg.exports.iter().find(|e| {
                pkg.export_class_name(e) == "MeshAnimation"
                    && pkg.name(e.object_name).eq_ignore_ascii_case(s)
            }),
            None => linked.first().copied(),
        }
        .ok_or("animation set not found in the mesh's package")?;
        let set_name = pkg.name(first.object_name).to_string();
        let mut sets = vec![first];
        sets.extend(linked.into_iter().filter(|e| !std::ptr::eq(*e, first)));
        let skeleton = Skeleton::from_mesh(&mesh, Convention::InvertRoot);
        let mut clips = HashMap::new();
        let mut read_sets: Vec<Option<rc_package::animation::MeshAnimation>> =
            (0..sets.len()).map(|_| None).collect();
        for (key, seq_name) in &spec.sequences {
            let mut found = false;
            for (k, set) in sets.iter().enumerate() {
                if read_sets[k].is_none() {
                    match read_mesh_animation(&pkg, &data, set) {
                        Ok(a) => read_sets[k] = Some(a),
                        Err(e) => {
                            notes.push(format!(
                                "{name}: animation set {} not read: {e}",
                                pkg.name(set.object_name)
                            ));
                            continue;
                        }
                    }
                }
                let Some(anim) = &read_sets[k] else { continue };
                if let Some(seq) = anim
                    .sequences
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(seq_name))
                {
                    clips.insert(key.clone(), Clip::new(anim, seq, &skeleton));
                    if k > 0 {
                        notes.push(format!(
                            "{name}: {key} = {seq_name} from {}",
                            pkg.name(set.object_name)
                        ));
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                notes.push(format!(
                    "{name}: no animation called {seq_name} in its {} animation sets; {key} stands still",
                    sets.len()
                ));
            }
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
        let flipped = volume < 0.0;

        // One section per material slot, in slot order. A triangle's
        // `material` indexes the mesh's material list, whose entries name
        // the slot in `textures`.
        let slot_of = |m: u8| -> usize {
            mesh.materials
                .get(m as usize)
                .map(|mm| mm.texture_index.max(0) as usize)
                .unwrap_or(m as usize)
        };
        let mut by_slot: std::collections::BTreeMap<usize, Section> = Default::default();
        let mut local: HashMap<(usize, u16), u32> = HashMap::new();
        for t in &mesh.triangles {
            let slot = slot_of(t.material);
            let s = by_slot.entry(slot).or_insert_with(|| Section {
                slot,
                material: String::new(),
                skin: None,
                wedges: Vec::new(),
                indices: Vec::new(),
            });
            let mut corner = |w: u16| -> u32 {
                *local.entry((slot, w)).or_insert_with(|| {
                    s.wedges.push(w as u32);
                    s.wedges.len() as u32 - 1
                })
            };
            let [a, b, c] = t.wedges.map(&mut corner);
            s.indices
                .extend(if flipped { [a, c, b] } else { [a, b, c] });
        }

        let mut packs = Packages {
            resolve,
            read: HashMap::new(),
        };
        let own = package_name(&spec.package);
        let mut sections: Vec<Section> = by_slot.into_values().collect();
        for s in &mut sections {
            let over = spec.skins.get(s.slot).cloned().flatten();
            let place = match &over {
                Some(path) => {
                    let (top, rest) = path.split_once('.').unwrap_or((path.as_str(), ""));
                    packs
                        .get(top)
                        .ok_or_else(|| format!("package {top} not found"))
                        .and_then(|(p, _)| {
                            find_in(p, rest, None).ok_or_else(|| format!("no material {path}"))
                        })
                        .map(|e| {
                            s.material = path.clone();
                            Place {
                                package: top.to_string(),
                                export: e,
                            }
                        })
                }
                None => match mesh.textures.get(s.slot).map(|&r| ObjectRef::from_index(r)) {
                    Some(ObjectRef::Import(i)) if i < pkg.imports.len() => {
                        let (top, obj, cls) = import_location(&pkg, i);
                        s.material = format!("{top}.{obj}");
                        packs
                            .get(&top)
                            .ok_or_else(|| format!("package {top} not found"))
                            .and_then(|(p, _)| {
                                find_in(p, &obj, Some(&cls))
                                    .ok_or_else(|| format!("no {cls} called {obj} in {top}"))
                            })
                            .map(|e| Place {
                                package: top,
                                export: e,
                            })
                    }
                    Some(ObjectRef::Export(e)) if e < pkg.exports.len() => {
                        s.material = format!("{own}.{}", pkg.name(pkg.exports[e].object_name));
                        Ok(Place {
                            package: own.clone(),
                            export: e,
                        })
                    }
                    _ => {
                        s.material = format!("slot {}", s.slot);
                        Err("the mesh names no material here".to_string())
                    }
                },
            };
            match place.and_then(|p| follow(&mut packs, p)) {
                Ok(skin) => s.skin = Some(skin),
                Err(e) => notes.push(format!(
                    "{name}: material {} ({}): no picture, drawn plain grey: {e}",
                    s.slot, s.material
                )),
            }
        }

        let mut model = Model {
            name,
            mesh,
            skeleton,
            weights,
            clips,
            sections,
            lift: 0.0,
            flipped,
        };
        // Stand the first animation's first frame on the floor (the mesh
        // itself when no animation was found).
        let rest = match spec.sequences.first() {
            Some((k, _)) if model.clips.contains_key(k) => model.pose_unreal(k, 0.0),
            _ => model
                .mesh
                .points
                .iter()
                .map(|&p| mesh_point(&model.mesh, p))
                .collect(),
        };
        let (lo, hi) = rest
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[2]), hi.max(p[2]))
            });
        model.lift = -lo;
        notes.push(format!(
            "mesh {}.{}: {} bones, {} points, {} triangles ({}), animation set {set_name}, {} of {} animations found, {:.0} uu tall (lowest point {lo:.0} uu, lifted to the floor), {} materials",
            own,
            model.name,
            model.mesh.bones.len(),
            model.mesh.points.len(),
            model.mesh.triangles.len(),
            if flipped { "flipped" } else { "kept as stored" },
            model.clips.len(),
            spec.sequences.len(),
            hi - lo,
            model.sections.len(),
        ));
        for s in &model.sections {
            notes.push(format!(
                "  material {}: {} ({} triangles): {}",
                s.slot,
                s.material,
                s.indices.len() / 3,
                match &s.skin {
                    Some(k) => {
                        let [r, g, b, _] = k.image.mean();
                        format!(
                            "picture {} {}x{}{}, average colour R {r:.0} G {g:.0} B {b:.0} (via {})",
                            k.texture,
                            k.image.width,
                            k.image.height,
                            if k.cutout { ", see-through parts" } else { "" },
                            k.chain.join(" -> ")
                        )
                    }
                    None => "no picture".into(),
                }
            ));
        }
        Ok((model, notes))
    }

    /// The animation called `key`, if it was found.
    pub fn clip(&self, key: &str) -> Option<&Clip> {
        self.clips.get(key)
    }

    /// Each bone's local transform at `frame` of animation `key` (the
    /// mesh's own pose if it wasn't found). The root bone's sideways
    /// travel is removed: the body moves the character, not the animation.
    pub fn local_pose(&self, key: &str, frame: f32) -> Vec<Transform> {
        let Some(clip) = self.clips.get(key) else {
            return self.skeleton.local.clone();
        };
        let mut local = clip.sample(&self.skeleton, frame);
        if let (Some(l), Some(rest)) = (local.first_mut(), self.skeleton.local.first()) {
            l.pos[0] = rest.pos[0];
            l.pos[1] = rest.pos[1];
        }
        local
    }

    /// Points posed at `frame` of `key`, Unreal units relative to the mesh
    /// origin (feet not yet lifted).
    fn pose_unreal(&self, key: &str, frame: f32) -> Vec<[f32; 3]> {
        let pose = globals(&self.skeleton.parents, &self.local_pose(key, frame));
        skin(&self.mesh.points, &self.weights, &self.skeleton, &pose)
            .into_iter()
            .map(|p| mesh_point(&self.mesh, p))
            .collect()
    }

    /// Points for bones placed at `local` (from `local_pose`, perhaps
    /// blended), in Bevy units relative to the feet, before the
    /// character's facing is applied.
    pub fn posed_local(&self, local: &[Transform]) -> Vec<[f32; 3]> {
        let pose = globals(&self.skeleton.parents, local);
        skin(&self.mesh.points, &self.weights, &self.skeleton, &pose)
            .into_iter()
            .map(|p| {
                let p = mesh_point(&self.mesh, p);
                to_bevy([p[0], p[1], p[2] + self.lift])
            })
            .collect()
    }

    /// Points posed at `frame` of animation `key`, in Bevy units (axes as
    /// [`to_bevy`]) relative to the mesh's own origin: nothing lifted and
    /// the root bone's travel kept. For models that aren't characters
    /// standing on a floor, such as the first-person arms, whose origin is
    /// the eye.
    pub fn posed_from_origin(&self, key: &str, frame: f32) -> Vec<[f32; 3]> {
        let local = match self.clips.get(key) {
            Some(clip) => clip.sample(&self.skeleton, frame),
            None => self.skeleton.local.clone(),
        };
        let pose = globals(&self.skeleton.parents, &local);
        skin(&self.mesh.points, &self.weights, &self.skeleton, &pose)
            .into_iter()
            .map(|p| to_bevy(mesh_point(&self.mesh, p)))
            .collect()
    }

    /// Points posed at `frame` of animation `key`, in Bevy units relative
    /// to the feet, before the character's facing is applied.
    pub fn posed(&self, key: &str, frame: f32) -> Vec<[f32; 3]> {
        self.posed_local(&self.local_pose(key, frame))
    }

    /// Smooth normals per mesh point, for posed `points`.
    pub fn normals(&self, points: &[[f32; 3]]) -> Vec<[f32; 3]> {
        use rc_anim::math::{cross, length, sub};
        let mut n = vec![[0f32; 3]; points.len()];
        for t in &self.mesh.triangles {
            let [a, b, d] = t
                .wedges
                .map(|w| self.mesh.wedges[w as usize].point as usize);
            let mut f = cross(sub(points[b], points[a]), sub(points[d], points[a]));
            if self.flipped {
                f = f.map(|x| -x);
            }
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
        n
    }

    /// One section's vertex positions and normals, from posed `points` and
    /// their `normals`.
    pub fn section_attributes(
        &self,
        s: &Section,
        points: &[[f32; 3]],
        normals: &[[f32; 3]],
    ) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let point = |w: &u32| self.mesh.wedges[*w as usize].point as usize;
        (
            s.wedges.iter().map(|w| points[point(w)]).collect(),
            s.wedges.iter().map(|w| normals[point(w)]).collect(),
        )
    }

    pub fn section_uvs(&self, s: &Section) -> Vec<[f32; 2]> {
        s.wedges
            .iter()
            .map(|&w| {
                let w = &self.mesh.wedges[w as usize];
                [w.u, w.v]
            })
            .collect()
    }

    /// The colour a section should show, 0-255 sRGB: its picture sampled at
    /// every triangle's middle, weighted by the triangle's size, for posed
    /// `points`. Pixels a cut-out picture leaves see-through are skipped.
    /// For on-screen checks: a wrong picture, wrong UVs or a renderer that
    /// lost its textures all change it.
    pub fn expected_colour(&self, s: &Section, points: &[[f32; 3]]) -> Option<[f64; 3]> {
        use rc_anim::math::{cross, length, sub};
        let skin = s.skin.as_ref()?;
        let img = &skin.image;
        let (mut sum, mut total) = ([0f64; 3], 0f64);
        for tri in s.indices.chunks_exact(3) {
            let w =
                [tri[0], tri[1], tri[2]].map(|i| &self.mesh.wedges[s.wedges[i as usize] as usize]);
            let p = w.map(|w| points[w.point as usize]);
            let area = length(cross(sub(p[1], p[0]), sub(p[2], p[0]))) as f64;
            let u = (w[0].u + w[1].u + w[2].u) / 3.0;
            let v = (w[0].v + w[1].v + w[2].v) / 3.0;
            let x = ((u.rem_euclid(1.0)) * img.width as f32) as usize % img.width.max(1);
            let y = ((v.rem_euclid(1.0)) * img.height as f32) as usize % img.height.max(1);
            let px = &img.pixels[(y * img.width + x) * 4..][..4];
            if skin.cutout && px[3] < 128 {
                continue;
            }
            for k in 0..3 {
                sum[k] += px[k] as f64 * area;
            }
            total += area;
        }
        (total > 0.0).then(|| sum.map(|c| c / total))
    }
}
