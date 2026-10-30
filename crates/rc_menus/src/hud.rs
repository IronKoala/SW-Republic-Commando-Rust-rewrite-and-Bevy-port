//! The in-game HUD built from the original's own pictures and fonts, as a
//! list of textured rectangles (the same `Quad`s the menus use), so the
//! game draws it with Bevy and `render.rs` can paint it without a GPU.
//!
//! The original draws its HUD in native code (`CTHUD`), so there is no
//! layout to read: which part of which picture goes where is our choice,
//! written in `data/ui/hud.toml` so it can be changed without building.
//!
//! **How it fits every screen.** Everything is measured in HUD units: a
//! 640 x 480 design area, like the original's menus. On the screen, one
//! unit is `min(height / 480, width / 640)` pixels, where `width` is the
//! HUD area (the screen, or a centred part of it no wider than
//! `max_aspect`). So the design area always fits inside the screen, and
//! each group is pinned to a corner or edge of the HUD area. On a wider
//! screen the corners move apart (Hor+). Groups that don't overlap in the
//! 640 x 480 design area can't overlap on any screen; `check_plan` tests
//! that, and the tests run it at many screen shapes. Phones held upright
//! are not supported (the game is played sideways).
//!
//! Text never runs into its neighbours: every text has a box (`width`
//! units wide) and is cut short with "..." when it would not fit.

use crate::library::FontData;
use crate::quads::{line_width, Quad, Sizes};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The design area, in HUD units.
pub const DESIGN_WIDTH: f32 = 640.0;
pub const DESIGN_HEIGHT: f32 = 480.0;

/// Render styles (the menus' numbers, see `layout.rs`).
const STY_ALPHA: u8 = 5;
const STY_ADDITIVE: u8 = 6;

/// `[layout]` and `[[element]]` in `data/ui/hud.toml`.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct HudLayout {
    #[serde(default)]
    pub layout: LayoutSettings,
    #[serde(default, rename = "element")]
    pub elements: Vec<Element>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayoutSettings {
    /// Use the original pictures for the HUD at all.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Make everything bigger or smaller. 1 = as designed. Kept within
    /// 0.5 to 1.0 of the fit, so a bigger HUD still fits the screen.
    #[serde(default = "one")]
    pub scale: f32,
    /// Fonts to pick from, smallest first. Each text uses the smallest
    /// one at least as tall as it is drawn, so letters stay sharp.
    #[serde(default)]
    pub fonts: Vec<String>,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        LayoutSettings {
            enabled: true,
            scale: 1.0,
            fonts: Vec::new(),
        }
    }
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// Every name an `[[element]]` may use.
const ELEMENT_KEYS: &[&str] = &[
    "group",
    "anchor",
    "at",
    "size",
    "when",
    "repeat_squad",
    "colour",
    "alert_colour",
    "alert_when",
    "blend",
    "picture",
    "part",
    "choose_by",
    "choices",
    "fill",
    "fill_toward",
    "mirror",
    "text",
    "align",
];

/// Which corner or edge of the HUD area an element is pinned to.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Centre,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Where the anchor is, as a fraction of the area (0 left/top, 1
    /// right/bottom).
    pub fn fraction(self) -> (f32, f32) {
        use Anchor::*;
        let x = match self {
            TopLeft | Left | BottomLeft => 0.0,
            Top | Centre | Bottom => 0.5,
            TopRight | Right | BottomRight => 1.0,
        };
        let y = match self {
            TopLeft | Top | TopRight => 0.0,
            Left | Centre | Right => 0.5,
            BottomLeft | Bottom | BottomRight => 1.0,
        };
        (x, y)
    }
}

/// One picture or text on the HUD.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Element {
    /// Elements of one group stay together; different groups must not
    /// overlap (checked by `check_plan`).
    pub group: String,
    #[serde(default)]
    pub anchor: Anchor,
    /// The element's top-left corner, in units from the anchor point
    /// (x to the right, y down). For a right or bottom anchor use
    /// negative numbers.
    pub at: [f32; 2],
    /// Width and height in units. For a text, the height is the line
    /// height and the width is the box it is cut to.
    pub size: [f32; 2],
    /// Show only when this is true (see `HudValues::flag`).
    #[serde(default)]
    pub when: Option<String>,
    /// For squad elements: repeat once per squad mate, moving this many
    /// units each time. `{n}` in values means the squad mate's place.
    #[serde(default)]
    pub repeat_squad: Option<[f32; 2]>,
    /// Tint, red green blue alpha, 0..255.
    #[serde(default = "white")]
    pub colour: [u8; 4],
    /// The colour used when `alert_when` is true (a squad mate is down,
    /// health is low).
    #[serde(default)]
    pub alert_colour: Option<[u8; 4]>,
    #[serde(default)]
    pub alert_when: Option<String>,
    /// "alpha" (default for pictures) or "add" (glowing; default for text).
    #[serde(default)]
    pub blend: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

