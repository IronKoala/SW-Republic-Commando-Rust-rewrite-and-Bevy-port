//! Turns a material reference from a map (a texture, shader or similar,
//! usually stored in another package) into decoded RGBA pixels.
//!
//! Unreal materials can wrap each other: a `Shader` points at its `Diffuse`
//! texture, a `TexPanner` at the `Material` it moves, and so on. We follow
//! those links until we reach a plain `Texture`, which is enough to show the
//! surface's main picture. Republic Commando's bump-mapped materials also
//! name a bump map, which is decoded alongside (see `Bump`). Animation
//! comes later.

use rc_image::{PixelFormat, Rgba};
use rc_package::properties::{self, Value};
use rc_package::texture;
use rc_package::{ObjectRef, Package};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

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
    /// True if the surface should have see-through holes: some pixels have
    /// a low alpha AND the material says to use alpha that way (see
    /// `alpha_pixels`).
    pub has_alpha: bool,
    /// True if some pixels have a low alpha (below 128), whatever it is for.
    /// Many Republic Commando textures keep a shine (specular) mask in
    /// alpha, so low alpha alone does not mean see-through. Unreal only cuts
    /// holes when the texture says `bMasked` or `bAlphaTexture`, or a
    /// wrapper turns it on (`FinalBlend` with `AlphaTest` or alpha
    /// blending, a `Shader` with an `Opacity` map or masked output).
    pub alpha_pixels: bool,
    /// Glow that adds light instead of covering what's behind (the game's
    /// `HsFalloff` light shafts).
    pub additive: bool,
    /// Blended over what's behind by its alpha, rather than cut into solid
    /// and see-through parts (`FinalBlend` with `FrameBufferBlending`
    /// `FB_AlphaBlend`: the briefing holograms' grids and text).
    pub blend: bool,
    /// A hologram (`HsHologram`): the picture is already turned into its
    /// glow colour (`rc_image::hologram`) and is drawn as added light,
    /// without the level's light on it.
    pub hologram: bool,
    /// The material's bump map, if it has one (`HsBump...` materials'
    /// `NormalMap`, or a `Shader`'s `Bumpmap`).
    pub bump: Option<Arc<Bump>>,
}

/// A bump map ready for drawing: which way each pixel of the surface tilts.
///
/// `image` holds the usual normal-map colours, already turned the way the
/// level shader wants them: red is the tilt along the texture's width (the
/// U direction), green along its height (V, down the picture), blue how much
/// the pixel faces straight out, each 0..255 for -1..1. A flat pixel is
/// (128, 128, 255).
///
/// Nothing documents which way the game's bump maps point, so it was
/// measured (docs/DESIGN.md "Drawing bump maps"). A real tilt map is the
/// slope of some height map, and the slopes of a height map have to fit
/// together (how the tilt along the width changes going down equals how the
/// tilt along the height changes going across), which tells which channel
/// runs which way. Whether a tilt means a bump or a dent comes from the
/// main picture: grooves are painted darker, so the picture is darker where
/// the tilts point inwards.
///
/// - Colour normal maps (most bump maps on level props: all 16 used by
///   yyy_06a, geo_01a and ras_01a): red along the width, green down the
///   picture, as stored. All 16 fit that way round; the main picture
///   agrees on the direction for 11 of the 13 we could compare (the
///   others use a different repeat).
/// - The game's two-byte formats (12 V8U8 and 14 L6V5U5, mostly on
///   characters and weapons): the other way round. The low bits (what
///   Direct3D calls U, and `rc_image` puts in red) follow the picture's
///   height, so the two are swapped here. On the 104 L6V5U5 maps (at mip
///   1, to quiet the 5-bit steps) that fitted clearly best on 49 and the
///   next best reading on 2. The 29 V8U8 maps are mixed (13 clearly
///   decided: 9 swapped, 4 not) and are read the same way.
pub struct Bump {
    pub image: Rgba,
    /// How many times the bump map repeats per repeat of the main picture
    /// (`BumpUVScale`, 1 when not set).
    pub uv_scale: f32,
    /// True when the map was stored in one of the game's two-byte bump
    /// formats (12 or 14), false for a colour normal map.
    pub two_byte_format: bool,
    /// Where the bump map came from, for the log.
    pub name: String,
}

