//! The GRAPHICS options that change how the 3D view is drawn: brightness,
//! texture and character detail, blur, bloom, squad shadows and FSAA (the
//! screen size and vsync are in `display.rs`).
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
use rc_game_core::{GameLog, GamePaths, PlayerCamera};
use rc_package::texture::{set_skip_mips, Detail};

pub struct GraphicsOptionsPlugin;

impl Plugin for GraphicsOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_presets)
            .add_systems(
                Update,
                (
                    find_fsaa_levels,
                    apply_detail,
                    apply_to_cameras,
                    apply_shadows,
                    squad_blobs,
                ),
            )
            .add_observer(match_new_camera);
    }
}

/// Read the GRAPHICS QUALITY presets from the install's `System.ini`
/// (`graphics_presets.rs`) and say in the log what they are.
fn load_presets(paths: Option<Res<GamePaths>>, log: Res<GameLog>) {
    let Some(paths) = paths else {
        return;
    };
    let shipped = paths.game_file("GameData/System/System.ini");
    let own = paths.game_file("GameData/Save/System.ini");
    match crate::graphics_presets::Presets::from_install(&shipped, &own) {
        Ok(p) => {
            for (i, x) in p.list.iter().enumerate() {
                log.line(format!(
                    "graphics: preset {i} {}: textures {:?}, DropLODBy {}, shadows {}, bump {}, blur {}, bloom {}, projectors {}, FSAA {}, size {}x{}",
                    x.key, x.textures, x.drop_lod_by, x.shadows, x.bump, x.blur, x.bloom, x.projectors, x.fsaa, x.windowed.0, x.windowed.1
                ));
            }
            if p.list.is_empty() {
                log.line("graphics: the install's System.ini has no GRAPHICS QUALITY presets; the row will show CUSTOM");
            }
            log.line(format!(
                "graphics: SET RECOMMENDED uses {:?}{}",
                p.recommended,
                if p.recommended().is_some() {
                    ""
                } else {
                    " (no such preset: SET RECOMMENDED does nothing)"
                }
            ));
            crate::graphics_presets::set(p);
        }
        Err(e) => log.line(format!("graphics: no GRAPHICS QUALITY presets: {e}")),
    }
}

/// The pieces of the video settings this file uses, to see when they
/// change.
#[derive(Clone, PartialEq, Debug)]
struct Look {
    gamma: f32,
    blur: bool,
    bloom: bool,
    fsaa: u32,
}

impl Look {
    fn of(v: &VideoSection) -> Look {
        Look {
            gamma: v.gamma,
            blur: v.blur,
            bloom: v.bloom_quality > 0,
            fsaa: v.fsaa,
        }
    }
}

/// How many mipmap steps (halvings) to skip for a detail setting of 0
/// (lowest) to 1 (full detail): 2, 1 or 0. The port's own sizes: what the
/// original's names (Lower, Normal, High, UltraHigh) do to each picture is
/// inside its engine, not in its files.
pub fn skipped_halvings(detail: f32) -> u8 {
    ((1.0 - detail.clamp(0.0, 1.0)) * 2.0).round() as u8
}

/// BRIGHTNESS as an exposure change on the 3D view, in stops (each stop
/// doubles or halves the light), for the original's Gamma (1.0 changes
/// nothing).
///
/// The original sets the screen's gamma ramp: each output level x (0 to 1)
/// becomes x ^ (1 / Gamma) (its D3DDrv.dll, UpdateGamma). Bevy can't bend
/// the whole screen like that yet, so this is the exposure that gives a
/// middle grey (output level 0.5) the same brightness as the original's
/// curve does. Darker and lighter parts differ a little from the original,
/// and the menus and display aren't changed (the original's are).
pub fn brightness_stops(gamma: f32) -> f32 {
    let gamma = gamma.clamp(0.5, 2.5);
    let lin = |x: f32| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    (lin(0.5f32.powf(1.0 / gamma)) / lin(0.5)).log2()
}

fn msaa(samples: u32) -> Msaa {
    // Only counts the graphics card offered are in the FSAA list
    // ([`find_fsaa_levels`]); a count it can't do (from an edited settings
    // file, or another PC) falls back to the most it can.
    let offered = crate::graphics_presets::fsaa_levels();
    let best = offered
        .iter()
        .copied()
        .filter(|&n| n <= samples)
        .max()
        .unwrap_or(1);
    match best {
        0 | 1 => Msaa::Off,
        2 | 3 => Msaa::Sample2,
        4..=7 => Msaa::Sample4,
        _ => Msaa::Sample8,
    }
}

/// The picture formats every camera may draw to: the window's, the wide
/// colour range bloom uses, and depth.
const DRAWN_FORMATS: [bevy::render::render_resource::TextureFormat; 4] = [
    bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb,
    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
    bevy::render::render_resource::TextureFormat::Rgba16Float,
    bevy::render::render_resource::TextureFormat::Depth32Float,
];

/// FSAA's list, as the original's: ask the graphics card which sample
/// counts it can draw with (the original asks for 1 to 16; Bevy draws 2, 4
/// or 8). Only counts every format above supports are offered.
fn find_fsaa_levels(
    adapter: Option<Res<bevy::render::renderer::RenderAdapter>>,
    mut done: Local<bool>,
    log: Res<GameLog>,
) {
    let Some(adapter) = adapter else {
        return;
    };
    if *done {
        return;
    }
    *done = true;
    let levels: Vec<u32> = [2, 4, 8]
        .into_iter()
        .filter(|&n| {
            DRAWN_FORMATS.iter().all(|&f| {
                adapter
                    .get_texture_format_features(f)
                    .flags
                    .sample_count_supported(n)
            })
        })
        .collect();
    log.line(format!(
        "graphics: the graphics card offers FSAA {:?}",
        levels
    ));
    crate::graphics_presets::set_fsaa_levels(levels);
}

