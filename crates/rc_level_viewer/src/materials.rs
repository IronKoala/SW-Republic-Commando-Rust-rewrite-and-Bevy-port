//! Turns a material reference from a map (a texture, shader or similar,
//! usually stored in another package) into decoded RGBA pixels.
//!
//! Unreal materials can wrap each other: a `Shader` points at its `Diffuse`
//! texture, a `TexPanner` at the `Material` it moves, and so on. We follow
//! those links until we reach a plain `Texture`, which is enough to show the
//! surface's main picture. Lighting, bump maps and animation come later.

use rc_image::{PixelFormat, Rgba};
use rc_package::properties::{self, Value};
use rc_package::texture;
use rc_package::{ObjectRef, Package};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A package and its file bytes, shared between users.
pub type Loaded = Rc<(Package, Vec<u8>)>;

/// Which property to follow for each wrapper class, in order of preference.
/// Class and property names are from the UE2 material classes in the game's
/// own `Engine.u` script package.
fn links_for(class: &str) -> &'static [&'static str] {
    match class {
        "Shader" => &["Diffuse", "SelfIllumination", "Specular"],
        "Combiner" => &["Material1", "Material2"],
        "FinalBlend" | "TexPanner" | "TexScaler" | "TexRotator" | "TexOscillator"
        | "TexCoordSource" | "TexEnvMap" | "TexMatrix" | "TexModifier" | "VariableTexPanner"
        | "ColorModifier" | "OpacityModifier" | "MaterialSequence" => &["Material"],
        "MaterialSwitch" => &["Materials", "Material"],
        // Republic Commando's own materials (HsBumpDiffSpec, HsBumpDiffBlend,
        // HsBumpIllumSpecMask, HsFalloff, ...) all name their main picture
        // `DiffuseTexture` and a plain `FallbackMaterial` (found by reading
        // their properties in arena_textures.utx).
        c if c.starts_with("Hs") => &["DiffuseTexture", "FallbackMaterial"],
        _ => &[],
    }
}

/// A decoded picture, ready for the GPU.
#[derive(Clone)]
pub struct Picture {
    pub image: Rgba,
    /// True if some pixels are see-through.
    pub has_alpha: bool,
    /// Glow that adds light instead of covering what's behind (the game's
    /// `HsFalloff` light shafts).
    pub additive: bool,
}

/// Why a material couldn't be shown, kept for the log.
pub type Failure = String;

pub struct Materials {
    /// Lowercase package name (file name without extension) -> file path.
    files: HashMap<String, PathBuf>,
    packages: HashMap<String, Option<Loaded>>,
    /// Cache by full object path, e.g. `geo_textures.walls.TierWall`.
    cache: HashMap<String, Result<Picture, Failure>>,
}

/// The dotted path of an import or export, outermost package first.
pub fn object_path(pkg: &Package, index: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = index;
    for _ in 0..16 {
        match ObjectRef::from_index(i) {
            ObjectRef::None => break,
            ObjectRef::Import(k) => {
                let Some(m) = pkg.imports.get(k) else { break };
                out.push(pkg.name(m.object_name).to_string());
                i = m.package_index;
            }
            ObjectRef::Export(k) => {
                let Some(e) = pkg.exports.get(k) else { break };
                out.push(pkg.name(e.object_name).to_string());
                i = e.package_index;
            }
        }
    }
    out.reverse();
    out
}

fn find_files(dir: &Path, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            find_files(&p, out);
        } else if let (Some(stem), Some(ext)) = (p.file_stem(), p.extension()) {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            if ["utx", "usx", "u", "ctm", "ukx"].contains(&ext.as_str()) {
                out.entry(stem.to_string_lossy().to_ascii_lowercase())
                    .or_insert(p);
            }
        }
    }
}

pub fn pixel_format(format: u8) -> Option<PixelFormat> {
    Some(match format {
        0 => PixelFormat::P8,
        3 => PixelFormat::Dxt1,
        5 => PixelFormat::Bgra8,
        7 => PixelFormat::Dxt3,
        8 => PixelFormat::Dxt5,
        10 => PixelFormat::G16,
        _ => return None,
    })
}

/// Decode a texture export's full-size picture.
fn decode_texture(pkg: &Package, data: &[u8], index: usize) -> Result<Picture, Failure> {
    let e = &pkg.exports[index];
    let tex = texture::read_texture(pkg, data, e).map_err(|e| e.to_string())?;
    let format = pixel_format(tex.format)
        .ok_or_else(|| format!("texture format {} not decoded yet", tex.format))?;
    let palette = match tex.palette_ref(pkg).map(ObjectRef::from_index) {
        Some(ObjectRef::Export(i)) => {
            let mut colors =
                texture::read_palette(pkg, data, &pkg.exports[i]).map_err(|e| e.to_string())?;
            if tex.masked(pkg) {
                colors[0][3] = 0;
            }
            Some(colors)
        }
        Some(_) => return Err("palette in another package".into()),
        None => None,
    };
    let mip = tex.mips.first().ok_or("texture has no mipmaps")?;
    let bytes = &data[mip.data_offset..mip.data_offset + mip.data_len];
    let image = rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        bytes,
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let has_alpha = image.pixels.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
    Ok(Picture {
        image,
        has_alpha,
        additive: false,
    })
}

