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

use crate::settings::{Difficulty, DisplayMode, Settings};

/// Every setting a chooser can show.
pub const ALL: &[&str] = &[
    "mouse_sensitivity",
    "invert_mouse",
    "difficulty",
    "effects_volume",
    "music_volume",
    "video_resolution",
    "vsync",
    // The port's own settings, on its PORT + page.
    "field_of_view",
    "panini",
    "display_mode",
    "master_volume",
    "stick_look_speed",
    "look_stick_dead_zone",
    "gyro_mode",
    "gyro_sensitivity",
];

/// A setting shown as evenly spaced numbers: (lowest, highest, step,
/// decimals shown, text after the number, number shown per unit).
type Numeric = (f32, f32, f32, usize, &'static str, f32);

fn numeric(name: &str) -> Option<Numeric> {
    Some(match name {
        "field_of_view" => (60.0, 120.0, 5.0, 0, "", 1.0),
        "panini" => (0.0, 1.0, 0.1, 0, "%", 100.0),
        "master_volume" => (0.0, 1.0, 0.1, 0, "", 10.0),
        "stick_look_speed" => (0.5, 8.0, 0.5, 1, "", 1.0),
        "look_stick_dead_zone" => (0.0, 0.5, 0.02, 2, "", 1.0),
        "gyro_sensitivity" => (0.25, 5.0, 0.25, 2, "", 1.0),
        _ => return None,
    })
}

fn numeric_values((lo, hi, step, ..): Numeric) -> Vec<f32> {
    let n = ((hi - lo) / step).round() as usize;
    (0..=n).map(|i| lo + step * i as f32).collect()
}

fn numeric_value(name: &str, s: &Settings) -> f32 {
    match name {
        "field_of_view" => s.video.fov,
        "panini" => s.video.panini,
        "master_volume" => s.audio.master_volume,
        "stick_look_speed" => s.controls.stick_look_speed,
        "look_stick_dead_zone" => s.controls.look_stick_dead_zone,
        "gyro_sensitivity" => s.gyro.sensitivity,
        _ => 0.0,
    }
}

fn set_numeric(name: &str, s: &mut Settings, v: f32) {
    match name {
        "field_of_view" => s.video.fov = v,
        "panini" => s.video.panini = v,
        "master_volume" => s.audio.master_volume = v,
        "stick_look_speed" => s.controls.stick_look_speed = v,
        "look_stick_dead_zone" => s.controls.look_stick_dead_zone = v,
        "gyro_sensitivity" => s.gyro.sensitivity = v,
        _ => {}
    }
}

const GYRO_MODES: [rc_pads::GyroMode; 3] = [
    rc_pads::GyroMode::Off,
    rc_pads::GyroMode::Always,
    rc_pads::GyroMode::Aiming,
];

/// The setting whose value opens a drop-down list when chosen (the
/// others change only with their arrows).
pub const LIST_SETTING: &str = "video_resolution";

pub fn known(name: &str) -> bool {
    ALL.contains(&name)
}

/// The original's default mouse sensitivity is number 6 of 10 (3.0 in its
/// own units); ours is the same feel at our default.
const SENSITIVITY_DEFAULT_STEP: f32 = 6.0;

/// Screen sizes offered: the original's list (all 4:3 and 5:4) plus wide
/// and ultrawide ones, which this port supports properly. The monitor's own
/// size and the size the game runs at are added if they aren't here.
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

const DISPLAY_MODES: [(DisplayMode, &str); 3] = [
    (DisplayMode::Windowed, "WINDOWED"),
    (DisplayMode::Borderless, "BORDERLESS"),
    (DisplayMode::Fullscreen, "FULL SCREEN"),
];

/// Two screen shapes (width over height) count as the same within this.
const SAME_SHAPE: f32 = 0.01;

/// True if `size` has the same shape as `desktop` (the monitor).
pub fn same_shape(size: (u32, u32), desktop: (u32, u32)) -> bool {
    let shape = |(w, h): (u32, u32)| w as f32 / h.max(1) as f32;
    (shape(size) / shape(desktop) - 1.0).abs() < SAME_SHAPE
}

/// The values to show instead of the original's, if this setting needs
/// its own list. `desktop` is the monitor's own size: sizes bigger than it
/// can't be shown, so they aren't offered.
pub fn items(name: &str, s: &Settings, desktop: Option<(u32, u32)>) -> Option<Vec<String>> {
    if let Some(n @ (_, _, _, decimals, suffix, unit)) = numeric(name) {
        return Some(
            numeric_values(n)
                .into_iter()
                .map(|v| format!("{:.*}{suffix}", decimals, v * unit))
                .collect(),
        );
    }
    match name {
        "display_mode" => Some(DISPLAY_MODES.iter().map(|(_, t)| t.to_string()).collect()),
        "gyro_mode" => Some(vec!["OFF".into(), "ALWAYS".into(), "WHEN AIMING".into()]),
        "video_resolution" => {
            let fits = |&(w, h): &(u32, u32)| desktop.is_none_or(|(dw, dh)| w <= dw && h <= dh);
            let mut sizes: Vec<(u32, u32)> = RESOLUTIONS.iter().copied().filter(fits).collect();
            for size in desktop.into_iter().chain([(s.video.width, s.video.height)]) {
                if !sizes.contains(&size) {
                    sizes.push(size);
                }
            }
            sizes.sort_by_key(|(w, h)| (*w, *h));
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
    if let Some(num) = numeric(name) {
        let v = numeric_value(name, s);
        let values = numeric_values(num);
        let nearest = (0..values.len())
            .min_by(|&a, &b| (values[a] - v).abs().total_cmp(&(values[b] - v).abs()))?;
        return Some(nearest.min(n - 1));
    }
    Some(match name {
        "display_mode" => DISPLAY_MODES
            .iter()
            .position(|(m, _)| *m == s.video.display_mode)
            .unwrap_or(0)
            .min(n - 1),
        "gyro_mode" => GYRO_MODES
            .iter()
            .position(|m| *m == s.gyro.mode)
            .unwrap_or(0)
            .min(n - 1),
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
/// doesn't know or a value it can't use. `desktop` is the monitor's own
/// size: in borderless full screen, a screen size of another shape than the
/// monitor's switches to a window of that size.
pub fn write(
    name: &str,
    s: &mut Settings,
    i: usize,
    items: &[String],
    desktop: Option<(u32, u32)>,
) -> bool {
    let Some(text) = items.get(i) else {
        return false;
    };
    let first = i == 0;
    if let Some(num) = numeric(name) {
        let Some(v) = numeric_values(num).get(i).copied() else {
            return false;
        };
        set_numeric(name, s, v);
        return true;
    }
    match name {
        "display_mode" => s.video.display_mode = DISPLAY_MODES[i.min(2)].0,
        "gyro_mode" => s.gyro.mode = GYRO_MODES[i.min(2)],
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
            if s.video.display_mode == DisplayMode::Borderless
                && desktop.is_some_and(|d| !same_shape((w, h), d))
            {
                s.video.display_mode = DisplayMode::Windowed;
            }
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
        "display_mode" => s.video.display_mode = d.video.display_mode,
        "gyro_mode" => s.gyro.mode = d.gyro.mode,
        _ if numeric(name).is_some() => set_numeric(name, s, numeric_value(name, &d)),
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
        assert!(write("mouse_sensitivity", &mut s, 9, &ten, None));
        assert_eq!(read("mouse_sensitivity", &s, &ten), Some(9));
        assert!(s.controls.mouse_sensitivity > Settings::default().controls.mouse_sensitivity);
    }

    #[test]
    fn every_setting_reads_back_what_was_written() {
        let s0 = Settings::default();
        for name in ALL {
            let items = items(name, &s0, None)
                .unwrap_or_else(|| list(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]));
            let mut s = s0.clone();
            for i in (0..3).filter(|&i| i < items.len()) {
                assert!(write(name, &mut s, i, &items, None), "{name} {i}");
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
        let items = items("video_resolution", &s, None).unwrap();
        assert!(items.contains(&"1234x567".to_string()));
        assert_eq!(
            read("video_resolution", &s, &items).map(|i| items[i].as_str()),
            Some("1234x567")
        );
    }

    #[test]
    fn sizes_that_fit_the_monitor_and_no_full_screen_entry() {
        let s = Settings::default();
        let items = items("video_resolution", &s, Some((3440, 1440))).unwrap();
        assert!(!items.iter().any(|i| i.contains("FULL")));
        assert!(items.contains(&"3440x1440".to_string()));
        assert!(items.contains(&"1920x1080".to_string()));
        assert!(!items.contains(&"3840x2160".to_string()));
        // The monitor's own size is offered even when it isn't a usual one.
        let odd = super::items("video_resolution", &s, Some((2256, 1504))).unwrap();
        assert!(odd.contains(&"2256x1504".to_string()));
    }

    #[test]
    fn display_modes_and_screen_sizes() {
        let desktop = Some((1920, 1080));
        let sizes = items("video_resolution", &Settings::default(), desktop).unwrap();
        let modes = items("display_mode", &Settings::default(), desktop).unwrap();
        assert_eq!(modes, ["WINDOWED", "BORDERLESS", "FULL SCREEN"]);
        let pick = |s: &mut Settings, size: &str| {
            let at = sizes.iter().position(|i| i == size).unwrap();
            assert!(write("video_resolution", s, at, &sizes, desktop));
        };
        // Windowed: the size is the window's.
        let mut s = Settings::default();
        pick(&mut s, "1600x900");
        assert_eq!((s.video.width, s.video.height, s.video.display_mode), (1600, 900, DisplayMode::Windowed));
        // Borderless: a size of the monitor's shape stays borderless...
        assert!(write("display_mode", &mut s, 1, &modes, desktop));
        assert_eq!(s.video.display_mode, DisplayMode::Borderless);
        pick(&mut s, "1280x720");
        assert_eq!(s.video.display_mode, DisplayMode::Borderless);
        // ...another shape goes to a window of that size.
        pick(&mut s, "1280x1024");
        assert_eq!((s.video.width, s.video.height, s.video.display_mode), (1280, 1024, DisplayMode::Windowed));
        // True full screen keeps any size (other shapes get black bars).
        assert!(write("display_mode", &mut s, 2, &modes, desktop));
        pick(&mut s, "1024x768");
        assert_eq!((s.video.width, s.video.height, s.video.display_mode), (1024, 768, DisplayMode::Fullscreen));
        assert_eq!(read("display_mode", &s, &modes), Some(2));
    }
}