fn white() -> [u8; 4] {
    [255, 255, 255, 255]
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Kind {
    Picture(Picture),
    Text(TextPart),
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Picture {
    /// Full object path, like `HUDTextures.Icons.CloneIcons`.
    pub picture: String,
    /// The part of the picture, in texels: x0, y0, x1, y1. Whole picture
    /// if left out.
    #[serde(default)]
    pub part: Option<[f32; 4]>,
    /// Other parts to use, by the value of `choose_by` (for example one
    /// grenade picture per grenade name). Falls back to `part`.
    #[serde(default)]
    pub choose_by: Option<String>,
    #[serde(default)]
    pub choices: BTreeMap<String, [f32; 4]>,
    /// Show only this fraction (0..1) of the picture: a bar that empties.
    #[serde(default)]
    pub fill: Option<String>,
    /// Which way a bar empties toward: "left" (default), "right", "down"
    /// or "up".
    #[serde(default)]
    pub fill_toward: Option<String>,
    /// Flip the picture left to right (to reuse one side's art).
    #[serde(default)]
    pub mirror: bool,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct TextPart {
    /// What to show; see `HudValues::text`. Plain words are shown as they
    /// are.
    pub text: String,
    /// "left" (default), "centre" or "right" within the box.
    #[serde(default)]
    pub align: Option<String>,
}

/// One squad mate as the HUD shows them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SquadLine {
    pub number: String,
    pub name: String,
    pub health: f32,
    pub down: bool,
    pub order: String,
}

/// Everything the HUD shows, already worked out by the game.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HudValues {
    /// 0..1.
    pub shield: f32,
    pub health: f32,
    pub weapon: String,
    pub clip: Option<u32>,
    pub clip_size: Option<u32>,
    pub reserve: Option<u32>,
    pub grenade: Option<(String, u32)>,
    pub squad: Vec<SquadLine>,
    /// Short messages, newest last.
    pub messages: Vec<String>,
}

impl HudValues {
    /// A 0..1 value by name, `None` if unknown.
    pub fn fraction(&self, name: &str, member: Option<&SquadLine>) -> Option<f32> {
        Some(
            match name {
                "shield" => self.shield,
                "health" => self.health,
                "clip" => match (self.clip, self.clip_size) {
                    (Some(c), Some(s)) if s > 0 => c as f32 / s as f32,
                    _ => 0.0,
                },
                "squad_health" => member?.health,
                _ => return None,
            }
            .clamp(0.0, 1.0),
        )
    }

    /// A yes/no by name, for `when` and `alert_when`.
    pub fn flag(&self, name: &str, member: Option<&SquadLine>) -> Option<bool> {
        let (not, name) = match name.strip_prefix('!') {
            Some(n) => (true, n),
            None => (false, name),
        };
        let v = match name {
            "weapon" => !self.weapon.is_empty(),
            "clip" => !self.weapon.is_empty() && self.clip_size.is_some(),
            "reserve" => !self.weapon.is_empty() && self.reserve.is_some(),
            "grenade" => self.grenade.is_some(),
            "shield_low" => self.shield < 0.25,
            "health_low" => self.health < 0.25,
            "squad_down" => member?.down,
            _ => return None,
        };
        Some(v != not)
    }

    /// A text by name. `{name}` parts are filled in; the rest is shown as
    /// it is.
    pub fn text(&self, pattern: &str, member: Option<&SquadLine>) -> String {
        let mut out = String::new();
        let mut rest = pattern;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let key = &rest[start + 1..start + end];
            out.push_str(&self.value(key, member).unwrap_or_default());
            rest = &rest[start + end + 1..];
        }
        out.push_str(rest);
        out
    }

    fn value(&self, key: &str, member: Option<&SquadLine>) -> Option<String> {
        Some(match key {
            "weapon" => self.weapon.to_uppercase(),
            "clip" => self.clip.map_or("--".into(), |c| c.to_string()),
            "reserve" => self.reserve.map_or("--".into(), |r| r.to_string()),
            "grenade" => self.grenade.as_ref()?.0.to_uppercase(),
            "grenades" => self.grenade.as_ref()?.1.to_string(),
            "number" => member?.number.clone(),
            "name" => member?.name.to_uppercase(),
            "order" => {
                let m = member?;
                if m.down {
                    "DOWN".into()
                } else {
                    m.order.to_uppercase()
                }
            }
            k => {
                // message1 = newest, message2 = the one before...
                let n: usize = k.strip_prefix("message")?.parse().ok()?;
                let i = self.messages.len().checked_sub(n)?;
                self.messages[i].clone()
            }
        })
    }
}

