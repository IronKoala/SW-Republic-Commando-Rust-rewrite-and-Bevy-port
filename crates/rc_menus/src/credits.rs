//! The credits page (`XInterfaceCTMenus.CTCreditsMenu`): the names roll
//! up the screen, then the page closes by itself.
//!
//! The lines are the install's `GameData/System/Credits.int` (section
//! `[Credits]`); how fast they move, how far apart they are and how they
//! look are the class's own defaults (`ScrollSpeed`, `YLineOffset`, the
//! `MenuCreditLineBuffer[25]` text widgets in the `CreditsLineTemplate`
//! style). What the class's script does (read from the install's
//! `XInterfaceCTMenus.u` with `strings -n1`; rewritten here by hand,
//! nothing copied):
//!
//! - `Init` reads `CreditsLine[0]`, `[1]`, ... in turn. Where a number has
//!   no `CreditsLine`, it tries `CreditsLine_PC` (used on a PC) and then
//!   `CreditsLine_Xbox` (skipped on a PC, but still counted). The first
//!   number with none of the three (or only empty ones) ends the list.
//! - A line `Scale=<n>` isn't shown: it sets the size of the lines after
//!   it (starting at 1.0).
//! - Every other line gets a place below the screen: the first at 1.01
//!   (the screen runs from 0 at the top to 1 at the bottom), each next one
//!   `YLineOffset` lower.
//! - Each frame (`Tick`, only while the page is the one on screen) every
//!   line moves up by `ScrollSpeed` times the seconds passed. The lines
//!   now between -0.05 and 1.05 are put, in order, into the 25 text
//!   widgets (text, height, size); the widgets left over hide.
//! - Once the last line has moved above -0.05 the page closes, as BACK
//!   does: back to EXTRAS when it was opened from there (`FROMEXTRAS`),
//!   else (the end of the game) out to the main menu.

use crate::layout::{Kind, MenuLayout};

/// The page's class.
pub const CREDITS_MENU: &str = "XInterfaceCTMenus.CTCreditsMenu";

/// How many text widgets the page draws the lines with
/// (`MENU_CREDIT_LINE_BUFFER_SIZE`).
pub const BUFFER_SIZE: usize = 25;

/// Where the first line starts (`1.01 + yOffset`).
const FIRST_LINE_Y: f32 = 1.01;
/// Lines show between these heights.
const SHOWN_FROM: f32 = -0.05;
const SHOWN_TO: f32 = 1.05;

/// One line of the credits.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    /// Height of the line on the screen (0 top, 1 bottom).
    pub y: f32,
    pub scale: f32,
}

/// The rolling credits.
#[derive(Debug, Clone, PartialEq)]
pub struct Roll {
    pub lines: Vec<Line>,
    /// Screen heights a second (`ScrollSpeed`).
    pub speed: f32,
    /// The page has closed itself (`bClosed`).
    pub closed: bool,
}

/// The `[Credits]` section's values by key, from the text of
/// `Credits.int`, without their quotes.
pub fn read_section(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            inside = line.eq_ignore_ascii_case("[Credits]");
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        out.push((key.trim().to_string(), value.to_string()));
    }
    out
}

