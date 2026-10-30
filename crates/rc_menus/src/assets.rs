//! A cache of decoded textures, fonts and sounds on top of `Library`, with
//! failures remembered (and logged once) so a missing picture doesn't stop
//! the menu.

use crate::layout::{Kind, MenuLayout};
use crate::library::{FontData, Library};
use crate::quads::Sizes;
use crate::render::Textures;
use rc_image::Rgba;
use std::collections::HashMap;

pub struct Assets {
    pub library: Library,
    textures: HashMap<String, Option<Rgba>>,
    fonts: HashMap<String, Option<FontData>>,
    /// One line per thing that couldn't be read, for the log.
    pub problems: Vec<String>,
}

impl Assets {
    pub fn new(library: Library) -> Assets {
        Assets {
            library,
            textures: HashMap::new(),
            fonts: HashMap::new(),
            problems: Vec::new(),
        }
    }

    pub fn texture(&mut self, path: &str) -> Option<&Rgba> {
        let key = path.to_ascii_lowercase();
        if !self.textures.contains_key(&key) {
            let t = match self.library.texture(path) {
                Ok(t) => Some(t),
                Err(e) => {
                    self.problems.push(format!("texture {path}: {e}"));
                    None
                }
            };
            self.textures.insert(key.clone(), t);
        }
        self.textures.get(&key).and_then(|t| t.as_ref())
    }

    pub fn font_data(&mut self, path: &str) -> Option<FontData> {
        let key = path.to_ascii_lowercase();
        if !self.fonts.contains_key(&key) {
            let f = match self.library.font(path) {
                Ok(f) => Some(f),
                Err(e) => {
                    self.problems.push(format!("font {path}: {e}"));
                    None
                }
            };
            self.fonts.insert(key.clone(), f);
        }
        self.fonts.get(&key).cloned().flatten()
    }

    /// Every texture path a menu draws (sprites, button backgrounds and
    /// font pages), for loading up front.
    pub fn textures_of(&mut self, layout: &MenuLayout) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |p: &Option<String>| {
            if let Some(p) = p {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(p)) {
                    out.push(p.clone());
                }
            }
        };
        let mut fonts = Vec::new();
        for w in &layout.widgets {
            match &w.kind {
                Kind::Sprite(s) => add(&s.texture),
                Kind::Text(t) => fonts.push(t.font.clone()),
                Kind::Button(b) => {
                    add(&b.background_blurred.texture);
                    add(&b.background_focused.texture);
                    add(&b.under_blurred.texture);
                    add(&b.under_focused.texture);
                    fonts.push(b.blurred.font.clone());
                    fonts.push(b.focused.font.clone());
                }
            }
        }
        for f in fonts.into_iter().flatten() {
            if let Some(fd) = self.font_data(&f) {
                for p in fd.pages {
                    add(&Some(p));
                }
            }
        }
        out
    }
}

impl Sizes for Assets {
    fn texture_size(&mut self, path: &str) -> Option<(f32, f32)> {
        self.texture(path)
            .map(|t| (t.width as f32, t.height as f32))
    }

    fn font(&mut self, path: &str) -> Option<FontData> {
        self.font_data(path)
    }
}

impl Textures for Assets {
    fn get(&mut self, path: &str) -> Option<&Rgba> {
        self.texture(path)
    }
}