impl HudLayout {
    pub fn parse(text: &str) -> Result<HudLayout, String> {
        // serde can't refuse unknown names in a flattened struct, so a
        // misspelt one is caught here.
        let mut raw: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        // `[crosshair]` in the same file is read by the game.
        raw.remove("crosshair");
        for (i, e) in raw
            .get("element")
            .and_then(|e| e.as_array())
            .into_iter()
            .flatten()
            .enumerate()
        {
            for key in e.as_table().into_iter().flat_map(|t| t.keys()) {
                if !ELEMENT_KEYS.contains(&key.as_str()) {
                    return Err(format!("element {}: unknown name `{key}`", i + 1));
                }
            }
            let kinds = ["picture", "text"]
                .iter()
                .filter(|k| e.get(**k).is_some())
                .count();
            if kinds != 1 {
                return Err(format!(
                    "element {}: needs exactly one of `picture` or `text`",
                    i + 1
                ));
            }
        }
        let l: HudLayout = toml::Value::Table(raw)
            .try_into()
            .map_err(|e: toml::de::Error| e.to_string())?;
        l.check()?;
        Ok(l)
    }

    /// Mistakes a modder would want to hear about at once.
    pub fn check(&self) -> Result<(), String> {
        let known_fill = ["shield", "health", "clip", "squad_health"];
        for e in &self.elements {
            if e.size[0] <= 0.0 || e.size[1] <= 0.0 {
                return Err(format!("{}: size must be above 0", e.group));
            }
            if let Kind::Picture(p) = &e.kind {
                if let Some(f) = &p.fill {
                    if !known_fill.contains(&f.as_str()) {
                        return Err(format!(
                            "{}: fill \"{f}\" is not one of {known_fill:?}",
                            e.group
                        ));
                    }
                }
            }
            let probe = HudValues::default();
            let member = SquadLine::default();
            for w in [&e.when, &e.alert_when].into_iter().flatten() {
                if probe.flag(w, Some(&member)).is_none() {
                    return Err(format!("{}: unknown condition \"{w}\"", e.group));
                }
            }
        }
        Ok(())
    }
}

/// Where the HUD goes on a `width` x `height` screen: the HUD area (x0,
/// y0, x1, y1 in pixels) and the size of one unit in pixels.
pub fn frame(width: f32, height: f32, max_aspect: f32, scale: f32) -> ([f32; 4], f32) {
    let area_w = if max_aspect > 0.0 {
        width.min(height * max_aspect)
    } else {
        width
    };
    let x0 = (width - area_w) / 2.0;
    let fit = (height / DESIGN_HEIGHT).min(area_w / DESIGN_WIDTH);
    let unit = fit * scale.clamp(0.5, 1.0);
    ([x0, 0.0, x0 + area_w, height], unit)
}

/// One drawn element, for checks and logs: its group and its box in
/// pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub group: String,
    pub rect: [f32; 4],
}

/// Where an element's box is in pixels.
fn place(e: &Element, offset: [f32; 2], area: [f32; 4], unit: f32) -> [f32; 4] {
    let (fx, fy) = e.anchor.fraction();
    let ax = area[0] + (area[2] - area[0]) * fx;
    let ay = area[1] + (area[3] - area[1]) * fy;
    let x0 = ax + (e.at[0] + offset[0]) * unit;
    let y0 = ay + (e.at[1] + offset[1]) * unit;
    [x0, y0, x0 + e.size[0] * unit, y0 + e.size[1] * unit]
}

