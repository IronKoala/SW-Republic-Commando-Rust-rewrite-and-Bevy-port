//! The original's loading screens. Everything here was read from the
//! player's install (nothing copied):
//!
//! - which screen a map gets: `GameData/System/LevelLoadingInfo.int`
//!   (`MapName`, `LoadingMenuClass`, `LoadingPic`, `LoadingTitle`,
//!   `LoadingText`, `LoadingShowHints`), read the way `Engine.u`'s
//!   `GameEngine.InitLevelLoadingInfo` reads it, and chosen the way
//!   `UGameEngine::LoadMap` in `Engine.dll` chooses ([`choose`]);
//! - the two screens: `XInterfaceCTMenus.CTLoading` (black, the mission
//!   name, a bar and "LOADING") and `XInterfaceCTMenus.CTLoadingInfo` (the
//!   briefing: a picture, the mission name and text, a bar and
//!   "LOADING"), whose pictures, fonts and places are the classes' own
//!   default values;
//! - what they do: their script (`Init`, `SetInfoOptions`,
//!   `UpdateLoadingProgress`, `PauseOnLoadingFinish`) and the `Console`'s
//!   `LoadingStarted` and `LoadingFinished` in `Engine.u`.
//!
//! On a PC the hints never show (`IsOnConsole()` is false, so both classes
//! set `bNoHints`). When a `CTLoadingInfo` screen has finished loading,
//! the original pauses the game and opens the same screen again with
//! "LOADINGDONE": the bar and "LOADING" go, the START button
//! (`AButton`) shows with the mouse pointer, and choosing it un-pauses
//! and closes the screen ([`finish`]). A `CTLoading` screen just closes.

use crate::Library;
use crate::layout::{Kind, MenuLayout};

/// The plain loading screen (`[Engine.GameEngine] LoadingMenuClass` in
/// the install's `System.ini`).
pub const LOADING_MENU: &str = "XInterfaceCTMenus.CTLoading";
/// The briefing loading screen (`LoadingInfoMenuClass`).
pub const LOADING_INFO_MENU: &str = "XInterfaceCTMenus.CTLoadingInfo";

/// What the START button calls in the original (`CTInfoBase.OnAButton`,
/// which `CTLoadingInfo` overrides to un-pause first).
pub const START_FUNCTION: &str = "OnAButton";

/// One map's row of `LevelLoadingInfo.int`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoadingInfo {
    pub map: String,
    pub menu_class: String,
    pub pic: String,
    pub title: String,
    pub text: String,
    pub show_hints: bool,
}

/// Read `LevelLoadingInfo.int` (already decoded to text). As in the
/// original, rows are read from index 0 up and reading stops at the first
/// index with no `MapName`.
pub fn parse_level_loading_info(text: &str) -> Vec<LoadingInfo> {
    let mut values: Vec<(String, usize, String)> = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[LevelLoadingInfo]");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((field, rest)) = key.trim().split_once('[') else {
            continue;
        };
        let Ok(i) = rest.trim_end_matches(']').parse::<usize>() else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        values.push((field.to_ascii_lowercase(), i, value.to_string()));
    }
    let get = |field: &str, i: usize| {
        values
            .iter()
            .rev()
            .find(|(f, j, _)| f == field && *j == i)
            .map(|v| v.2.clone())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for i in 0.. {
        let map = get("mapname", i);
        if map.is_empty() {
            break;
        }
        out.push(LoadingInfo {
            map,
            menu_class: get("loadingmenuclass", i),
            pic: get("loadingpic", i),
            title: get("loadingtitle", i),
            text: get("loadingtext", i),
            // `int(value) != 0`
            show_hints: get("loadingshowhints", i)
                .trim()
                .parse::<i64>()
                .unwrap_or(0)
                != 0,
        });
    }
    out
}

/// The two classes the engine falls back to, from the install's
/// `System.ini` (`[Engine.GameEngine]`).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineClasses {
    pub loading: String,
    pub loading_info: String,
}

/// Read `LoadingMenuClass` and `LoadingInfoMenuClass` from the
/// `[Engine.GameEngine]` section of a `System.ini`. `None` for a value
/// that isn't there.
pub fn engine_classes(ini: &str) -> (Option<String>, Option<String>) {
    let mut section = false;
    let (mut loading, mut info) = (None, None);
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.eq_ignore_ascii_case("[Engine.GameEngine]");
            continue;
        }
        if !section || line.starts_with(';') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "loadingmenuclass" => loading = Some(v),
            "loadinginfomenuclass" => info = Some(v),
            _ => {}
        }
    }
    (loading, info)
}

