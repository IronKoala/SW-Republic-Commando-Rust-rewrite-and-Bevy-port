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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    #[default]
    Unavailable,
    /// Show another page in this one's place (the options pages' GAME,
    /// SOUND, GRAPHICS and CONTROLS tabs): DONE then goes back to the page
    /// before them all.
    Switch(String),
    /// The arrows beside a chooser: step that row's value.
    OptionLeft,
    OptionRight,
    /// Put this page's settings back to the game's defaults.
    RestoreDefaults,
    /// Choose a new key for this row's action (the controls page).
    Rebind,
    /// Show the previous or next set of rows (the controls page).
    PrevSet,
    NextSet,
}

impl PageCmd {
    /// Read a command as written in the data file: `resume`, `restart`,
    /// `back`, `quit_to_menu`, `quit_game`, `unavailable`, `open:<page>`,
    /// `play:<map>`, `switch:<page>`, `option_left`, `option_right`,
    /// `restore_defaults`, `rebind`, `prev_set`, `next_set`.
    pub fn parse(s: &str) -> Option<PageCmd> {
        let s = s.trim();
        if let Some(p) = s.strip_prefix("open:") {
            return Some(PageCmd::Open(p.trim().to_ascii_lowercase()));
        }
        if let Some(p) = s.strip_prefix("switch:") {
            return Some(PageCmd::Switch(p.trim().to_ascii_lowercase()));
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
            "option_left" => PageCmd::OptionLeft,
            "option_right" => PageCmd::OptionRight,
            "restore_defaults" => PageCmd::RestoreDefaults,
            "rebind" => PageCmd::Rebind,
            "prev_set" => PageCmd::PrevSet,
            "next_set" => PageCmd::NextSet,
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
    /// Chooser (by widget name, such as `Options[3]`) to the setting it
    /// shows, or `unavailable`.
    pub choosers: Vec<(String, String)>,
    /// Key button (by widget name, such as `OptionSet0[2]`) to the action
    /// it rebinds, or `unavailable`.
    pub keys: Vec<(String, String)>,
    /// Buttons this port adds to the original page, clearly marked.
    pub extra_buttons: Vec<ExtraButton>,
    /// New text for widgets (a label or a button), by widget name: lets
    /// the port's own page reuse an original page's look.
    pub texts: Vec<(String, String)>,
    /// Widgets moved down (up when negative) by a fraction of the screen.
    pub moves: Vec<(String, f32)>,
    /// Rows hidden whole (the row's button, label and arrows), by the
    /// row button's name.
    pub hidden_rows: Vec<String>,
}

/// A button the port adds to an original page, made as a copy of one of
/// the page's own buttons, moved down. Its text should say it's an
/// addition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtraButton {
    /// The new widget's name.
    pub name: String,
    /// The page's button to copy the look from.
    pub copy_of: String,
    pub text: String,
    /// How far down from the copied button, as a fraction of the screen.
    pub move_down: f32,
    pub command: PageCmd,
}

/// Whether a chooser or key row's entry means "not in this port yet".
pub fn is_unavailable(setting: &str) -> bool {
    setting.trim().eq_ignore_ascii_case("unavailable")
}

fn lookup<'a>(list: &'a [(String, String)], widget: &str) -> Option<&'a str> {
    list.iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(widget))
        .map(|(_, s)| s.as_str())
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
    /// Choosers that show a setting: widget index and setting name.
    pub choosers: Vec<(usize, String)>,
    /// Key buttons: widget index and the action they rebind.
    pub keys: Vec<(usize, String)>,
}

impl PageButtons {
    pub fn command(&self, widget: usize) -> Option<&PageCmd> {
        self.commands
            .iter()
            .find(|(i, _)| *i == widget)
            .map(|(_, c)| c)
    }

    pub fn chooser(&self, widget: usize) -> Option<&str> {
        self.choosers
            .iter()
            .find(|(i, _)| *i == widget)
            .map(|(_, s)| s.as_str())
    }

