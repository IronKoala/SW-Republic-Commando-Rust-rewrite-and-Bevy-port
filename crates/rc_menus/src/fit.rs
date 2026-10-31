//! Fitting the original's 4:3 menus to wider screens by moving things
//! apart, never by shrinking them.
//!
//! The original lays every menu out on a 4:3 screen (`quads.rs`). On a
//! wider screen we keep each picture and letter at the size it has on a
//! 4:3 screen of the same height, and use the extra width to spread the
//! menu out:
//!
//! - **Groups.** Widgets that touch or overlap form a group that moves as
//!   one (a column of buttons, a planet and its ring, a label and the bar
//!   behind it). A text label joins only the pictures it touches, never a
//!   button or another text: the options rows' labels and their value
//!   buttons are separate groups, so they can move apart.
//! - **Spreading.** Each group's middle keeps its place as a fraction of
//!   the width, so the gaps between groups grow and the groups keep their
//!   size. (A group that fits a 4:3 screen stays on the wider one, and two
//!   groups that didn't overlap still don't.)
//! - **Frames** (the big stretchable boxes, such as the options box) widen
//!   with the screen when some of what is inside them sits at their left
//!   edge and some at their right edge: the left part keeps to the left
//!   edge and the right part to the right edge, so an options label and
//!   its value move apart. Frames lined up with a widening frame (the
//!   options page's help box under its options box) widen with it. Groups
//!   touching a widening frame from outside (the tabs joined to the
//!   options box) keep touching it. A frame that doesn't widen moves as
//!   one with everything in it.
//! - **Bridges.** A stretchable bar joining a group to a frame beside it
//!   (the main menu's "MAIN MENU" bar and the button box) grows to keep
//!   joining them when they move apart.
//! - **Backdrops** (pictures covering the whole menu) grow to cover the
//!   whole screen, keeping their shape.
//!
//! Units: as everywhere in this crate, 0..1 across is a 4:3 menu. A screen
//! `across` times as wide as 4:3 runs from `0.5 - across / 2` to
//! `0.5 + across / 2`.

use crate::layout::{pivot_fraction, Kind, MenuLayout, Sprite, MSCM_FIT_STRETCH, MSCM_STRETCH};
use crate::quads::{sprite_box, text_box, Sizes};

/// Boxes closer than this touch (and their widgets form a group).
const TOUCH: f32 = 0.01;
/// A group this close to a frame's outside keeps to that frame.
const TOUCH_FRAME: f32 = 0.03;
/// Part of a frame's width counted as "at its edge".
const EDGE: f32 = 0.1;

/// The menu's width in 4:3 menus for a screen of this shape (width over
/// height): 1 for 4:3 and taller screens, more for wider ones, up to
/// `widest` (a shape; 0 for no limit).
pub fn across_for(aspect: f32, widest: f32) -> f32 {
    let aspect = if widest > 0.0 {
        aspect.min(widest)
    } else {
        aspect
    };
    (aspect / (4.0 / 3.0)).max(1.0)
}

/// Where the screen starts, in menu units, for a menu `across` wide.
pub fn left_edge(across: f32) -> f32 {
    0.5 - across / 2.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Backdrop,
    Frame,
    Item,
}

