//! Draws the objective tracker on the screen: the tipped ring with its
//! arrow and the distance at the bottom middle, or the four pulsing corner
//! arrows round the goal when it is in view. The rules (what to show, where
//! the arrow points) are in `rc_scripts_plugin::tracker`; the pictures and
//! the font are the original's, read from the player's install
//! (`HUDTextures.Icons.ObjectiveRing`, `HUDTextures.Icons.LargeVisorIcons`,
//! `OrbitFonts.OrbitBold8`), and how they are drawn is the original
//! helmet's (Engine.dll `UHelmet::DrawHelmet`, see `data/ui/objective_tracker.toml`).
//!
//! The original draws in its own screen's pixels, with places given in
//! 800x600 units scaled to the screen. Here the unit is the screen's height
//! over 600 both ways, centred across, so the tracker is the same as the
//! original's on a 4:3 screen and keeps its shape on wider ones. It is
//! drawn on the screen itself, not through the helmet glass, as the
//! original draws it after the glass.

use crate::menu::GamePathsFound;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use rc_game_core::units::to_bevy;
use rc_game_core::{GameLog, GamePaths, GameSet, GameState, PlayerCamera};
use rc_menus::gpu;
use rc_menus::library::FontData;
use rc_scripts_plugin::tracker::{
    glow_alpha, pulse_alpha, table_cos, table_sin, widen_box, ObjectiveTracker, TrackerLook,
    TrackerView,
};

/// The original's screen height, in its units.
const BASE_LINES: f32 = 600.0;
const BASE_WIDTH: f32 = 800.0;

pub struct ObjectiveTrackerPlugin;

