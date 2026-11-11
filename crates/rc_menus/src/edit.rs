//! Boxes the player types into: the original's `MenuEditBox` (the name
//! box on the NEW PROFILE page).
//!
//! The struct and its comments are in the game's `XInterface.u`; what it
//! does is native code in `XInterface.dll`, read by disassembly (in a
//! scratch folder, never copied):
//!
//! - `AMenuBase::HandleInputKey(FMenuEditBox*, ...)`: a typed character is
//!   ignored below a space (except backspace) or above 255. Backspace
//!   takes the last character off. Otherwise nothing is added once the
//!   text has `MaxLength` characters (0: no limit); with `bNoSpaces` a
//!   space is refused; then `FilterMode` decides: `MBFM_Alpha` refuses
//!   digits, `MBFM_Numeric` refuses letters, `MBFM_AlphaNumeric` takes only
//!   letters, digits and spaces. "Letters" are A-Z, a-z and the
//!   Windows-1252 accented letters listed in [`is_letter`].
//! - `AMenuBase::HandleInputKeyRaw`: Escape is the menu's
//!   `HandleInputBack`; Enter calls the box's `OnSelect`, or, without one,
//!   moves focus to the nearest widget (below first).
//! - `AMenuBase::Tick(FMenuEditBox*, ...)`: every `TimePerCursorBlink`
//!   seconds the cursor turns on or off.
//! - `AMenuBase::DrawMenuEditBox`: only while the box has focus, the
//!   cursor is drawn after the text: the original's `WhiteTexture`, as wide
//!   and as high as the letter "A" in the box's font times `CursorScale`,
//!   its left edge at the text's right edge and centred on the text's
//!   middle, in the focused text's colour (see `quads.rs`).

use crate::value::V;

/// `MenuBoxFilterMode` from the game's `XInterface.u`.
pub const MBFM_NONE: u8 = 0;
pub const MBFM_ALPHA: u8 = 1;
pub const MBFM_NUMERIC: u8 = 2;
pub const MBFM_ALPHANUMERIC: u8 = 3;

/// What makes a button an edit box.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditBox {
    pub no_spaces: bool,
    pub filter: u8,
    /// Most characters; 0 for no limit.
    pub max_length: usize,
    pub min_length: usize,
    /// Seconds the cursor stays on, then off.
    pub blink: f32,
    /// The cursor's size as a part of the letter "A".
    pub cursor_scale: f32,
    /// The cursor shows now (it blinks).
    pub show_cursor: bool,
    /// Time since the cursor last turned on or off.
    pub since_blink: f32,
}

impl EditBox {
    pub fn from_value(v: &V) -> EditBox {
        EditBox {
            no_spaces: v.int("bNoSpaces") != 0,
            filter: v.byte("FilterMode"),
            max_length: v.int("MaxLength").max(0) as usize,
            min_length: v.int("MinLength").max(0) as usize,
            blink: v.float("TimePerCursorBlink"),
            cursor_scale: v.float("CursorScale"),
            show_cursor: true,
            since_blink: 0.0,
        }
    }

    /// The cursor's blinking (the original's `Tick`). Returns whether it
    /// turned on or off.
    pub fn tick(&mut self, seconds: f32) -> bool {
        if self.blink == 0.0 {
            return false;
        }
        self.since_blink += seconds;
        if self.since_blink < self.blink {
            return false;
        }
        self.since_blink = 0.0;
        self.show_cursor = !self.show_cursor;
        true
    }

    /// Type one character into `text` as the original does. Returns
    /// whether the text changed. `'\u{8}'` is backspace.
    pub fn type_char(&self, text: &mut String, c: char) -> bool {
        let code = c as u32;
        if (code < 0x20 && c != '\u{8}') || (code >= 0x100 && cp1252(c).is_none()) {
            return false;
        }
        if c == '\u{8}' {
            return text.pop().is_some();
        }
        if self.max_length != 0 && text.chars().count() >= self.max_length {
            return false;
        }
        if self.no_spaces && c.is_whitespace() {
            return false;
        }
        let letter = is_letter(c);
        let digit = c.is_ascii_digit();
        let ok = match self.filter {
            MBFM_ALPHA => !digit,
            MBFM_NUMERIC => !letter,
            MBFM_ALPHANUMERIC => letter || digit || c == ' ',
            _ => true,
        };
        if ok {
            text.push(c);
        }
        ok
    }
}

/// The few characters above 255 that Windows-1252 (the original's
/// character set) has in the 128-159 range and the original counts as
/// letters.
fn cp1252(c: char) -> Option<u8> {
    match c {
        'Š' => Some(0x8A),
        'Ž' => Some(0x8E),
        'š' => Some(0x9A),
        'ž' => Some(0x9E),
        _ => None,
    }
}

/// The letters `XInterface.dll` lets through, by their Windows-1252 code:
/// A-Z, a-z, 0xC0-0xCF, 0xD1-0xD6, 0xD8-0xDD, 0xDE, 0xE0-0xEF, 0xF1-0xF6,
/// 0xF8-0xFD, 0xFE, and Š, Ž, š, ž. (So not Ð, ð, ×, ÷, ß or ÿ.)
pub fn is_letter(c: char) -> bool {
    let code = match cp1252(c) {
        Some(b) => b as u32,
        None => c as u32,
    };
    matches!(code,
        0x41..=0x5A | 0x61..=0x7A
        | 0xC0..=0xCF | 0xD1..=0xD6 | 0xD8..=0xDE
        | 0xE0..=0xEF | 0xF1..=0xF6 | 0xF8..=0xFE
        | 0x8A | 0x8E | 0x9A | 0x9E)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_box() -> EditBox {
        // The NEW PROFILE page's NameBox: letters, digits and spaces, at
        // most 15.
        EditBox {
            filter: MBFM_ALPHANUMERIC,
            max_length: 15,
            ..Default::default()
        }
    }

    #[test]
    fn letters_digits_and_spaces_only() {
        let b = name_box();
        let mut t = String::new();
        for c in "Ab 1-_!éðß".chars() {
            b.type_char(&mut t, c);
        }
        assert_eq!(t, "Ab 1é");
    }

    #[test]
    fn stops_at_the_most_and_backspace_takes_one_off() {
        let b = name_box();
        let mut t = String::new();
        for c in "ABCDEFGHIJKLMNOPQ".chars() {
            b.type_char(&mut t, c);
        }
        assert_eq!(t, "ABCDEFGHIJKLMNO");
        assert!(b.type_char(&mut t, '\u{8}'));
        assert_eq!(t, "ABCDEFGHIJKLMN");
        let mut empty = String::new();
        assert!(!b.type_char(&mut empty, '\u{8}'));
    }

    #[test]
    fn the_cursor_blinks() {
        let mut b = EditBox {
            blink: 0.2,
            show_cursor: true,
            ..Default::default()
        };
        assert!(!b.tick(0.1));
        assert!(b.tick(0.1));
        assert!(!b.show_cursor);
        assert!(b.tick(0.25));
        assert!(b.show_cursor);
    }
}