/// Find the export whose path inside its package (groups, then name)
/// matches `rest`, ignoring upper/lower case.
pub fn find_export(pkg: &Package, rest: &[String]) -> Option<usize> {
    let last = rest.last()?;
    (0..pkg.exports.len()).find(|&i| {
        let e = &pkg.exports[i];
        pkg.name(e.object_name).eq_ignore_ascii_case(last) && {
            let p = object_path(pkg, i as i32 + 1);
            p.len() == rest.len() && p.iter().zip(rest).all(|(a, b)| a.eq_ignore_ascii_case(b))
        }
    })
}

impl Materials {
    pub fn new(game_data: &Path) -> Self {
        let mut files = HashMap::new();
        find_files(game_data, &mut files);
        Materials {
            files,
            packages: HashMap::new(),
            cache: HashMap::new(),
        }
    }

    pub fn package_files(&self) -> usize {
        self.files.len()
    }

    /// Open a package by name (cached).
    pub fn package(&mut self, name: &str) -> Option<Loaded> {
        let key = name.to_ascii_lowercase();
        if !self.packages.contains_key(&key) {
            let loaded = self.files.get(&key).and_then(|p| {
                let data = std::fs::read(p).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some(Rc::new((pkg, data)))
            });
            self.packages.insert(key.clone(), loaded);
        }
        self.packages.get(&key).and_then(|p| p.clone())
    }

    /// Resolve a material reference found in package `pkg` (the map, or a
    /// static mesh package).
    pub fn resolve(&mut self, pkg: &Package, data: &[u8], index: i32) -> Result<Picture, Failure> {
        match ObjectRef::from_index(index) {
            ObjectRef::None => Err("no material".into()),
            ObjectRef::Export(k) => {
                // Stored in the same package (for example textures saved
                // inside the map itself).
                let key = format!("(local){}", object_path(pkg, index).join("."));
                if let Some(c) = self.cache.get(&key) {
                    return c.clone();
                }
                let result = self.follow(pkg, data, k, 0);
                self.cache.insert(key, result.clone());
                result
            }
            ObjectRef::Import(_) => {
                let path = object_path(pkg, index);
                let key = path.join(".");
                if let Some(c) = self.cache.get(&key) {
                    return c.clone();
                }
                let result = self.resolve_path(&path, 0);
                self.cache.insert(key, result.clone());
                result
            }
        }
    }

    fn resolve_path(&mut self, path: &[String], depth: usize) -> Result<Picture, Failure> {
        let Some((top, rest)) = path.split_first() else {
            return Err("empty path".into());
        };
        let Some(loaded) = self.package(top) else {
            return Err(format!("package {top} not found"));
        };
        let (pkg, data) = (&loaded.0, &loaded.1);
        let Some(k) = find_export(pkg, rest) else {
            return Err(format!("{} not found in {top}", rest.join(".")));
        };
        self.follow(pkg, data, k, depth)
    }

    /// Follow wrapper materials until a texture is reached.
    fn follow(
        &mut self,
        pkg: &Package,
        data: &[u8],
        k: usize,
        depth: usize,
    ) -> Result<Picture, Failure> {
        if depth > 8 {
            return Err("material links nest too deep".into());
        }
        let e = &pkg.exports[k];
        let class = pkg.export_class_name(e).to_string();
        if class == "Texture" {
            return decode_texture(pkg, data, k);
        }
        let links = links_for(&class);
        if links.is_empty() {
            return Err(format!("material class {class} not handled"));
        }
        let additive = class == "HsFalloff";
        let start = e.serial_offset as usize;
        let bytes = &data[start..start + e.serial_size as usize];
        let mut r = rc_package::Reader::new(bytes);
        let props = properties::read_properties(pkg, &mut r).map_err(|e| e.to_string())?;
        for link in links {
            if let Some(Value::Object(o)) = properties::find(pkg, &props, link).map(|p| &p.value) {
                if *o == 0 {
                    continue;
                }
                let found = match ObjectRef::from_index(*o) {
                    ObjectRef::Export(j) => self.follow(pkg, data, j, depth + 1),
                    ObjectRef::Import(_) => {
                        let path = object_path(pkg, *o);
                        self.resolve_path(&path, depth + 1)
                    }
                    ObjectRef::None => continue,
                };
                return found.map(|mut p| {
                    p.additive |= additive;
                    p
                });
            }
        }
        let names: Vec<_> = props.iter().map(|p| pkg.name(p.name).to_string()).collect();
        Err(format!("{class} has none of {links:?} (it has {names:?})"))
    }
}
