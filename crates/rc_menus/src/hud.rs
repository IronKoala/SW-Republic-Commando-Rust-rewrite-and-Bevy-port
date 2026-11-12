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
    /// `[pickup_message]`: how long the pickup message stays.
    #[serde(default)]
    pub pickup_message: PickupTiming,
}

/// How long the "just picked up" message stays and fades.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PickupTiming {
    /// Seconds on screen, fading included (game: LocalMessage LifeTime 3).
    #[serde(default = "three")]
    pub seconds: f32,
    /// Seconds at the end over which it fades out. **Our choice.**
    #[serde(default = "one")]
    pub fade: f32,
}

impl Default for PickupTiming {
    fn default() -> Self {
        PickupTiming {
            seconds: 3.0,
            fade: 1.0,
        }
    }
}

impl PickupTiming {
    /// How much of the message shows, `left` seconds before it ends:
    /// 1 until the fade starts, then down to 0. Stepped in tenths so the
    /// HUD is redrawn only ten times while it fades.
    pub fn strength(&self, left: f32) -> f32 {
        if left <= 0.0 {
            return 0.0;
        }
        if self.fade <= 0.0 || left >= self.fade {
            return 1.0;
        }
        ((left / self.fade) * 10.0).ceil() / 10.0
    }
}

fn three() -> f32 {
    3.0
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
    /// Seconds the advisor's words and the objective take to fade in
    /// (the original's `Helmet.PromptFadeInTime`, 0.5 in engine.u).
    #[serde(default = "half")]
    pub text_fade_in: f32,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        LayoutSettings {
            enabled: true,
            scale: 1.0,
            fonts: Vec::new(),
            text_fade_in: 0.5,
        }
    }
}

impl LayoutSettings {
    /// How far a text shown for `age` seconds has faded in, 0..1, in
    /// tenths (so the HUD is redrawn only ten times while it fades).
    pub fn faded_in(&self, age: f32) -> f32 {
        if self.text_fade_in <= 0.0 {
            return 1.0;
        }
        ((age / self.text_fade_in).clamp(0.0, 1.0) * 10.0)
            .floor()
            .max(1.0)
            / 10.0
    }

    /// How far the use prompt shown for `age` seconds has faded in, 0..1,
    /// exactly as the original works it out (Engine.dll
    /// `UHelmet::DrawActivateText`): its opacity climbs in a straight line
    /// from 0 to 220 over `PromptFadeInTime` (our `text_fade_in`), cut
    /// down to a whole number each frame. Given as that opacity over 220,
    /// so the element's own 220 times this is the original's number.
    pub fn prompt_faded_in(&self, age: f32) -> f32 {
        if self.text_fade_in <= 0.0 || age >= self.text_fade_in {
            return 1.0;
        }
        (age.max(0.0) / self.text_fade_in * 220.0).floor() / 220.0
    }
}