/// The box a widget covers (for a button, its background and text in both
/// looks).
pub fn widget_box(kind: &Kind, sizes: &mut dyn Sizes) -> Option<[f32; 4]> {
    let boxes: Vec<[f32; 4]> = match kind {
        Kind::Sprite(s) => sprite_box(s, sizes).into_iter().collect(),
        Kind::Text(t) => text_box(t, sizes)
            .filter(|_| !t.text.is_empty())
            .into_iter()
            .collect(),
        Kind::Button(b) => [
            sprite_box(&b.background_blurred, sizes),
            sprite_box(&b.background_focused, sizes),
            text_box(&b.blurred, sizes).filter(|_| !b.blurred.text.is_empty()),
            text_box(&b.focused, sizes).filter(|_| !b.focused.text.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    boxes.into_iter().reduce(union)
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn near(a: [f32; 4], b: [f32; 4], m: f32) -> bool {
    a[0] <= b[2] + m && b[0] <= a[2] + m && a[1] <= b[3] + m && b[1] <= a[3] + m
}

fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
    const T: f32 = 0.005;
    inner[0] >= outer[0] - T
        && inner[1] >= outer[1] - T
        && inner[2] <= outer[2] + T
        && inner[3] <= outer[3] + T
}

fn role(kind: &Kind, b: [f32; 4]) -> Role {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    match kind {
        Kind::Sprite(_) if w >= 0.95 && h >= 0.95 => Role::Backdrop,
        Kind::Sprite(s)
            if matches!(s.scale_mode, MSCM_STRETCH | MSCM_FIT_STRETCH) && w >= 0.3 && h >= 0.15 =>
        {
            Role::Frame
        }
        _ => Role::Item,
    }
}

/// What `spread` did, for the log and for boxes drawn over the menu.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spread {
    /// How far each widget moved across (menu units), by widget index.
    pub moved: Vec<f32>,
    /// Frames widened, by widget name.
    pub widened: Vec<String>,
    /// Why each frame widened, for the log.
    pub why: Vec<String>,
    /// Bars stretched to keep joining a frame, by widget name.
    pub bridged: Vec<String>,
    pub groups: usize,
}

/// Spread `layout` over a screen `across` 4:3 menus wide (see the top of
/// this file). Does nothing at 1.
pub fn spread(layout: &mut MenuLayout, sizes: &mut dyn Sizes, across: f32) -> Spread {
    let n = layout.widgets.len();
    let mut out = Spread {
        moved: vec![0.0; n],
        ..Default::default()
    };
    if across <= 1.0 + 1e-4 {
        return out;
    }
    let k = across;
    let p = |x: f32| 0.5 + (x - 0.5) * k;
    let boxes: Vec<Option<[f32; 4]>> = layout
        .widgets
        .iter()
        .map(|w| {
            w.shown_on_pc()
                .then(|| widget_box(&w.kind, sizes))
                .flatten()
        })
        .collect();
    let roles: Vec<Option<Role>> = (0..n)
        .map(|i| boxes[i].map(|b| role(&layout.widgets[i].kind, b)))
        .collect();
    let is_text = |i: usize| matches!(layout.widgets[i].kind, Kind::Text(_));
    let is_sprite = |i: usize| matches!(layout.widgets[i].kind, Kind::Sprite(_));

    // Groups of items that touch.
    let items: Vec<usize> = (0..n).filter(|&i| roles[i] == Some(Role::Item)).collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        parent[i] = r;
        r
    }
    for (a_at, &a) in items.iter().enumerate() {
        for &b in &items[a_at + 1..] {
            // A text joins only pictures.
            let mixed = (is_text(a) && !is_sprite(b)) || (is_text(b) && !is_sprite(a));
            if mixed || !near(boxes[a].unwrap(), boxes[b].unwrap(), TOUCH) {
                continue;
            }
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra] = rb;
        }
    }
    // group root -> (members, box)
    let mut groups: Vec<(Vec<usize>, [f32; 4])> = Vec::new();
    let mut root_at: Vec<Option<usize>> = vec![None; n];
    for &i in &items {
        let r = find(&mut parent, i);
        match root_at[r] {
            Some(g) => {
                groups[g].0.push(i);
                groups[g].1 = union(groups[g].1, boxes[i].unwrap());
            }
            None => {
                root_at[r] = Some(groups.len());
                groups.push((vec![i], boxes[i].unwrap()));
            }
        }
    }
    out.groups = groups.len();

    // Each group's frame: the smallest frame it is inside.
    let frames: Vec<usize> = (0..n).filter(|&i| roles[i] == Some(Role::Frame)).collect();
    let area = |b: [f32; 4]| (b[2] - b[0]) * (b[3] - b[1]);
    let frame_of: Vec<Option<usize>> = groups
        .iter()
        .map(|(_, gb)| {
            frames
                .iter()
                .copied()
                .filter(|&f| inside(*gb, boxes[f].unwrap()))
                .min_by(|&a, &b| area(boxes[a].unwrap()).total_cmp(&area(boxes[b].unwrap())))
        })
        .collect();
    #[derive(Clone, Copy, PartialEq)]
    enum Side {
        Left,
        Right,
        Middle,
    }
    // A text on its own, centred on its spot (such as a help line whose
    // words change with the button in focus), stays in the middle.
    let centred: Vec<bool> = groups
        .iter()
        .map(|(m, _)| {
            m.len() == 1
                && matches!(&layout.widgets[m[0]].kind, Kind::Text(t) if pivot_fraction(t.pivot).0 == 0.5)
        })
        .collect();
    let side = |g: usize, fb: [f32; 4]| {
        if centred[g] {
            return Side::Middle;
        }
        let gb = groups[g].1;
        let (l, r, edge) = (gb[0] - fb[0], fb[2] - gb[2], (fb[2] - fb[0]) * EDGE);
        if l <= edge && l < r {
            Side::Left
        } else if r <= edge && r < l {
            Side::Right
        } else {
            Side::Middle
        }
    };
    // A frame widens when it holds things at both edges (things with no
    // width, such as an empty help text, don't count), or lines up with a
    // frame that does.
    let mut widens: Vec<usize> = frames
        .iter()
        .copied()
        .filter(|&f| {
            let fb = boxes[f].unwrap();
            let sides: Vec<Side> = (0..groups.len())
                .filter(|&g| frame_of[g] == Some(f) && groups[g].1[2] - groups[g].1[0] > 1e-4)
                .map(|g| side(g, fb))
                .collect();
            sides.contains(&Side::Left) && sides.contains(&Side::Right)
        })
        .collect();
    for &f in &widens {
        let fb = boxes[f].unwrap();
        let names = |want: Side| -> Vec<String> {
            (0..groups.len())
                .filter(|&g| frame_of[g] == Some(f) && groups[g].1[2] - groups[g].1[0] > 1e-4)
                .filter(|&g| side(g, fb) == want)
                .map(|g| layout.widgets[groups[g].0[0]].name.clone())
                .collect()
        };
        out.why.push(format!(
            "{} holds {:?} at its left and {:?} at its right",
            layout.widgets[f].name,
            names(Side::Left),
            names(Side::Right)
        ));
    }
    let lined_up =
        |a: [f32; 4], b: [f32; 4]| (a[0] - b[0]).abs() < TOUCH && (a[2] - b[2]).abs() < TOUCH;
    for &f in &frames {
        if !widens.contains(&f)
            && widens
                .iter()
                .any(|&w| lined_up(boxes[w].unwrap(), boxes[f].unwrap()))
        {
            widens.push(f);
        }
    }
    // A frame that doesn't widen moves by its middle, with what is in it.
    let frame_dx = |f: usize| {
        let fb = boxes[f].unwrap();
        ((fb[0] + fb[2]) / 2.0 - 0.5) * (k - 1.0)
    };

    // Groups outside a widening frame that touch it keep touching it:
    // (frame, on its left?).
    let attached: Vec<Option<(usize, bool)>> = groups
        .iter()
        .enumerate()
        .map(|(g, (_, gb))| {
            if frame_of[g].is_some() {
                return None;
            }
            let c = (gb[0] + gb[2]) / 2.0;
            widens
                .iter()
                .copied()
                .find(|&f| near(*gb, boxes[f].unwrap(), TOUCH_FRAME))
                .map(|f| {
                    let fb = boxes[f].unwrap();
                    (f, c < (fb[0] + fb[2]) / 2.0)
                })
        })
        .collect();
    // Where each frame's edges go. A widening frame's edges are placed so
    // that what is joined to it keeps the same share of the screen at the
    // sides as before (so the page stays balanced).
    let edges: Vec<(f32, f32)> = (0..n)
        .map(|f| {
            let Some(fb) = boxes[f].filter(|_| roles[f] == Some(Role::Frame)) else {
                return (0.0, 0.0);
            };
            if !widens.contains(&f) {
                return (fb[0] + frame_dx(f), fb[2] + frame_dx(f));
            }
            let mut l = fb[0];
            let mut r = fb[2];
            for (g, a) in attached.iter().enumerate() {
                match a {
                    Some((af, true)) if *af == f => l = l.min(groups[g].1[0]),
                    Some((af, false)) if *af == f => r = r.max(groups[g].1[2]),
                    _ => {}
                }
            }
            (p(l) + (fb[0] - l), p(r) - (r - fb[2]))
        })
        .collect();

    let mut group_dx = vec![0.0f32; groups.len()];
    for (g, (_, gb)) in groups.iter().enumerate() {
        let c = (gb[0] + gb[2]) / 2.0;
        group_dx[g] = match (frame_of[g], attached[g]) {
            (Some(f), _) if !widens.contains(&f) => frame_dx(f),
            (Some(f), _) => {
                let fb = boxes[f].unwrap();
                let (x0, x1) = edges[f];
                match side(g, fb) {
                    Side::Left => x0 - fb[0],
                    Side::Right => x1 - fb[2],
                    Side::Middle => x0 + (c - fb[0]) * (x1 - x0) / (fb[2] - fb[0]) - c,
                }
            }
            (None, Some((f, true))) => edges[f].0 - boxes[f].unwrap()[0],
            (None, Some((f, false))) => edges[f].1 - boxes[f].unwrap()[2],
            // Elsewhere: spread by its middle.
            (None, None) => (c - 0.5) * (k - 1.0),
        };
    }
    let frame_edges = |f: usize| edges[f];
    // Bridges: in a group beside a frame it doesn't move with, the
    // stretchable bar nearest the frame grows to keep joining it.
    let mut bridges: Vec<(usize, [f32; 4])> = Vec::new();
    for (g, (members, _)) in groups.iter().enumerate() {
        if frame_of[g].is_some() || attached[g].is_some() {
            continue;
        }
        let dx = group_dx[g];
        let bars: Vec<(usize, [f32; 4])> = members
            .iter()
            .copied()
            .filter(|&i| {
                matches!(&layout.widgets[i].kind, Kind::Sprite(s) if matches!(s.scale_mode, MSCM_STRETCH | MSCM_FIT_STRETCH))
            })
            .map(|i| (i, boxes[i].unwrap()))
            .collect();
        for &f in &frames {
            let fb = boxes[f].unwrap();
            let (x0, x1) = frame_edges(f);
            let beside = |b: &[f32; 4]| b[1] < fb[3] && fb[1] < b[3];
            // A bar on the frame's left...
            let left = bars
                .iter()
                .filter(|(_, b)| beside(b) && b[2] <= fb[0] + TOUCH && fb[0] - b[2] < TOUCH_FRAME)
                .max_by(|a, b| a.1[2].total_cmp(&b.1[2]));
            if let Some(&(i, b)) = left {
                if x0 - fb[0] - dx > 1e-4 {
                    bridges.push((i, [b[0] + dx, b[1], x0 - (fb[0] - b[2]), b[3]]));
                }
            }
            // ...or on its right.
            let right = bars
                .iter()
                .filter(|(_, b)| beside(b) && b[0] >= fb[2] - TOUCH && b[0] - fb[2] < TOUCH_FRAME)
                .min_by(|a, b| a.1[0].total_cmp(&b.1[0]));
            if let Some(&(i, b)) = right {
                if dx - (x1 - fb[2]) > 1e-4 {
                    bridges.push((i, [x1 + (b[0] - fb[2]), b[1], b[2] + dx, b[3]]));
                }
            }
        }
    }
    for &f in &frames {
        let fb = boxes[f].unwrap();
        let (x0, x1) = frame_edges(f);
        if widens.contains(&f) {
            set_box_x(&mut layout.widgets[f].kind, sizes, x0, x1);
            out.widened.push(layout.widgets[f].name.clone());
        } else {
            shift(&mut layout.widgets[f].kind, x0 - fb[0]);
        }
        out.moved[f] = x0 - fb[0];
    }
    for (g, (members, _)) in groups.iter().enumerate() {
        for &i in members {
            shift(&mut layout.widgets[i].kind, group_dx[g]);
            out.moved[i] = group_dx[g];
        }
    }
    for (i, to) in bridges {
        if to[2] > to[0] {
            set_box_x(&mut layout.widgets[i].kind, sizes, to[0], to[2]);
            out.bridged.push(layout.widgets[i].name.clone());
        }
    }
    // Backdrops cover the screen, keeping their shape.
    for i in (0..n).filter(|&i| roles[i] == Some(Role::Backdrop)) {
        let b = boxes[i].unwrap();
        if let Kind::Sprite(s) = &mut layout.widgets[i].kind {
            let ([x0, x1], [y0, y1]) = ([p(b[0]), p(b[2])], [p(b[1]), p(b[3])]);
            set_sprite_box(s, sizes, [x0, y0, x1, y1]);
        }
    }
    // Anything without a box (an empty text) moves by where it is.
    for i in 0..n {
        if boxes[i].is_none() {
            let x = match &layout.widgets[i].kind {
                Kind::Sprite(s) => s.pos[0],
                Kind::Text(t) => t.pos[0],
                Kind::Button(b) => b.blurred.pos[0],
            };
            let dx = (x - 0.5) * (k - 1.0);
            shift(&mut layout.widgets[i].kind, dx);
            out.moved[i] = dx;
        }
    }
    out
}