/// The boxes of every element (all squad places, everything shown), for
/// the overlap check.
pub fn boxes(
    layout: &HudLayout,
    width: f32,
    height: f32,
    max_aspect: f32,
    squad: usize,
) -> Vec<Placed> {
    let (area, unit) = frame(width, height, max_aspect, layout.layout.scale);
    let mut out = Vec::new();
    for e in &layout.elements {
        let count = if e.repeat_squad.is_some() { squad } else { 1 };
        for i in 0..count {
            let step = e.repeat_squad.unwrap_or([0.0, 0.0]);
            let off = [step[0] * i as f32, step[1] * i as f32];
            out.push(Placed {
                group: e.group.clone(),
                rect: place(e, off, area, unit),
            });
        }
    }
    out
}

/// Problems with a layout at one screen size: two groups that overlap, or
/// an element off the screen.
pub fn check_plan(
    layout: &HudLayout,
    width: f32,
    height: f32,
    max_aspect: f32,
    squad: usize,
) -> Vec<String> {
    let placed = boxes(layout, width, height, max_aspect, squad);
    let mut groups: BTreeMap<&str, [f32; 4]> = BTreeMap::new();
    let mut out = Vec::new();
    for p in &placed {
        let r = p.rect;
        if r[0] < -0.5 || r[1] < -0.5 || r[2] > width + 0.5 || r[3] > height + 0.5 {
            out.push(format!(
                "{} goes off the {width}x{height} screen: {r:?}",
                p.group
            ));
        }
        groups
            .entry(&p.group)
            .and_modify(|g| {
                g[0] = g[0].min(r[0]);
                g[1] = g[1].min(r[1]);
                g[2] = g[2].max(r[2]);
                g[3] = g[3].max(r[3]);
            })
            .or_insert(r);
    }
    let list: Vec<_> = groups.into_iter().collect();
    for (i, (a, ra)) in list.iter().enumerate() {
        for (b, rb) in &list[i + 1..] {
            if ra[0] < rb[2] && rb[0] < ra[2] && ra[1] < rb[3] && rb[1] < ra[3] {
                out.push(format!("{a} overlaps {b} at {width}x{height}"));
            }
        }
    }
    out
}

/// The quads to draw for `values` on a `width` x `height` screen, in
/// screen fractions (0..1), in drawing order.
pub fn hud_quads(
    layout: &HudLayout,
    values: &HudValues,
    width: f32,
    height: f32,
    max_aspect: f32,
    sizes: &mut dyn Sizes,
) -> Vec<Quad> {
    let (area, unit) = frame(width, height, max_aspect, layout.layout.scale);
    let fonts: Vec<FontData> = layout
        .layout
        .fonts
        .iter()
        .filter_map(|f| sizes.font(f))
        .collect();
    let mut out = Vec::new();
    for e in &layout.elements {
        let members: Vec<Option<&SquadLine>> = if e.repeat_squad.is_some() {
            values.squad.iter().map(Some).collect()
        } else {
            vec![None]
        };
        for (i, member) in members.into_iter().enumerate() {
            if let Some(w) = &e.when {
                if values.flag(w, member) != Some(true) {
                    continue;
                }
            }
            let step = e.repeat_squad.unwrap_or([0.0, 0.0]);
            let rect = place(e, [step[0] * i as f32, step[1] * i as f32], area, unit);
            let alert = e
                .alert_when
                .as_deref()
                .is_some_and(|w| values.flag(w, member) == Some(true));
            let colour = match (alert, e.alert_colour) {
                (true, Some(c)) => c,
                _ => e.colour,
            };
            let to_screen =
                |r: [f32; 4]| [r[0] / width, r[1] / height, r[2] / width, r[3] / height];
            match &e.kind {
                Kind::Picture(p) => {
                    let Some((tw, th)) = sizes.texture_size(&p.picture) else {
                        continue;
                    };
                    let mut src = p.part.unwrap_or([0.0, 0.0, tw, th]);
                    if let Some(key) = &p.choose_by {
                        let k = values.text(key, member);
                        if let Some(c) = p.choices.get(&k) {
                            src = *c;
                        }
                    }
                    let mut dst = rect;
                    if let Some(f) = &p.fill {
                        let v = values.fraction(f, member).unwrap_or(1.0);
                        if v <= 0.0 {
                            continue;
                        }
                        cut(&mut src, &mut dst, v, p.fill_toward.as_deref(), p.mirror);
                    }
                    if p.mirror {
                        src.swap(0, 2);
                    }
                    out.push(Quad {
                        texture: p.picture.clone(),
                        src,
                        dst: to_screen(dst),
                        color: colour,
                        render_style: style(e.blend.as_deref(), STY_ALPHA),
                        widget: e.group.clone(),
                    });
                }
                Kind::Text(t) => {
                    let text = values.text(&t.text, member);
                    let line_px = rect[3] - rect[1];
                    let Some(font) = pick_font(&fonts, line_px) else {
                        continue;
                    };
                    text_line(
                        &text,
                        font,
                        rect,
                        t.align.as_deref(),
                        colour,
                        style(e.blend.as_deref(), STY_ADDITIVE),
                        &e.group,
                        width,
                        height,
                        &mut out,
                    );
                }
            }
        }
    }
    out
}

