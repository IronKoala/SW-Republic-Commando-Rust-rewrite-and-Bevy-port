//! Colours and text sizes for the menus and display, from
//! `data/ui/theme.toml`.

use bevy::prelude::*;
use bevy::text::FontSize;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Resource, Debug, Clone)]
pub struct Theme {
    pub text: Color,
    pub text_dim: Color,
    pub highlight: Color,
    pub background: Color,
    pub panel: Color,
    pub shield: Color,
    pub health: Color,
    pub crosshair: Color,
    pub crosshair_on_target: Color,
    pub squad_ok: Color,
    pub squad_down: Color,
    pub warning: Color,
    pub title: f32,
    pub heading: f32,
    pub item: f32,
    pub small: f32,
    pub hud_number: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::parse(include_str!("../../../data/ui/theme.toml")).expect("built-in theme")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    colours: BTreeMap<String, String>,
    sizes: BTreeMap<String, f32>,
}

/// "#RRGGBB" or "#RRGGBBAA".
pub fn hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color::srgb_u8(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color::srgba_u8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

impl Theme {
    pub fn parse(text: &str) -> Result<Theme, String> {
        let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let c = |name: &str| -> Result<Color, String> {
            let v = f
                .colours
                .get(name)
                .ok_or(format!("[colours] {name} is missing"))?;
            hex(v).ok_or(format!(
                "[colours] {name}: `{v}` is not #RRGGBB or #RRGGBBAA"
            ))
        };
        let s = |name: &str| -> Result<f32, String> {
            f.sizes
                .get(name)
                .copied()
                .ok_or(format!("[sizes] {name} is missing"))
        };
        Ok(Theme {
            text: c("text")?,
            text_dim: c("text_dim")?,
            highlight: c("highlight")?,
            background: c("background")?,
            panel: c("panel")?,
            shield: c("shield")?,
            health: c("health")?,
            crosshair: c("crosshair")?,
            crosshair_on_target: c("crosshair_on_target")?,
            squad_ok: c("squad_ok")?,
            squad_down: c("squad_down")?,
            warning: c("warning")?,
            title: s("title")?,
            heading: s("heading")?,
            item: s("item")?,
            small: s("small")?,
            hud_number: s("hud_number")?,
        })
    }

    /// Text that scales with the screen height.
    pub fn font(&self, size: f32) -> TextFont {
        TextFont {
            font_size: FontSize::Vh(size),
            ..default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_theme_reads() {
        let t = Theme::default();
        assert_eq!(t.text, Color::srgb_u8(0xF2, 0xB5, 0x44));
        assert!(hex("#12345").is_none());
    }
}