/// Move a widget `dx` across.
pub fn shift(kind: &mut Kind, dx: f32) {
    match kind {
        Kind::Sprite(s) => s.pos[0] += dx,
        Kind::Text(t) => t.pos[0] += dx,
        Kind::Button(b) => {
            b.blurred.pos[0] += dx;
            b.focused.pos[0] += dx;
            b.background_blurred.pos[0] += dx;
            b.background_focused.pos[0] += dx;
        }
    }
}

/// Move every widget of `layout` `dx` across (a box drawn over a menu,
/// following the widget it belongs to).
pub fn shift_all(layout: &mut MenuLayout, dx: f32) {
    for w in &mut layout.widgets {
        shift(&mut w.kind, dx);
    }
}

fn set_box_x(kind: &mut Kind, sizes: &mut dyn Sizes, x0: f32, x1: f32) {
    if let Kind::Sprite(s) = kind {
        if let Some(b) = sprite_box(s, sizes) {
            set_sprite_box(s, sizes, [x0, b[1], x1, b[3]]);
        }
    }
}

/// Resize and move a sprite to cover `to`.
fn set_sprite_box(s: &mut Sprite, sizes: &mut dyn Sizes, to: [f32; 4]) {
    let Some(b) = sprite_box(s, sizes) else {
        return;
    };
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let (nw, nh) = (to[2] - to[0], to[3] - to[1]);
    if w > 0.0 {
        s.scale[0] *= nw / w;
    }
    if h > 0.0 {
        s.scale[1] *= nh / h;
    }
    let (px, py) = pivot_fraction(s.pivot);
    s.pos = [to[0] + nw * px, to[1] + nh * py];
}

