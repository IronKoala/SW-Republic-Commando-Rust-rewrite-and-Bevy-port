//! Turns a menu class's default values into a list of things to draw:
//! pictures (`MenuSprite`), text (`MenuText`), buttons
//! (`MenuButtonText`), picture buttons such as arrows (`MenuButtonSprite`)
//! and choosers that step through a list of values (`MenuButtonEnum`), with
//! every default filled in the way the original menu code does.
//!
//! The rules come from the comments in the game's own `MenuBase` and
//! `XInterface` script classes (read from the install, not copied):
//!
//! 1. Positions are interpolated down arrays (`MenuOptions[0]` and
//!    `MenuOptions[1]` set the first position and the step; later items
//!    continue the line).
//! 2. Other values are copied down arrays, to the array's full size (the
//!    controls page sets only the first two of its fourteen key buttons).
//! 3. Sub-widgets are copied: `Blurred` -> `Focused`, `BackgroundBlurred` ->
//!    `BackgroundFocused`, and so on.
//! 4. A widget's `Style` names a template in the class (such as
//!    `ButtonTextStyle1`) whose values fill in what the widget leaves unset
//!    (then sub-widgets are copied again, for what the style added).
//! 5. The class's `DefaultMenu...` blocks fill in the rest.
//!
//! "Non-zero properties aren't overwritten": every step only fills fields
//! that are still zero. Templates are the properties in the editor category
//! `MenuDefault`; they are never drawn.

use crate::library::ClassData;
use crate::value::{fill_zero, Field, V};

/// `ERenderStyle` from the game's `Engine.u` (read with `rc_probe --classes`).
pub const STY_NONE: u8 = 0;
pub const STY_NORMAL: u8 = 1;
pub const STY_MASKED: u8 = 2;
pub const STY_TRANSLUCENT: u8 = 3;
pub const STY_ALPHA: u8 = 5;
pub const STY_ADDITIVE: u8 = 6;

/// `EMenuScaleMode` from the game's `XInterface.u`.
pub const MSCM_SCALE: u8 = 0;
pub const MSCM_STRETCH: u8 = 1;
pub const MSCM_FIT: u8 = 2;
pub const MSCM_FIT_STRETCH: u8 = 3;

/// `EMenuWidgetPlatform` from the game's `XInterface.u`.
pub const MWP_ALL: u8 = 0;
pub const MWP_PC: u8 = 1;
pub const MWP_CONSOLE: u8 = 2;

