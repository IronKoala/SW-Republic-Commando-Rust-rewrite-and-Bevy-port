//! The title card (`XInterfaceCTMenus.CTTitleCardMenu`): the lines of
//! text that fade in and out over black before a campaign starts ("WAR
//! HAS COME TO THE GALAXY..." before Geonosis).
//!
//! A level script opens it with `ACTION_ShowTitleCardScreen` (CTGame.u),
//! whose `Args` names the card's text file (`GEO_TitleCard` is
//! `GameData/System/geo_titlecard.int`). What the original does, read from
//! the classes' script in the install (rewritten here by hand, nothing
//! copied):
//!
//! - `ACTION_ShowTitleCardScreen` calls the player's `ShowTitleCardMenu`
//!   (Engine.u's `PlayerController`): it pauses the game if nothing has
//!   paused it already, and opens the menu. The script goes straight on
//!   to its next action, which then waits for as long as the game is
//!   paused.
//! - `Init` takes the first word of `Args` that isn't `RESET` as the card
//!   file. From its `[TitleCard]` section it reads `BackgroundMusic`, then
//!   for entry 0, 1, 2, ... `FadeInStartTime[n]`, `FullOnStartTime[n]`,
//!   `FadeOutStartTime[n]`, `FadeOutEndTime[n]` and `Text[n]`; the first
//!   missing or empty one ends the list. The class has 10 text widgets
//!   (`TextEntries[0..9]`, all at the same place, drawn in the
//!   `TitleCardTextTemplate` style).
//! - Each frame (`Tick`, while the card is open and on screen) the seconds
//!   are added up, and every entry gets: hidden (alpha 0) before its fade
//!   in; alpha rising from 0 to 255 between fade in and full on; 255 until
//!   fade out; falling to 0 by fade out end; hidden after. Alpha is turned
//!   into a whole number by cutting off the fraction.
//! - Once every entry's fade out has ended, the card closes and the game
//!   is unpaused. Back does nothing (`HandleInputBack` is empty).
//! - The music plays during the level because the class sets
//!   `bBackgroundMusicDuringLevel` (Engine.u's `ManageMenuBackgroundMusic`).

use crate::layout::{Kind, MenuLayout};

/// The class `ACTION_ShowTitleCardScreen` opens unless the map names
/// another (its `MenuClass` default in CTGame.u).
pub const TITLE_CARD_MENU: &str = "XInterfaceCTMenus.CTTitleCardMenu";

/// How many text widgets the class has (`MAX_TEXT_ENTRIES`).
pub const MAX_TEXT_ENTRIES: usize = 10;

/// One line of the card and its times, in seconds from the card opening.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub fade_in_start: f32,
    pub full_on_start: f32,
    pub fade_out_start: f32,
    pub fade_out_end: f32,
    pub text: String,
}

/// An entry's look at a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub hidden: bool,
    pub alpha: u8,
}

/// The title card while it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The card file's name, as `Args` gives it (`GEO_TitleCard`).
    pub file: String,
    /// The piece of music to play, as an object path
    /// (`GEO_Music.GEO_episodes.musGEO_titleCard_ep`), if any.
    pub music: Option<String>,
    pub entries: Vec<Entry>,
    /// Seconds the card has been on screen (`ElapsedTextTime`).
    pub elapsed: f32,
    /// What each entry looks like now. Every one starts hidden, as the
    /// class's widgets do.
    pub looks: Vec<Look>,
    /// The card has closed itself (`bClosed`).
    pub closed: bool,
}

/// The card file named by `Args`: its first word that isn't `RESET`
/// (UnrealScript's `ParseToken` in a loop; the compare is case-sensitive).
/// Empty when there is none.
pub fn card_file(args: &str) -> String {
    args.split_whitespace()
        .find(|w| *w != "RESET")
        .unwrap_or("")
        .to_string()
}