/// Why a material couldn't be shown, kept for the log.
pub type Failure = String;

pub struct Materials {
    /// Lowercase package name (file name without extension) -> file path.
    files: HashMap<String, PathBuf>,
    packages: HashMap<String, Option<Loaded>>,
    /// Cache by full object path, e.g. `geo_textures.walls.TierWall`.
    cache: HashMap<String, Result<Picture, Failure>>,
    /// Bump maps by full object path.
    bumps: HashMap<String, Result<Arc<Bump>, Failure>>,
    /// Bump maps named by a material that couldn't be read, with why.
    pub bump_failures: BTreeMap<String, usize>,
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
    let mip = tex
        .mip_for(texture::Detail::World)
        .ok_or("texture has no mipmaps")?;
    let bytes = &data[mip.data_offset..mip.data_offset + mip.data_len];
    let image = rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        bytes,
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let alpha_pixels = image.pixels.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
    let flag = |name: &str| {
        matches!(
            tex.property(pkg, name).map(|p| &p.value),
            Some(Value::Bool(true))
        )
    };
    let uses_alpha = flag("bMasked") || flag("bAlphaTexture");
    Ok(Picture {
        image,
        has_alpha: alpha_pixels && uses_alpha,
        alpha_pixels,
        additive: false,
        blend: false,
        hologram: false,
        bump: None,
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
            bumps: HashMap::new(),
            bump_failures: BTreeMap::new(),
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
        if let Some(colour_prop) = match class.as_str() {
            "ConstantColor" => Some("Color"),
            "FadeColor" => Some("Color1"),
            _ => None,
        } {
            return constant_colour(pkg, data, k, colour_prop);
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
        if class == "HsHologram" {
            return self.hologram(pkg, data, &props, depth);
        }
        // `HsFalloff` (light shafts, the prologue's tank liquid) tints its
        // picture by `DiffuseTint` and `GradientTint` (white, no change,
        // when not set: the class defaults in Engine.u). Its falloff
        // picture, which fades the edges by viewing angle, isn't used.
        let falloff_tint = additive.then(|| {
            let a = setting_colour(pkg, &props, "DiffuseTint").unwrap_or([255; 4]);
            let b = setting_colour(pkg, &props, "GradientTint").unwrap_or([255; 4]);
            [0, 1, 2].map(|k| ((a[k] as u32 * b[k] as u32 + 127) / 255) as u8)
        });
        let blend = class == "FinalBlend"
            && matches!(
                properties::find(pkg, &props, "FrameBufferBlending").map(|p| &p.value),
                Some(Value::Byte(2))
            );
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
                let cuts_holes = wrapper_uses_alpha(pkg, &class, &props);
                let bump = self.material_bump(pkg, data, &class, &props, depth);
                return found.map(|mut p| {
                    p.additive |= additive;
                    p.blend |= blend && !p.hologram;
                    if let Some(t) = falloff_tint.filter(|t| *t != [255; 3]) {
                        rc_image::tint(&mut p.image, t);
                    }
                    if cuts_holes {
                        p.has_alpha = p.alpha_pixels;
                    }
                    if bump.is_some() {
                        p.bump = bump;
                    }
                    p
                });
            }
        }
        let names: Vec<_> = props.iter().map(|p| pkg.name(p.name).to_string()).collect();
        Err(format!("{class} has none of {links:?} (it has {names:?})"))
    }
}

