//! The settings behind the original options pages' choosers (the rows with
//! arrows, such as MOUSE SENSITIVITY < 6 >).
//!
//! `data/ui/original_menus.toml` names a setting for each chooser. Here each
//! name is turned into "which of the chooser's values is the setting now"
//! and "set the setting to value number i". The values themselves (the
//! text, such as "1".."10" or "ON"/"OFF") come from the original menu in the
//! install, except where the port needs a different list (the screen sizes).
//!
//! How each value maps onto our settings follows the original's options
//! scripts, rewritten by hand (never copied): for example its mouse
//! sensitivity is the chosen number times 0.5, with 3.0 (number 6) as the
//! default, and its volumes are the chosen number divided by 10.

use crate::settings::{Difficulty, Settings};

/// Every setting a chooser can show.
pub const ALL: &[&str] = &[
    "mouse_sensitivity",
    "invert_mouse",
    "difficulty",
    "effects_volume",
    "music_volume",
    "video_resolution",
    "vsync",
];

pub fn known(name: &str) -> bool {
    ALL.contains(&name)
}

/// The original's default mouse sensitivity is number 6 of 10 (3.0 in its
/// own units); ours is the same feel at our default.
const SENSITIVITY_DEFAULT_STEP: f32 = 6.0;

/// Screen sizes offered: the original's list (all 4:3 and 5:4) plus wide
/// and ultrawide ones, which this port supports properly. The size the game
/// runs at is added if it isn't here.
const RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480),
    (800, 600),
    (1024, 768),
    (1152, 864),
    (1280, 720),
    (1280, 960),
    (1280, 1024),
    (1366, 768),
    (1600, 900),
    (1600, 1200),
    (1920, 1080),
    (1920, 1200),
    (2560, 1080),
    (2560, 1440),
    (3440, 1440),
    (3840, 1600),
    (3840, 2160),
    (5120, 1440),
];

/// The values to show instead of the original's, if this setting needs
/// its own list.
pub fn items(name: &str, s: &Settings) -> Option<Vec<String>> {
    match name {
        "video_resolution" => {
            let mut sizes = RESOLUTIONS.to_vec();
            let now = (s.video.width, s.video.height);
            if !sizes.contains(&now) {
                sizes.push(now);
                sizes.sort_by_key(|(w, h)| (*w, *h));
            }
            Some(sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect())
        }
        _ => None,
    }
}

/// Which of `items` the setting is now.
pub fn read(name: &str, s: &Settings, items: &[String]) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let tenth = |v: f32| ((v * 10.0).round() as usize).min(n - 1);
    let yes_no = |on: bool| if on { 0 } else { 1.min(n - 1) };
    Some(match name {
        "mouse_sensitivity" => {
            let base = Settings::default().controls.mouse_sensitivity;
            let step = (s.controls.mouse_sensitivity / base * SENSITIVITY_DEFAULT_STEP).round();
            (step.max(1.0) as usize - 1).min(n - 1)
        }
        // The original lists YES first: YES means the mouse is inverted.
        "invert_mouse" => yes_no(s.controls.invert_y),
        "difficulty" => match s.game.difficulty {
            Difficulty::Easy => 0,
            Difficulty::Medium => 1,
            Difficulty::Hard => 2,
        }
        .min(n - 1),
        "effects_volume" => tenth(s.audio.effects_volume),
        "music_volume" => tenth(s.audio.music_volume),
        "video_resolution" => {
            let now = format!("{}x{}", s.video.width, s.video.height);
            items.iter().position(|i| i.eq_ignore_ascii_case(&now))?
        }
        "vsync" => yes_no(s.video.vsync),
        _ => return None,
    })
}

/// Set the setting to value `i` of `items`. Returns false for a setting it
/// doesn't know or a value it can't use.
pub fn write(name: &str, s: &mut Settings, i: usize, items: &[String]) -> bool {
    let Some(text) = items.get(i) else {
        return false;
    };
    let first = i == 0;
    match name {
        "mouse_sensitivity" => {
            let base = Settings::default().controls.mouse_sensitivity;
            s.controls.mouse_sensitivity = base * (i + 1) as f32 / SENSITIVITY_DEFAULT_STEP;
        }
        "invert_mouse" => s.controls.invert_y = first,
        "difficulty" => {
            s.game.difficulty = match i {
                0 => Difficulty::Easy,
                1 => Difficulty::Medium,
                _ => Difficulty::Hard,
            }
        }
        "effects_volume" => s.audio.effects_volume = (i as f32 / 10.0).clamp(0.0, 1.0),
        "music_volume" => s.audio.music_volume = (i as f32 / 10.0).clamp(0.0, 1.0),
        "video_resolution" => {
            let Some((w, h)) = text
                .to_ascii_lowercase()
                .split_once('x')
                .and_then(|(w, h)| {
                    Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?))
                })
            else {
                return false;
            };
            s.video.width = w;
            s.video.height = h;
        }
        "vsync" => s.video.vsync = first,
        _ => return false,
    }
    true
}

/// Put the setting back to the game's default (RESTORE DEFAULTS, SET
/// RECOMMENDED).
pub fn restore(name: &str, s: &mut Settings) {
    let d = Settings::default();
    match name {
        "mouse_sensitivity" => s.controls.mouse_sensitivity = d.controls.mouse_sensitivity,
        "invert_mouse" => s.controls.invert_y = d.controls.invert_y,
        "difficulty" => s.game.difficulty = d.game.difficulty,
        "effects_volume" => s.audio.effects_volume = d.audio.effects_volume,
        "music_volume" => s.audio.music_volume = d.audio.music_volume,
        "video_resolution" => {
            s.video.width = d.video.width;
            s.video.height = d.video.height;
        }
        "vsync" => s.video.vsync = d.video.vsync,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mouse_sensitivity_default_is_number_six() {
        let ten = list(&["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        let mut s = Settings::default();
        assert_eq!(read("mouse_sensitivity", &s, &ten), Some(5));
        assert!(write("mouse_sensitivity", &mut s, 9, &ten));
        assert_eq!(read("mouse_sensitivity", &s, &ten), Some(9));
        assert!(s.controls.mouse_sensitivity > Settings::default().controls.mouse_sensitivity);
    }

    #[test]
    fn every_setting_reads_back_what_was_written() {
        let s0 = Settings::default();
        for name in ALL {
            let items = items(name, &s0)
                .unwrap_or_else(|| list(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]));
            let mut s = s0.clone();
            for i in [0, 1, 2] {
                assert!(write(name, &mut s, i, &items), "{name} {i}");
                let back = read(name, &s, &items);
                // Two-value settings (on/off) read back as 0 or 1.
                assert!(
                    back == Some(i) || (i == 2 && back.is_some()),
                    "{name} {i} {back:?}"
                );
            }
            restore(name, &mut s);
            assert_eq!(read(name, &s, &items), read(name, &s0, &items), "{name}");
        }
    }

    #[test]
    fn the_running_screen_size_is_always_offered() {
        let mut s = Settings::default();
        s.video.width = 1234;
        s.video.height = 567;
        let items = items("video_resolution", &s).unwrap();
        assert!(items.contains(&"1234x567".to_string()));
        assert_eq!(
            read("video_resolution", &s, &items).map(|i| items[i].as_str()),
            Some("1234x567")
        );
    }
}
