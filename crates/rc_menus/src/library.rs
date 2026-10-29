//! Opens the game's packages on demand and reads what the menus need from
//! them: class default values (merged down the class family), pictures,
//! fonts and sounds. Everything is read from the player's install at run
//! time and kept in memory only.

use crate::value::{decode_all, Field, V};
use rc_image::{PixelFormat, Rgba};
use rc_package::class::{self, object_class, object_path};
use rc_package::font::{self, Glyph};
use rc_package::properties::{self, Value};
use rc_package::{sound, texture, ObjectRef, Package};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A package and its file bytes.
pub struct Loaded {
    pub pkg: Package,
    pub data: Vec<u8>,
}

/// A class's default values, merged with those of every parent class.
#[derive(Debug, Clone)]
pub struct ClassData {
    /// Full name, like `XInterfaceCTMenus.CTMenuMain`.
    pub path: String,
    /// The class and its parents, nearest first.
    pub family: Vec<String>,
    /// All default values, parents' first, in the order they are declared.
    pub defaults: Vec<Field>,
    /// Lowercase property names in the order the classes declare them,
    /// parents first (the order of the property objects in each package).
    pub declared: Vec<String>,
    /// Lowercase property name -> editor category ("MenuDefault" marks
    /// style templates, which are never drawn).
    pub categories: HashMap<String, String>,
}

impl ClassData {
    pub fn get(&self, name: &str) -> Option<&V> {
        self.get_at(name, 0)
    }

    pub fn get_at(&self, name: &str, index: u32) -> Option<&V> {
        self.defaults
            .iter()
            .find(|f| f.index == index && f.name.eq_ignore_ascii_case(name))
            .map(|f| &f.value)
    }

    /// Where a property is declared, parents first (`usize::MAX` if not
    /// found).
    pub fn declared_at(&self, name: &str) -> usize {
        let n = name.to_ascii_lowercase();
        self.declared
            .iter()
            .position(|d| *d == n)
            .unwrap_or(usize::MAX)
    }

    pub fn category(&self, name: &str) -> &str {
        self.categories
            .get(&name.to_ascii_lowercase())
            .map_or("", |s| s.as_str())
    }
}

/// A font ready to draw: its letters and the full paths of its pages.
#[derive(Debug, Clone)]
pub struct FontData {
    pub path: String,
    pub glyphs: Vec<Glyph>,
    pub pages: Vec<String>,
    pub kerning: i32,
    pub line_height: i32,
}

impl FontData {
    pub fn glyph(&self, c: char) -> Option<Glyph> {
        let g = *self.glyphs.get(c as usize)?;
        (g.width > 0 && g.height > 0).then_some(g)
    }
}

pub struct Library {
    /// Lowercase package name -> file.
    files: HashMap<String, PathBuf>,
    packages: HashMap<String, Option<Arc<Loaded>>>,
    classes: HashMap<String, Result<Arc<ClassData>, String>>,
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
            if ["u", "utx", "uax", "usx", "ukx"].contains(&ext.as_str()) {
                out.entry(stem.to_string_lossy().to_ascii_lowercase())
                    .or_insert(p);
            }
        }
    }
}

