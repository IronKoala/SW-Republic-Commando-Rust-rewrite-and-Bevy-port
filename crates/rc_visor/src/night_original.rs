//! What the low light mode reads from the original game at run time.
//!
//! The original's low light view is the vision mode object
//! `FrameFX.VisionModes.VisionNight` (a `VisionModeSniper`). Its settings
//! name a hardware shader (`VisionShaderNight`), how much film grain to add
//! (`Noise`, 0 to 255), how much of the last picture to keep
//! (`MotionBlur`), and a distance fog (`DistanceFogStart`/`End`, Unreal
//! units). The shader in turn names two pictures: an overlay whose
//! transparency is the visor-shaped window you see through and whose colour
//! is added on top (`HUDTextures.Helmet.HUD_GlassNight`), and a tiling film
//! grain (`FrameFX.FBFX.FilmGrain`), plus how many times the grain repeats
//! across and down the screen (its vertex constant 1).
//!
//! We read those facts and pictures from the install each time the game
//! starts; none of them is stored in this repository. The shader that uses
//! them (`night_vision.wesl`) is our own, written from what the original's
//! settings say it does: see `docs/DESIGN.md`.

use rc_image::Rgba;
use rc_menus::library::Library;
use rc_package::class::{export_data, object_path};
use rc_package::properties::{read_properties, Property, Value};
use rc_package::{Package, Reader};

/// What was found. Anything missing is `None` and the data file's own
/// value is used instead.
#[derive(Debug, Clone, Default)]
pub struct NightOriginal {
    /// The visor window: alpha is how much of the view shows through,
    /// colour is added on top.
    pub mask: Option<Rgba>,
    /// Where the mask came from, for the log.
    pub mask_path: Option<String>,
    /// The film grain picture (grey around the middle).
    pub grain_picture: Option<Rgba>,
    pub grain_path: Option<String>,
    /// Grain strength, 0 to 1 (`Noise` / 255).
    pub grain: Option<f32>,
    /// How many times the grain repeats across and down a 4:3 screen.
    pub grain_tiles: Option<[f32; 2]>,
    /// Share of the last picture kept each frame, 0 to 1 (`MotionBlur` /
    /// 255).
    pub trail: Option<f32>,
    /// Fog start and end, Unreal units, when the mode sets its own fog.
    pub fog: Option<(f32, f32)>,
    pub problems: Vec<String>,
}

fn props_of(pkg: &Package, data: &[u8], index: usize) -> Result<Vec<Property>, String> {
    let e = &pkg.exports[index];
    let object = export_data(data, e).map_err(|e| e.to_string())?;
    let mut r = Reader::new(object);
    read_properties(pkg, &mut r).map_err(|e| e.to_string())
}

fn get<'a>(pkg: &Package, props: &'a [Property], name: &str, index: u32) -> Option<&'a Value> {
    props
        .iter()
        .find(|p| p.array_index == index && pkg.name(p.name).eq_ignore_ascii_case(name))
        .map(|p| &p.value)
}

/// A full path for an object reference found in `top`'s package.
fn path_of(pkg: &Package, top: &str, o: i32) -> String {
    match rc_package::ObjectRef::from_index(o) {
        rc_package::ObjectRef::Import(_) => object_path(pkg, o),
        _ => format!("{top}.{}", object_path(pkg, o)),
    }
}

/// A shader constant (`SConstantsInfo`: a `Type` byte and a `Value` plane
/// X, Y, Z, W) stored as raw bytes: its X and Y.
fn constant_xy(pkg: &Package, v: &Value) -> Option<[f32; 2]> {
    let Value::Raw(raw) = v else {
        return None;
    };
    let mut r = Reader::new(raw);
    let outer = read_properties(pkg, &mut r).ok()?;
    let plane = outer
        .iter()
        .find(|p| pkg.name(p.name).eq_ignore_ascii_case("Value"))?;
    let Value::Raw(inner) = &plane.value else {
        return None;
    };
    let mut r = Reader::new(inner);
    let xyzw = read_properties(pkg, &mut r).ok()?;
    let f = |n: &str| match get(pkg, &xyzw, n, 0) {
        Some(Value::Float(x)) => Some(*x),
        _ => None,
    };
    Some([f("X")?, f("Y")?])
}