/// Gives a camera the edge smoothing (and wide colour range) the others
/// have the moment it is made. Cameras that draw into the same window must
/// agree, or the graphics card refuses the frame and Bevy quits ("the depth
/// attachment's texture view has count 1 but ... the color attachment ...
/// has count 4"). [`apply_to_cameras`] alone was a frame late: the black
/// bars' camera (true full screen at a size other than the screen's) is made
/// after it runs, and was drawn once with Bevy's default of 4 samples.
fn match_new_camera(add: On<Add<Camera>>, settings: Option<Res<Settings>>, mut commands: Commands) {
    let Some(settings) = settings else {
        return;
    };
    let mut c = commands.entity(add.entity);
    c.insert(msaa(settings.video.fsaa));
    if settings.video.bloom_quality > 0 {
        c.insert(Hdr);
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
            "graphics: gamma {:.2} ({:+.2} stops), blur {}, bloom {}, fsaa {}x",
            look.gamma,
            brightness_stops(look.gamma),
            look.blur,
            look.bloom,
            look.fsaa.max(1)
        ));
        *last = Some(look.clone());
    }
    let grading = ColorGrading {
        global: ColorGradingGlobal {
            exposure: brightness_stops(look.gamma),
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

/// A soft dark patch on the ground under a squad mate (SQUAD SHADOWS).
#[derive(Component)]
struct Blob;

/// Size of the patch, Bevy units across (a commando's shoulders are about
/// 0.7).
const BLOB_SIZE: f32 = 1.1;
/// How dark its middle is (0 clear, 1 black).
const BLOB_DARKNESS: f32 = 0.55;

/// A round picture, dark in the middle and fading to clear at the edge.
fn blob_picture() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: usize = 64;
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / N as f32 * 2.0 - Vec2::ONE;
            let fade = (1.0 - d.length()).clamp(0.0, 1.0);
            // Smooth edge, darker middle.
            let a = fade * fade * (3.0 - 2.0 * fade) * BLOB_DARKNESS;
            px.extend([0, 0, 0, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// SQUAD SHADOWS also puts a soft shadow under each squad mate, as the
/// original's squad shadows do (it projects one onto the ground). Ours is
/// a flat patch at their feet, which needs no light and costs almost
/// nothing; it doesn't follow slopes or stairs. Moving lights casting
/// shadows (above) only shows where the level has one, which is rare.
#[allow(clippy::too_many_arguments)]
fn squad_blobs(
    mut commands: Commands,
    settings: Res<Settings>,
    mates: Query<(Entity, Has<Children>), With<rc_squad_plugin::SquadMate>>,
    blobs: Query<(Entity, &ChildOf), With<Blob>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut look: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
) {
    if !settings.video.squad_shadows {
        for (e, _) in &blobs {
            commands.entity(e).despawn();
        }
        return;
    }
    let (mesh, material) = look
        .get_or_insert_with(|| {
            let mesh = meshes.add(Plane3d::default().mesh().size(BLOB_SIZE, BLOB_SIZE));
            let material = materials.add(StandardMaterial {
                base_color_texture: Some(images.add(blob_picture())),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            });
            (mesh, material)
        })
        .clone();
    for (mate, _) in &mates {
        if blobs.iter().any(|(_, p)| p.parent() == mate) {
            continue;
        }
        commands.entity(mate).with_children(|c| {
            c.spawn((
                Blob,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                // Just above the floor, so it isn't hidden in it.
                Transform::from_xyz(0.0, 0.02, 0.0),
                bevy::light::NotShadowCaster,
            ));
        });
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
    characters: Option<ResMut<rc_shadows::CharacterShadows>>,
) {
    // Checked every frame so lights a level adds later follow it too; only
    // a light that differs is touched.
    let on = settings.video.squad_shadows;
    // The original's "Shadows" option (this row) is what switches its
    // character shadows on and off.
    // Those shadows are projectors in the original, so PROJECTORS off
    // switches them off too.
    if let Some(mut c) = characters {
        let shadows = on && settings.video.projectors;
        if c.on != shadows {
            c.on = shadows;
        }
    }
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

    /// A camera made at any time (like the black bars' one) has the
    /// setting's edge smoothing straight away, not a frame later.
    #[test]
    fn a_new_camera_matches_the_others_at_once() {
        crate::graphics_presets::set_fsaa_levels(vec![2, 4, 8]);
        for (fsaa, bloom, want) in [(1, false, Msaa::Off), (4, true, Msaa::Sample4)] {
            let mut world = World::new();
            let mut settings = Settings::default();
            settings.video.fsaa = fsaa;
            settings.video.bloom_quality = if bloom { 2 } else { 0 };
            world.insert_resource(settings);
            world.add_observer(match_new_camera);
            let e = world.spawn(Camera::default()).id();
            world.flush();
            assert_eq!(world.get::<Msaa>(e), Some(&want));
            assert_eq!(world.get::<Hdr>(e).is_some(), bloom);
        }
    }

    #[test]
    fn detail_levels_skip_up_to_two_halvings() {
        assert_eq!(skipped_halvings(1.0), 0);
        assert_eq!(skipped_halvings(0.5), 1);
        assert_eq!(skipped_halvings(0.0), 2);
    }

    #[test]
    fn gamma_one_changes_nothing() {
        assert!(brightness_stops(1.0).abs() < 1e-6);
        assert!(brightness_stops(2.5) > 1.0);
        assert!(brightness_stops(0.5) < -1.0);
    }
}
