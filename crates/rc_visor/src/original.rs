//! What the visor reads from the original game at run time: each glowing
//! thing's colour (its class's `EnhancedVisionColor` default) and the words
//! shown when the mode changes ("TACTICAL MODE", "LOW LIGHT MODE", from the
//! `Helmet` class's defaults). Nothing from the game is stored in this
//! repository; `data/visor/tactical.toml` only names the classes.

use crate::rules::glow_rgb;
use rc_package::class_library::{ClassLibrary, DefaultValue};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The data file, under `data/` (or a mod's folder).
pub const DATA_FILE: &str = "visor/tactical.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlowShape {
    /// Glow strength at TACTICAL MODE INTENSITY 1.0 (10 on the menu).
    pub strength_scale: f32,
    /// How sharply the glow gathers at the edges (higher: thinner outline).
    pub edge_power: f32,
    /// Share of the glow that also covers the middle of a body, 0 to 1.
    pub fill: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Headlamp {
    /// Bevy light intensity (lumens) of the lamp in low light mode.
    pub lumens: f32,
    /// How far it reaches, Unreal units.
    pub range: f32,
    /// Full width of the beam, degrees.
    pub angle: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisorFile {
    /// Script packages searched for the classes below, under the install.
    pub packages: Vec<String>,
    pub glow: GlowShape,
    /// Glow name (a squad mate's name in lower case, or "pickup") to the
    /// original class whose `EnhancedVisionColor` it uses.
    pub colours: BTreeMap<String, String>,
    /// Colours (red, green, blue, 0 to 255) used instead of the game's, for
    /// mods.
    #[serde(default)]
    pub colour_overrides: BTreeMap<String, [u8; 3]>,
    pub headlamp: Headlamp,
}

impl VisorFile {
    pub fn parse(text: &str) -> Result<VisorFile, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn read(path: &Path) -> Result<VisorFile, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("can't read {}: {e}", path.display()))?;
        VisorFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// What was found in the install.
#[derive(Debug, Clone, Default)]
pub struct Original {
    /// Glow name to colour (red, green, blue).
    pub colours: BTreeMap<String, [u8; 3]>,
    /// Shown when the visor switches to tactical / low light mode.
    pub tactical_text: Option<String>,
    pub low_light_text: Option<String>,
    /// Seconds the text shows (the original's `VisorModeTextTime`).
    pub text_seconds: Option<f32>,
    /// What went wrong, for the log. Missing pieces fall back to nothing
    /// (no glow for that thing, no text).
    pub problems: Vec<String>,
}

/// A string default as stored: a length byte (counting the closing zero),
/// the letters, then a zero.
fn string_value(v: &DefaultValue) -> Option<String> {
    let DefaultValue::Raw(b) = v else {
        return None;
    };
    let (&len, rest) = b.split_first()?;
    let text = rest.get(..(len as usize).checked_sub(1)?)?;
    Some(String::from_utf8_lossy(text).into_owned())
}

/// Read the colours and texts. `game_file` turns a path under the install
/// (like `GameData/System/Engine.u`) into a real path.
pub fn read(file: &VisorFile, game_file: &dyn Fn(&str) -> PathBuf) -> Original {
    let mut out = Original::default();
    let mut lib = ClassLibrary::new();
    for rel in &file.packages {
        let path = game_file(rel);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(&path) {
            Ok(data) => {
                if let Err(e) = lib.add_package(&name, data) {
                    out.problems.push(format!("{rel}: {e}"));
                }
            }
            Err(e) => out
                .problems
                .push(format!("can't read {}: {e}", path.display())),
        }
    }
    for (what, class) in &file.colours {
        if let Some(rgb) = file.colour_overrides.get(what) {
            out.colours.insert(what.clone(), *rgb);
            continue;
        }
        if !lib.has_class(class) {
            out.problems
                .push(format!("{what}: class {class} not found"));
            continue;
        }
        let d = lib.defaults(class);
        match d.get("EnhancedVisionColor").map(|e| &e.value) {
            Some(DefaultValue::Raw(b)) => match glow_rgb(b) {
                Some(rgb) => {
                    out.colours.insert(what.clone(), rgb);
                }
                None => out
                    .problems
                    .push(format!("{what}: {class} has no glow (black)")),
            },
            _ => out
                .problems
                .push(format!("{what}: {class} sets no EnhancedVisionColor")),
        }
    }
    // Overrides for things with no class named.
    for (what, rgb) in &file.colour_overrides {
        out.colours.entry(what.clone()).or_insert(*rgb);
    }
    let helmet = lib.defaults("Helmet");
    out.tactical_text = helmet
        .get("TacticalVisionModeText")
        .and_then(|e| string_value(&e.value));
    out.low_light_text = helmet
        .get("LowLightVisionModeText")
        .and_then(|e| string_value(&e.value));
    out.text_seconds = helmet.number("VisorModeTextTime");
    if out.tactical_text.is_none() {
        out.problems
            .push("Helmet's TacticalVisionModeText not found".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_file_reads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(DATA_FILE);
        let f = VisorFile::read(&path).unwrap();
        for who in ["sev", "fixer", "scorch", "pickup"] {
            assert!(f.colours.contains_key(who), "{who}");
        }
        assert!(f.glow.strength_scale > 0.0);
    }

    #[test]
    fn strings_are_length_letters_zero() {
        let v = DefaultValue::Raw(b"\x0eTACTICAL MODE\0".to_vec());
        assert_eq!(string_value(&v).as_deref(), Some("TACTICAL MODE"));
        assert_eq!(string_value(&DefaultValue::Raw(vec![])), None);
    }

    /// Reads the real install when RC_GAME_DIR points at it (skipped
    /// otherwise): Sev red, Fixer green, Scorch yellow, pickups pale blue.
    #[test]
    fn reads_the_install_if_present() {
        let Some(dir) = std::env::var_os("RC_GAME_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(DATA_FILE);
        let f = VisorFile::read(&path).unwrap();
        let o = read(&f, &|rel| rc_game_core::paths::resolve(&dir, rel));
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        let [r, g, b] = o.colours["sev"];
        assert!(r > g && r > b);
        let [r, g, b] = o.colours["fixer"];
        assert!(g > r && g > b);
        let [r, g, b] = o.colours["scorch"];
        assert!(r > b && g > b);
        let [r, _, b] = o.colours["pickup"];
        assert!(b > r);
        assert_eq!(o.tactical_text.as_deref(), Some("TACTICAL MODE"));
    }
}