impl Materials {
    /// An `HsHologram`: its `DiffuseTexture` (or plain white if it has
    /// none, like `Holo_Blank`) turned into glow of its `HologramColor`.
    fn hologram(
        &mut self,
        pkg: &Package,
        data: &[u8],
        props: &[properties::Property],
        depth: usize,
    ) -> Result<Picture, Failure> {
        let colour = setting_colour(pkg, props, "HologramColor").unwrap_or(HOLOGRAM_COLOUR);
        let base = match properties::find(pkg, props, "DiffuseTexture").map(|p| &p.value) {
            Some(Value::Object(o)) if *o != 0 => match ObjectRef::from_index(*o) {
                ObjectRef::Export(j) => Some(self.follow(pkg, data, j, depth + 1)?),
                ObjectRef::Import(_) => Some(self.resolve_path(&object_path(pkg, *o), depth + 1)?),
                ObjectRef::None => None,
            },
            _ => None,
        };
        let image = match base {
            Some(p) => p.image,
            None => Rgba {
                width: 1,
                height: 1,
                pixels: vec![255; 4],
            },
        };
        Ok(Picture {
            image: rc_image::hologram(&image, [colour[0], colour[1], colour[2]]),
            has_alpha: false,
            alpha_pixels: false,
            additive: true,
            blend: false,
            hologram: true,
            bump: None,
        })
    }
}

/// `HsHologram`'s default `HologramColor` (from its class defaults in the
/// game's Engine.u), a light blue, as red, green, blue, alpha.
pub const HOLOGRAM_COLOUR: [u8; 4] = [122, 206, 255, 50];

/// A material's `Color` setting as red, green, blue, alpha (see
/// `rc_image::unreal_color` for the stored order), if it is set.
pub fn setting_colour(pkg: &Package, props: &[properties::Property], name: &str) -> Option<[u8; 4]> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Raw(b)) if b.len() == 4 => Some(rc_image::unreal_color([b[0], b[1], b[2], b[3]])),
        _ => None,
    }
}

impl Materials {
    /// The bump map a material names, if any: `NormalMap` on Republic
    /// Commando's `Hs...` materials (with `BumpUVScale`), `Bumpmap` on a
    /// `Shader` (with `BumpUVScale`). Failures are counted in
    /// `bump_failures` and the surface is drawn without one.
    fn material_bump(
        &mut self,
        pkg: &Package,
        data: &[u8],
        class: &str,
        props: &[properties::Property],
        depth: usize,
    ) -> Option<Arc<Bump>> {
        let link = match class {
            c if c.starts_with("Hs") => "NormalMap",
            "Shader" => "Bumpmap",
            _ => return None,
        };
        let Some(Value::Object(o)) = properties::find(pkg, props, link).map(|p| &p.value) else {
            return None;
        };
        let o = *o;
        if o == 0 {
            return None;
        }
        let uv_scale = match properties::find(pkg, props, "BumpUVScale").map(|p| &p.value) {
            Some(Value::Float(f)) if *f > 0.0 => *f,
            _ => 1.0,
        };
        let path = object_path(pkg, o);
        let key = match ObjectRef::from_index(o) {
            ObjectRef::Export(_) => format!("(local){}", path.join(".")),
            _ => path.join("."),
        };
        let cached = match self.bumps.get(&key) {
            Some(b) => b.clone(),
            None => {
                let found = match ObjectRef::from_index(o) {
                    ObjectRef::Export(j) => self.bump_texture(pkg, data, j, depth + 1, &key),
                    _ => self.bump_path(&path, depth + 1, &key),
                };
                self.bumps.insert(key.clone(), found.clone());
                found
            }
        };
        match cached {
            Ok(b) if (b.uv_scale - uv_scale).abs() < 1e-6 => Some(b),
            Ok(b) => Some(Arc::new(Bump {
                image: b.image.clone(),
                uv_scale,
                two_byte_format: b.two_byte_format,
                name: b.name.clone(),
            })),
            Err(e) => {
                *self.bump_failures.entry(e).or_default() += 1;
                None
            }
        }
    }

    fn bump_path(
        &mut self,
        path: &[String],
        depth: usize,
        key: &str,
    ) -> Result<Arc<Bump>, Failure> {
        let Some((top, rest)) = path.split_first() else {
            return Err("empty bump map path".into());
        };
        let loaded = self
            .package(top)
            .ok_or_else(|| format!("bump map package {top} not found"))?;
        let k = find_export(&loaded.0, rest)
            .ok_or_else(|| format!("bump map {} not found in {top}", rest.join(".")))?;
        self.bump_texture(&loaded.0, &loaded.1, k, depth, key)
    }