fn style(blend: Option<&str>, default: u8) -> u8 {
    match blend {
        Some("add") => STY_ADDITIVE,
        Some("alpha") => STY_ALPHA,
        _ => default,
    }
}

/// Shorten a bar to `v` of its length, emptying toward `toward` on the
/// screen. A mirrored picture is flipped after this, so its texels are
/// cut from the other end.
fn cut(src: &mut [f32; 4], dst: &mut [f32; 4], v: f32, toward: Option<&str>, mirror: bool) {
    let toward = toward.unwrap_or("left");
    let src_toward = match (toward, mirror) {
        ("left", true) => "right",
        ("right", true) => "left",
        (t, _) => t,
    };
    shorten(src, v, src_toward);
    shorten(dst, v, toward);
}

fn shorten(r: &mut [f32; 4], v: f32, toward: &str) {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    match toward {
        "right" => r[0] = r[2] - w * v,
        "down" => r[1] = r[3] - h * v,
        "up" => r[3] = r[1] + h * v,
        _ => r[2] = r[0] + w * v,
    }
}

/// The smallest font at least `px` tall, else the tallest.
fn pick_font(fonts: &[FontData], px: f32) -> Option<&FontData> {
    fonts
        .iter()
        .filter(|f| f.line_height as f32 >= px)
        .min_by_key(|f| f.line_height)
        .or_else(|| fonts.iter().max_by_key(|f| f.line_height))
}

