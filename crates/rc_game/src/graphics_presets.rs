//! The original's GRAPHICS QUALITY presets, read from the install at start.
//!
//! The original's GRAPHICS options page (`CTGraphicsOptionsPCMenu` in
//! `XInterfaceCTMenus.u`) keeps no presets of its own: it names five
//! sections of `System.ini` (its `PresetKeys`: `GraphicsQualityLowest` to
//! `GraphicsQualityHighest`) and reads each row's value from them. SET
//! RECOMMENDED uses the section named by `[VideoSettings] DefaultConfig`,
//! which the original writes on its first run after looking at the
//! graphics card.
//!
//! Like the original, this reads `GameData/System/system.ini` (the shipped
//! values) and then `GameData/Save/System.ini` (the player's own copy, where
//! lines that are only commented out keep the shipped value). Nothing here
//! is copied into the port: the numbers come from the install every run.
//!
//! What the port does with each value is in `option_settings.rs`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

/// The preset sections, in the order of the GRAPHICS QUALITY list (LOWEST,
/// LOW, MEDIUM, HIGH, HIGHEST): the menu's `PresetKeys`.
pub const PRESET_KEYS: [&str; 5] = [
    "GraphicsQualityLowest",
    "GraphicsQualityLow",
    "GraphicsQualityMedium",
    "GraphicsQualityHigh",
    "GraphicsQualityHighest",
];

/// The texture groups a preset names, in the order the menu reads them
/// (`UseTextureDetailPreset`).
pub const TEXTURE_KEYS: [&str; 7] = [
    "TextureDetailInterface",
    "TextureDetailTerrain",
    "TextureDetailWeaponSkin",
    "TextureDetailPlayerSkin",
    "TextureDetailWorld",
    "TextureDetailRenderMap",
    "TextureDetailLightmap",
];

/// One preset section, read the way the original's script reads it
/// (`int("")` is 0, `bool("")` is false).
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub key: String,
    /// `WindowedViewportX/Y`.
    pub windowed: (u32, u32),
    /// `FullscreenViewportX/Y`.
    pub fullscreen: (u32, u32),
    /// The seven texture groups' detail names (`UltraHigh` ... `Lower`),
    /// upper case.
    pub textures: [String; 7],
    /// `DropLODBy`: 0 (CHARACTER DETAIL HIGH), 1 (MEDIUM) or 2 (LOW).
    pub drop_lod_by: u8,
    /// `Shadows` (SQUAD SHADOWS).
    pub shadows: bool,
    /// `BumpmappingQuality`: 0 LOW, 1 MEDIUM, 2 HIGH.
    pub bump: u8,
    /// `BlurEnabled`.
    pub blur: bool,
    /// `BloomQuality`: 0 NONE, 1 LOW, 2 HIGH.
    pub bloom: u8,
    /// `Projectors`.
    pub projectors: bool,
    /// `FSAA`: 0 is none, else the number of samples.
    pub fsaa: u32,
}

/// What the install says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Presets {
    /// LOWEST to HIGHEST. Empty if the install's `System.ini` couldn't be
    /// read; the GRAPHICS QUALITY row then always shows CUSTOM.
    pub list: Vec<Preset>,
    /// `[VideoSettings] DefaultConfig`: the section SET RECOMMENDED uses.
    pub recommended: Option<String>,
}

impl Presets {
    pub fn recommended(&self) -> Option<&Preset> {
        let key = self.recommended.as_deref()?;
        self.list.iter().find(|p| p.key.eq_ignore_ascii_case(key))
    }

    /// Read from the install's two `System.ini` files.
    pub fn from_install(shipped: &Path, own: &Path) -> Result<Presets, String> {
        let shipped_text = read_text(shipped)?;
        let mut ini = Ini::parse(&shipped_text);
        // The player's copy only counts where it has a line that isn't
        // commented out.
        if let Ok(own_text) = read_text(own) {
            ini.overlay(&Ini::parse(&own_text));
        }
        Ok(Presets::from_ini(&ini))
    }

    fn from_ini(ini: &Ini) -> Presets {
        let list = PRESET_KEYS
            .iter()
            .filter_map(|k| ini.section(k).map(|s| Preset::from_section(k, s)))
            .collect::<Vec<_>>();
        // A preset missing from the file would shift the list: use none.
        let list = if list.len() == PRESET_KEYS.len() {
            list
        } else {
            Vec::new()
        };
        Presets {
            list,
            recommended: ini
                .value("VideoSettings", "DefaultConfig")
                .map(str::to_string),
        }
    }
}

impl Preset {
    fn from_section(key: &str, s: &HashMap<String, String>) -> Preset {
        let get = |k: &str| {
            s.get(&k.to_ascii_lowercase())
                .map(String::as_str)
                .unwrap_or("")
        };
        let int = |k: &str| script_int(get(k));
        Preset {
            key: key.to_string(),
            windowed: (
                int("WindowedViewportX").max(0) as u32,
                int("WindowedViewportY").max(0) as u32,
            ),
            fullscreen: (
                int("FullscreenViewportX").max(0) as u32,
                int("FullscreenViewportY").max(0) as u32,
            ),
            textures: TEXTURE_KEYS.map(|k| get(k).trim().to_ascii_uppercase()),
            drop_lod_by: int("DropLODBy").clamp(0, 2) as u8,
            shadows: script_bool(get("Shadows")),
            bump: int("BumpmappingQuality").clamp(0, 2) as u8,
            blur: script_bool(get("BlurEnabled")),
            bloom: int("BloomQuality").clamp(0, 2) as u8,
            projectors: script_bool(get("Projectors")),
            fsaa: int("FSAA").max(0) as u32,
        }
    }
}