    /// Decode a bump map texture (following wrapper materials, as for
    /// pictures, if it isn't a plain texture).
    fn bump_texture(
        &mut self,
        pkg: &Package,
        data: &[u8],
        k: usize,
        depth: usize,
        key: &str,
    ) -> Result<Arc<Bump>, Failure> {
        let e = &pkg.exports[k];
        if pkg.export_class_name(e) != "Texture" {
            // Something else used as a bump map: take its picture.
            let p = self.follow(pkg, data, k, depth)?;
            return Ok(Arc::new(Bump {
                image: p.image,
                uv_scale: 1.0,
                two_byte_format: false,
                name: key.to_string(),
            }));
        }
        let tex = texture::read_texture(pkg, data, e).map_err(|e| e.to_string())?;
        let (image, two_byte_format) = match tex.format {
            12 | 14 => {
                let format = if tex.format == 12 {
                    PixelFormat::V8U8
                } else {
                    PixelFormat::L6V5U5
                };
                let mip = tex
                    .mip_for(texture::Detail::World)
                    .ok_or("bump map has no mipmaps")?;
                let bytes = &data[mip.data_offset..mip.data_offset + mip.data_len];
                let mut image =
                    rc_image::decode(format, mip.width as usize, mip.height as usize, bytes, None)
                        .map_err(|e| e.to_string())?;
                // Red follows the picture's height and green its width in
                // the stored maps (see `Bump`); swap them.
                for p in image.pixels.as_chunks_mut::<4>().0 {
                    p.swap(0, 1);
                }
                (image, true)
            }
            _ => (decode_texture(pkg, data, k)?.image, false),
        };
        Ok(Arc::new(Bump {
            image,
            uv_scale: 1.0,
            two_byte_format,
            name: key.to_string(),
        }))
    }
}

/// Whether a wrapper material turns on see-through pixels for what it
/// wraps. Property names and values are from the game's `Engine.u`:
/// `FinalBlend.FrameBufferBlending` 2 is `FB_AlphaBlend`, and
/// `Shader.OutputBlending` 1 is `OB_Masked`.
fn wrapper_uses_alpha(pkg: &Package, class: &str, props: &[properties::Property]) -> bool {
    let get = |name: &str| properties::find(pkg, props, name).map(|p| &p.value);
    match class {
        "FinalBlend" => {
            matches!(get("AlphaTest"), Some(Value::Bool(true)))
                || matches!(get("FrameBufferBlending"), Some(Value::Byte(2)))
        }
        "Shader" => {
            matches!(get("Opacity"), Some(Value::Object(o)) if *o != 0)
                || matches!(get("OutputBlending"), Some(Value::Byte(1)))
        }
        _ => false,
    }
}

/// A material that is one flat colour (`ConstantColor`, or the first colour
/// of a `FadeColor`), as a 1x1 picture. The colour is a 4-byte `Color`
/// struct, read red, green, blue, alpha (UE Viewer's `FColor` order). An
/// unset colour is the engine default, opaque black.
fn constant_colour(pkg: &Package, data: &[u8], k: usize, name: &str) -> Result<Picture, Failure> {
    let e = &pkg.exports[k];
    let start = e.serial_offset as usize;
    let bytes = &data[start..start + e.serial_size as usize];
    let mut r = rc_package::Reader::new(bytes);
    let props = properties::read_properties(pkg, &mut r).map_err(|e| e.to_string())?;
    let rgba = match properties::find(pkg, &props, name).map(|p| &p.value) {
        Some(Value::Raw(b)) if b.len() == 4 => [b[0], b[1], b[2], b[3]],
        None => [0, 0, 0, 255],
        Some(v) => return Err(format!("{name} is {v:?}, expected 4 colour bytes")),
    };
    Ok(Picture {
        image: Rgba {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        },
        has_alpha: rgba[3] < 255,
        alpha_pixels: rgba[3] < 255,
        additive: false,
        blend: false,
        hologram: false,
        bump: None,
    })
}