/// The texture format numbers we can decode (UE2's ETextureFormat).
fn pixel_format(format: u8) -> Option<PixelFormat> {
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

impl Library {
    /// Index every package under the game's `GameData` folder.
    pub fn new(game_data: &Path) -> Library {
        let mut files = HashMap::new();
        find_files(game_data, &mut files);
        Library {
            files,
            packages: HashMap::new(),
            classes: HashMap::new(),
        }
    }

    pub fn package_count(&self) -> usize {
        self.files.len()
    }

    /// Open a package by name (no extension, any case). Cached.
    pub fn package(&mut self, name: &str) -> Option<Arc<Loaded>> {
        let key = name.to_ascii_lowercase();
        if !self.packages.contains_key(&key) {
            let loaded = self.files.get(&key).and_then(|p| {
                let data = std::fs::read(p).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some(Arc::new(Loaded { pkg, data }))
            });
            self.packages.insert(key.clone(), loaded);
        }
        self.packages.get(&key).cloned().flatten()
    }

    /// Find an object by its full dotted path, like
    /// `GUIContent.Menu.CT_Kamino`. Returns the package and export index.
    pub fn find(&mut self, path: &str) -> Result<(Arc<Loaded>, usize), String> {
        let parts: Vec<&str> = path.split('.').collect();
        let (top, rest) = parts.split_first().ok_or("empty object path")?;
        let loaded = self
            .package(top)
            .ok_or_else(|| format!("package {top} not found"))?;
        let pkg = &loaded.pkg;
        let last = rest.last().ok_or("object path has no object name")?;
        let found = (0..pkg.exports.len()).find(|&i| {
            let e = &pkg.exports[i];
            pkg.name(e.object_name).eq_ignore_ascii_case(last)
                && object_path(pkg, i as i32 + 1).eq_ignore_ascii_case(&rest.join("."))
        });
        match found {
            Some(i) => Ok((loaded, i)),
            None => Err(format!("{path} not found")),
        }
    }

    /// A class's default values merged with its parents'. `path` is
    /// `Package.Class`.
    pub fn class(&mut self, path: &str) -> Result<Arc<ClassData>, String> {
        let key = path.to_ascii_lowercase();
        if let Some(c) = self.classes.get(&key) {
            return c.clone();
        }
        let result = self.read_class(path).map(Arc::new);
        self.classes.insert(key, result.clone());
        result
    }

    fn read_class(&mut self, path: &str) -> Result<ClassData, String> {
        let (loaded, i) = self.find(path)?;
        let (pkg, data) = (&loaded.pkg, &loaded.data);
        let e = &pkg.exports[i];
        if pkg.export_class_name(e) != "Class" {
            return Err(format!("{path} is not a class"));
        }
        let own = class::read_class_defaults(pkg, data, e).map_err(|e| e.to_string())?;
        let own_fields = decode_all(pkg, &own.properties);
        let mut categories = HashMap::new();
        let mut declared = Vec::new();
        for p in &pkg.exports {
            if p.package_index == i as i32 + 1 && pkg.export_class_name(p).ends_with("Property") {
                let name = pkg.name(p.object_name).to_ascii_lowercase();
                if let Ok(c) = class::read_property_category(pkg, data, p) {
                    categories.insert(name.clone(), c);
                }
                declared.push(name);
            }
        }
        // The parent: an import names its package; an export is in this
        // package.
        let parent = match ObjectRef::from_index(own.super_class) {
            ObjectRef::None => None,
            ObjectRef::Import(_) => Some(object_path(pkg, own.super_class)),
            ObjectRef::Export(_) => {
                let package = path.split('.').next().unwrap_or_default();
                Some(format!("{package}.{}", object_path(pkg, own.super_class)))
            }
        };
        let mut data = match parent {
            Some(p) => {
                let parent = self.class(&p)?;
                (*parent).clone()
            }
            None => ClassData {
                path: String::new(),
                family: vec![],
                defaults: vec![],
                declared: vec![],
                categories: HashMap::new(),
            },
        };
        data.path = path.to_string();
        data.family.insert(0, path.to_string());
        // Each class stores only the values that differ from its parent's.
        for f in own_fields {
            match data
                .defaults
                .iter_mut()
                .find(|d| d.index == f.index && d.name.eq_ignore_ascii_case(&f.name))
            {
                Some(d) => d.value = f.value,
                None => data.defaults.push(f),
            }
        }
        data.categories.extend(categories);
        // A class may declare a property with the same name as a parent's
        // (CTMenuMain has its own `Border`); the class's own one is the one
        // that counts, at its own place in the order.
        for d in declared {
            data.declared.retain(|x| *x != d);
            data.declared.push(d);
        }
        Ok(data)
    }

    /// Decode a picture by full path. Wrapper materials (`FinalBlend`,
    /// `Shader`, `TexModifier` and friends) are followed to the texture they
    /// show.
    pub fn texture(&mut self, path: &str) -> Result<Rgba, String> {
        let mut path = path.to_string();
        for _ in 0..8 {
            let (loaded, i) = self.find(&path)?;
            let (pkg, data) = (&loaded.pkg, &loaded.data);
            let e = &pkg.exports[i];
            let kind = pkg.export_class_name(e).to_string();
            if kind == "Texture" {
                return decode_texture(pkg, data, i);
            }
            let links: &[&str] = match kind.as_str() {
                "Shader" => &["Diffuse", "SelfIllumination"],
                "Combiner" => &["Material1", "Material2"],
                _ => &["Material"],
            };
            let object = class::export_data(data, e).map_err(|e| e.to_string())?;
            let mut r = rc_package::Reader::new(object);
            let props = properties::read_properties(pkg, &mut r).map_err(|e| e.to_string())?;
            let next = links
                .iter()
                .find_map(|l| match properties::find(pkg, &props, l) {
                    Some(p) => match p.value {
                        Value::Object(o) if o != 0 => Some(o),
                        _ => None,
                    },
                    None => None,
                });
            let Some(o) = next else {
                return Err(format!("{path}: {kind} has no picture to follow"));
            };
            path = match ObjectRef::from_index(o) {
                ObjectRef::Import(_) => object_path(pkg, o),
                _ => format!(
                    "{}.{}",
                    path.split('.').next().unwrap_or_default(),
                    object_path(pkg, o)
                ),
            };
        }
        Err(format!("{path}: material links nest too deep"))
    }

    pub fn font(&mut self, path: &str) -> Result<FontData, String> {
        let (loaded, i) = self.find(path)?;
        let (pkg, data) = (&loaded.pkg, &loaded.data);
        let e = &pkg.exports[i];
        if pkg.export_class_name(e) != "Font" {
            return Err(format!("{path} is not a font"));
        }
        let f = font::read_font(pkg, data, e).map_err(|e| e.to_string())?;
        let package = path.split('.').next().unwrap_or_default();
        let pages = f
            .pages
            .iter()
            .map(|&p| match ObjectRef::from_index(p) {
                ObjectRef::Import(_) => object_path(pkg, p),
                _ => format!("{package}.{}", object_path(pkg, p)),
            })
            .collect();
        Ok(FontData {
            path: path.to_string(),
            line_height: f.line_height(),
            glyphs: f.glyphs,
            pages,
            kerning: f.kerning,
        })
    }

    /// A sound's embedded `.wav` file. `SoundMultiple` picks its first sound.
    pub fn sound(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let mut path = path.to_string();
        for _ in 0..4 {
            let (loaded, i) = self.find(&path)?;
            let (pkg, data) = (&loaded.pkg, &loaded.data);
            let e = &pkg.exports[i];
            match pkg.export_class_name(e) {
                "Sound" => {
                    let s = sound::read_sound(pkg, data, e).map_err(|e| e.to_string())?;
                    return Ok(s.bytes(data).to_vec());
                }
                "SoundMultiple" => {
                    let m = sound::read_multiple(pkg, data, e).map_err(|e| e.to_string())?;
                    let first = *m.sounds.first().ok_or("SoundMultiple is empty")?;
                    path = match ObjectRef::from_index(first) {
                        ObjectRef::Import(_) => object_path(pkg, first),
                        _ => format!(
                            "{}.{}",
                            path.split('.').next().unwrap_or_default(),
                            object_path(pkg, first)
                        ),
                    };
                }
                other => return Err(format!("{path} is a {other}, not a sound")),
            }
        }
        Err(format!("{path}: sound links nest too deep"))
    }

    /// The class name of an object (for logs).
    pub fn class_of(&mut self, path: &str) -> String {
        match self.find(path) {
            Ok((l, i)) => object_class(&l.pkg, i as i32 + 1),
            Err(_) => String::new(),
        }
    }
}

fn decode_texture(pkg: &Package, data: &[u8], index: usize) -> Result<Rgba, String> {
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
    rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        bytes,
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())
}
