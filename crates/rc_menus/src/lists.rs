//! Scrolling lists on the original menus: the campaign page's mission list
//! and the load and save pages' list of saved games.
//!
//! In the original these are three widgets that work together:
//!
//! - `MenuStringList`: a `Template` button copied once per row it shows
//!   (`DisplayCount`), spaced evenly from (`PosX1`, `PosY1`) to (`PosX2`,
//!   `PosY2`). Its `Items` are the texts; `Position` is the first one shown.
//! - `MenuScrollBar`: the bar beside it. Its `Blurred` picture is the
//!   handle, which moves between (`PosX1`, `PosY1`) and (`PosX2`, `PosY2`)
//!   and is at least `MinScaleX` by `MinScaleY` big.
//! - `MenuScrollArea`: the box the mouse wheel scrolls in.
//!
//! How they are laid out is native code in the original
//! (`LayoutMenuStringList`, `LayoutMenuScrollBar`), so the spacing and the
//! handle's size here are our reading of their values: the rows land
//! exactly inside the page's border and the handle exactly between the
//! scroll arrows. The page's two `MenuActiveWidget`s (`PageUpArea`,
//! `PageDownArea`, the bar above and below the handle) are worked out from
//! the handle instead of being drawn.
//!
//! The rows become ordinary buttons on the page, named like `LevelList[2]`
//! (the third row on screen, whatever item it shows), so drawing, focus and
//! the mouse work as they do for any button.

use crate::layout::{Kind, MenuLayout};

/// A `MenuStringList` on a page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StringList {
    /// The property name, such as `LevelList`; row `r` on screen is the
    /// button `LevelList[r]`.
    pub name: String,
    pub items: Vec<String>,
    /// The first item shown.
    pub position: usize,
    /// How many rows it shows.
    pub rows: usize,
    /// The item chosen with a click or the keys (the original's
    /// `SelectedLevel` / `SelectedSave`), if any.
    pub selected: Option<usize>,
}

/// A `MenuScrollBar`: only its handle is drawn (the widget of the same
/// name, a picture).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScrollBar {
    pub name: String,
    /// Where the handle's centre can go: x, top y, bottom y.
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
    /// The smallest the handle can be, as fractions of the screen.
    pub min_size: [f32; 2],
}

/// Everything list-like on a page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lists {
    pub lists: Vec<StringList>,
    pub bars: Vec<ScrollBar>,
    /// Boxes the mouse wheel scrolls the list in: x0, y0, x1, y1.
    pub wheel_areas: Vec<[f32; 4]>,
}

/// The list a row button belongs to, and which row it is.
pub fn row_of(layout: &MenuLayout, widget: usize) -> Option<(usize, usize)> {
    let name = &layout.widgets.get(widget)?.name;
    let open = name.find('[')?;
    let base = &name[..open];
    let row: usize = name[open + 1..].strip_suffix(']')?.parse().ok()?;
    let l = layout.lists.lists.iter().position(|l| l.name == base)?;
    Some((l, row))
}

/// The item a row button shows now.
pub fn item_of(layout: &MenuLayout, widget: usize) -> Option<usize> {
    let (l, row) = row_of(layout, widget)?;
    let list = &layout.lists.lists[l];
    let item = list.position + row;
    (item < list.items.len()).then_some(item)
}

/// The row button showing `item` of list `l`, if it's on screen.
pub fn widget_of(layout: &MenuLayout, l: usize, item: usize) -> Option<usize> {
    let list = layout.lists.lists.get(l)?;
    if item < list.position || item >= list.position + list.rows {
        return None;
    }
    let name = format!("{}[{}]", list.name, item - list.position);
    layout.widgets.iter().position(|w| w.name == name)
}

/// Put new texts in list `l`, scrolled to the top with nothing chosen
/// (what the original's pages do each time they fill a list).
pub fn set_items(layout: &mut MenuLayout, l: usize, items: Vec<String>) {
    let Some(list) = layout.lists.lists.get_mut(l) else {
        return;
    };
    list.items = items;
    list.position = 0;
    list.selected = None;
    refresh(layout);
}

/// Scroll list `l` so its first row shows item `position` (kept in range,
/// as the original's `ScrollListTo` does). Returns whether it moved.
pub fn scroll_to(layout: &mut MenuLayout, l: usize, position: i32) -> bool {
    let Some(list) = layout.lists.lists.get_mut(l) else {
        return false;
    };
    let last = list.items.len().saturating_sub(list.rows) as i32;
    let p = position.clamp(0, last.max(0)) as usize;
    if p == list.position {
        return false;
    }
    list.position = p;
    refresh(layout);
    true
}

/// Scroll just enough to show `item`, and return its row button.
pub fn show_item(layout: &mut MenuLayout, l: usize, item: usize) -> Option<usize> {
    let list = layout.lists.lists.get(l)?;
    if item >= list.items.len() {
        return None;
    }
    let (pos, rows) = (list.position, list.rows);
    if item < pos {
        scroll_to(layout, l, item as i32);
    } else if item >= pos + rows {
        scroll_to(layout, l, (item + 1 - rows) as i32);
    }
    widget_of(layout, l, item)
}

