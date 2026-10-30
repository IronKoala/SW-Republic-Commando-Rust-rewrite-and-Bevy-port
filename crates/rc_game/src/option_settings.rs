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
    "subtitles",
    "prompt_text",
    "effects_volume",
    "music_volume",
    "voices_volume",
    "sound_quality",
    "sound_3d",
    "audio_channels",
    "video_resolution",
    "vsync",
    "graphics_quality",
    "brightness",
    "texture_detail",
    "character_detail",
    "blur",
    "bloom",
    "squad_shadows",
    "fsaa",
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


/// Whether a value's text means on (ON, YES, ENABLED) or off (OFF, NO,
/// DISABLED, NONE), if it says.
fn text_on(text: &str) -> Option<bool> {
    match text.trim().to_ascii_uppercase().as_str() {
        "ON" | "YES" | "ENABLED" | "TRUE" => Some(true),
        "OFF" | "NO" | "DISABLED" | "FALSE" | "NONE" => Some(false),
        _ => None,
    }
}

/// On/off settings: the value whose text says so, else the first value
/// for on and the second for off (as the original's YES/NO lists).
fn on_off_index(items: &[String], on: bool) -> usize {
    items
        .iter()
        .position(|t| text_on(t) == Some(on))
        .unwrap_or(if on { 0 } else { 1.min(items.len() - 1) })
}

fn on_off_value(items: &[String], i: usize) -> bool {
    items.get(i).and_then(|t| text_on(t)).unwrap_or(i == 0)
}

/// Settings stored as 0 (lowest) to 1 (highest), spread evenly over the
/// original's values (LOW, MEDIUM, HIGH or 0 to 10).
fn level_value(i: usize, n: usize) -> f32 {
    if n < 2 {
        1.0
    } else {
        i as f32 / (n - 1) as f32
    }
}

fn level_index(v: f32, n: usize) -> usize {
    ((v.clamp(0.0, 1.0) * (n.max(1) - 1) as f32).round() as usize).min(n.max(1) - 1)
}

/// FSAA values: "2X", "4X" and so on are that many samples; OFF and NONE
/// are 1. We draw 1, 2 or 4 (8 isn't supported by every graphics card),
/// so a higher value draws 4.
fn fsaa_samples(text: &str) -> u32 {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    match digits.parse::<u32>().unwrap_or(1) {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 4,
    }
}

/// What a GRAPHICS QUALITY value sets: texture and character detail,
/// blur, bloom, shadows and FSAA. `f` is the value's place in the list,
/// 0 (lowest) to 1 (highest). Our own choice: the original's presets set
/// its own engine's options, which don't all exist here.
fn quality_preset(f: f32) -> (f32, f32, bool, bool, bool, u32) {
    if f < 0.25 {
        (0.0, 0.0, false, false, false, 1)
    } else if f < 0.75 {
        (0.5, 0.5, false, false, false, 1)
    } else if f < 1.0 {
        (1.0, 1.0, false, false, false, 1)
    } else {
        (1.0, 1.0, false, true, true, 4)
    }
}

fn quality_now(s: &Settings) -> (f32, f32, bool, bool, bool, u32) {
    let v = &s.video;
    (
        v.texture_detail,
        v.character_detail,
        v.blur,
        v.bloom,
        v.squad_shadows,
        v.fsaa,
    )
}

fn is_custom(text: &str) -> bool {
    text.to_ascii_uppercase().contains("CUSTOM")
}

/// The GRAPHICS QUALITY values that are presets (not CUSTOM), in order.
fn presets(items: &[String]) -> Vec<usize> {
    (0..items.len()).filter(|&i| !is_custom(&items[i])).collect()
}

