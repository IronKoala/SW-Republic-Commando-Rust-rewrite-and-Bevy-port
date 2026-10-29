//! Reads `Font` objects: a list of letter rectangles and the texture pages
//! they are cut from.
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov), `Unreal/UnrealMaterial/UnFont.h` (UE2 `UFont`, `FFontCharacter`),
//! and checked against the game's own `WarfareFonts.utx` and `OrbitFonts.utx`
//! (see `docs/ORIGINAL_MENUS.md`). Written from those descriptions; no code
//! copied.

use crate::class::export_data;
use crate::properties::{self, Property};
use crate::{Export, Package, Reader, Result};

/// One letter: a rectangle on one of the font's texture pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub u: i32,
    pub v: i32,
    pub width: i32,
    pub height: i32,
    /// Which of `Font::pages` the letter is on.
    pub page: u8,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub properties: Vec<Property>,
    /// Letters by character code (index 65 is 'A'). Unused codes have a zero
    /// size.
    pub glyphs: Vec<Glyph>,
    /// Object references to the texture pages.
    pub pages: Vec<i32>,
    /// Extra space between letters, in pixels.
    pub kerning: i32,
    /// Bytes left over after what we read. Expected to be small (the
    /// character remapping table, unused by these fonts).
    pub leftover: usize,
}

impl Font {
    /// The glyph for a character, if the font has one with a size.
    pub fn glyph(&self, c: char) -> Option<Glyph> {
        let g = *self.glyphs.get(c as usize)?;
        (g.width > 0 && g.height > 0).then_some(g)
    }

    /// The tallest letter, used as the line height.
    pub fn line_height(&self) -> i32 {
        self.glyphs.iter().map(|g| g.height).max().unwrap_or(0)
    }
}

pub fn read_font(pkg: &Package, data: &[u8], export: &Export) -> Result<Font> {
    let object = export_data(data, export)?;
    let mut r = Reader::new(object);
    let properties = properties::read_properties(pkg, &mut r)?;
    let count = r.compact_index()?;
    if !(0..=65536).contains(&count) {
        return r.invalid(format!("font letter count {count} is implausible"));
    }
    let mut glyphs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        glyphs.push(Glyph {
            u: r.i32()?,
            v: r.i32()?,
            width: r.i32()?,
            height: r.i32()?,
            page: r.u8()?,
        });
    }
    let page_count = r.compact_index()?;
    if !(0..=256).contains(&page_count) {
        return r.invalid(format!("font page count {page_count} is implausible"));
    }
    let pages = (0..page_count)
        .map(|_| r.compact_index())
        .collect::<Result<Vec<_>>>()?;
    let kerning = if object.len() - r.pos() >= 4 {
        r.i32()?
    } else {
        0
    };
    for g in &glyphs {
        if g.page as i32 >= page_count && g.width > 0 {
            return r.invalid(format!("letter on page {} of {page_count}", g.page));
        }
    }
    Ok(Font {
        properties,
        glyphs,
        pages,
        kerning,
        leftover: object.len() - r.pos(),
    })
}