/// Which screen a map gets and what goes on it, as the original's
/// `UGameEngine::LoadMap` chooses (Engine.dll): the map's name without
/// its extension is looked up in the rows; for a row found, its own class
/// (or `LoadingInfoMenuClass` when it names none) with its picture,
/// title, text and hints; for a map with no row, `LoadingMenuClass` with
/// no picture, title or text and no hints.
pub fn choose(map: &str, rows: &[LoadingInfo], classes: &EngineClasses) -> LoadingInfo {
    let name = match map.rfind('.') {
        Some(i) if i > 0 => &map[..i],
        _ => map,
    };
    match rows.iter().find(|r| r.map.eq_ignore_ascii_case(name)) {
        Some(r) => {
            let mut r = r.clone();
            if r.menu_class.is_empty() {
                r.menu_class = classes.loading_info.clone();
            }
            r
        }
        None => LoadingInfo {
            map: name.to_string(),
            menu_class: classes.loading.clone(),
            ..Default::default()
        },
    }
}

/// Whether a class's `PauseOnLoadingFinish` returns true: `CTLoadingInfo`
/// and its children (by the class's family, as
/// `crate::library::ClassData::family` lists it). `Menu`'s own returns
/// false.
pub fn pauses_when_done(family: &[String]) -> bool {
    family
        .iter()
        .any(|c| c.eq_ignore_ascii_case(LOADING_INFO_MENU))
}

/// What happened while setting the screen up, for the log.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setup {
    /// The picture's full path, if one was named and found.
    pub picture: Option<String>,
    pub problems: Vec<String>,
}

fn hide(layout: &mut MenuLayout, name: &str, hidden: bool) -> bool {
    match layout.widget_mut(name) {
        Some(w) => {
            w.hidden = hidden;
            true
        }
        None => false,
    }
}

fn set_text(layout: &mut MenuLayout, name: &str, value: &str) -> bool {
    match layout.widget_mut(name).map(|w| &mut w.kind) {
        Some(Kind::Text(t)) => {
            t.text = value.to_string();
            true
        }
        _ => false,
    }
}

/// Set a freshly built loading screen up as the original's `Console`
/// does when loading starts: `Init("")`, then
/// `SetInfoOptions(Pic, Title, Text, "", ShowHints)`, on a PC.
/// `in_level` says whether a level (not the front end) was showing:
/// `MenuBase.Init` shows `InLevelBackgroundMovieReplacement` instead of
/// starting the background video when it was.
pub fn setup(
    layout: &mut MenuLayout,
    family: &[String],
    library: &mut Library,
    info: &LoadingInfo,
    in_level: bool,
) -> Setup {
    let mut out = Setup::default();
    // MenuBase.Init: a menu with a background video shows its
    // "in level" stand-in during a level; on the front end the video
    // starts instead (on these screens it would sit under their
    // opaque Background picture).
    if !layout.background_movie.is_empty() {
        hide(layout, "InLevelBackgroundMovieReplacement", !in_level);
    }
    set_progress(layout, 0.0);
    let is_info = family
        .iter()
        .any(|c| c.eq_ignore_ascii_case("XInterfaceCTMenus.CTInfoBase"));
    // Hints: on a PC `bNoHints` is always set.
    for n in ["HintBorder", "HintTitle", "HintText", "HintSeparator"] {
        hide(layout, n, true);
    }
    for i in 0..6 {
        hide(layout, &format!("HintPics[{i}]"), true);
    }
    if is_info {
        // CTInfoBase.SetInfoOptions: the picture by name, all of it;
        // the title and the text.
        let found = if info.pic.trim().is_empty() {
            None
        } else {
            library.resolve_loose(info.pic.trim())
        };
        match layout.widget_mut("InfoPic").map(|w| &mut w.kind) {
            Some(Kind::Sprite(s)) => match &found {
                Some(p) => {
                    s.texture = Some(p.clone());
                    s.coords = [0; 4];
                }
                None => {
                    // DynamicLoadObject of a missing or empty name gives
                    // no picture.
                    s.texture = None;
                    if !info.pic.trim().is_empty() {
                        out.problems
                            .push(format!("picture {:?} not found; none shows", info.pic));
                    }
                }
            },
            _ => out.problems.push("no InfoPic picture on the screen".into()),
        }
        out.picture = found;
        for (name, value) in [("InfoTitle", &info.title), ("InfoText", &info.text)] {
            if !set_text(layout, name, value) {
                out.problems.push(format!("no {name} text on the screen"));
            }
        }
        // CTLoadingInfo.Init("") on a PC: START stays hidden while loading.
        hide_start(layout, true);
    } else if !set_text(layout, "InfoBlurb", &info.title) {
        // CTLoading.SetInfoOptions: the title in InfoBlurb.
        out.problems.push("no InfoBlurb text on the screen".into());
    }
    out
}