/// The number in a value's text ("32 CHANNELS" is 32), if it has one.
fn number_in(text: &str) -> Option<u32> {
    let digits: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn says_low(text: &str) -> bool {
    text.to_ascii_uppercase().contains("LOW")
}

/// Values to use when the original's list for a row has fewer than two
/// (the original fills some lists, such as FSAA, from the graphics card
/// while the game runs, so the stored list can be empty).
pub fn fallback_items(name: &str) -> Option<Vec<String>> {
    let l = |v: &[&str]| Some(v.iter().map(|s| s.to_string()).collect());
    match name {
        "fsaa" => l(&["OFF", "2X", "4X"]),
        "vsync" | "subtitles" | "prompt_text" | "blur" | "bloom" | "squad_shadows" | "sound_3d" => {
            l(&["ON", "OFF"])
        }
        "graphics_quality" => l(&["LOWEST", "LOW", "MEDIUM", "HIGH", "HIGHEST"]),
        "texture_detail" | "character_detail" | "sound_quality" => l(&["LOW", "MEDIUM", "HIGH"]),
        "audio_channels" => l(&["16", "32", "64"]),
        "brightness" | "voices_volume" | "effects_volume" | "music_volume" => l(&[
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]),
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
        "voices_volume" => tenth(s.audio.voices_volume),
        "sound_quality" => {
            if s.audio.low_quality {
                items.iter().position(|t| says_low(t)).unwrap_or(0)
            } else {
                items.iter().rposition(|t| !says_low(t)).unwrap_or(n - 1)
            }
        }
        // The original may name the kinds of 3D sound; anything but OFF is on.
        "sound_3d" => {
            let off = items.iter().position(|t| text_on(t) == Some(false));
            match (s.audio.sound_3d, off) {
                (true, Some(o)) => (0..n).find(|&i| i != o).unwrap_or(0),
                (false, Some(o)) => o,
                (on, None) => on_off_index(items, on),
            }
        }
        "audio_channels" => {
            let want = s.audio.channels;
            (0..n)
                .filter(|&i| number_in(&items[i]).is_some())
                .min_by_key(|&i| number_in(&items[i]).unwrap_or(0).abs_diff(want))
                .unwrap_or(0)
        }
        "video_resolution" => {
            let now = format!("{}x{}", s.video.width, s.video.height);
            items.iter().position(|i| i.eq_ignore_ascii_case(&now))?
        }
        "vsync" => yes_no(s.video.vsync),
        "subtitles" => on_off_index(items, s.game.subtitles),
        "prompt_text" => on_off_index(items, s.game.prompt_text),
        "blur" => on_off_index(items, s.video.blur),
        "bloom" => on_off_index(items, s.video.bloom),
        "squad_shadows" => on_off_index(items, s.video.squad_shadows),
        "brightness" => level_index(s.video.brightness, n),
        "texture_detail" => level_index(s.video.texture_detail, n),
        "character_detail" => level_index(s.video.character_detail, n),
        "fsaa" => {
            let want = s.video.fsaa;
            (0..n)
                .min_by_key(|&i| fsaa_samples(&items[i]).abs_diff(want))
                .unwrap_or(0)
        }
        "graphics_quality" => {
            let p = presets(items);
            let now = quality_now(s);
            // The value last chosen, if the rows still match it. Several
            // values can set the same rows (LOW and MEDIUM on a five-value
            // list), so the choice itself is kept, not worked out from the
            // rows, or the arrows would get stuck on the first of them.
            let chosen = level_index(s.video.quality, p.len());
            if quality_preset(level_value(chosen, p.len())) == now {
                return p.get(chosen).copied();
            }
            match p
                .iter()
                .enumerate()
                .find(|(k, _)| quality_preset(level_value(*k, p.len())) == now)
                .map(|(_, &i)| i)
            {
                Some(i) => i,
                // Changed by hand: CUSTOM if the list has it, else the
                // preset with the nearest texture detail.
                None => items.iter().position(|t| is_custom(t)).unwrap_or_else(|| {
                    p.get(level_index(s.video.texture_detail, p.len()))
                        .copied()
                        .unwrap_or(0)
                }),
            }
        }
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
        "voices_volume" => s.audio.voices_volume = (i as f32 / 10.0).clamp(0.0, 1.0),
        "sound_quality" => s.audio.low_quality = says_low(text),
        "sound_3d" => {
            s.audio.sound_3d = match text_on(text) {
                Some(on) => on,
                None if items.iter().any(|t| text_on(t) == Some(false)) => true,
                None => first,
            }
        }
        "audio_channels" => {
            let Some(c) = number_in(text) else {
                return false;
            };
            s.audio.channels = c.max(1);
        }
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
        "subtitles" => s.game.subtitles = on_off_value(items, i),
        "prompt_text" => s.game.prompt_text = on_off_value(items, i),
        "blur" => s.video.blur = on_off_value(items, i),
        "bloom" => s.video.bloom = on_off_value(items, i),
        "squad_shadows" => s.video.squad_shadows = on_off_value(items, i),
        "brightness" => s.video.brightness = level_value(i, items.len()),
        "texture_detail" => s.video.texture_detail = level_value(i, items.len()),
        "character_detail" => s.video.character_detail = level_value(i, items.len()),
        "fsaa" => s.video.fsaa = fsaa_samples(text),
        "graphics_quality" => {
            let p = presets(items);
            // CUSTOM leaves everything as it is.
            let Some(k) = p.iter().position(|&j| j == i) else {
                return true;
            };
            let place = level_value(k, p.len());
            let (tex, chars, blur, bloom, shadows, fsaa) = quality_preset(place);
            let v = &mut s.video;
            v.quality = place;
            v.texture_detail = tex;
            v.character_detail = chars;
            v.blur = blur;
            v.bloom = bloom;
            v.squad_shadows = shadows;
            v.fsaa = fsaa;
        }
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
        "subtitles" => s.game.subtitles = d.game.subtitles,
        "prompt_text" => s.game.prompt_text = d.game.prompt_text,
        "effects_volume" => s.audio.effects_volume = d.audio.effects_volume,
        "music_volume" => s.audio.music_volume = d.audio.music_volume,
        "voices_volume" => s.audio.voices_volume = d.audio.voices_volume,
        "sound_quality" => s.audio.low_quality = d.audio.low_quality,
        "sound_3d" => s.audio.sound_3d = d.audio.sound_3d,
        "audio_channels" => s.audio.channels = d.audio.channels,
        "video_resolution" => {
            s.video.width = d.video.width;
            s.video.height = d.video.height;
        }
        "vsync" => s.video.vsync = d.video.vsync,
        "brightness" => s.video.brightness = d.video.brightness,
        "texture_detail" => s.video.texture_detail = d.video.texture_detail,
        "character_detail" => s.video.character_detail = d.video.character_detail,
        "blur" => s.video.blur = d.video.blur,
        "bloom" => s.video.bloom = d.video.bloom,
        "squad_shadows" => s.video.squad_shadows = d.video.squad_shadows,
        "fsaa" => s.video.fsaa = d.video.fsaa,
        "graphics_quality" => {
            s.video.quality = d.video.quality;
            for name in [
                "texture_detail",
                "character_detail",
                "blur",
                "bloom",
                "squad_shadows",
                "fsaa",
            ] {
                restore(name, s);
            }
        }
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
            let items = items(name, &s0, None).unwrap_or_else(|| match *name {
                "fsaa" => list(&["OFF", "2X", "4X"]),
                "graphics_quality" => list(&["LOW", "MEDIUM", "HIGH", "CUSTOM"]),
                "blur" | "bloom" | "squad_shadows" | "sound_3d" | "subtitles" | "prompt_text" => list(&["ON", "OFF"]),
                "sound_quality" => list(&["LOW", "HIGH"]),
                "audio_channels" => list(&["16", "32", "64"]),
                _ => list(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]),
            });
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

    #[test]
    fn on_off_follows_the_words_not_the_order() {
        let off_first = list(&["OFF", "ON"]);
        let mut s = Settings::default();
        assert!(write("bloom", &mut s, 1, &off_first, None));
        assert!(s.video.bloom);
        assert_eq!(read("bloom", &s, &off_first), Some(1));
        assert!(write("bloom", &mut s, 0, &off_first, None));
        assert!(!s.video.bloom);
    }

    #[test]
    fn graphics_quality_sets_the_detail_rows_and_shows_custom_after_a_change() {
        let q = list(&["LOW", "MEDIUM", "HIGH", "CUSTOM"]);
        let mut s = Settings::default();
        assert!(write("graphics_quality", &mut s, 0, &q, None));
        assert_eq!(s.video.texture_detail, 0.0);
        assert_eq!(read("graphics_quality", &s, &q), Some(0));
        assert!(write("graphics_quality", &mut s, 2, &q, None));
        assert!(s.video.bloom && s.video.squad_shadows);
        assert_eq!(s.video.fsaa, 4);
        assert_eq!(read("graphics_quality", &s, &q), Some(2));
        // Changing one row by hand makes it CUSTOM.
        s.video.bloom = false;
        s.video.fsaa = 2;
        assert_eq!(read("graphics_quality", &s, &q), Some(3));
        // Choosing CUSTOM changes nothing.
        let before = s.clone();
        assert!(write("graphics_quality", &mut s, 3, &q, None));
        assert_eq!(s, before);
    }

    #[test]
    fn fsaa_reads_the_sample_count_from_the_text() {
        let f = list(&["NONE", "2X", "4X"]);
        let mut s = Settings::default();
        assert_eq!(read("fsaa", &s, &f), Some(0));
        assert!(write("fsaa", &mut s, 2, &f, None));
        assert_eq!(s.video.fsaa, 4);
        assert_eq!(read("fsaa", &s, &f), Some(2));
    }

    #[test]
    fn brightness_middle_value_is_the_default() {
        let eleven = list(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        let s = Settings::default();
        assert_eq!(read("brightness", &s, &eleven), Some(5));
    }

    #[test]
    fn sound_rows() {
        let mut s = Settings::default();
        let ch = list(&["8 CHANNELS", "16 CHANNELS", "32 CHANNELS"]);
        assert_eq!(read("audio_channels", &s, &ch), Some(2));
        assert!(write("audio_channels", &mut s, 0, &ch, None));
        assert_eq!(s.audio.channels, 8);
        let q = list(&["LOW", "MEDIUM", "HIGH"]);
        assert_eq!(read("sound_quality", &s, &q), Some(2));
        assert!(write("sound_quality", &mut s, 0, &q, None));
        assert!(s.audio.low_quality);
        assert_eq!(read("sound_quality", &s, &q), Some(0));
        // 3D sound named by kind: anything but OFF is on.
        let kinds = list(&["SOFTWARE", "HARDWARE", "OFF"]);
        assert!(write("sound_3d", &mut s, 2, &kinds, None));
        assert!(!s.audio.sound_3d);
        assert_eq!(read("sound_3d", &s, &kinds), Some(2));
        assert!(write("sound_3d", &mut s, 1, &kinds, None));
        assert!(s.audio.sound_3d);
        assert_eq!(read("sound_3d", &s, &kinds), Some(0));
    }

    #[test]
    fn graphics_quality_steps_through_values_that_share_a_preset() {
        // Found by the user: on a five-value list LOW and MEDIUM set the same
        // rows, and the arrows got stuck on LOW.
        let q = list(&["LOWEST", "LOW", "MEDIUM", "HIGH", "HIGHEST"]);
        let mut s = Settings::default();
        assert_eq!(read("graphics_quality", &s, &q), Some(3));
        for i in [4, 0, 1, 2, 3, 4, 3, 2, 1, 0] {
            assert!(write("graphics_quality", &mut s, i, &q, None));
            assert_eq!(read("graphics_quality", &s, &q), Some(i), "step to {}", q[i]);
        }
    }
}