impl Plugin for ObjectiveTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrackerPictures>()
            .add_systems(OnEnter(GameState::InGame), spawn_parts)
            .add_systems(
                Update,
                draw.in_set(GameSet::Hud)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

/// A picture ready to draw (the colour baked in).
#[derive(Clone)]
struct Picture {
    image: Handle<Image>,
}

#[derive(Resource, Default)]
struct TrackerPictures {
    tried: bool,
    ring: Option<Picture>,
    arrow: Option<Picture>,
    font: Option<(FontData, Vec<Option<Picture>>)>,
    /// What was last written to the log, to write each change once.
    said: String,
}

#[derive(Component)]
struct TrackerRoot;

/// The ring's tipping parent (shadow first, then the ring).
#[derive(Component, Clone, Copy, PartialEq)]
enum RingPart {
    Shadow,
    Ring,
}

/// The picture inside a [`RingPart`], turned toward the goal.
#[derive(Component)]
struct RingPicture;

#[derive(Component)]
struct DistanceText {
    shown: String,
    for_height: f32,
}

/// One corner arrow: which corner, and which of its three copies.
#[derive(Component, Clone, Copy)]
struct Arrow {
    corner: usize,
    copy: usize,
}

fn absolute() -> Node {
    Node {
        position_type: PositionType::Absolute,
        ..default()
    }
}

fn spawn_parts(mut commands: Commands, roots: Query<(), With<TrackerRoot>>) {
    if !roots.is_empty() {
        return;
    }
    commands
        .spawn((
            TrackerRoot,
            crate::cutscene::HudRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            // Over the HUD (2), under the menus (10).
            GlobalZIndex(3),
            DespawnOnExit(GameState::InGame),
        ))
        .with_children(|root| {
            for part in [RingPart::Shadow, RingPart::Ring] {
                root.spawn((part, absolute(), UiTransform::IDENTITY, Visibility::Hidden))
                    .with_children(|p| {
                        p.spawn((
                            RingPicture,
                            ImageNode::default(),
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            UiTransform::IDENTITY,
                        ));
                    });
            }
            root.spawn((
                DistanceText {
                    shown: String::new(),
                    for_height: 0.0,
                },
                absolute(),
                Visibility::Hidden,
            ));
            for corner in 0..4 {
                for copy in 0..3 {
                    root.spawn((
                        Arrow { corner, copy },
                        ImageNode::default(),
                        absolute(),
                        UiTransform::IDENTITY,
                        Visibility::Hidden,
                    ));
                }
            }
        });
}

/// A texture with the tracker's colour baked in (red green blue; opacity
/// is set per drawing), as the rest of the original HUD is drawn.
fn load(
    library: &mut rc_menus::Library,
    path: &str,
    colour: [u8; 3],
    images: &mut Assets<Image>,
    log: &GameLog,
) -> Option<Picture> {
    match library.texture(path) {
        Ok(rgba) => {
            let mut pixels = rgba.pixels.clone();
            gpu::prepare(
                &mut pixels,
                gpu::Variant::AsIs,
                [colour[0], colour[1], colour[2], 255],
            );
            let image = Image::new(
                Extent3d {
                    width: rgba.width as u32,
                    height: rgba.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            log.line(format!(
                "objective tracker: picture {path}: {}x{}",
                rgba.width, rgba.height
            ));
            Some(Picture {
                image: images.add(image),
            })
        }
        Err(e) => {
            log.line(format!(
                "objective tracker: picture {path}: {e}; that part is not drawn"
            ));
            None
        }
    }
}

fn read_pictures(
    pictures: &mut TrackerPictures,
    look: &TrackerLook,
    paths: &GamePaths,
    images: &mut Assets<Image>,
    log: &GameLog,
) {
    pictures.tried = true;
    let mut library = rc_menus::Library::new(&paths.game_data());
    pictures.ring = load(
        &mut library,
        &look.ring_texture,
        look.ring_colour,
        images,
        log,
    );
    pictures.arrow = load(
        &mut library,
        &look.bracket_texture,
        look.ring_colour,
        images,
        log,
    );
    let c = look.distance_colour;
    pictures.font = match library.font(&look.distance_font) {
        Ok(f) => {
            let pages = f
                .pages
                .iter()
                .map(|p| load(&mut library, p, [c[0], c[1], c[2]], images, log))
                .collect();
            Some((f, pages))
        }
        Err(e) => {
            log.line(format!(
                "objective tracker: font {}: {e}; the distance is not drawn",
                look.distance_font
            ));
            None
        }
    };
}

fn set_visible(v: &mut Visibility, on: bool) {
    let want = if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *v != want {
        *v = want;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    tracker: Option<Res<rc_scripts_plugin::tracker::ObjectiveTracker>>,
    mut pictures: ResMut<TrackerPictures>,
    (paths, found, log, time): (Res<GamePaths>, Res<GamePathsFound>, Res<GameLog>, Res<Time>),
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut rings: Query<(
        &RingPart,
        &mut Node,
        &mut UiTransform,
        &mut Visibility,
        &Children,
    )>,
    mut ring_pictures: Query<
        (&mut ImageNode, &mut UiTransform),
        (With<RingPicture>, Without<RingPart>, Without<Arrow>),
    >,
    mut text: Query<
        (Entity, &mut DistanceText, &mut Node, &mut Visibility),
        (Without<RingPart>, Without<Arrow>),
    >,
    mut arrows: Query<
        (
            &Arrow,
            &mut ImageNode,
            &mut Node,
            &mut UiTransform,
            &mut Visibility,
        ),
        (
            Without<RingPart>,
            Without<RingPicture>,
            Without<DistanceText>,
        ),
    >,
) {
    let Some(tracker) = tracker else { return };
    let tracker: &ObjectiveTracker = &tracker;
    let look = &tracker.look;
    if !pictures.tried && found.0 && tracker.view != TrackerView::Hidden {
        read_pictures(&mut pictures, look, &paths, &mut images, &log);
    }
    let Ok(window) = windows.single() else {
        return;
    };
    // Physical pixels: the original works in its screen's pixels.
    let scale = window.scale_factor();
    let (w, h) = (
        window.physical_width() as f32,
        window.physical_height() as f32,
    );
    let u = h / BASE_LINES;
    let to_x = |x: f32| w * 0.5 + (x - BASE_WIDTH * 0.5) * u;
    let px = |v: f32| Val::Px(v / scale);

    let (ring_on, angle, distance) = match &tracker.view {
        TrackerView::Ring { angle, distance } => (true, *angle, distance.as_str()),
        _ => (false, 0, ""),
    };
    // The ring: a picture centred at `ring_centre`, turned clockwise by
    // the angle, then flattened by cos(tilt) (DrawRotatedTile). The
    // parent node flattens; the picture inside it turns.
    let ring = pictures.ring.clone().filter(|_| ring_on);
    for (part, mut node, mut tf, mut vis, children) in &mut rings {
        set_visible(&mut vis, ring.is_some());
        let Some(ring) = &ring else { continue };
        let size = [look.ring_size[0] * u, look.ring_size[1] * u];
        let down = if *part == RingPart::Shadow {
            look.ring_shadow_down * u
        } else {
            0.0
        };
        let (cx, cy) = (to_x(look.ring_centre[0]), look.ring_centre[1] * u + down);
        let want = Node {
            position_type: PositionType::Absolute,
            left: px(cx - size[0] * 0.5),
            top: px(cy - size[1] * 0.5),
            width: px(size[0]),
            height: px(size[1]),
            ..default()
        };
        if *node != want {
            *node = want;
        }
        let flat = UiTransform::from_scale(Vec2::new(1.0, table_cos(look.ring_tilt)));
        if *tf != flat {
            *tf = flat;
        }
        let alpha = if *part == RingPart::Shadow {
            look.ring_shadow_alpha
        } else {
            255
        };
        for c in children.iter() {
            let Ok((mut image, mut turn)) = ring_pictures.get_mut(c) else {
                continue;
            };
            image.image = ring.image.clone();
            let s = look.ring_source;
            image.rect = Some(Rect::new(s[0], s[1], s[0] + s[2], s[1] + s[3]));
            image.color = Color::srgba_u8(255, 255, 255, alpha);
            *turn =
                UiTransform::from_rotation(Rot2::from_sin_cos(table_sin(angle), table_cos(angle)));
        }
    }

    // The distance, centred on the ring's centre (DrawString with the
    // centre flag: each letter its width and height times the scale,
    // whole pixels; the line moved left half its width and up half its
    // height, whole pixels).
    if let Ok((entity, mut shown, mut node, mut vis)) = text.single_mut() {
        let font = pictures.font.as_ref().filter(|_| ring_on);
        set_visible(&mut vis, font.is_some());
        if let Some((font, pages)) = font {
            if shown.shown != distance || shown.for_height != h {
                shown.shown = distance.to_string();
                shown.for_height = h;
                commands.entity(entity).despawn_related::<Children>();
                let glyphs: Vec<_> = distance
                    .chars()
                    .filter_map(|c| font.glyph(c))
                    .map(|g| {
                        (
                            g,
                            (g.width as f32 * u).trunc(),
                            (g.height as f32 * u).trunc(),
                        )
                    })
                    .collect();
                let width: f32 = glyphs.iter().map(|g| g.1).sum();
                let height = glyphs.iter().map(|g| g.2).fold(0.0, f32::max);
                let x0 = to_x(look.ring_centre[0]).trunc() - (width / 2.0).trunc();
                let y0 = (look.ring_centre[1] * u).trunc() - (height / 2.0).trunc();
                *node = Node {
                    position_type: PositionType::Absolute,
                    left: px(x0),
                    top: px(y0),
                    ..default()
                };
                let mut x = 0.0;
                commands.entity(entity).with_children(|t| {
                    for (g, gw, gh) in glyphs {
                        if let Some(Some(page)) = pages.get(g.page as usize) {
                            t.spawn((
                                ImageNode {
                                    image: page.image.clone(),
                                    rect: Some(Rect::new(
                                        g.u as f32,
                                        g.v as f32,
                                        (g.u + g.width) as f32,
                                        (g.v + g.height) as f32,
                                    )),
                                    color: Color::srgba_u8(255, 255, 255, look.distance_colour[3]),
                                    ..default()
                                },
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(x),
                                    top: px(0.0),
                                    width: px(gw),
                                    height: px(gh),
                                    ..default()
                                },
                            ));
                        }
                        x += gw;
                    }
                });
            }
        }
    }

    // The corner arrows: the corners put on the screen with the player's
    // camera, the box pushed out to its least size, then at each corner
    // the arrow picture turned by its corner's turn, three times.
    let corners = match (&tracker.view, camera.single(), &pictures.arrow) {
        (TrackerView::Brackets { corners }, Ok((cam, cam_t)), Some(_)) => {
            let on_screen: Option<Vec<[f32; 2]>> = corners
                .iter()
                .map(|c| {
                    let p = rc_movement::Vec3::new(c[0], c[1], c[2]);
                    cam.world_to_viewport(cam_t, to_bevy(p))
                        .ok()
                        .map(|v| [v.x * scale, v.y * scale])
                })
                .collect();
            on_screen.map(|p| widen_box([p[0], p[1], p[2], p[3]], look.bracket_min_pixels))
        }
        _ => None,
    };
    let alpha = pulse_alpha(look, time.elapsed_secs());
    let glow = glow_alpha(look, alpha);
    for (arrow, mut image, mut node, mut tf, mut vis) in &mut arrows {
        set_visible(&mut vis, corners.is_some());
        let (Some(corners), Some(pic)) = (&corners, &pictures.arrow) else {
            continue;
        };
        let [cx, cy] = corners[arrow.corner];
        let size = look.bracket_size
            * u
            * match arrow.copy {
                0 => 1.0,
                n => look.glow_sizes[n - 1],
            };
        if image.image != pic.image {
            image.image = pic.image.clone();
        }
        let s = look.bracket_source;
        image.rect = Some(Rect::new(s[0], s[1], s[2], s[3]));
        image.color = Color::srgba_u8(255, 255, 255, if arrow.copy == 0 { alpha } else { glow });
        *node = Node {
            position_type: PositionType::Absolute,
            left: px(cx - size * 0.5),
            top: px(cy - size * 0.5),
            width: px(size),
            height: px(size),
            ..default()
        };
        let a = look.bracket_turns[arrow.corner];
        *tf = UiTransform::from_rotation(Rot2::from_sin_cos(table_sin(a), table_cos(a)));
    }

    // The log: each change of what is drawn, once.
    let said = match &tracker.view {
        TrackerView::Hidden => String::new(),
        TrackerView::Ring { .. } if pictures.ring.is_some() => "the ring".to_string(),
        TrackerView::Brackets { .. } if corners.is_some() => "the corner arrows".to_string(),
        _ => String::new(),
    };
    if said != pictures.said {
        if !said.is_empty() {
            log.line(format!(
                "objective tracker: drawing {said} at {w:.0}x{h:.0}"
            ));
        }
        pictures.said = said;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At 800x600 the unit is one pixel and the ring sits where the
    /// original puts it.
    #[test]
    fn four_by_three_is_the_original() {
        let (w, h) = (800.0, 600.0);
        let u = h / BASE_LINES;
        let to_x = |x: f32| w * 0.5 + (x - BASE_WIDTH * 0.5) * u;
        assert_eq!(to_x(400.0), 400.0);
        assert_eq!(to_x(0.0), 0.0);
        // Wider screens: the same height, centred.
        let (w, h) = (3440.0, 1440.0);
        let u = h / BASE_LINES;
        let to_x = |x: f32| w * 0.5 + (x - BASE_WIDTH * 0.5) * u;
        assert_eq!(to_x(400.0), 1720.0);
        assert!((463.0 * u - 1111.2).abs() < 0.01);
    }
}