/// Show each list's current items on its rows (empty rows are hidden) and
/// move the scroll bars' handles to match.
pub fn refresh(layout: &mut MenuLayout) {
    let lists = layout.lists.lists.clone();
    for list in &lists {
        for row in 0..list.rows {
            let name = format!("{}[{row}]", list.name);
            let item = list.items.get(list.position + row);
            if let Some(w) = layout.widget_mut(&name) {
                w.hidden = item.is_none();
                if let Kind::Button(b) = &mut w.kind {
                    let text = item.cloned().unwrap_or_default();
                    b.blurred.text = text.clone();
                    b.focused.text = text;
                }
            }
        }
    }
    let bars = layout.lists.bars.clone();
    for (bar, list) in bars.iter().zip(lists.iter()) {
        let track = bar.bottom - bar.top;
        let (len, rows) = (list.items.len(), list.rows.max(1));
        let size = if len <= rows {
            track
        } else {
            (track * rows as f32 / len as f32).max(bar.min_size[1])
        };
        let travel = track - size;
        let along = if len > rows {
            list.position as f32 / (len - rows) as f32
        } else {
            0.0
        };
        let centre = bar.top + size / 2.0 + travel * along;
        if let Some(w) = layout.widget_mut(&bar.name) {
            if let Kind::Sprite(s) = &mut w.kind {
                s.pos = [bar.x, centre];
                s.scale = [bar.min_size[0], size];
            }
        }
    }
}

/// What a click on scroll bar `b`'s track at height `y` does: -1 for
/// "page up" (above the handle), 1 for "page down", 0 on the handle.
pub fn bar_click(layout: &MenuLayout, b: usize, y: f32) -> i32 {
    let Some(bar) = layout.lists.bars.get(b) else {
        return 0;
    };
    let Some(Kind::Sprite(s)) = layout.widget(&bar.name).map(|w| &w.kind) else {
        return 0;
    };
    let half = s.scale[1] / 2.0;
    if y < s.pos[1] - half {
        -1
    } else if y > s.pos[1] + half {
        1
    } else {
        0
    }
}

/// The box of scroll bar `b`'s track, for the mouse: x0, y0, x1, y1.
pub fn bar_track(layout: &MenuLayout, b: usize) -> Option<[f32; 4]> {
    let bar = layout.lists.bars.get(b)?;
    let half = bar.min_size[0] / 2.0;
    Some([bar.x - half, bar.top, bar.x + half, bar.bottom])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Button, Sounds, Sprite, Widget};

    fn row(name: &str) -> Widget {
        Widget {
            name: name.into(),
            pass: 3,
            hidden: true,
            platform: 0,
            kind: Kind::Button(Button::default()),
        }
    }

    fn page(items: usize) -> MenuLayout {
        let mut widgets: Vec<Widget> = (0..3).map(|r| row(&format!("List[{r}]"))).collect();
        widgets.push(Widget {
            name: "Bar".into(),
            pass: 2,
            hidden: false,
            platform: 0,
            kind: Kind::Sprite(Sprite::default()),
        });
        let mut l = MenuLayout {
            class: "Test".into(),
            widgets,
            skipped: vec![],
            sounds: Sounds::default(),
            background_movie: String::new(),
            music: None,
            lists: Lists {
                lists: vec![StringList {
                    name: "List".into(),
                    rows: 3,
                    ..Default::default()
                }],
                bars: vec![ScrollBar {
                    name: "Bar".into(),
                    x: 0.9,
                    top: 0.5,
                    bottom: 0.8,
                    min_size: [0.02, 0.075],
                }],
                wheel_areas: vec![],
            },
        };
        set_items(&mut l, 0, (0..items).map(|i| format!("item {i}")).collect());
        l
    }

    fn text(l: &MenuLayout, w: usize) -> String {
        match &l.widgets[w].kind {
            Kind::Button(b) => b.blurred.text.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn rows_show_items_and_hide_when_empty() {
        let l = page(2);
        assert_eq!(text(&l, 0), "item 0");
        assert_eq!(text(&l, 1), "item 1");
        assert!(!l.widgets[1].hidden);
        assert!(l.widgets[2].hidden);
        assert_eq!(item_of(&l, 1), Some(1));
        assert_eq!(item_of(&l, 2), None);
    }

    #[test]
    fn scrolling_stays_in_range_and_moves_the_handle() {
        let mut l = page(9);
        let handle = |l: &MenuLayout| match &l.widgets[3].kind {
            Kind::Sprite(s) => (s.pos[1], s.scale[1]),
            _ => panic!(),
        };
        let (top_y, size) = handle(&l);
        assert!(
            (size - 0.1).abs() < 1e-5,
            "3 of 9 rows: a third of the track"
        );
        assert!((top_y - 0.55).abs() < 1e-5);
        assert!(!scroll_to(&mut l, 0, -4));
        assert!(scroll_to(&mut l, 0, 100));
        assert_eq!(l.lists.lists[0].position, 6);
        assert_eq!(text(&l, 0), "item 6");
        let (bottom_y, _) = handle(&l);
        assert!((bottom_y - 0.75).abs() < 1e-5);
        assert_eq!(bar_click(&l, 0, 0.6), -1);
        assert_eq!(bar_click(&l, 0, 0.76), 0);
    }

    #[test]
    fn showing_an_item_scrolls_only_as_far_as_needed() {
        let mut l = page(9);
        assert_eq!(show_item(&mut l, 0, 4), Some(2));
        assert_eq!(l.lists.lists[0].position, 2);
        assert_eq!(show_item(&mut l, 0, 3), Some(1));
        assert_eq!(l.lists.lists[0].position, 2);
        assert_eq!(show_item(&mut l, 0, 0), Some(0));
        assert_eq!(l.lists.lists[0].position, 0);
        assert_eq!(widget_of(&l, 0, 5), None);
    }

    #[test]
    fn a_short_list_fills_the_track() {
        let l = page(2);
        match &l.widgets[3].kind {
            Kind::Sprite(s) => assert!((s.scale[1] - 0.3).abs() < 1e-5),
            _ => panic!(),
        }
    }
}