impl Roll {
    /// Build the roll the way the original's `Init` does, from the
    /// `[Credits]` values (`read_section`), with the class's `ScrollSpeed`
    /// and `YLineOffset`.
    pub fn new(section: &[(String, String)], speed: f32, line_gap: f32) -> Self {
        let get = |key: &str| {
            section
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty())
        };
        let mut lines = Vec::new();
        let mut gap = 0.0;
        let mut scale = 1.0;
        for i in 0.. {
            let value = match get(&format!("CreditsLine[{i}]")) {
                Some(v) => Some(v),
                None => match get(&format!("CreditsLine_PC[{i}]")) {
                    Some(v) => Some(v),
                    // An Xbox-only line is counted but not shown on a PC.
                    None => match get(&format!("CreditsLine_Xbox[{i}]")) {
                        Some(_) => continue,
                        None => None,
                    },
                },
            };
            let Some(value) = value else { break };
            if let Some(n) = value.strip_prefix("Scale=") {
                // UnrealScript's float(): a number that doesn't read is 0.
                scale = n.trim().parse().unwrap_or(0.0);
                continue;
            }
            lines.push(Line {
                text: value.to_string(),
                y: FIRST_LINE_Y + gap,
                scale,
            });
            gap += line_gap;
        }
        Roll {
            lines,
            speed,
            closed: false,
        }
    }

    /// Move the lines up for `seconds` (the original's `Tick`). Returns
    /// true the one time the roll ends, when the page should close.
    pub fn step(&mut self, seconds: f32) -> bool {
        if self.closed {
            return false;
        }
        for l in &mut self.lines {
            l.y -= self.speed * seconds;
        }
        // The original looks at the last line's height; with no lines
        // that is 0, so an empty roll never closes by itself (BACK still
        // works). Same here.
        let last = self.lines.last().map_or(0.0, |l| l.y);
        if last < SHOWN_FROM {
            self.closed = true;
            return true;
        }
        false
    }

    /// The lines on screen now, in order, at most [`BUFFER_SIZE`].
    pub fn shown(&self) -> impl Iterator<Item = &Line> {
        self.lines
            .iter()
            .filter(|l| l.y > SHOWN_FROM && l.y < SHOWN_TO)
            .take(BUFFER_SIZE)
    }

    /// Put the lines on screen now into the page's 25 text widgets, and
    /// hide the rest.
    pub fn fill(&self, layout: &mut MenuLayout) {
        let mut shown = self.shown();
        for i in 0..BUFFER_SIZE {
            let Some(w) = layout.widget_mut(&format!("MenuCreditLineBuffer[{i}]")) else {
                continue;
            };
            match (shown.next(), &mut w.kind) {
                (Some(l), Kind::Text(t)) => {
                    t.text = l.text.clone();
                    t.pos[1] = l.y;
                    t.scale = [l.scale, l.scale];
                    w.hidden = false;
                }
                _ => w.hidden = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INT: &str = "[Other]\nCreditsLine[0]=\"WRONG\"\n\n[Credits]\n\
CreditsLine[0]=\"Scale=1.0\"\r\nCreditsLine[1]=\"GAME DESIGN\"\r\n\
CreditsLine[2]=\"Scale=0.8\"\r\nCreditsLine[3]=\"THE TEAM\"\r\n\r\n\
CreditsLine[4]=\" \"\r\nCreditsLine_Xbox[5]=\"XBOX ONLY\"\r\n\
CreditsLine_PC[6]=\"PC ONLY\"\r\nCreditsLine[8]=\"AFTER A GAP\"\r\n";

    fn roll() -> Roll {
        Roll::new(&read_section(INT), 0.175, 0.05)
    }

    #[test]
    fn reads_lines_like_the_original_init() {
        let r = roll();
        let texts: Vec<&str> = r.lines.iter().map(|l| l.text.as_str()).collect();
        // Scale lines aren't shown, the Xbox line is skipped, and the list
        // ends at the first missing number (7).
        assert_eq!(texts, ["GAME DESIGN", "THE TEAM", " ", "PC ONLY"]);
        assert_eq!(r.lines[0].scale, 1.0);
        assert_eq!(r.lines[1].scale, 0.8);
        assert!((r.lines[0].y - 1.01).abs() < 1e-6);
        assert!((r.lines[3].y - 1.16).abs() < 1e-6);
    }

    #[test]
    fn rolls_up_and_closes_after_the_last_line() {
        let mut r = roll();
        // The first line starts just under the bottom edge, already in the
        // shown band (under 1.05).
        assert_eq!(r.shown().count(), 1);
        assert!(!r.step(0.1));
        // 0.0175 up: the first line (1.01 -> 0.9925) is on screen; so is
        // the second (1.06 -> 1.0425), still under 1.05.
        assert_eq!(r.shown().count(), 2);
        // The last line starts at 1.16; it must pass -0.05: 1.21 / 0.175 s.
        let mut t: f32 = 0.1;
        let mut closed = false;
        while t < 10.0 && !closed {
            closed = r.step(0.1);
            t += 0.1;
        }
        assert!(closed);
        assert!((t - 1.21 / 0.175).abs() < 0.11, "closed after {t} s");
        assert!(!r.step(0.1), "closes once");
    }
}
