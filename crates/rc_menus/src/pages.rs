//! The original menus behind the main menu (pause, campaign, options and
//! so on), described as data: which class draws a page, and what each of
//! its buttons does. The list lives in `data/ui/original_menus.toml`, so a
//! mod can point a page at another class or rename a button's function.
//!
//! A page only uses its original menu when everything on it can work in
//! this port: the class reads, every widget is a kind we can draw, and
//! every button that can be chosen has a known command. Otherwise the game
//! keeps the placeholder page and logs why, so one run shows exactly what
//! is missing.

use crate::layout::{Kind, MenuLayout};
use serde::Deserialize;
use std::collections::BTreeMap;

/// What choosing a button on an original page asks the game to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageCmd {
    /// Close the pause menu and carry on playing.
    Resume,
    /// Load the current level again from its start.
    Restart,
    /// Open another page (by its name in `original_menus.toml`, such as
    /// `options`; the placeholder page is used if it has no original).
    Open(String),
    /// Go back to the page before.
    Back,
    /// Leave the level for the main menu.
    QuitToMenu,
    /// Ask "are you sure?", then quit the game.
    QuitGame,
    /// Start a map by name.
    Play(String),
    /// A button this port can't offer yet: greyed out, can't be chosen.
    Unavailable,
}

impl PageCmd {
    /// Read a command as written in the data file: `resume`, `restart`,
    /// `back`, `quit_to_menu`, `quit_game`, `unavailable`, `open:<page>`,
    /// `play:<map>`.
    pub fn parse(s: &str) -> Option<PageCmd> {
        let s = s.trim();
        if let Some(p) = s.strip_prefix("open:") {
            return Some(PageCmd::Open(p.trim().to_ascii_lowercase()));
        }
        if let Some(m) = s.strip_prefix("play:") {
            return Some(PageCmd::Play(m.trim().to_string()));
        }
        Some(match s.to_ascii_lowercase().as_str() {
            "resume" => PageCmd::Resume,
            "restart" => PageCmd::Restart,
            "back" => PageCmd::Back,
            "quit_to_menu" => PageCmd::QuitToMenu,
            "quit_game" => PageCmd::QuitGame,
            "unavailable" => PageCmd::Unavailable,
            _ => return None,
        })
    }
}

/// One page's entry in the data file, already read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PagePlan {
    /// The page's name (`paused`, `campaigns`, `options`...).
    pub page: String,
    /// The menu class, such as `XInterfaceCTMenus.CTPausePCMenu`.
    pub class: String,
    /// Button function name (the original's `OnSelect`, any case) to
    /// command.
    pub buttons: Vec<(String, PageCmd)>,
    /// What the back button (Escape, B) does on this page.
    pub back: Option<PageCmd>,
    /// Draw over the game (the pause menu) instead of over black.
    pub over_game: bool,
}

impl PagePlan {
    pub fn command(&self, on_select: &str) -> Option<&PageCmd> {
        self.buttons
            .iter()
            .find(|(f, _)| f.eq_ignore_ascii_case(on_select))
            .map(|(_, c)| c)
    }
}

/// The checked page: for each button the player can reach (index into
/// `layout.widgets`), its command.
#[derive(Debug, Clone, PartialEq)]
pub struct PageButtons {
    pub commands: Vec<(usize, PageCmd)>,
}

impl PageButtons {
    pub fn command(&self, widget: usize) -> Option<&PageCmd> {
        self.commands
            .iter()
            .find(|(i, _)| *i == widget)
            .map(|(_, c)| c)
    }
}

/// Can this page be drawn and used from its original menu? Buttons whose
/// command is `Unavailable` are greyed out here. Returns every reason it
/// can't, so they can all be logged at once.
pub fn check(layout: &mut MenuLayout, plan: &PagePlan) -> Result<PageButtons, Vec<String>> {
    let mut why = Vec::new();
    for s in &layout.skipped {
        why.push(format!("widget {s} is a kind we can't draw yet"));
    }
    let mut commands = Vec::new();
    let mut greyed = Vec::new();
    for i in layout.buttons_on_pc() {
        let w = &layout.widgets[i];
        let Kind::Button(b) = &w.kind else { continue };
        if b.disabled {
            continue;
        }
        match plan.command(&b.on_select) {
            Some(PageCmd::Unavailable) => greyed.push(w.name.clone()),
            Some(c) => commands.push((i, c.clone())),
            None => why.push(format!(
                "button {} calls {:?}, which has no command in original_menus.toml",
                w.name, b.on_select
            )),
        }
    }
    if commands.is_empty() && why.is_empty() {
        why.push("no button can be chosen".into());
    }
    if !why.is_empty() {
        return Err(why);
    }
    for name in greyed {
        crate::screens::disable(layout, &name);
    }
    crate::screens::setup_titled(layout);
    Ok(PageButtons { commands })
}

