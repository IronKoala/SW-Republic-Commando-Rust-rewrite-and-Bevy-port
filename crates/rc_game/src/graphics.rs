//! The GRAPHICS options that change how the 3D view is drawn: brightness,
//! texture and character detail, blur, bloom, squad shadows and FSAA (the
//! screen size and vsync are in `menu::apply_video`).
//!
//! Every camera gets the same edge smoothing and colour range, because the
//! cameras draw on top of one another (sky, level, arms, then menus and the
//! display) and Bevy can only stack them when they match. Brightness, bloom
//! and blur go on the 3D cameras only, so the menus and the display keep the
//! original's colours.
//!
//! Each change is applied straight away, except texture and character
//! detail: pictures are read when a level loads, so those count from the
//! next level load. The log says what was applied (`graphics:` lines).

use crate::settings::{Settings, VideoSection};
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, Msaa};
use rc_game_core::{GameLog, PlayerCamera};
use rc_package::texture::{set_skip_mips, Detail};

pub struct GraphicsOptionsPlugin;

impl Plugin for GraphicsOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply_detail, apply_to_cameras, apply_shadows));
    }
}

/// The pieces of the video settings this file uses, to see when they
/// change.
#[derive(Clone, PartialEq, Debug)]
struct Look {
    brightness: f32,
    blur: bool,
    bloom: bool,
    fsaa: u32,
}

impl Look {
    fn of(v: &VideoSection) -> Look {
        Look {
            brightness: v.brightness,
            blur: v.blur,
            bloom: v.bloom,
            fsaa: v.fsaa,
        }
    }
}

/// How many mipmap steps (halvings) to skip for a detail setting of 0
/// (lowest) to 1 (full detail): 2, 1 or 0.
pub fn skipped_halvings(detail: f32) -> u8 {
    ((1.0 - detail.clamp(0.0, 1.0)) * 2.0).round() as u8
}

/// BRIGHTNESS as an exposure change on the 3D view, in stops (each stop
/// doubles or halves the light): 0.5 changes nothing, 0 is one stop
/// darker, 1 one stop brighter. Our own range, not measured against the
/// original. (A gamma curve would only work with bloom on: Bevy grades
/// colours as it draws only by exposure.)
pub fn brightness_stops(brightness: f32) -> f32 {
    (brightness.clamp(0.0, 1.0) - 0.5) * 2.0
}

fn msaa(samples: u32) -> Msaa {
    match samples {
        0 | 1 => Msaa::Off,
        2 | 3 => Msaa::Sample2,
        // 8 isn't supported by every graphics card for the wide colour
        // range bloom needs (the cloud test's isn't), and 4 is: every card
        // and phone can do 4.
        _ => Msaa::Sample4,
    }
}

fn apply_detail(settings: Res<Settings>, mut last: Local<Option<(u8, u8)>>, log: Res<GameLog>) {
    let now = (
        skipped_halvings(settings.video.texture_detail),
        skipped_halvings(settings.video.character_detail),
    );
    if *last == Some(now) {
        return;
    }
    set_skip_mips(Detail::World, now.0);
    set_skip_mips(Detail::Characters, now.1);
    log.line(format!(
        "graphics: textures at 1/{} size, characters at 1/{} size (from the next level load)",
        1 << now.0,
        1 << now.1
    ));
    *last = Some(now);
}

#[allow(clippy::type_complexity)]
fn apply_to_cameras(
    mut commands: Commands,
    settings: Res<Settings>,
    cameras: Query<(Entity, &Camera, Has<Camera3d>, Has<PlayerCamera>)>,
    new: Query<(), Added<Camera>>,
    mut last: Local<Option<Look>>,
    log: Res<GameLog>,
) {
    let look = Look::of(&settings.video);
    let changed = last.as_ref() != Some(&look);
    if !changed && new.is_empty() {
        return;
    }
    if changed {
        log.line(format!(
            "graphics: brightness {:.2} ({:+.2} stops), blur {}, bloom {}, fsaa {}x",
            look.brightness,
            brightness_stops(look.brightness),
            look.blur,
            look.bloom,
            look.fsaa.max(1)
        ));
        *last = Some(look.clone());
    }
    let grading = ColorGrading {
        global: ColorGradingGlobal {
            exposure: brightness_stops(look.brightness),
            ..default()
        },
        ..default()
    };
    // Without bloom each 3D camera grades what it draws as it draws it, so
    // all of them get the brightness. With bloom (wide colour range) the
    // grading is a pass over the whole picture after each camera, so only
    // the last 3D camera gets it, or the sky would be brightened three times.
    let last_3d = cameras
        .iter()
        .filter(|(_, _, is_3d, _)| *is_3d)
        .max_by_key(|(_, c, _, _)| c.order)
        .map(|(e, ..)| e);
    for (e, _, is_3d, is_player) in &cameras {
        let mut c = commands.entity(e);
        c.insert(msaa(look.fsaa));
        if look.bloom {
            c.insert(Hdr);
        } else {
            c.remove::<Hdr>();
        }
        if !is_3d {
            continue;
        }
        // Linear draws the same as None (the colours the light was matched
        // to) but, unlike None, lets the colour grading run.
        let graded = !look.bloom || Some(e) == last_3d;
        c.insert((
            Tonemapping::Linear,
            if graded {
                grading.clone()
            } else {
                ColorGrading::default()
            },
        ));
        // Bloom and blur on the level's own view: it is drawn over the sky
        // and under the arms, so the glow and blur cover the world once.
        if look.bloom && is_player {
            c.insert(Bloom::OLD_SCHOOL);
        } else {
            c.remove::<Bloom>();
        }
        if look.blur && is_player {
            c.insert(MotionBlur::default());
        } else {
            c.remove::<MotionBlur>();
        }
    }
}

#[allow(clippy::type_complexity)]
fn apply_shadows(
    settings: Res<Settings>,
    mut lights: ParamSet<(
        Query<&mut PointLight>,
        Query<&mut SpotLight>,
        Query<&mut DirectionalLight>,
    )>,
    mut last: Local<Option<bool>>,
    log: Res<GameLog>,
) {
    // Checked every frame so lights a level adds later follow it too; only
    // a light that differs is touched.
    let on = settings.video.squad_shadows;
    if *last != Some(on) {
        log.line(format!("graphics: squad shadows {on}"));
        *last = Some(on);
    }
    for mut l in &mut lights.p0() {
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
    for mut l in &mut lights.p1() {
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
    for mut l in &mut lights.p2() {
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_levels_skip_up_to_two_halvings() {
        assert_eq!(skipped_halvings(1.0), 0);
        assert_eq!(skipped_halvings(0.5), 1);
        assert_eq!(skipped_halvings(0.0), 2);
    }

    #[test]
    fn middle_brightness_changes_nothing() {
        assert_eq!(brightness_stops(0.5), 0.0);
        assert_eq!(brightness_stops(1.0), 1.0);
        assert_eq!(brightness_stops(0.0), -1.0);
    }
}