/// The `[TitleCard]` section's values by key, from a card file's text,
/// without their quotes.
pub fn read_section(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            inside = line.eq_ignore_ascii_case("[TitleCard]");
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

/// UnrealScript's `float(string)`: the number at the start of the text,
/// 0 when there is none.
fn unreal_float(s: &str) -> f32 {
    let s = s.trim_start();
    let mut end = 0;
    for (i, c) in s.char_indices() {
        let ok = c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0);
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    let mut part = &s[..end];
    while !part.is_empty() {
        if let Ok(f) = part.parse::<f32>() {
            return f;
        }
        part = &part[..part.len() - 1];
    }
    0.0
}

impl Card {
    /// Read the card the way the original's `Init` does, from the
    /// `[TitleCard]` values of its file (`read_section`).
    pub fn new(file: &str, section: &[(String, String)]) -> Card {
        let get = |key: &str| {
            section
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty())
        };
        let music = get("BackgroundMusic").map(str::to_string);
        let mut entries = Vec::new();
        // The class's array holds 10; the original would run past it on a
        // longer file (UnrealScript then logs and ignores the write), so
        // the 11th and later never show.
        for n in 0..MAX_TEXT_ENTRIES {
            let mut times = [0.0; 4];
            let mut all = true;
            for (k, key) in [
                "FadeInStartTime",
                "FullOnStartTime",
                "FadeOutStartTime",
                "FadeOutEndTime",
            ]
            .iter()
            .enumerate()
            {
                match get(&format!("{key}[{n}]")) {
                    Some(v) => times[k] = unreal_float(v),
                    None => {
                        all = false;
                        break;
                    }
                }
            }
            let text = if all {
                get(&format!("Text[{n}]"))
            } else {
                None
            };
            let Some(text) = text else { break };
            entries.push(Entry {
                fade_in_start: times[0],
                full_on_start: times[1],
                fade_out_start: times[2],
                fade_out_end: times[3],
                text: text.to_string(),
            });
        }
        let looks = vec![
            Look {
                hidden: true,
                alpha: 0
            };
            entries.len()
        ];
        Card {
            file: file.to_string(),
            music,
            entries,
            elapsed: 0.0,
            looks,
            closed: false,
        }
    }

    /// When the card closes by itself: the latest fade out end (0 with no
    /// entries: it closes on its first frame).
    pub fn length(&self) -> f32 {
        self.entries
            .iter()
            .map(|e| e.fade_out_end)
            .fold(0.0, f32::max)
    }

    /// One frame of `seconds` (the original's `Tick`). Returns true the
    /// one time the card closes, when the game should be unpaused.
    pub fn step(&mut self, seconds: f32) -> bool {
        if self.closed {
            return false;
        }
        self.elapsed += seconds;
        let t = self.elapsed;
        let mut all_elapsed = true;
        for (e, look) in self.entries.iter().zip(self.looks.iter_mut()) {
            if t < e.fade_out_end {
                all_elapsed = false;
                if t < e.fade_in_start {
                    *look = Look {
                        hidden: true,
                        alpha: 0,
                    };
                } else {
                    look.hidden = false;
                    // int() cuts off the fraction; DrawColor.A is a byte.
                    let a = if t >= e.fade_out_start {
                        255.0
                            - 255.0 * ((t - e.fade_out_start) / (e.fade_out_end - e.fade_out_start))
                    } else if t >= e.full_on_start {
                        255.0
                    } else {
                        255.0 * ((t - e.fade_in_start) / (e.full_on_start - e.fade_in_start))
                    };
                    look.alpha = (a as i32) as u8;
                }
            } else {
                look.hidden = true;
            }
        }
        if all_elapsed {
            self.closed = true;
            return true;
        }
        false
    }

    /// Put the entries' text and look into the page's `TextEntries`
    /// widgets.
    pub fn fill(&self, layout: &mut MenuLayout) {
        for i in 0..MAX_TEXT_ENTRIES {
            let Some(w) = layout.widget_mut(&format!("TextEntries[{i}]")) else {
                continue;
            };
            match (self.entries.get(i), self.looks.get(i), &mut w.kind) {
                (Some(e), Some(l), Kind::Text(t)) => {
                    t.text = e.text.clone();
                    t.color[3] = l.alpha;
                    w.hidden = l.hidden;
                }
                _ => w.hidden = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INT: &str = "[Other]\nText[0]=\"WRONG\"\n\n[TitleCard]\r\n\r\n\
BackgroundMusic=\"GEO_Music.GEO_episodes.musGEO_titleCard_ep\"\r\n\r\n\
FadeInStartTime[0]=1\r\nFullOnStartTime[0]=2\r\nFadeOutStartTime[0]=6\r\n\
FadeOutEndTime[0]=7\r\nText[0]=\"WAR HAS COME\"\r\n\r\n\
FadeInStartTime[1]=8\r\nFullOnStartTime[1]=9\r\nFadeOutStartTime[1]=18\r\n\
FadeOutEndTime[1]=19\r\nText[1]=\"SECOND\"\r\n\
FadeInStartTime[2]=20\r\nFullOnStartTime[2]=21\r\nText[2]=\"NO FADE OUT\"\r\n\
FadeInStartTime[3]=30\r\nFullOnStartTime[3]=31\r\nFadeOutStartTime[3]=32\r\n\
FadeOutEndTime[3]=33\r\nText[3]=\"AFTER A GAP\"\r\n";

    fn card() -> Card {
        Card::new("GEO_TitleCard", &read_section(INT))
    }

    #[test]
    fn card_file_skips_reset() {
        assert_eq!(card_file("GEO_TitleCard"), "GEO_TitleCard");
        assert_eq!(card_file("RESET  RAS_TitleCard x"), "RAS_TitleCard");
        assert_eq!(card_file(""), "");
        // The original's compare is case-sensitive.
        assert_eq!(card_file("reset A"), "reset");
    }

    #[test]
    fn reads_entries_like_the_original_init() {
        let c = card();
        assert_eq!(
            c.music.as_deref(),
            Some("GEO_Music.GEO_episodes.musGEO_titleCard_ep")
        );
        // Entry 2 has no fade out times, so the list ends there.
        let texts: Vec<&str> = c.entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["WAR HAS COME", "SECOND"]);
        assert_eq!(c.entries[1].fade_out_end, 19.0);
        assert_eq!(c.length(), 19.0);
        assert!(c.looks.iter().all(|l| l.hidden));
    }

    #[test]
    fn fades_in_holds_and_fades_out() {
        let mut c = card();
        assert!(!c.step(0.5));
        assert_eq!(
            c.looks[0],
            Look {
                hidden: true,
                alpha: 0
            }
        );
        assert!(!c.step(1.0)); // 1.5 s: half way up
        assert_eq!(
            c.looks[0],
            Look {
                hidden: false,
                alpha: 127
            }
        );
        assert!(!c.step(1.0)); // 2.5 s: full
        assert_eq!(c.looks[0].alpha, 255);
        assert!(!c.step(4.25)); // 6.75 s: three quarters down
        assert_eq!(
            c.looks[0],
            Look {
                hidden: false,
                alpha: 63
            }
        );
        assert!(!c.step(0.5)); // 7.25 s: gone; the second not yet in
        assert!(c.looks[0].hidden);
        assert!(c.looks[1].hidden);
        assert!(!c.step(11.0)); // 18.25 s
        assert_eq!(
            c.looks[1],
            Look {
                hidden: false,
                alpha: 191
            }
        );
        assert!(c.step(1.0), "closes once the last fade out ends");
        assert!(c.closed);
        assert!(!c.step(1.0), "closes once");
    }

    #[test]
    fn floats_read_like_unrealscript() {
        assert_eq!(unreal_float("1"), 1.0);
        assert_eq!(unreal_float(" 2.5s"), 2.5);
        assert_eq!(unreal_float("x"), 0.0);
        assert_eq!(unreal_float("-3"), -3.0);
    }
}
