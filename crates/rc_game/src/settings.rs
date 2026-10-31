//! The player's settings file, `settings.toml`.
//!
//! It says where the original game is installed, which mods are on, and the
//! video, sound and control options. If it doesn't exist the game writes one
//! with every option and its default, so there is always a file to edit.
//! Missing options use their defaults; an option with a wrong name or type
//! stops the game with a message naming it.

use bevy::prelude::Resource;
use rc_game_core::paths::resolve;
use rc_pads::GyroSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Resource, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub game: GameSection,
    pub mods: ModsSection,
    pub video: VideoSection,
    pub audio: AudioSection,
    pub controls: ControlsSection,
    pub gyro: GyroSettings,
    pub save: SaveSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GameSection {
    /// The original game's install folder (the one holding `GameData`).
    /// Empty means: look in the usual places.
    pub folder: String,
    /// Our own data files (`data` in the project folder).
    pub data_folder: String,
    pub difficulty: Difficulty,
}

/// The original's three difficulties.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    #[default]
    Medium,
    Hard,
}

impl Difficulty {
    /// How much of an attack's damage reaches the player: the original's
    /// numbers from its `User.ini` (found by the weapons part).
    pub fn damage_to_player(self) -> f32 {
        match self {
            Difficulty::Easy => 0.25,
            Difficulty::Medium => 1.0,
            Difficulty::Hard => 1.75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModsSection {
    /// The folder holding one sub-folder per mod.
    pub folder: String,
    /// Mods to switch on, by sub-folder name. The first one listed wins when
    /// two mods change the same file.
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VideoSection {
    /// In a window, full screen in a borderless window the size of the
    /// monitor, or true full screen (the monitor switches to the chosen
    /// size).
    pub display_mode: DisplayMode,
    /// The screen size: the window's size when windowed. In true full
    /// screen, a size of another shape than the monitor's is shown at that
    /// shape with black bars filling the rest.
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
    /// With vsync off, the most pictures a second the game draws. 0 is no
    /// limit. Drawing a menu thousands of times a second only heats the
    /// graphics card, and on some Windows PCs it can stop the picture
    /// updating while the game runs on.
    pub max_fps: u32,
    /// Which way the game talks to the graphics card: `auto` (DirectX 12
    /// on Windows, usually Vulkan elsewhere), `vulkan`, `dx12` (Windows
    /// only) or `gl`. Takes effect at the next start.
    pub graphics_api: String,
    /// Field of view across a 4:3 screen, degrees. The original's is 85.
    /// Wider screens see more at the sides with the same view up and down
    /// ("Hor+"), so nothing is cut off or stretched to fit.
    pub fov: f32,
    /// The longest screen shape (long side over short side) the view keeps
    /// widening for. On longer screens (an ultrawide, a phone held
    /// sideways or upright) a flat view would stretch the sides into a
    /// fish-eye look, so it sees what this shape sees along its long side
    /// and a little less along its short side. 0 is no limit. Not used
    /// while `panini` is on.
    pub widest_shape: f32,
    /// The widest shape the on-screen display spreads to, as width over
    /// height. On wider screens it stays in a centred area this shape, so
    /// health and ammo aren't in the far corners of an ultrawide.
    /// 0 lets it fill the screen.
    pub hud_max_aspect: f32,
    /// The Panini view for very wide screens: 0 is off, up to 1. Bends the
    /// 3D view so things near the sides aren't stretched; the middle keeps
    /// its size.
    pub panini: f32,
}

/// How the game window covers the screen (`video.display_mode`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Windowed,
    /// A window without edges that covers the whole monitor at its own
    /// size; quick to switch away from.
    Borderless,
    /// The game takes over the monitor and switches it to the chosen size.
    Fullscreen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioSection {
    /// All volumes are 0 (silent) to 1 (full).
    pub master_volume: f32,
    pub music_volume: f32,
    pub effects_volume: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SaveSection {
    /// Where save files go. Empty means the system's usual place
    /// (`%APPDATA%\rc_game\saves` on Windows, `~/.local/share/rc_game/saves`
    /// on Linux, `saves` next to the settings file elsewhere). Never the
    /// game's install.
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ControlsSection {
    /// Our data file with the key and button bindings.
    pub bindings: String,
    /// Controls changed in the game are saved to this file, next to the
    /// settings file, and used instead of `bindings`. Delete it to go back
    /// to the defaults.
    pub user_bindings: String,
    /// Mouse turn per count of mouse movement, radians.
    pub mouse_sensitivity: f32,
    /// Gamepad and touch turning speed at full stick, radians per second.
    pub stick_look_speed: f32,
    pub invert_y: bool,
    /// Move-stick movement smaller than this is ignored (0 to 1).
    pub stick_dead_zone: f32,
    /// The same for the look stick.
    pub look_stick_dead_zone: f32,
    /// Move-stick response: 1 is straight, higher makes small pushes walk
    /// slower.
    pub stick_move_curve: f32,
    /// Look-stick response: 1 is straight, 2 or more makes small pushes
    /// gentler for fine aiming while full tilt still turns at full speed.
    pub stick_look_curve: f32,
    /// Look-stick up and down speed compared with left and right.
    pub stick_look_vertical: f32,
    /// How far a trigger must be pulled to count as pressed (0 to 1).
    pub trigger_threshold: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            game: GameSection {
                folder: String::new(),
                data_folder: "data".into(),
                difficulty: Difficulty::Medium,
            },
            mods: ModsSection {
                folder: "mods".into(),
                enabled: vec![],
            },
            video: VideoSection {
                display_mode: DisplayMode::Windowed,
                width: 1280,
                height: 720,
                vsync: true,
                max_fps: 240,
                graphics_api: "auto".into(),
                fov: 85.0,
                widest_shape: 2.0,
                hud_max_aspect: 16.0 / 9.0,
                panini: 0.0,
            },
            audio: AudioSection {
                master_volume: 1.0,
                music_volume: 0.6,
                effects_volume: 1.0,
            },
            controls: ControlsSection {
                bindings: "input/bindings.toml".into(),
                user_bindings: "my_bindings.toml".into(),
                mouse_sensitivity: 0.0025,
                stick_look_speed: 2.5,
                invert_y: false,
                stick_dead_zone: 0.15,
                look_stick_dead_zone: 0.12,
                stick_move_curve: 1.0,
                stick_look_curve: 1.5,
                stick_look_vertical: 1.0,
                trigger_threshold: 0.3,
            },
            gyro: GyroSettings::default(),
            save: SaveSection::default(),
        }
    }
}
impl Default for GameSection {
    fn default() -> Self {
        Settings::default().game
    }
}
impl Default for ModsSection {
    fn default() -> Self {
        Settings::default().mods
    }
}
impl Default for VideoSection {
    fn default() -> Self {
        Settings::default().video
    }
}
impl Default for AudioSection {
    fn default() -> Self {
        Settings::default().audio
    }
}
impl Default for ControlsSection {
    fn default() -> Self {
        Settings::default().controls
    }
}

/// The settings file written on first run, with notes for each option.
pub const DEFAULT_FILE: &str = r#"# Republic Commando (Rust and Bevy) settings.
#
# Edit and restart the game. Delete a line to use its default.
# A misspelt option stops the game with a message naming it.

[game]
# The original game's install folder, the one that holds GameData.
# Leave empty to look in the usual places (Steam on Windows and Linux).
# Windows example: folder = 'C:\Games\Steam\steamapps\common\Star Wars Republic Commando'
# Linux example:   folder = '/home/me/.steam/steam/steamapps/common/Star Wars Republic Commando'
folder = ''
# Our own data files (menus, controls, music lists).
data_folder = 'data'
# easy, medium or hard (how much damage reaches you: 0.25, 1, 1.75 times).
difficulty = 'medium'

[mods]
# One sub-folder per mod. A mod holds files with the same names as in
# data_folder; its files are used instead of those.
folder = 'mods'
# Mods to switch on, by folder name. Earlier ones win.
enabled = []

[video]
# windowed, borderless (full screen in a window the size of the monitor) or
# fullscreen (true full screen: the monitor switches to width x height).
display_mode = 'windowed'
# The window's size. In true full screen, a size of another shape than the
# monitor's is shown at that shape with black bars around it.
width = 1280
height = 720
vsync = true
# With vsync off, the most pictures a second (0 = no limit).
max_fps = 240
# How the game talks to the graphics card: 'auto' (DirectX 12 on Windows,
# usually Vulkan elsewhere), 'vulkan', 'dx12' (Windows only) or 'gl'. Takes
# effect at the next start.
graphics_api = 'auto'
# Field of view across a 4:3 screen, degrees (the original's is 85). Wider
# screens see more at the sides; the view up and down stays the same.
fov = 85
# Longest screen shape (long side / short side) the view keeps widening for.
# Longer screens, like ultrawides, see what this shape sees across and a
# little less up and down, so the sides don't get a fish-eye look. 2.0 is
# 18:9. 0 is no limit (pure Hor+). Not used while panini (below) is on.
widest_shape = 2.0
# Widest shape (width / height) the display spreads to; on an ultrawide it
# stays in a centred area this shape. 1.7777778 is 16:9. 0 fills the screen.
hud_max_aspect = 1.7777778
# Panini view for ultrawide screens: 0 is off (the original's straight
# view), up to 1. Around 0.3 to 0.5 takes the stretching out of the sides on
# 21:9 and 32:9; the middle stays the same size, a little softer.
panini = 0.0

[audio]
master_volume = 1.0   # 0 is silent, 1 is full
music_volume = 0.6
effects_volume = 1.0

[controls]
bindings = 'input/bindings.toml'   # inside data_folder (or a mod)
# Controls you change in the game are saved here (next to this file) and
# used instead. Delete that file to go back to the defaults.
user_bindings = 'my_bindings.toml'
mouse_sensitivity = 0.0025         # radians per mouse count
stick_look_speed = 2.5             # radians per second at full stick
invert_y = false
stick_dead_zone = 0.15             # move stick: smaller pushes are ignored
look_stick_dead_zone = 0.12        # look stick
stick_move_curve = 1.0             # 1 is straight; higher walks slower on small pushes
stick_look_curve = 1.5             # 1 is straight; higher is finer near the middle
stick_look_vertical = 1.0          # look stick: up and down speed compared with across
trigger_threshold = 0.3            # how far to pull a trigger to fire (0 to 1)

[gyro]
# Aim by turning the controller (PlayStation, Switch Pro, Joy-Con, Steam
# Deck and others with a gyro) or a handheld PC with a built-in gyro.
# always: whenever the ratchet button isn't held. aiming: only while
# zooming. off: never. The gyro_toggle button switches it on and off.
mode = 'always'
# player: turn around the real up direction, however the controller is
# held (best for most people). local: only the controller's own up axis.
space = 'player'
sensitivity = 2.0                  # view turn per real turn (1 is one to one)
vertical_scale = 1.0               # up and down compared with across
invert_x = false
invert_y = false
smoothing_threshold = 2.0          # degrees per second; slower moves are smoothed
tightening = 0.0                   # degrees per second; slower moves are damped (0 is off)
auto_calibrate = true              # learn the gyro's drift while it lies still

[save]
# Where save files go (quicksave, checkpoint and named saves, as readable
# text files). Leave empty for the usual place: %APPDATA%\rc_game\saves on
# Windows, ~/.local/share/rc_game/saves on Linux.
folder = ''
"#;

/// The option `display_mode` replaced; still read.
const OLD_FULLSCREEN: &str = "fullscreen";

impl Settings {
    pub fn parse(text: &str) -> Result<Settings, String> {
        let mut s: Settings = toml::from_str(text).map_err(|e| e.to_string())?;
        // Older versions had `fullscreen = true` (a borderless window) in
        // place of `display_mode`.
        if let Ok(t) = text.parse::<toml::Table>() {
            let video = t.get("video").and_then(|v| v.as_table());
            let old = video
                .and_then(|v| v.get(OLD_FULLSCREEN))
                .and_then(|v| v.as_bool());
            if old == Some(true) && video.is_some_and(|v| !v.contains_key("display_mode")) {
                s.video.display_mode = DisplayMode::Borderless;
            }
        }
        Ok(s)
    }

    /// Options in `text` the game doesn't know: typos, or ones from another
    /// version of the game. They are skipped with a warning in the log
    /// rather than stopping the game, so one settings file works with every
    /// version being tested. As `section.option`.
    pub fn unknown_options(text: &str) -> Vec<String> {
        let (Ok(given), Ok(known)) = (
            text.parse::<toml::Table>(),
            toml::Table::try_from(Settings::default()),
        ) else {
            return vec![];
        };
        let mut out = vec![];
        for (section, value) in &given {
            let Some(known_section) = known.get(section) else {
                out.push(section.clone());
                continue;
            };
            if let (Some(v), Some(k)) = (value.as_table(), known_section.as_table()) {
                out.extend(
                    v.keys()
                        .filter(|key| !k.contains_key(*key))
                        .filter(|key| !(section == "video" && *key == OLD_FULLSCREEN))
                        .map(|key| format!("{section}.{key}")),
                );
            }
        }
        out
    }

    /// Read `path`. If it doesn't exist, write the default file there and
    /// use the defaults. Returns the settings and a note for the log.
    pub fn load_or_create(path: &Path) -> Result<(Settings, String), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Settings::parse(&text)
                .map(|s| {
                    let mut note = format!("settings from {}", path.display());
                    let unknown = Settings::unknown_options(&text);
                    if !unknown.is_empty() {
                        note += &format!(
                            "; WARNING: settings file options this version doesn't know, skipped (a typo, or from another version): {}",
                            unknown.join(", ")
                        );
                    }
                    (s, note)
                })
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => {
                let note = match std::fs::write(path, DEFAULT_FILE) {
                    Ok(()) => format!("no settings file: wrote the defaults to {}", path.display()),
                    Err(e) => format!(
                        "no settings file, and can't write {} ({e}): using defaults",
                        path.display()
                    ),
                };
                Ok((Settings::default(), note))
            }
        }
    }

    /// Write the settings back (from the options menu). The notes in the
    /// first-run file are replaced by a short header.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let text = format!(
            "# Republic Commando (Rust and Bevy) settings, saved from the options menu.\n# Delete this file to get the default one back, with notes on every option.\n\n{body}"
        );
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Our data folders in search order: enabled mods first, base last.
    pub fn data_dirs(&self) -> Vec<PathBuf> {
        let mods = PathBuf::from(&self.mods.folder);
        let mut out: Vec<PathBuf> = self.mods.enabled.iter().map(|m| mods.join(m)).collect();
        out.push(PathBuf::from(&self.game.data_folder));
        out
    }
}

/// Where the settings file is, for saving.
#[derive(Resource, Debug, Clone)]
pub struct SettingsPath(pub PathBuf);

/// True if `dir` looks like the game's install: it has `GameData/Maps`.
pub fn is_game_folder(dir: &Path) -> bool {
    resolve(dir, "GameData/Maps").is_dir()
}

/// Where to look for the game, in order: the settings file, the
/// `RC_GAME_DIR` environment variable, then the usual Steam folders for this
/// system. Returns every place tried, and the first that holds the game.
pub fn find_game_folder(settings: &Settings) -> (Vec<PathBuf>, Option<PathBuf>) {
    let mut tried = Vec::new();
    if !settings.game.folder.trim().is_empty() {
        tried.push(PathBuf::from(settings.game.folder.trim()));
    }
    if let Some(d) = std::env::var_os("RC_GAME_DIR") {
        tried.push(PathBuf::from(d));
    }
    const GAME: &str = "Star Wars Republic Commando";
    if cfg!(windows) {
        tried.push(PathBuf::from(r"C:\Games\Steam\steamapps\common").join(GAME));
        tried.push(PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common").join(GAME));
        tried.push(PathBuf::from(r"C:\Program Files\Steam\steamapps\common").join(GAME));
    } else if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for steam in [
            ".steam/steam",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        ] {
            tried.push(home.join(steam).join("steamapps/common").join(GAME));
        }
    }
    let found = tried.iter().find(|d| is_game_folder(d)).cloned();
    (tried, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_defaults() {
        assert_eq!(Settings::parse(DEFAULT_FILE).unwrap(), Settings::default());
    }

    #[test]
    fn missing_options_use_defaults_and_typos_are_warnings() {
        let s = Settings::parse("[audio]\nmusic_volume = 0.2\n").unwrap();
        assert_eq!(s.audio.music_volume, 0.2);
        assert_eq!(s.video.fov, 85.0);
        assert_eq!(
            Settings::parse("[game]\ndifficulty = 'hard'\n")
                .unwrap()
                .game
                .difficulty,
            Difficulty::Hard
        );
        // A wrong value is still an error: there is no sensible guess.
        assert!(Settings::parse("[game]\ndifficulty = 'brutal'\n").is_err());
        // An unknown option is skipped and named for the log warning.
        let text = "[audio]\nmusic_volum = 0.2\n[gyro]\nspin = 1\n[extra]\na = 1\n";
        assert_eq!(Settings::parse(text).unwrap(), Settings::default());
        assert_eq!(
            Settings::unknown_options(text),
            ["audio.music_volum", "extra", "gyro.spin"]
        );
        assert!(Settings::unknown_options(DEFAULT_FILE).is_empty());
    }

    #[test]
    fn saved_file_reads_back() {
        let mut s = Settings::default();
        s.audio.music_volume = 0.3;
        s.mods.enabled = vec!["x".into()];
        let p = std::env::temp_dir().join(format!("rc_game_settings_{}.toml", std::process::id()));
        s.save(&p).unwrap();
        let (back, _) = Settings::load_or_create(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(back, s);
    }

    #[test]
    fn file_from_the_first_fish_eye_fix_still_loads() {
        // That test build saved `widest_fov`, which no version knows now.
        let text = "[video]\nfov = 120.0\nwidest_fov = 110.0\n";
        let s = Settings::parse(text).unwrap();
        assert_eq!(s.video.fov, 120.0);
        assert_eq!(s.video.widest_shape, 2.0);
        assert_eq!(Settings::unknown_options(text), ["video.widest_fov"]);
    }

    #[test]
    fn old_full_screen_option_becomes_borderless() {
        let old = "[video]\nfullscreen = true\n";
        let s = Settings::parse(old).unwrap();
        assert_eq!(s.video.display_mode, DisplayMode::Borderless);
        assert!(Settings::unknown_options(old).is_empty());
        let s = Settings::parse("[video]\nfullscreen = false\n").unwrap();
        assert_eq!(s.video.display_mode, DisplayMode::Windowed);
        let both = "[video]\nfullscreen = true\ndisplay_mode = 'fullscreen'\n";
        assert_eq!(
            Settings::parse(both).unwrap().video.display_mode,
            DisplayMode::Fullscreen
        );
    }

    #[test]
    fn mods_come_before_base_data() {
        let s = Settings::parse("[mods]\nenabled = ['a', 'b']\n").unwrap();
        let d = s.data_dirs();
        assert_eq!(
            d,
            vec![
                PathBuf::from("mods/a"),
                PathBuf::from("mods/b"),
                PathBuf::from("data")
            ]
        );
    }
}