/// `data/ui/original_menus.toml`.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct PagesFile {
    pages: BTreeMap<String, PageDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageDef {
    class: String,
    #[serde(default)]
    over_game: bool,
    back: Option<String>,
    #[serde(default)]
    buttons: BTreeMap<String, String>,
}

/// Read `data/ui/original_menus.toml`.
pub fn parse_plans(text: &str) -> Result<Vec<PagePlan>, String> {
    let f: PagesFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (page, d) in f.pages {
        let cmd = |s: &str| PageCmd::parse(s).ok_or(format!("page {page}: unknown command {s:?}"));
        let mut buttons = Vec::new();
        for (f, c) in &d.buttons {
            buttons.push((f.clone(), cmd(c)?));
        }
        let back = d.back.as_deref().map(cmd).transpose()?;
        out.push(PagePlan {
            page: page.to_ascii_lowercase(),
            class: d.class,
            buttons,
            back,
            over_game: d.over_game,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Button, Sounds, Widget};

    fn button(name: &str, on_select: &str) -> Widget {
        Widget {
            name: name.into(),
            pass: 0,
            hidden: false,
            platform: 0,
            kind: Kind::Button(Button {
                on_select: on_select.into(),
                ..Default::default()
            }),
        }
    }

    fn layout(widgets: Vec<Widget>) -> MenuLayout {
        MenuLayout {
            class: "Test.Menu".into(),
            widgets,
            skipped: vec![],
            sounds: Sounds::default(),
            background_movie: String::new(),
            music: None,
        }
    }

    fn plan() -> PagePlan {
        PagePlan {
            page: "paused".into(),
            class: "Test.Menu".into(),
            buttons: vec![
                ("ResumeSelected".into(), PageCmd::Resume),
                ("OptionsSelected".into(), PageCmd::Open("options".into())),
                ("SaveSelected".into(), PageCmd::Unavailable),
            ],
            back: Some(PageCmd::Resume),
            over_game: true,
        }
    }

    #[test]
    fn commands_parse() {
        assert_eq!(PageCmd::parse("resume"), Some(PageCmd::Resume));
        assert_eq!(PageCmd::parse(" Quit_Game "), Some(PageCmd::QuitGame));
        assert_eq!(
            PageCmd::parse("open:Options"),
            Some(PageCmd::Open("options".into()))
        );
        assert_eq!(
            PageCmd::parse("play:geo_01a"),
            Some(PageCmd::Play("geo_01a".into()))
        );
        assert_eq!(PageCmd::parse("fly"), None);
    }

    #[test]
    fn known_buttons_pass_and_unavailable_ones_grey_out() {
        let mut l = layout(vec![
            button("Options[0]", "resumeselected"),
            button("Options[1]", "OptionsSelected"),
            button("Options[2]", "SaveSelected"),
        ]);
        let b = check(&mut l, &plan()).expect("usable");
        assert_eq!(b.command(0), Some(&PageCmd::Resume));
        assert_eq!(b.command(1), Some(&PageCmd::Open("options".into())));
        assert_eq!(b.command(2), None);
        let Kind::Button(saved) = &l.widgets[2].kind else {
            panic!()
        };
        assert!(saved.disabled);
    }

    #[test]
    fn an_unknown_button_or_widget_keeps_the_placeholder() {
        let mut l = layout(vec![
            button("Options[0]", "ResumeSelected"),
            button("Options[1]", "MysterySelected"),
        ]);
        l.skipped.push("Volume (MenuSlider)".into());
        let why = check(&mut l, &plan()).unwrap_err();
        assert_eq!(why.len(), 2, "{why:?}");
        assert!(why[0].contains("MenuSlider"));
        assert!(why[1].contains("MysterySelected"));
    }

    #[test]
    fn hidden_and_console_buttons_are_ignored() {
        let mut hidden = button("Options[1]", "MysterySelected");
        hidden.hidden = true;
        let mut console = button("Options[2]", "XboxLiveSelected");
        console.platform = crate::layout::MWP_CONSOLE;
        let mut l = layout(vec![
            button("Options[0]", "ResumeSelected"),
            hidden,
            console,
        ]);
        assert!(check(&mut l, &plan()).is_ok());
    }
}