/// Texts that run into the next button in their row after spreading (on a
/// screen too narrow to move them apart), for the log.
pub fn text_overlaps(layout: &MenuLayout, sizes: &mut dyn Sizes) -> Vec<String> {
    let mut out = Vec::new();
    let texts: Vec<(usize, [f32; 4])> = layout
        .widgets
        .iter()
        .enumerate()
        .filter(|(_, w)| w.shown_on_pc())
        .filter_map(|(i, w)| match &w.kind {
            Kind::Text(t) if !t.text.is_empty() => text_box(t, sizes).map(|b| (i, b)),
            _ => None,
        })
        .collect();
    for (i, tb) in texts {
        for w in layout.widgets.iter().filter(|w| w.shown_on_pc()) {
            if let Kind::Button(b) = &w.kind {
                let bb = sprite_box(&b.background_blurred, sizes)
                    .or_else(|| text_box(&b.blurred, sizes).filter(|_| !b.blurred.text.is_empty()));
                if let Some(bb) = bb {
                    let overlap_y = tb[1].max(bb[1]) < tb[3].min(bb[3]) - 0.005;
                    if overlap_y && tb[0] < bb[0] && tb[2] > bb[0] + 0.002 {
                        out.push(format!("{} runs into {}", layout.widgets[i].name, w.name));
                        break;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Button, Widget};
    use crate::library::FontData;

    struct NoSizes;
    impl Sizes for NoSizes {
        fn texture_size(&mut self, _: &str) -> Option<(f32, f32)> {
            Some((64.0, 64.0))
        }
        fn font(&mut self, _: &str) -> Option<FontData> {
            None
        }
    }

    /// A Fit-mode sprite covering `b` (Fit sizes are fractions).
    fn sprite(name: &str, b: [f32; 4], mode: u8) -> Widget {
        Widget {
            name: name.into(),
            pass: 0,
            hidden: false,
            platform: 0,
            kind: Kind::Sprite(Sprite {
                texture: Some("T.x".into()),
                color: [255; 4],
                pos: [b[0], b[1]],
                scale: [b[2] - b[0], b[3] - b[1]],
                scale_mode: mode,
                ..Default::default()
            }),
        }
    }

    fn button(name: &str, b: [f32; 4]) -> Widget {
        let Kind::Sprite(s) = sprite(name, b, crate::layout::MSCM_FIT).kind else {
            unreachable!()
        };
        Widget {
            kind: Kind::Button(Button {
                background_blurred: s.clone(),
                background_focused: s,
                ..Default::default()
            }),
            ..sprite(name, b, 0)
        }
    }

    fn layout(widgets: Vec<Widget>) -> MenuLayout {
        MenuLayout {
            class: "Test".into(),
            widgets,
            skipped: vec![],
            sounds: Default::default(),
            background_movie: String::new(),
            music: None,
        }
    }

    fn box_of(l: &MenuLayout, name: &str) -> [f32; 4] {
        let w = l.widgets.iter().find(|w| w.name == name).unwrap();
        widget_box(&w.kind, &mut NoSizes).unwrap()
    }

    #[test]
    fn shapes() {
        assert_eq!(across_for(4.0 / 3.0, 2.4), 1.0);
        assert_eq!(across_for(5.0 / 4.0, 2.4), 1.0);
        assert!((across_for(16.0 / 9.0, 2.4) - 4.0 / 3.0).abs() < 1e-5);
        assert!((across_for(32.0 / 9.0, 2.4) - 1.8).abs() < 1e-5);
        assert!((across_for(32.0 / 9.0, 0.0) - 8.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn groups_move_apart_and_keep_their_size() {
        let mut l = layout(vec![
            button("Left", [0.05, 0.4, 0.35, 0.45]),
            button("Right", [0.6, 0.4, 0.9, 0.45]),
            sprite("Back", [0.0, 0.0, 1.0, 1.0], crate::layout::MSCM_FIT),
        ]);
        let before = (box_of(&l, "Left"), box_of(&l, "Right"));
        let s = spread(&mut l, &mut NoSizes, 1.8);
        let (a, b) = (box_of(&l, "Left"), box_of(&l, "Right"));
        assert_eq!(s.groups, 2);
        // Same sizes, bigger gap, still on the screen.
        assert!(((a[2] - a[0]) - (before.0[2] - before.0[0])).abs() < 1e-5);
        assert!(b[0] - a[2] > before.1[0] - before.0[2]);
        assert!(a[0] >= left_edge(1.8) && b[2] <= 1.0 - left_edge(1.8));
        // The backdrop covers the whole screen.
        let back = box_of(&l, "Back");
        assert!(
            (back[0] - left_edge(1.8)).abs() < 1e-5
                && (back[2] - (1.0 - left_edge(1.8))).abs() < 1e-5
        );
    }

    #[test]
    fn options_box_widens_and_its_rows_move_apart() {
        // Like the original's options page: tabs joined to a box holding
        // labels at its left edge and values at its right edge.
        let mut l = layout(vec![
            sprite(
                "OptionsBorder",
                [0.306, 0.06, 0.956, 0.75],
                crate::layout::MSCM_FIT_STRETCH,
            ),
            button("Tab", [0.044, 0.18, 0.289, 0.22]),
            sprite(
                "Connector",
                [0.295, 0.18, 0.3, 0.22],
                crate::layout::MSCM_FIT_STRETCH,
            ),
            sprite("Label", [0.34, 0.10, 0.68, 0.14], crate::layout::MSCM_FIT),
            button("Value", [0.614, 0.10, 0.935, 0.14]),
        ]);
        // (The label is a sprite here, so it would join the value: use the
        // value's box to check instead.)
        let s = spread(&mut l, &mut NoSizes, 1.8);
        assert!(
            s.widened.is_empty(),
            "label and value touch: one group, nothing to widen"
        );

        let mut l = layout(vec![
            sprite(
                "OptionsBorder",
                [0.306, 0.06, 0.956, 0.75],
                crate::layout::MSCM_FIT_STRETCH,
            ),
            button("Tab", [0.044, 0.18, 0.289, 0.22]),
            sprite(
                "Connector",
                [0.295, 0.18, 0.3, 0.22],
                crate::layout::MSCM_FIT_STRETCH,
            ),
            sprite("Label", [0.34, 0.10, 0.5, 0.14], crate::layout::MSCM_FIT),
            button("Value", [0.614, 0.10, 0.935, 0.14]),
        ]);
        let s = spread(&mut l, &mut NoSizes, 1.8);
        assert_eq!(s.widened, ["OptionsBorder"]);
        let (frame, tab, conn, label, value) = (
            box_of(&l, "OptionsBorder"),
            box_of(&l, "Tab"),
            box_of(&l, "Connector"),
            box_of(&l, "Label"),
            box_of(&l, "Value"),
        );
        // Label to the box's left edge, value to its right edge, tab still
        // joined to the box.
        assert!((label[0] - frame[0] - 0.034).abs() < 1e-4);
        assert!((frame[2] - value[2] - 0.021).abs() < 1e-4);
        assert!((frame[0] - conn[2] - 0.006).abs() < 1e-4);
        assert!((tab[2] - tab[0] - 0.245).abs() < 1e-4);
        assert!(value[0] - label[2] > 0.614 - 0.5);
    }

    #[test]
    fn nothing_moves_on_a_4_3_screen() {
        let mut l = layout(vec![button("A", [0.1, 0.1, 0.3, 0.2])]);
        let before = l.clone();
        spread(&mut l, &mut NoSizes, 1.0);
        assert_eq!(l, before);
    }
}