/// UnrealScript's `int(string)`: the leading number, 0 if there isn't one.
fn script_int(s: &str) -> i64 {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, s),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    sign * digits.parse::<i64>().unwrap_or(0)
}

/// UnrealScript's `bool(string)`: "True" (any case) or a non-zero number.
fn script_bool(s: &str) -> bool {
    let s = s.trim();
    s.eq_ignore_ascii_case("true") || script_int(s) != 0
}

/// An `.ini` file read as text (the original writes them as plain 8-bit
/// text; UTF-16 is read too in case a tool rewrote it).
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Sections of `key=value` lines, names compared without case. Lines
/// starting with `;` are comments.
#[derive(Debug, Default)]
struct Ini {
    sections: HashMap<String, HashMap<String, String>>,
}

impl Ini {
    fn parse(text: &str) -> Ini {
        let mut ini = Ini::default();
        let mut current: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                let name = name.trim().to_ascii_lowercase();
                ini.sections.entry(name.clone()).or_default();
                current = Some(name);
                continue;
            }
            let (Some(section), Some((k, v))) = (&current, line.split_once('=')) else {
                continue;
            };
            ini.sections
                .get_mut(section)
                .unwrap()
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        ini
    }

    fn overlay(&mut self, other: &Ini) {
        for (name, values) in &other.sections {
            let s = self.sections.entry(name.clone()).or_default();
            for (k, v) in values {
                s.insert(k.clone(), v.clone());
            }
        }
    }

    fn section(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.sections.get(&name.to_ascii_lowercase())
    }

    fn value(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }
}

static PRESETS: RwLock<Option<Presets>> = RwLock::new(None);

/// Keep the presets read at start for the options page to use.
pub fn set(p: Presets) {
    *PRESETS.write().unwrap() = Some(p);
}

/// The presets read at start (none if they haven't been read).
pub fn get() -> Presets {
    PRESETS.read().unwrap().clone().unwrap_or_default()
}

/// FSAA sample counts the graphics card can draw with (2, 4, 8), found at
/// start from the card, as the original's list is (`SupportsFSAALevel`).
static FSAA_LEVELS: RwLock<Vec<u32>> = RwLock::new(Vec::new());

pub fn set_fsaa_levels(levels: Vec<u32>) {
    *FSAA_LEVELS.write().unwrap() = levels;
}

pub fn fsaa_levels() -> Vec<u32> {
    FSAA_LEVELS.read().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: &str = "[VideoSettings]\nVideoCard=NotSet\nDefaultConfig=Generic_64MB\n\n\
        [GraphicsQualityLowest]\nWindowedViewportX=640\nTextureDetailWorld=Lower\nDropLODBy=2\nShadows=False\nBloomQuality=0\nProjectors=False\nFSAA=0\n\
        [GraphicsQualityLow]\nDropLODBy=1\nBlurEnabled=True\nBloomQuality=1\nProjectors=True\n\
        [GraphicsQualityMedium]\nDropLODBy=0\n\
        [GraphicsQualityHigh]\nDropLODBy=0\n\
        [GraphicsQualityHighest]\nTextureDetailWorld=UltraHigh\nShadows=True\nBloomQuality=2\nProjectors=True\n";

    #[test]
    fn reads_presets_and_the_players_own_copy_wins_where_it_isnt_commented_out() {
        let mut ini = Ini::parse(SHIPPED);
        ini.overlay(&Ini::parse(
            "[VideoSettings]\nDefaultConfig=GraphicsQualityHighest\n[GraphicsQualityHighest]\n;  BloomQuality=0\n",
        ));
        let p = Presets::from_ini(&ini);
        assert_eq!(p.list.len(), 5);
        assert_eq!(p.recommended().unwrap().key, "GraphicsQualityHighest");
        let top = &p.list[4];
        assert_eq!(top.bloom, 2, "a commented-out line keeps the shipped value");
        assert!(top.shadows && top.projectors);
        assert_eq!(top.textures[4], "ULTRAHIGH");
        assert_eq!(p.list[0].drop_lod_by, 2);
        assert_eq!(p.list[0].windowed.0, 640);
        // Missing values read as the script reads them: 0 and false.
        assert_eq!(p.list[2].bump, 0);
        assert!(!p.list[2].blur);
    }

    #[test]
    fn a_recommendation_with_no_section_is_none() {
        let p = Presets::from_ini(&Ini::parse(SHIPPED));
        assert_eq!(p.recommended.as_deref(), Some("Generic_64MB"));
        assert!(p.recommended().is_none());
    }

    #[test]
    fn script_conversions() {
        assert_eq!(script_int("1024"), 1024);
        assert_eq!(script_int(""), 0);
        assert!(script_bool("TRUE") && script_bool("true") && !script_bool("False"));
        assert!(!script_bool(""));
    }
}