    pub fn key(&self, widget: usize) -> Option<&str> {
        self.keys
            .iter()
            .find(|(i, _)| *i == widget)
            .map(|(_, s)| s.as_str())
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
    for (name, text) in &plan.texts {
        if !crate::screens::set_widget_text(layout, name, text) {
            why.push(format!("text for {name}, which isn't on this page"));
        }
    }
    for (name, by) in &plan.moves {
        if !crate::screens::move_widget(layout, name, *by) {
            why.push(format!("move for {name}, which isn't on this page"));
        }
    }
    for name in &plan.hidden_rows {
        for i in crate::screens::row_widgets(layout, name) {
            layout.widgets[i].hidden = true;
        }
    }
    for e in &plan.extra_buttons {
        if !crate::screens::add_button_copy(layout, e) {
            why.push(format!(
                "extra button {} copies {}, which isn't a button on this page",
                e.name, e.copy_of
            ));
        }
    }
    // A row the original fills or switches on while it runs (FSAA and VSYNC,
    // from what the graphics card can do) can be stored switched off or
    // with no values. A row given a setting here is switched on, and gets a
    // stand-in value for the game to replace with the setting's own list.
    for (name, setting) in &plan.choosers {
        if is_unavailable(setting) {
            continue;
        }
        let Some(at) = layout
            .widgets
            .iter()
            .position(|w| w.name.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        if !matches!(&layout.widgets[at].kind, Kind::Button(_)) {
            continue;
        }
        let row = layout.widgets[at].name.clone();
        for i in crate::screens::row_widgets(layout, &row)
            .into_iter()
            .chain([at])
        {
            if let Kind::Button(b) = &mut layout.widgets[i].kind {
                b.disabled = false;
            }
        }
        if let Kind::Button(b) = &mut layout.widgets[at].kind {
            if b.items.is_empty() {
                b.items.push(String::new());
            }
        }
    }
    let mut commands = Vec::new();
    let mut choosers = Vec::new();
    let mut keys = Vec::new();
    let mut greyed = Vec::new();
    let mut greyed_rows = Vec::new();
    for i in layout.buttons_on_pc() {
        let w = &layout.widgets[i];
        let Kind::Button(b) = &w.kind else { continue };
        if b.disabled {
            continue;
        }
        if let Some(e) = plan.extra_buttons.iter().find(|e| e.name == w.name) {
            commands.push((i, e.command.clone()));
            continue;
        }
        if b.is_chooser() {
            match lookup(&plan.choosers, &w.name) {
                Some(s) if is_unavailable(s) => greyed_rows.push(w.name.clone()),
                Some(s) => choosers.push((i, s.to_string())),
                None => why.push(format!(
                    "chooser {} ({:?}...) has no setting in original_menus.toml",
                    w.name,
                    b.items.first().map(String::as_str).unwrap_or("")
                )),
            }
            continue;
        }
        match plan.command(&b.on_select) {
            Some(PageCmd::Unavailable) => greyed.push(w.name.clone()),
            Some(PageCmd::Rebind) => match lookup(&plan.keys, &w.name) {
                Some(a) if is_unavailable(a) => greyed_rows.push(w.name.clone()),
                Some(a) => keys.push((i, a.to_string())),
                None => why.push(format!(
                    "key button {} has no action in original_menus.toml",
                    w.name
                )),
            },
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
    for name in greyed_rows {
        crate::screens::disable_row(layout, &name);
    }
    // A row's arrows go with its chooser: drop the arrows of a greyed row.
    commands.retain(|(i, _)| matches!(&layout.widgets[*i].kind, Kind::Button(b) if !b.disabled));
    crate::screens::setup_titled(layout);
    crate::screens::show_set(layout, 0);
    Ok(PageButtons {
        commands,
        choosers,
        keys,
    })
}

/// `data/ui/original_menus.toml`.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct PagesFile {
    pages: BTreeMap<String, PageDef>,
    fit: FitDef,
}

/// `[fit]`: how menus use screens wider than 4:3 (`fit.rs`).
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FitDef {
    widest_shape: f32,
}

impl Default for FitDef {
    fn default() -> Self {
        FitDef {
            widest_shape: DEFAULT_WIDEST_SHAPE,
        }
    }
}

/// The widest screen shape (width over height) menus spread out to; on a
/// wider screen they stay in a centred area this shape. 21.5:9 covers the
/// usual ultrawide monitors.
pub const DEFAULT_WIDEST_SHAPE: f32 = 2.4;

/// `[fit] widest_shape` from `data/ui/original_menus.toml` (0 is no limit).
pub fn parse_widest_shape(text: &str) -> Result<f32, String> {
    let f: PagesFile = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(f.fit.widest_shape.max(0.0))
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
    #[serde(default)]
    choosers: BTreeMap<String, String>,
    #[serde(default)]
    keys: BTreeMap<String, String>,
    #[serde(default)]
    extra_buttons: BTreeMap<String, ExtraDef>,
    #[serde(default)]
    texts: BTreeMap<String, String>,
    #[serde(default)]
    moves: BTreeMap<String, f32>,
    #[serde(default)]
    hidden_rows: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraDef {
    copy_of: String,
    text: String,
    #[serde(default)]
    move_down: f32,
    command: String,
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
        let mut extra_buttons = Vec::new();
        for (name, e) in d.extra_buttons {
            extra_buttons.push(ExtraButton {
                command: cmd(&e.command)?,
                name,
                copy_of: e.copy_of,
                text: e.text,
                move_down: e.move_down,
            });
        }
        out.push(PagePlan {
            page: page.to_ascii_lowercase(),
            class: d.class,
            buttons,
            back,
            over_game: d.over_game,
            choosers: d.choosers.into_iter().collect(),
            keys: d.keys.into_iter().collect(),
            extra_buttons,
            texts: d.texts.into_iter().collect(),
            moves: d.moves.into_iter().collect(),
            hidden_rows: d.hidden_rows,
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
            ..Default::default()
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
    fn a_row_switched_off_or_empty_in_the_original_works_when_given_a_setting() {
        // As the original's FSAA row: stored switched off with no values.
        let mut fsaa = button("Options[11]", "");
        if let Kind::Button(b) = &mut fsaa.kind {
            b.disabled = true;
        }
        let mut l = layout(vec![button("Options[0]", "resumeselected"), fsaa]);
        let mut p = plan();
        p.choosers = vec![("Options[11]".into(), "fsaa".into())];
        let got = check(&mut l, &p).unwrap();
        assert_eq!(got.chooser(1), Some("fsaa"));
        let Kind::Button(b) = &l.widgets[1].kind else {
            panic!()
        };
        assert!(!b.disabled && b.is_chooser());
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