/// `CTInfoBase.HideAButton` on a PC: only the START button itself
/// (`AButton`) shows or hides; the gamepad's icon and label stay hidden.
pub fn hide_start(layout: &mut MenuLayout, hidden: bool) -> bool {
    hide(layout, "AButtonIcon", true);
    hide(layout, "ALabel", true);
    hide(layout, "AButton", hidden)
}

/// `UpdateLoadingProgress`: the bar's `Value` (its `Low` and `High` are
/// set to 0 and 1 by both classes' `Init`). False if the screen has no
/// `LoadingBar`.
pub fn set_progress(layout: &mut MenuLayout, fraction: f32) -> bool {
    match layout.widget_mut("LoadingBar").map(|w| &mut w.kind) {
        Some(Kind::ProgressBar(p)) => {
            p.low = 0.0;
            p.high = 1.0;
            p.value = fraction;
            true
        }
        _ => false,
    }
}

/// The screen once loading has finished, for a class that pauses
/// ([`pauses_when_done`]): `CTLoadingInfo.Init("LOADINGDONE")` keeps the
/// picture, title and text (`TransferTransientElements`), hides the bar,
/// "LOADING" and the hints, and shows START. False if there is no START
/// button.
pub fn finish(layout: &mut MenuLayout) -> bool {
    hide(layout, "LoadingBar", true);
    hide(layout, "LoadingText", true);
    for n in ["HintTitle", "HintText", "HintSeparator"] {
        hide(layout, n, true);
    }
    for i in 0..6 {
        hide(layout, &format!("HintPics[{i}]"), true);
    }
    hide_start(layout, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INT: &str = "[LevelLoadingInfo]\r\n\r\nMapName[0]=\"GEO_01A\"\r\nLoadingMenuClass[0]=\"Pkg.Info\"\r\nLoadingPic[0]=\"Pics.One\"\r\nLoadingTitle[0]=\"FIRST\"\r\nLoadingText[0]=\"SOME TEXT.\"\r\nLoadingShowHints[0]=1\r\n\r\nMapName[1]=\"GEO_01B\"\r\nLoadingTitle[1]=\"SECOND\"\r\n\r\nMapName[3]=\"LOST\"\r\n";

    fn classes() -> EngineClasses {
        EngineClasses {
            loading: "Pkg.Plain".into(),
            loading_info: "Pkg.Info".into(),
        }
    }

    #[test]
    fn rows_read_until_the_first_gap() {
        let rows = parse_level_loading_info(INT);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "FIRST");
        assert!(rows[0].show_hints);
        assert_eq!(rows[1].menu_class, "");
        assert!(!rows[1].show_hints);
    }

    #[test]
    fn a_map_gets_its_row_or_the_plain_screen() {
        let rows = parse_level_loading_info(INT);
        let a = choose("geo_01a.ctm", &rows, &classes());
        assert_eq!(
            (a.menu_class.as_str(), a.pic.as_str()),
            ("Pkg.Info", "Pics.One")
        );
        // A row without a class gets LoadingInfoMenuClass.
        assert_eq!(choose("GEO_01B", &rows, &classes()).menu_class, "Pkg.Info");
        let other = choose("ras_01a", &rows, &classes());
        assert_eq!(other.menu_class, "Pkg.Plain");
        assert!(other.title.is_empty() && other.pic.is_empty() && !other.show_hints);
    }

    #[test]
    fn engine_classes_come_from_the_game_engine_section() {
        let ini = "[Engine.Engine]\nLoadingMenuClass=Wrong.One\n[Engine.GameEngine]\n;LoadingMenuClass=GUI.Old\nLoadingMenuClass=A.B\nLoadingInfoMenuClass=C.D\n";
        assert_eq!(
            engine_classes(ini),
            (Some("A.B".into()), Some("C.D".into()))
        );
    }
}