/// Read the vision mode at `mode_path` (like
/// `FrameFX.VisionModes.VisionNight`) and the pictures its shader uses.
pub fn read(lib: &mut Library, mode_path: &str) -> NightOriginal {
    let mut out = NightOriginal::default();
    let (loaded, i) = match lib.find(mode_path) {
        Ok(x) => x,
        Err(e) => {
            out.problems.push(e);
            return out;
        }
    };
    let top = mode_path.split('.').next().unwrap_or_default().to_string();
    let (pkg, data) = (&loaded.pkg, &loaded.data);
    let mode = match props_of(pkg, data, i) {
        Ok(p) => p,
        Err(e) => {
            out.problems.push(format!("{mode_path}: {e}"));
            return out;
        }
    };
    let byte = |n: &str| match get(pkg, &mode, n, 0) {
        Some(Value::Byte(b)) => Some(*b as f32 / 255.0),
        _ => None,
    };
    let float = |n: &str| match get(pkg, &mode, n, 0) {
        Some(Value::Float(x)) => Some(*x),
        _ => None,
    };
    out.grain = byte("Noise");
    out.trail = byte("MotionBlur");
    let own_fog = matches!(get(pkg, &mode, "OverideFog", 0), Some(Value::Bool(true)));
    if own_fog {
        if let (Some(s), Some(e)) = (float("DistanceFogStart"), float("DistanceFogEnd")) {
            out.fog = Some((s, e));
        }
    }
    let shader = match get(pkg, &mode, "VisionShader", 0) {
        Some(Value::Object(o)) if *o != 0 => path_of(pkg, &top, *o),
        _ => {
            out.problems.push(format!("{mode_path} names no shader"));
            return out;
        }
    };
    let (sloaded, si) = match lib.find(&shader) {
        Ok(x) => x,
        Err(e) => {
            out.problems.push(e);
            return out;
        }
    };
    let (spkg, sdata) = (&sloaded.pkg, &sloaded.data);
    let stop = shader.split('.').next().unwrap_or_default().to_string();
    let sprops = match props_of(spkg, sdata, si) {
        Ok(p) => p,
        Err(e) => {
            out.problems.push(format!("{shader}: {e}"));
            return out;
        }
    };
    out.grain_tiles = get(spkg, &sprops, "VSConstants", 1).and_then(|v| constant_xy(spkg, v));
    // Its pictures: 2 is the overlay and window, 3 the film grain.
    let picture = |n: u32| match get(spkg, &sprops, "Textures", n) {
        Some(Value::Object(o)) if *o != 0 => Some(path_of(spkg, &stop, *o)),
        _ => None,
    };
    let (mask, grain) = (picture(2), picture(3));
    drop(sloaded);
    drop(loaded);
    for (path, slot, slot_path, what) in [
        (mask, &mut out.mask, &mut out.mask_path, "overlay"),
        (
            grain,
            &mut out.grain_picture,
            &mut out.grain_path,
            "film grain",
        ),
    ] {
        match path {
            Some(p) => match lib.texture(&p) {
                Ok(img) => {
                    *slot = Some(img);
                    *slot_path = Some(p);
                }
                Err(e) => out.problems.push(format!("{what} {p}: {e}")),
            },
            None => out
                .problems
                .push(format!("{shader} names no {what} picture")),
        }
    }
    out
}

/// A plain vision mode's film grain (normal and tactical): the mode's
/// `Noise` (0 to 1) and how many times its shader's grain repeats across
/// and down the screen (vertex constant 1). A mode that names no shader
/// uses `default_shader` (engine.u's `VisionMode` default,
/// `VisionShaderNormal`). The shader adds `strength * (grain - 0.5)` to the
/// picture (its pixel shader: `mad r0.rgb, c1, t1_bias, t0`), where engine.dll
/// sets the strength to `Noise` plus the HUD's static.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModeGrain {
    pub noise: f32,
    pub tiles: Option<[f32; 2]>,
    pub shader: Option<String>,
    pub problems: Vec<String>,
}