/// One line of text in the box `rect` (pixels), cut with "..." to fit.
#[allow(clippy::too_many_arguments)]
fn text_line(
    text: &str,
    font: &FontData,
    rect: [f32; 4],
    align: Option<&str>,
    colour: [u8; 4],
    render_style: u8,
    group: &str,
    width: f32,
    height: f32,
    out: &mut Vec<Quad>,
) {
    if text.is_empty() {
        return;
    }
    let k = (rect[3] - rect[1]) / font.line_height.max(1) as f32;
    let max = (rect[2] - rect[0]) / k;
    let mut s = text.to_string();
    if line_width(font, &s, 0) > max {
        while !s.is_empty() && line_width(font, &format!("{s}..."), 0) > max {
            s.pop();
        }
        s = format!("{}...", s.trim_end());
    }
    let w = line_width(font, &s, 0) * k;
    let mut x = match align {
        Some("right") => rect[2] - w,
        Some("centre") | Some("center") => (rect[0] + rect[2] - w) / 2.0,
        _ => rect[0],
    };
    for c in s.chars() {
        let Some(g) = font.glyph(c) else {
            continue;
        };
        if let Some(page) = font.pages.get(g.page as usize) {
            let (gw, gh) = (g.width as f32 * k, g.height as f32 * k);
            out.push(Quad {
                texture: page.clone(),
                src: [
                    g.u as f32,
                    g.v as f32,
                    (g.u + g.width) as f32,
                    (g.v + g.height) as f32,
                ],
                dst: [
                    x / width,
                    rect[1] / height,
                    (x + gw) / width,
                    (rect[1] + gh) / height,
                ],
                color: colour,
                render_style,
                widget: group.to_string(),
            });
        }
        x += (g.width + font.kerning) as f32 * k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: &str = include_str!("../../../data/ui/hud.toml");

    fn shipped() -> HudLayout {
        HudLayout::parse(SHIPPED).unwrap()
    }

    /// Screen shapes from 4:3 to 32:9, phones and flip phones on their
    /// side. Phones held upright are not supported (the game is played
    /// sideways).
    const SCREENS: &[(f32, f32)] = &[
        (640.0, 480.0),
        (800.0, 600.0),
        (1024.0, 768.0),
        (1280.0, 1024.0),
        (1280.0, 720.0),
        (1366.0, 768.0),
        (1920.0, 1080.0),
        (1920.0, 1200.0),
        (2560.0, 1080.0),
        (3440.0, 1440.0),
        (3840.0, 1080.0),
        (5120.0, 1440.0),
        (3840.0, 2160.0),
        (1280.0, 800.0),  // Steam Deck
        (2400.0, 1080.0), // phone on its side
        (2640.0, 1080.0), // tall flip phone, open, on its side
        (2208.0, 1768.0), // foldable, open
        (480.0, 320.0),
    ];

    #[test]
    fn shipped_layout_reads() {
        let l = shipped();
        assert!(l.layout.enabled);
        assert!(!l.elements.is_empty());
        assert!(!l.layout.fonts.is_empty());
    }

    #[test]
    fn nothing_overlaps_at_any_screen_shape() {
        let l = shipped();
        for &(w, h) in SCREENS {
            for max_aspect in [16.0 / 9.0, 0.0, 21.0 / 9.0] {
                let problems = check_plan(&l, w, h, max_aspect, 3);
                assert!(problems.is_empty(), "{w}x{h}: {problems:?}");
            }
        }
    }

    #[test]
    fn the_unit_fits_the_design_area_in_the_screen() {
        for &(w, h) in SCREENS {
            let (area, unit) = frame(w, h, 16.0 / 9.0, 1.0);
            assert!(DESIGN_WIDTH * unit <= area[2] - area[0] + 0.01);
            assert!(DESIGN_HEIGHT * unit <= h + 0.01);
            // Never wider than 16:9, centred.
            assert!((area[2] - area[0]) <= h * 16.0 / 9.0 + 0.01);
            assert!((area[0] - (w - area[2])).abs() < 0.01);
        }
    }

    #[test]
    fn overlapping_groups_are_reported() {
        let mut l = shipped();
        let mut e = l.elements[0].clone();
        e.group = "intruder".into();
        l.elements.push(e);
        assert!(!check_plan(&l, 1920.0, 1080.0, 16.0 / 9.0, 3).is_empty());
    }

    #[test]
    fn values_fill_texts_and_flags() {
        let v = HudValues {
            weapon: "DC-17m Blaster".into(),
            clip: Some(30),
            clip_size: Some(60),
            reserve: None,
            grenade: Some(("Thermal Detonator".into(), 2)),
            messages: vec!["old".into(), "new".into()],
            ..Default::default()
        };
        let m = SquadLine {
            number: "07".into(),
            name: "Sev".into(),
            health: 0.5,
            down: true,
            order: "Form up".into(),
        };
        assert_eq!(v.text("{clip}", None), "30");
        assert_eq!(v.text("{reserve}", None), "--");
        assert_eq!(v.text("{number} {name}", Some(&m)), "07 SEV");
        assert_eq!(v.text("{order}", Some(&m)), "DOWN");
        assert_eq!(v.text("{grenades}", None), "2");
        assert_eq!(v.text("{message1}", None), "new");
        assert_eq!(v.text("{message3}", None), "");
        assert_eq!(v.fraction("clip", None), Some(0.5));
        assert_eq!(v.flag("reserve", None), Some(false));
        assert_eq!(v.flag("!reserve", None), Some(true));
        assert_eq!(v.flag("squad_down", Some(&m)), Some(true));
        assert_eq!(v.flag("nonsense", None), None);
    }

    #[test]
    fn a_bar_shortens_toward_its_end() {
        let (mut s, mut d) = ([0.0, 0.0, 100.0, 10.0], [10.0, 0.0, 210.0, 20.0]);
        cut(&mut s, &mut d, 0.25, None, false);
        assert_eq!((s, d), ([0.0, 0.0, 25.0, 10.0], [10.0, 0.0, 60.0, 20.0]));
        let (mut s, mut d) = ([0.0, 0.0, 100.0, 10.0], [0.0, 0.0, 100.0, 100.0]);
        cut(&mut s, &mut d, 0.5, Some("up"), false);
        assert_eq!(d, [0.0, 0.0, 100.0, 50.0]);
    }

    #[test]
    fn unknown_conditions_are_refused() {
        let text = r#"
[[element]]
group = "x"
at = [0, 0]
size = [10, 10]
when = "sometimes"
text = "hi"
"#;
        assert!(HudLayout::parse(text).unwrap_err().contains("sometimes"));
        let typo = text.replace("when = \"sometimes\"", "colur = [1, 2, 3, 4]");
        assert!(HudLayout::parse(&typo).unwrap_err().contains("colur"));
    }
}