fn half() -> f32 {
    0.5
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// The group drawn behind and round everything else (the visor frame); it
/// may overlap other groups.
pub const BACKGROUND_GROUP: &str = "visor";

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
    "fill_steps",
    "fill_least_gap",
    "fill_stretch",
    "fill_inset",
    "slide_with",
    "slide",
    "grow_with",
    "grow",
    "redden_with",
    "colour_by",
    "colours",
    "scroll",
    "mirror",
    "text",
    "align",
    "fade",
    "font",
    "wrap",
    "glass",
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
    /// Other colours by the value of a text (see `HudValues::text`), for
    /// example `colour_by = "{health_level}"` with `colours = { "3" =
    /// [72, 255, 72, 255] }`. Falls back to `colour`; `alert_colour` wins.
    #[serde(default)]
    pub colour_by: Option<String>,
    #[serde(default)]
    pub colours: BTreeMap<String, [u8; 4]>,
    /// "alpha" (default for pictures) or "add" (glowing; default for text).
    #[serde(default)]
    pub blend: Option<String>,
    /// A 0..1 value (see `HudValues::fraction`) the element fades with:
    /// 1 fully shown, 0 gone. For example `fade = "pickup"` makes the
    /// pickup message fade out as it ends.
    #[serde(default)]
    pub fade: Option<String>,
    /// Shown through the helmet's glass, as the original draws its HUD
    /// (see `helmet::GlassMap`): moved where the glass shows that part
    /// of the HUD picture. Only while the helmet is drawn.
    #[serde(default)]
    pub glass: bool,
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
    /// Empty the bar in this many steps instead of smoothly (the
    /// original's meters drop in elevenths).
    #[serde(default)]
    pub fill_steps: Option<u32>,
    /// While the value is below 1, hide at least this fraction of the
    /// bar, so any hurt shows.
    #[serde(default)]
    pub fill_least_gap: f32,
    /// Show the whole `part` squeezed into the shorter bar, instead of
    /// cutting the part down with it (the original's shield bars stretch
    /// a few texels across whatever width is left).
    #[serde(default)]
    pub fill_stretch: bool,
    /// Units taken off the bar's fixed end (the end it empties toward)
    /// while it shows at all: the original's shield fill is 4 shorter
    /// than its share.
    #[serde(default)]
    pub fill_inset: f32,
    /// Move the element by `slide` units (x, y) times this 0..1 value
    /// (see `HudValues::fraction`): the end cap riding on a bar's edge.
    #[serde(default)]
    pub slide_with: Option<String>,
    #[serde(default)]
    pub slide: [f32; 2],
    /// Move the top and bottom edges by `grow` units (top, bottom) times
    /// this count (see `HudValues::steps`): the original's shield bars
    /// swell for a moment when hit.
    #[serde(default)]
    pub grow_with: Option<String>,
    #[serde(default)]
    pub grow: [f32; 2],
    /// Take this amount (see `HudValues::redden`) off the colour's green
    /// and blue: the original's shield parts flash red when hit.
    #[serde(default)]
    pub redden_with: Option<String>,
    /// Flip the picture left to right (to reuse one side's art).
    #[serde(default)]
    pub mirror: bool,
    /// Slide the picture sideways by this value (see `HudValues::scroll`),
    /// in picture widths, repeating it: the original's compass strip. The
    /// `part` may then be wider than the picture.
    #[serde(default)]
    pub scroll: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct TextPart {
    /// What to show; see `HudValues::text`. Plain words are shown as they
    /// are.
    pub text: String,
    /// "left" (default), "centre" or "right" within the box.
    #[serde(default)]
    pub align: Option<String>,
    /// Always this font (one of `[layout] fonts`, like
    /// `OrbitFonts.OrbitBold15`), scaled to the line height, instead of
    /// the nearest size.
    #[serde(default)]
    pub font: Option<String>,
    /// Break long text onto more lines, at spaces, to fit the box's
    /// width; each line is `size`'s height further down. Without it a
    /// long text is cut short with "...".
    #[serde(default)]
    pub wrap: bool,
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
    /// What was just picked up ("DC-17m Blaster Clips"), empty when
    /// nothing is shown.
    pub pickup: String,
    /// How much of the pickup message is still shown, 0..1 (it fades out).
    pub pickup_fade: f32,
    /// The use prompt ("PRESS AND HOLD 'F' TO HEAL"), as lines; empty
    /// when none is shown.
    pub prompt: Vec<String>,
    /// How far the prompt has faded in, 0..1.
    pub prompt_fade: f32,
    /// What the person speaking says (the advisor's words), empty when
    /// nobody speaks.
    pub subtitle: String,
    /// How far it has faded in, 0..1.
    pub subtitle_fade: f32,
    /// The current mission objective, empty when none is shown.
    pub objective: String,
    /// How far it has faded in, 0..1.
    pub objective_fade: f32,
    /// The compass strip's slide, in picture widths (the original's
    /// `UHelmet::DrawHUD`: the view's yaw, 65536 a turn, modulo 65535,
    /// times 0.0001, followed at 3 a second).
    pub compass: f32,
    /// The original's shield hit flash (Engine.dll `UHelmet::DrawHUD`):
    /// `int(ShieldFlashTime * 6)`, how far the shield bars swell, 0..3.
    pub shield_hit_steps: u32,
    /// How much green and blue the shield parts lose while flashing:
    /// `int(ShieldFlashTime / ShieldFlashDuration * 155)` while the shields
    /// aren't charging, else 0.
    pub shield_hit_red: u8,
}

/// How hurt, as the original sorts it (Engine.dll
/// `APawn::CalcHealthLevel`, health over the most health): 0 dead, 1 below
/// half, 2 below 0.91, 3 above.
pub fn health_level(health: f32) -> u8 {
    if health <= 0.0 {
        0
    } else if health < 0.5 {
        1
    } else if health < 0.91 {
        2
    } else {
        3
    }
}

/// A part `src` (texels, maybe wider than the picture) slid right by
/// `shift` texels, cut where the picture repeats (every `tw` texels), each
/// piece with its share of `dst`.
fn tiles(src: [f32; 4], dst: [f32; 4], shift: f32, tw: f32) -> Vec<([f32; 4], [f32; 4])> {
    let (u0, u1) = (src[0] + shift, src[2] + shift);
    if tw <= 0.0 || u1 <= u0 {
        return vec![(src, dst)];
    }
    let mut out = Vec::new();
    let mut u = u0;
    while u < u1 - 1e-3 && out.len() < 64 {
        let tile = (u / tw).floor();
        let end = ((tile + 1.0) * tw).min(u1);
        let x = |u: f32| dst[0] + (dst[2] - dst[0]) * (u - u0) / (u1 - u0);
        out.push((
            [u - tile * tw, src[1], end - tile * tw, src[3]],
            [x(u), dst[1], x(end), dst[3]],
        ));
        u = end;
    }
    out
}

/// `v` (0..1) as a bar that drops in `steps` steps and, while below 1,
/// hides at least `least_gap` of itself.
fn stepped(v: f32, steps: Option<u32>, least_gap: f32) -> f32 {
    let mut gap = 1.0 - v;
    if let Some(n) = steps.filter(|n| *n > 0) {
        gap = (gap * n as f32).floor() / n as f32;
    }
    if v < 1.0 && gap < least_gap {
        gap = least_gap;
    }
    (1.0 - gap).clamp(0.0, 1.0)
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
                "pickup" => self.pickup_fade,
                "prompt" => self.prompt_fade,
                "subtitle" => self.subtitle_fade,
                "objective" => self.objective_fade,
                _ => return None,
            }
            .clamp(0.0, 1.0),
        )
    }

    /// A sliding amount by name, for `scroll` (not limited to 0..1).
    pub fn scroll(&self, name: &str) -> Option<f32> {
        match name {
            "compass" => Some(self.compass),
            _ => None,
        }
    }

    /// A whole count by name (for `grow_with`), `None` if unknown.
    pub fn steps(&self, name: &str) -> Option<u32> {
        match name {
            "shield_hit" => Some(self.shield_hit_steps),
            _ => None,
        }
    }

    /// How much green and blue to take off by name (for `redden_with`),
    /// `None` if unknown.
    pub fn redden(&self, name: &str) -> Option<u8> {
        match name {
            "shield_hit" => Some(self.shield_hit_red),
            _ => None,
        }
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
            "grenades_left" => self.grenade.as_ref().is_some_and(|g| g.1 > 0),
            "shield" => self.shield > 0.0,
            "shield_low" => self.shield < 0.25,
            "health_low" => self.health < 0.25,
            "health_full" => self.health >= 0.999,
            "squad_down" => member?.down,
            "pickup" => !self.pickup.is_empty(),
            "prompt" => !self.prompt.is_empty(),
            "subtitle" => !self.subtitle.is_empty(),
            "objective" => !self.objective.is_empty(),
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
            "health_level" => health_level(self.health).to_string(),
            "squad_health_level" => {
                let m = member?;
                health_level(if m.down { 0.0 } else { m.health }).to_string()
            }
            "pickup" => self.pickup.clone(),
            "subtitle" => self.subtitle.clone(),
            "objective" => self.objective.clone(),
            "prompt1" => self.prompt.first().cloned().unwrap_or_default(),
            "prompt2" => self.prompt.get(1).cloned().unwrap_or_default(),
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
        // `[crosshair]` and `[use_panel]` in the same file are read by
        // the game.
        raw.remove("crosshair");
        raw.remove("use_panel");
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
        if !(self.pickup_message.seconds > 0.0) || self.pickup_message.fade < 0.0 {
            return Err("[pickup_message]: `seconds` must be above 0 and `fade` 0 or more".into());
        }
        let known_fill = ["shield", "health", "clip", "squad_health"];
        for e in &self.elements {
            if e.size[0] <= 0.0 || e.size[1] <= 0.0 {
                return Err(format!("{}: size must be above 0", e.group));
            }
            if let Kind::Picture(p) = &e.kind {
                if let Some(f) = &p.scroll {
                    if HudValues::default().scroll(f).is_none() {
                        return Err(format!("{}: scroll \"{f}\" is not \"compass\"", e.group));
                    }
                }
                if let Some(f) = &p.slide_with {
                    if !known_fill.contains(&f.as_str()) {
                        return Err(format!(
                            "{}: slide_with \"{f}\" is not one of {known_fill:?}",
                            e.group
                        ));
                    }
                }
                if let Some(f) = &p.grow_with {
                    if HudValues::default().steps(f).is_none() {
                        return Err(format!(
                            "{}: grow_with \"{f}\" is not \"shield_hit\"",
                            e.group
                        ));
                    }
                }
                if let Some(f) = &p.redden_with {
                    if HudValues::default().redden(f).is_none() {
                        return Err(format!(
                            "{}: redden_with \"{f}\" is not \"shield_hit\"",
                            e.group
                        ));
                    }
                }
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
        // The visor frame is round everything else.
        let background = p.group == BACKGROUND_GROUP;
        let r = p.rect;
        if r[0] < -0.5 || r[1] < -0.5 || r[2] > width + 0.5 || r[3] > height + 0.5 {
            out.push(format!(
                "{} goes off the {width}x{height} screen: {r:?}",
                p.group
            ));
        }
        if background {
            continue;
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
/// screen fractions (0..1), in drawing order. `glass` moves a point of
/// the HUD (screen fractions) to where the helmet's glass shows it, for
/// elements with `glass = true`; pass `None` without a helmet.
pub fn hud_quads(
    layout: &HudLayout,
    values: &HudValues,
    width: f32,
    height: f32,
    max_aspect: f32,
    sizes: &mut dyn Sizes,
    glass: Option<&dyn Fn([f32; 2]) -> [f32; 2]>,
) -> Vec<Quad> {
    hud_quads_of(
        layout,
        values,
        width,
        height,
        max_aspect,
        sizes,
        glass,
        &|_| true,
    )
}

/// [`hud_quads`] for only the elements `pick` chooses (the game draws the
/// `glass` ones onto the helmet's glass and the rest onto the screen).
#[allow(clippy::too_many_arguments)]
pub fn hud_quads_of(
    layout: &HudLayout,
    values: &HudValues,
    width: f32,
    height: f32,
    max_aspect: f32,
    sizes: &mut dyn Sizes,
    glass: Option<&dyn Fn([f32; 2]) -> [f32; 2]>,
    pick: &dyn Fn(&Element) -> bool,
) -> Vec<Quad> {
    let (area, unit) = frame(width, height, max_aspect, layout.layout.scale);
    let fonts: Vec<FontData> = layout
        .layout
        .fonts
        .iter()
        .filter_map(|f| sizes.font(f))
        .collect();
    let mut out = Vec::new();
    for e in layout.elements.iter().filter(|e| pick(e)) {
        let members: Vec<Option<&SquadLine>> = if e.repeat_squad.is_some() {
            values.squad.iter().map(Some).collect()
        } else {
            vec![None]
        };
        for (i, member) in members.into_iter().enumerate() {
            let first_quad = out.len();
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
            let chosen = e
                .colour_by
                .as_deref()
                .and_then(|k| e.colours.get(&values.text(k, member)).copied());
            let mut colour = match (alert, e.alert_colour) {
                (true, Some(c)) => c,
                _ => chosen.unwrap_or(e.colour),
            };
            if let Some(f) = &e.fade {
                let v = values.fraction(f, member).unwrap_or(1.0);
                if v <= 0.0 {
                    continue;
                }
                colour = faded(colour, v, &e.kind, e.blend.as_deref());
            }
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
                    if let Some(f) = &p.slide_with {
                        let v = values.fraction(f, member).unwrap_or(0.0);
                        for k in 0..4 {
                            dst[k] += p.slide[k % 2] * v * unit;
                        }
                    }
                    if let Some(n) = p.grow_with.as_deref().and_then(|g| values.steps(g)) {
                        dst[1] += p.grow[0] * n as f32 * unit;
                        dst[3] += p.grow[1] * n as f32 * unit;
                    }
                    if let Some(r) = p.redden_with.as_deref().and_then(|g| values.redden(g)) {
                        colour[1] = colour[1].saturating_sub(r);
                        colour[2] = colour[2].saturating_sub(r);
                    }
                    if let Some(f) = &p.fill {
                        let v = values.fraction(f, member).unwrap_or(1.0);
                        let v = stepped(v, p.fill_steps, p.fill_least_gap);
                        if v <= 0.0 {
                            continue;
                        }
                        let keep = src;
                        cut(&mut src, &mut dst, v, p.fill_toward.as_deref(), p.mirror);
                        if p.fill_stretch {
                            src = keep;
                        }
                        if p.fill_inset != 0.0 {
                            inset(&mut dst, p.fill_inset * unit, p.fill_toward.as_deref());
                            // The original then draws a sliver turned
                            // round; too small to matter, so nothing.
                            if dst[2] <= dst[0] || dst[3] <= dst[1] {
                                continue;
                            }
                        }
                    }
                    let slide = p.scroll.as_deref().and_then(|k| values.scroll(k));
                    let pieces = match slide {
                        Some(v) => tiles(src, dst, v * tw, tw),
                        None => vec![(src, dst)],
                    };
                    for (mut src, dst) in pieces {
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
                }
                Kind::Text(t) => {
                    let text = values.text(&t.text, member);
                    let line_px = rect[3] - rect[1];
                    let font = match &t.font {
                        Some(f) => fonts.iter().find(|d| d.path.eq_ignore_ascii_case(f)),
                        None => pick_font(&fonts, line_px),
                    };
                    let Some(font) = font else {
                        continue;
                    };
                    let lines = if t.wrap {
                        let k = line_px / font.line_height.max(1) as f32;
                        wrap_words(font, &text, (rect[2] - rect[0]) / k)
                    } else {
                        vec![text]
                    };
                    for (n, line) in lines.iter().enumerate() {
                        let down = n as f32 * line_px;
                        text_line(
                            line,
                            font,
                            [rect[0], rect[1] + down, rect[2], rect[3] + down],
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
            if let (true, Some(g)) = (e.glass, glass) {
                for q in &mut out[first_quad..] {
                    let a = g([q.dst[0], q.dst[1]]);
                    let b = g([q.dst[2], q.dst[3]]);
                    q.dst = [a[0], a[1], b[0], b[1]];
                }
            }
        }
    }
    out
}

/// `text` broken at spaces into lines no wider than `max` font pixels (a
/// word longer than that gets a line of its own).
pub fn wrap_words(font: &FontData, text: &str, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let tried = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if line.is_empty() || line_width(font, &tried, 0) <= max {
            line = tried;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `colour` at `v` (0..1) of its strength. A glowing ("add") part fades
/// by getting darker, as adding black adds nothing; a see-through one by
/// its alpha.
fn faded(colour: [u8; 4], v: f32, kind: &Kind, blend: Option<&str>) -> [u8; 4] {
    let default = match kind {
        Kind::Picture(_) => STY_ALPHA,
        Kind::Text(_) => STY_ADDITIVE,
    };
    let v = v.clamp(0.0, 1.0);
    let scale = |c: u8| (c as f32 * v).round() as u8;
    if style(blend, default) == STY_ADDITIVE {
        [
            scale(colour[0]),
            scale(colour[1]),
            scale(colour[2]),
            colour[3],
        ]
    } else {
        [colour[0], colour[1], colour[2], scale(colour[3])]
    }
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

/// Take `by` pixels off the end of `r` a bar empties toward (its fixed
/// end).
fn inset(r: &mut [f32; 4], by: f32, toward: Option<&str>) {
    match toward.unwrap_or("left") {
        "right" => r[2] -= by,
        "down" => r[3] -= by,
        "up" => r[1] += by,
        _ => r[0] += by,
    }
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
    fn the_use_prompt_fades_in_like_the_original() {
        // Engine.dll DrawActivateText: opacity = int(age / 0.5 * 220),
        // 220 from 0.5 s on.
        let l = LayoutSettings::default();
        let opacity = |age: f32| (l.prompt_faded_in(age) * 220.0).round() as u32;
        assert_eq!(opacity(0.0), 0);
        assert_eq!(opacity(0.1), 44);
        assert_eq!(opacity(0.25), 110);
        assert_eq!(opacity(0.499), 219);
        assert_eq!(opacity(0.5), 220);
        assert_eq!(opacity(3.0), 220);
    }

    #[test]
    fn the_pickup_message_fades_at_the_end() {
        let t = shipped().pickup_message;
        assert_eq!(t.seconds, 3.0);
        assert_eq!(t.strength(3.0), 1.0);
        assert_eq!(t.strength(1.5), 1.0);
        assert_eq!(t.strength(0.55), 0.6);
        assert_eq!(t.strength(0.0), 0.0);
        let v = HudValues {
            pickup: "Thermal Detonators".into(),
            pickup_fade: 0.5,
            ..Default::default()
        };
        assert_eq!(v.text("{pickup}", None), "Thermal Detonators");
        assert_eq!(v.flag("pickup", None), Some(true));
        assert_eq!(v.fraction("pickup", None), Some(0.5));
        assert_eq!(HudValues::default().flag("pickup", None), Some(false));
        // Glowing text fades by darkening; see-through pictures by alpha.
        let text = Kind::Text(TextPart {
            text: String::new(),
            align: None,
            font: None,
            wrap: false,
        });
        assert_eq!(
            faded([255, 200, 100, 255], 0.5, &text, None),
            [128, 100, 50, 255]
        );
        assert_eq!(
            faded([255, 200, 100, 255], 0.5, &text, Some("alpha")),
            [255, 200, 100, 128]
        );
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
        assert_eq!(v.flag("grenades_left", None), Some(true));
        assert_eq!(v.text("{squad_health_level}", Some(&m)), "0");
    }

    #[test]
    fn a_sliding_strip_repeats_its_picture() {
        // 414 texels of a 128-wide picture over 414 pixels, slid by 0.5.
        let t = tiles([0.0, 0.0, 414.0, 16.0], [0.0, 0.0, 414.0, 11.0], 64.0, 128.0);
        assert_eq!(t[0], ([64.0, 0.0, 128.0, 16.0], [0.0, 0.0, 64.0, 11.0]));
        assert_eq!(t[1], ([0.0, 0.0, 128.0, 16.0], [64.0, 0.0, 192.0, 11.0]));
        let last = t.last().unwrap();
        assert_eq!(last.1[2], 414.0);
        let covered: f32 = t.iter().map(|(s, _)| s[2] - s[0]).sum();
        assert!((covered - 414.0).abs() < 1e-3);
    }

    #[test]
    fn health_levels_and_stepped_bars_as_the_original() {
        assert_eq!(
            [0.0, 0.2, 0.5, 0.9, 0.91, 1.0].map(health_level),
            [0, 1, 2, 2, 3, 3]
        );
        // Elevenths, and at least 5/55 gone while hurt at all.
        assert_eq!(stepped(1.0, Some(11), 5.0 / 55.0), 1.0);
        assert_eq!(stepped(0.99, Some(11), 5.0 / 55.0), 1.0 - 5.0 / 55.0);
        assert!((stepped(0.5, Some(11), 5.0 / 55.0) - 6.0 / 11.0).abs() < 1e-6);
        assert_eq!(stepped(0.3, None, 0.0), 0.3);
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

    #[test]
    fn long_text_wraps_at_spaces() {
        // Every letter 10 pixels wide, no kerning.
        let g = rc_package::font::Glyph {
            u: 0,
            v: 0,
            width: 10,
            height: 10,
            page: 0,
        };
        let font = FontData {
            path: "F".into(),
            glyphs: vec![g; 128],
            pages: vec!["p".into()],
            kerning: 0,
            line_height: 10,
        };
        assert_eq!(
            wrap_words(&font, "ab cd efghijk l", 50.0),
            ["ab cd", "efghijk", "l"]
        );
        assert!(wrap_words(&font, "  ", 50.0).is_empty());
    }
}