/// Where a widget's position sits on it, as fractions of its width and
/// height, for each `EDrawPivot` value (order from the game's `Core.u`:
/// UpperLeft, UpperMiddle, UpperRight, MiddleRight, LowerRight,
/// LowerMiddle, LowerLeft, MiddleLeft, MiddleMiddle).
pub fn pivot_fraction(pivot: u8) -> (f32, f32) {
    match pivot {
        1 => (0.5, 0.0),
        2 => (1.0, 0.0),
        3 => (1.0, 0.5),
        4 => (1.0, 1.0),
        5 => (0.5, 1.0),
        6 => (0.0, 1.0),
        7 => (0.0, 0.5),
        8 => (0.5, 0.5),
        _ => (0.0, 0.0),
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sprite {
    /// Full object path of the picture, if any.
    pub texture: Option<String>,
    /// Part of the picture to use (x1, y1, x2, y2); all zero means all of it.
    pub coords: [i32; 4],
    pub render_style: u8,
    /// Red, green, blue, alpha. Already defaulted to opaque white.
    pub color: [u8; 4],
    pub pivot: u8,
    /// 0..1 across and down the menu area.
    pub pos: [f32; 2],
    /// Already defaulted to 1.
    pub scale: [f32; 2],
    pub scale_mode: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub font: Option<String>,
    pub text: String,
    pub render_style: u8,
    pub color: [u8; 4],
    pub pivot: u8,
    pub pos: [f32; 2],
    pub scale: [f32; 2],
    pub kerning: i32,
    /// Widest the text may be, as a fraction of the menu width; 0 for no
    /// limit.
    pub max_size_x: f32,
    /// Break the text into lines at spaces to fit `max_size_x`, instead
    /// of cutting it short with "...".
    pub word_wrap: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Button {
    pub blurred: Text,
    pub focused: Text,
    pub background_blurred: Sprite,
    pub background_focused: Sprite,
    /// Name of the script function the original calls when chosen.
    pub on_select: String,
    pub disabled: bool,
    pub help: String,
    /// A chooser's values (`MenuButtonEnum`'s `Items`); empty for an
    /// ordinary button. The chosen one is shown as the button's text.
    pub items: Vec<String>,
    /// The chosen value, an index into `items`.
    pub current: usize,
    /// Stops at the ends instead of going round.
    pub no_wrap: bool,
    /// Only the mouse can use it (`bIgnoreController`): the arrows beside
    /// a chooser. Keys and controllers skip it.
    pub mouse_only: bool,
}

impl Button {
    /// A chooser (`MenuButtonEnum`).
    pub fn is_chooser(&self) -> bool {
        !self.items.is_empty()
    }

    /// Show value `i` of a chooser.
    pub fn choose(&mut self, i: usize) {
        if self.items.is_empty() {
            return;
        }
        self.current = i.min(self.items.len() - 1);
        self.blurred.text = self.items[self.current].clone();
        self.focused.text = self.items[self.current].clone();
    }

    /// Step a chooser one value left (-1) or right (1). Returns whether
    /// the value changed.
    pub fn step(&mut self, step: i32) -> bool {
        let n = self.items.len() as i32;
        if n == 0 {
            return false;
        }
        let at = self.current as i32 + step;
        let next = if self.no_wrap {
            at.clamp(0, n - 1)
        } else {
            at.rem_euclid(n)
        } as usize;
        let changed = next != self.current;
        self.choose(next);
        changed
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // few widgets; boxing gains nothing
pub enum Kind {
    Sprite(Sprite),
    Text(Text),
    Button(Button),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// Property name, with the array index when it has one:
    /// `MenuOptions[3]`.
    pub name: String,
    /// Drawing pass: lower passes are drawn first.
    pub pass: i32,
    pub hidden: bool,
    pub platform: u8,
    pub kind: Kind,
}

impl Widget {
    /// Shown on a PC: not hidden and not a console-only widget.
    pub fn shown_on_pc(&self) -> bool {
        !self.hidden && self.platform != MWP_CONSOLE
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sounds {
    pub focus: Option<String>,
    pub select: Option<String>,
    pub error: Option<String>,
    pub tween_in: Option<String>,
    pub tween_out: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    pub class: String,
    /// In drawing order (by pass, then declaration order).
    pub widgets: Vec<Widget>,
    /// Widgets of kinds we don't draw yet, as "name (kind)".
    pub skipped: Vec<String>,
    pub sounds: Sounds,
    /// The `.xmv` background video's name (the PC game plays
    /// `Movies/menu_bg.avi` for it).
    pub background_movie: String,
    pub music: Option<String>,
}

impl MenuLayout {
    pub fn widget(&self, name: &str) -> Option<&Widget> {
        self.widgets
            .iter()
            .find(|w| w.name.eq_ignore_ascii_case(name))
    }

    pub fn widget_mut(&mut self, name: &str) -> Option<&mut Widget> {
        self.widgets
            .iter_mut()
            .find(|w| w.name.eq_ignore_ascii_case(name))
    }

    /// The buttons a key or controller can move between, in order (not
    /// the mouse-only arrows).
    pub fn focusable_on_pc(&self) -> Vec<usize> {
        self.buttons_on_pc()
            .into_iter()
            .filter(|&i| matches!(&self.widgets[i].kind, Kind::Button(b) if !b.mouse_only))
            .collect()
    }

    /// Every button shown on a PC, mouse-only ones included, in order.
    pub fn buttons_on_pc(&self) -> Vec<usize> {
        (0..self.widgets.len())
            .filter(|&i| {
                let w = &self.widgets[i];
                w.shown_on_pc() && matches!(w.kind, Kind::Button(_))
            })
            .collect()
    }
}

const DRAWN: &[&str] = &[
    "MenuSprite",
    "MenuText",
    "MenuButtonText",
    "MenuButtonSprite",
    "MenuButtonEnum",
];
const SUBWIDGET_COPIES: &[(&str, &str)] = &[
    ("Blurred", "Focused"),
    ("Blurred", "SelectedBlurred"),
    ("SelectedBlurred", "SelectedFocused"),
    ("BackgroundBlurred", "BackgroundFocused"),
    ("BackgroundFocused", "BackgroundSelected"),
];

/// Build a menu's widgets from its class.
pub fn build(class: &ClassData) -> MenuLayout {
    // Group the widget properties by name, keeping declaration order.
    let mut groups: Vec<(String, Vec<(u32, V)>)> = Vec::new();
    let mut skipped = Vec::new();
    for f in &class.defaults {
        let V::Struct { kind, .. } = &f.value else {
            continue;
        };
        if !kind.starts_with("Menu") || class.category(&f.name) == "MenuDefault" {
            continue;
        }
        if !DRAWN.contains(&kind.as_str()) {
            if f.index == 0 && !matches!(kind.as_str(), "MenuMovie") {
                skipped.push(format!("{} ({kind})", f.name));
            }
            continue;
        }
        match groups
            .iter_mut()
            .find(|g| g.0.eq_ignore_ascii_case(&f.name))
        {
            Some(g) => g.1.push((f.index, f.value.clone())),
            None => groups.push((f.name.clone(), vec![(f.index, f.value.clone())])),
        }
    }

    // Declaration order (parents' properties first), as the original
    // draws them.
    groups.sort_by_key(|g| class.declared_at(&g.0));

    let mut widgets = Vec::new();
    for (name, mut elems) in groups {
        elems.sort_by_key(|e| e.0);
        let size = class.array_size(&name) as usize;
        let is_array = size > 1 || elems.len() > 1 || elems[0].0 > 0;
        // Every element of a fixed array, set or not (an unset one is
        // filled from the one before it).
        let kind = elems[0].1.kind().to_string();
        let count = size.max(elems.last().map_or(0, |e| e.0 as usize + 1));
        let mut values: Vec<V> = (0..count)
            .map(|i| match elems.iter().find(|e| e.0 as usize == i) {
                Some(e) => e.1.clone(),
                None => V::Struct {
                    kind: kind.clone(),
                    fields: Vec::new(),
                },
            })
            .collect();
        if values.len() > 1 {
            interpolate_positions(&mut values);
            for i in 1..values.len() {
                let prev = values[i - 1].clone();
                fill_zero(&mut values[i], &prev);
            }
        }
        for (i, mut v) in values.into_iter().enumerate() {
            resolve(class, &mut v, 0);
            let label = if is_array {
                format!("{name}[{i}]")
            } else {
                name.clone()
            };
            if let Some(w) = to_widget(label, &v) {
                widgets.push(w);
            }
        }
    }
    // Draw by pass; the sort is stable so declaration order stays within a
    // pass.
    widgets.sort_by_key(|w| w.pass);

    let obj = |n: &str| match class.get(n) {
        Some(V::Object { path, .. }) if !path.is_empty() => Some(path.clone()),
        _ => None,
    };
    MenuLayout {
        class: class.path.clone(),
        widgets,
        skipped,
        sounds: Sounds {
            focus: obj("SoundOnFocus"),
            select: obj("SoundOnSelect"),
            error: obj("SoundOnError"),
            tween_in: obj("SoundTweenIn"),
            tween_out: obj("SoundTweenOut"),
        },
        background_movie: match class.get("BackgroundMovieName") {
            Some(V::Str(s)) => s.clone(),
            _ => String::new(),
        },
        music: obj("BackgroundMusic"),
    }
}

/// Fill in a widget from its style, its sub-widgets and the class's
/// `DefaultMenu...` block.
fn resolve(class: &ClassData, v: &mut V, depth: usize) {
    if depth > 8 {
        return;
    }
    // The widget's own sub-widgets first ("Sub-widgets like Blurred &
    // Focused are copied", then "Properties from the MenuDefault blocks
    // are copied", in MenuBase's own order): a button that places only its
    // `Blurred` text keeps its focused text in the same place.
    copy_subwidgets(v);
    apply_style(class, v, depth);
    copy_subwidgets(v);
    // A struct with no `Default...` block of its own uses its parent
    // struct's: `MenuButtonEnum extends MenuButtonText` (XInterface's
    // `MenuBase`), so a chooser's value is centred like a button's text.
    let kind = match v.kind() {
        "MenuButtonEnum" if class.get("DefaultMenuButtonEnum").is_none() => "MenuButtonText",
        k => k,
    };
    let default_name = format!("Default{kind}");
    if let Some(d) = class.get(&default_name) {
        let mut d = d.clone();
        apply_style(class, &mut d, depth + 1);
        copy_subwidgets(&mut d);
        fill_zero(v, &d);
    }
    copy_subwidgets(v);
}

fn apply_style(class: &ClassData, v: &mut V, depth: usize) {
    if depth > 8 {
        return;
    }
    let style = v.text("Style");
    if !style.is_empty() {
        if let Some(s) = class.get(&style) {
            let mut s = s.clone();
            apply_style(class, &mut s, depth + 1);
            fill_zero(v, &s);
        }
    }
    if let V::Struct { fields, .. } = v {
        for f in fields.iter_mut() {
            if matches!(f.value, V::Struct { .. }) {
                apply_style(class, &mut f.value, depth + 1);
            }
        }
    }
}

fn copy_subwidgets(v: &mut V) {
    for (from, to) in SUBWIDGET_COPIES {
        let Some(src) = v.get(from).cloned() else {
            continue;
        };
        if let Some(dst) = v.get_mut(to) {
            fill_zero(dst, &src);
        }
    }
}

/// Every path (list of field names) inside `v` that ends in a position.
fn position_paths(v: &V, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for f in v.fields() {
        if f.index != 0 {
            continue;
        }
        prefix.push(f.name.clone());
        match &f.value {
            V::Struct { .. } => position_paths(&f.value, prefix, out),
            V::Float(_) if (f.name == "PosX" || f.name == "PosY") && !out.contains(prefix) => {
                out.push(prefix.clone());
            }
            _ => {}
        }
        prefix.pop();
    }
}

fn get_path<'a>(v: &'a V, path: &[String]) -> Option<&'a V> {
    path.iter().try_fold(v, |v, n| v.get(n))
}

fn set_path(v: &mut V, path: &[String], value: f32) {
    let (last, rest) = path.split_last().unwrap();
    let mut cur = v;
    for n in rest {
        // An unset array element has no sub-structs yet.
        if cur.get(n).is_none() {
            cur.set(
                n,
                V::Struct {
                    kind: String::new(),
                    fields: Vec::new(),
                },
            );
        }
        match cur.get_mut(n) {
            Some(c) => cur = c,
            None => return,
        }
    }
    cur.set(last, V::Float(value));
}

/// Fill unset positions in an array: between two set ones, interpolate;
/// after the last, continue with the last step; before the first, copy it.
fn interpolate_positions(values: &mut [V]) {
    let mut paths = Vec::new();
    for v in values.iter() {
        position_paths(v, &mut Vec::new(), &mut paths);
    }
    for path in paths {
        let known: Vec<(usize, f32)> = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| match get_path(v, &path) {
                Some(V::Float(f)) if *f != 0.0 => Some((i, *f)),
                _ => None,
            })
            .collect();
        if known.is_empty() {
            continue;
        }
        for (i, slot) in values.iter_mut().enumerate() {
            if known.iter().any(|k| k.0 == i) {
                continue;
            }
            let before: Vec<&(usize, f32)> = known.iter().filter(|k| k.0 < i).collect();
            let after = known.iter().find(|k| k.0 > i);
            let value = match (before.last(), after) {
                (Some(p), Some(q)) => p.1 + (q.1 - p.1) * (i - p.0) as f32 / (q.0 - p.0) as f32,
                (Some(p), None) => match before.len().checked_sub(2).map(|j| before[j]) {
                    Some(pp) => p.1 + (p.1 - pp.1) / (p.0 - pp.0) as f32 * (i - p.0) as f32,
                    None => p.1,
                },
                (None, Some(q)) => q.1,
                (None, None) => continue,
            };
            set_path(slot, &path, value);
        }
    }
}

fn opaque_white_if_unset(c: [u8; 4]) -> [u8; 4] {
    if c == [0, 0, 0, 0] {
        [255, 255, 255, 255]
    } else {
        c
    }
}

fn one_if_unset(f: f32) -> f32 {
    if f == 0.0 {
        1.0
    } else {
        f
    }
}

fn style_or_alpha(s: u8) -> u8 {
    if s == STY_NONE {
        STY_ALPHA
    } else {
        s
    }
}

pub fn sprite(v: &V) -> Sprite {
    let c = v.get("TextureCoords");
    let coord = |n: &str| match c.and_then(|c| c.get(n)) {
        Some(V::Int(i)) => *i,
        _ => 0,
    };
    Sprite {
        texture: v.object("WidgetTexture"),
        coords: [coord("X1"), coord("Y1"), coord("X2"), coord("Y2")],
        render_style: style_or_alpha(v.byte("RenderStyle")),
        color: opaque_white_if_unset(v.color("DrawColor")),
        pivot: v.byte("DrawPivot"),
        pos: [v.float("PosX"), v.float("PosY")],
        scale: [
            one_if_unset(v.float("ScaleX")),
            one_if_unset(v.float("ScaleY")),
        ],
        scale_mode: v.byte("ScaleMode"),
    }
}

pub fn text(v: &V) -> Text {
    Text {
        font: v.object("MenuFont"),
        text: v.text("Text"),
        render_style: style_or_alpha(v.byte("RenderStyle")),
        color: opaque_white_if_unset(v.color("DrawColor")),
        pivot: v.byte("DrawPivot"),
        pos: [v.float("PosX"), v.float("PosY")],
        scale: [
            one_if_unset(v.float("ScaleX")),
            one_if_unset(v.float("ScaleY")),
        ],
        kerning: v.int("Kerning"),
        max_size_x: v.float("MaxSizeX"),
        word_wrap: v.int("bWordWrap") != 0,
    }
}

fn empty() -> V {
    V::Struct {
        kind: String::new(),
        fields: Vec::<Field>::new(),
    }
}

fn to_widget(name: String, v: &V) -> Option<Widget> {
    let kind = match v.kind() {
        "MenuSprite" => Kind::Sprite(sprite(v)),
        "MenuText" => Kind::Text(text(v)),
        "MenuButtonText" | "MenuButtonEnum" => {
            let e = empty();
            let mut background_blurred = sprite(v.get("BackgroundBlurred").unwrap_or(&e));
            let mut background_focused = sprite(v.get("BackgroundFocused").unwrap_or(&e));
            let blurred = text(v.get("Blurred").unwrap_or(&e));
            let focused = text(v.get("Focused").unwrap_or(&e));
            // "If set, Background PosX is Blurred.PosX + Background.PosX"
            // (MenuBase), and the same down the screen.
            if v.int("bRelativeBackgroundCoords") != 0 {
                for b in [&mut background_blurred, &mut background_focused] {
                    b.pos[0] += blurred.pos[0];
                    b.pos[1] += blurred.pos[1];
                }
            }
            let mut b = Button {
                blurred,
                focused,
                background_blurred,
                background_focused,
                on_select: v.text("OnSelect"),
                disabled: v.int("bDisabled") != 0,
                help: v.text("HelpText"),
                items: v.list("Items"),
                current: 0,
                no_wrap: v.int("bNoWrap") != 0,
                mouse_only: v.int("bIgnoreController") != 0,
            };
            if b.is_chooser() {
                b.choose(v.int("Current").max(0) as usize);
            }
            Kind::Button(b)
        }
        // A picture that works as a button: `Blurred` and `Focused` are
        // pictures, drawn as the button's background.
        "MenuButtonSprite" => {
            let e = empty();
            Kind::Button(Button {
                background_blurred: sprite(v.get("Blurred").unwrap_or(&e)),
                background_focused: sprite(v.get("Focused").unwrap_or(&e)),
                on_select: v.text("OnSelect"),
                disabled: v.int("bDisabled") != 0,
                help: v.text("HelpText"),
                mouse_only: v.int("bIgnoreController") != 0,
                ..Default::default()
            })
        }
        _ => return None,
    };
    Some(Widget {
        name,
        pass: v.int("Pass"),
        hidden: v.int("bHidden") != 0,
        platform: v.byte("Platform"),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, value: V) -> Field {
        Field {
            name: name.into(),
            index: 0,
            value,
        }
    }

    fn text_struct(pos_y: f32) -> V {
        V::Struct {
            kind: "MenuText".into(),
            fields: vec![f("PosY", V::Float(pos_y)), f("PosX", V::Float(0.0))],
        }
    }

    #[test]
    fn positions_continue_down_an_array() {
        let mut v = vec![
            text_struct(0.48),
            text_struct(0.53),
            text_struct(0.0),
            text_struct(0.0),
        ];
        interpolate_positions(&mut v);
        let y: Vec<f32> = v.iter().map(|t| t.float("PosY")).collect();
        assert!((y[2] - 0.58).abs() < 1e-5, "{y:?}");
        assert!((y[3] - 0.63).abs() < 1e-5, "{y:?}");
        // PosX was never set, so it stays unset.
        assert_eq!(v[3].float("PosX"), 0.0);
    }

    #[test]
    fn positions_interpolate_between_set_ones() {
        let mut v = vec![text_struct(0.2), text_struct(0.0), text_struct(0.4)];
        interpolate_positions(&mut v);
        assert!((v[1].float("PosY") - 0.3).abs() < 1e-5);
    }

    #[test]
    fn pivots_match_the_enum_order() {
        assert_eq!(pivot_fraction(0), (0.0, 0.0));
        assert_eq!(pivot_fraction(3), (1.0, 0.5));
        assert_eq!(pivot_fraction(8), (0.5, 0.5));
    }
}