pub fn read_mode_grain(lib: &mut Library, mode_path: &str, default_shader: &str) -> ModeGrain {
    let mut out = ModeGrain::default();
    let mut shader = default_shader.to_string();
    match lib.find(mode_path) {
        Ok((loaded, i)) => {
            let top = mode_path.split('.').next().unwrap_or_default().to_string();
            match props_of(&loaded.pkg, &loaded.data, i) {
                Ok(mode) => {
                    if let Some(Value::Byte(b)) = get(&loaded.pkg, &mode, "Noise", 0) {
                        out.noise = *b as f32 / 255.0;
                    }
                    if let Some(Value::Object(o)) = get(&loaded.pkg, &mode, "VisionShader", 0) {
                        if *o != 0 {
                            shader = path_of(&loaded.pkg, &top, *o);
                        }
                    }
                }
                Err(e) => out.problems.push(format!("{mode_path}: {e}")),
            }
        }
        Err(e) => out.problems.push(e),
    }
    match lib.find(&shader) {
        Ok((loaded, i)) => match props_of(&loaded.pkg, &loaded.data, i) {
            Ok(p) => {
                out.tiles = get(&loaded.pkg, &p, "VSConstants", 1)
                    .and_then(|v| constant_xy(&loaded.pkg, v));
                if out.tiles.is_none() {
                    out.problems.push(format!("{shader} has no grain repeats"));
                }
            }
            Err(e) => out.problems.push(format!("{shader}: {e}")),
        },
        Err(e) => out.problems.push(e),
    }
    out.shader = Some(shader);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Reads the real install when RC_GAME_DIR points at it (skipped
    /// otherwise).
    #[test]
    fn reads_the_install_if_present() {
        let Some(dir) = std::env::var_os("RC_GAME_DIR") else {
            return;
        };
        let mut lib = Library::new(&PathBuf::from(dir).join("GameData"));
        let n = read(&mut lib, "FrameFX.VisionModes.VisionNight");
        assert!(n.problems.is_empty(), "{:?}", n.problems);
        let mask = n.mask.clone().unwrap();
        assert_eq!((mask.width, mask.height), (512, 512));
        // The window is open in the middle and closed in the corner.
        let a = |x: usize, y: usize| mask.pixels[(y * mask.width + x) * 4 + 3];
        assert!(a(256, 256) > 200 && a(0, 0) < 20);
        assert!(n.grain_picture.is_some());
        let g = n.grain.unwrap();
        assert!(g > 0.05 && g < 0.3, "{g}");
        let [x, y] = n.grain_tiles.unwrap();
        assert!(x > 1.0 && y > 1.0, "{x} {y}");
        let (s, e) = n.fog.unwrap();
        assert!(s > 0.0 && e > s);
        println!("{n:?}");
        // The plain modes: no grain of their own, the repeats of their
        // shaders (VisionNormal names none: engine.u's default).
        let normal = read_mode_grain(
            &mut lib,
            "FrameFX.VisionModes.VisionNormal",
            "FrameFX.VisionShaders.VisionShaderNormal",
        );
        assert!(normal.problems.is_empty(), "{:?}", normal.problems);
        assert_eq!(normal.noise, 0.0);
        assert_eq!(normal.tiles, Some([6.265625, 6.7421875]));
        let tactical = read_mode_grain(
            &mut lib,
            "FrameFX.VisionModes.VisionEnhance",
            "FrameFX.VisionShaders.VisionShaderNormal",
        );
        assert!(tactical.problems.is_empty(), "{:?}", tactical.problems);
        assert_eq!(
            tactical.shader.as_deref(),
            Some("FrameFX.VisionShaders.VisionShaderEnhance")
        );
        assert_eq!(tactical.tiles, Some([9.21875, 6.5390625]));
    }
}
