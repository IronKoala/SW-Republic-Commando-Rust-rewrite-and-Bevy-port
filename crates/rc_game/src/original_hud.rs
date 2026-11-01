//! The original game's HUD pictures, read from the player's install, in
//! place of the placeholder HUD's drawn shapes. The original draws its HUD
//! in native code (`CTHUD`), so there is no layout to read: which picture
//! goes where is our own choice, in `data/ui/hud.toml`.
//!
//! The crosshair: each weapon (by the name in its weapon file) can name its
//! reticle picture; the rest use the default one. When a picture can't be
//! read, the placeholder's four bars stay. The size (one picture pixel per
//! line of a 480-line screen) is a guess to check against the real game.
//!
//! The rest (shield, health, ammunition, grenades, squad, messages) is laid
//! out by `rc_menus::hud` from the `[layout]` and `[[element]]` parts of
//! the same file, and drawn here as pictures. While it is drawn, the
//! placeholder's own drawings are hidden; if any of its pictures or fonts
//! can't be read, the placeholder stays and the log says why.
#![allow(deprecated)]

use crate::hud::{CrossBar, PlaceholderPart};
use crate::menu::GamePathsFound;
use crate::settings::Settings;
use crate::theme::Theme;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use rc_game_core::{
    GameLog, GamePaths, GameSet, GameState, HudMessage, PlayerVitals, SquadReadout, SquadStatus,
    WeaponReadout,
};
use rc_menus::gpu;
use rc_menus::hud::{HudLayout, HudValues, SquadLine};
use rc_menus::quads::Quad;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

/// The original's HUD is laid out for this many lines.
const BASE_LINES: f32 = 480.0;

/// `data/ui/hud.toml`.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HudPlan {
    pub crosshair: CrosshairPlan,
    /// Read by `rc_menus::hud` (see `HudLayout`).
    #[serde(default)]
    layout: Option<toml::Value>,
    #[serde(default)]
    element: Option<toml::Value>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CrosshairPlan {
    /// Use the original's pictures at all.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The picture for weapons not listed below.
    pub default: String,
    /// 1 = one picture pixel per line of a 480-line screen.
    #[serde(default = "one")]
    pub scale: f32,
    /// Weapon name (as in its weapon file) to picture.
    #[serde(default)]
    pub weapons: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl HudPlan {
    pub fn parse(text: &str) -> Result<HudPlan, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The reticle picture for a weapon.
    pub fn reticle_for(&self, weapon: &str) -> &str {
        self.crosshair
            .weapons
            .iter()
            .find(|(w, _)| w.eq_ignore_ascii_case(weapon))
            .map_or(self.crosshair.default.as_str(), |(_, p)| p.as_str())
    }
}

/// A picture ready to draw, and its size in pixels.
type Picture = Option<(Handle<Image>, Vec2)>;

#[derive(Resource, Default)]
struct OriginalHud {
    plan: Option<HudPlan>,
    library: Option<rc_menus::Library>,
    /// By picture path (lower case); `None` if it couldn't be read.
    pictures: HashMap<String, Picture>,
    /// The picture shown now.
    showing: Option<String>,
}

/// The rest of the original HUD (see the top of this file).
#[derive(Resource, Default)]
struct OriginalLayout {
    layout: Option<HudLayout>,
    assets: Option<rc_menus::Assets>,
    /// Pictures ready to draw, by (texture, how it blends, tint).
    images: HashMap<(String, gpu::Variant, [u8; 4]), Handle<Image>>,
    /// What was drawn last, and on what screen, to redraw only on change.
    drawn_for: Option<(HudValues, [u32; 2], u32)>,
    /// The quads drawn now, in screen fractions (for the script's check).
    pub drawn: Vec<Quad>,
    /// False once a picture or font couldn't be read: the placeholder
    /// stays.
    working: bool,
    /// Short messages and their seconds left.
    messages: Vec<(String, f32)>,
}

#[derive(Component)]
struct LayoutRoot;

#[derive(Component)]
struct CrossPicture;

pub struct OriginalHudPlugin;

impl Plugin for OriginalHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OriginalHud>()
            .init_resource::<OriginalLayout>()
            .add_systems(Startup, read_plan)
            .add_systems(OnExit(GameState::InGame), forget_drawn)
            .add_systems(
                Update,
                (
                    spawn_picture,
                    update_crosshair,
                    collect_messages,
                    draw_layout,
                    hide_placeholder,
                )
                    .chain()
                    .in_set(GameSet::Hud)
                    .after(crate::hud::HudSystems)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn read_plan(
    mut hud: ResMut<OriginalHud>,
    mut original: ResMut<OriginalLayout>,
    paths: Res<GamePaths>,
    found: Res<GamePathsFound>,
    log: Res<GameLog>,
) {
    const REL: &str = "ui/hud.toml";
    let text = paths
        .data_file(REL)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| include_str!("../../../data/ui/hud.toml").to_string());
    match HudPlan::parse(&text) {
        Ok(p) if p.crosshair.enabled => hud.plan = Some(p),
        Ok(_) => log.line("original hud: crosshair pictures switched off in hud.toml"),
        Err(e) => log.line(format!(
            "original hud: {REL}: {e}; using the placeholder HUD"
        )),
    }
    if found.0 && hud.plan.is_some() {
        hud.library = Some(rc_menus::Library::new(&paths.game_data()));
    }
    match HudLayout::parse(&text) {
        Ok(l) if l.layout.enabled && !l.elements.is_empty() => {
            if found.0 {
                original.assets = Some(rc_menus::Assets::new(rc_menus::Library::new(
                    &paths.game_data(),
                )));
                original.working = true;
            }
            original.layout = Some(l);
        }
        Ok(_) => log.line("original hud: layout switched off in hud.toml; placeholder HUD"),
        Err(e) => log.line(format!(
            "original hud: {REL} layout: {e}; using the placeholder HUD"
        )),
    }
}

fn forget_drawn(mut original: ResMut<OriginalLayout>) {
    original.drawn_for = None;
    original.drawn.clear();
    original.messages.clear();
}

/// Keep the short messages for as long as they asked.
fn collect_messages(
    mut original: ResMut<OriginalLayout>,
    mut incoming: MessageReader<HudMessage>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    original.messages.retain_mut(|m| {
        m.1 -= dt;
        m.1 > 0.0
    });
    for m in incoming.read() {
        original.messages.push((m.text.clone(), m.seconds));
    }
    // Three lines fit.
    let extra = original.messages.len().saturating_sub(3);
    original.messages.drain(..extra);
}

/// What the HUD shows now. Bars are rounded to 1/200 so a recharging
/// shield doesn't redraw every frame.
fn values(
    vitals: &PlayerVitals,
    weapon: &WeaponReadout,
    squad: &SquadReadout,
    messages: &[(String, f32)],
) -> HudValues {
    let frac = |v: f32, max: f32| {
        if max > 0.0 {
            ((v / max).clamp(0.0, 1.0) * 200.0).round() / 200.0
        } else {
            0.0
        }
    };
    HudValues {
        shield: frac(vitals.shield, vitals.max_shield),
        health: frac(vitals.health, vitals.max_health),
        weapon: weapon.name.clone(),
        clip: weapon.clip,
        clip_size: weapon.clip_size,
        reserve: weapon.reserve,
        grenade: weapon.grenade.clone(),
        squad: squad
            .members
            .iter()
            .map(|m| SquadLine {
                number: m.number.clone(),
                name: m.name.clone(),
                health: frac(m.health, 1.0),
                down: m.status == SquadStatus::Incapacitated,
                order: m.order.clone(),
            })
            .collect(),
        messages: messages.iter().map(|m| m.0.clone()).collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_layout(
    mut commands: Commands,
    mut original: ResMut<OriginalLayout>,
    vitals: Res<PlayerVitals>,
    weapon: Res<WeaponReadout>,
    squad: Res<SquadReadout>,
    settings: Res<Settings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    roots: Query<Entity, With<LayoutRoot>>,
    mut image_assets: ResMut<Assets<Image>>,
    log: Res<GameLog>,
) {
    let original = &mut *original;
    if !original.working {
        for r in &roots {
            commands.entity(r).despawn();
        }
        return;
    }
    let (Some(layout), Some(assets)) = (&original.layout, original.assets.as_mut()) else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    if w < 1.0 || h < 1.0 {
        return;
    }
    let now = values(&vitals, &weapon, &squad, &original.messages);
    let max_aspect = settings.video.hud_max_aspect;
    let key = (now, [w as u32, h as u32], max_aspect.to_bits());
    if original.drawn_for.as_ref() == Some(&key) && !roots.is_empty() {
        return;
    }
    let first = original.drawn_for.is_none();
    let quads = rc_menus::hud::hud_quads(layout, &key.0, w, h, max_aspect, assets);
    original.drawn_for = Some(key);
    if !assets.problems.is_empty() {
        for p in assets.problems.drain(..) {
            log.line(format!("original hud: {p}; using the placeholder HUD"));
        }
        original.working = false;
        return;
    }
    if first {
        log.line(format!(
            "original hud: drawn at {w:.0}x{h:.0} with {} pictures",
            quads.len()
        ));
    }
    for q in &quads {
        let k = (
            q.texture.to_ascii_lowercase(),
            gpu::variant(q.render_style),
            q.color,
        );
        if original.images.contains_key(&k) {
            continue;
        }
        if let Some(rgba) = assets.texture(&q.texture) {
            let mut pixels = rgba.pixels.clone();
            gpu::prepare(&mut pixels, k.1, k.2);
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
            original.images.insert(k, image_assets.add(image));
        }
    }
    for r in &roots {
        commands.entity(r).despawn();
    }
    let images = &original.images;
    commands
        .spawn((
            LayoutRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            // Over the placeholder (1), under the menus (10).
            GlobalZIndex(2),
            DespawnOnExit(GameState::InGame),
        ))
        .with_children(|root| {
            for q in &quads {
                let Some(image) = images.get(&(
                    q.texture.to_ascii_lowercase(),
                    gpu::variant(q.render_style),
                    q.color,
                )) else {
                    continue;
                };
                // A mirrored part has its left edge after its right.
                let (x0, x1) = (q.src[0].min(q.src[2]), q.src[0].max(q.src[2]));
                root.spawn((
                    ImageNode {
                        image: image.clone(),
                        rect: Some(Rect::new(x0, q.src[1], x1, q.src[3])),
                        flip_x: q.src[0] > q.src[2],
                        // The tint is baked into the picture (gpu.rs).
                        color: Color::WHITE,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(q.dst[0] * 100.0),
                        top: Val::Percent(q.dst[1] * 100.0),
                        width: Val::Percent((q.dst[2] - q.dst[0]) * 100.0),
                        height: Val::Percent((q.dst[3] - q.dst[1]) * 100.0),
                        ..default()
                    },
                ));
            }
        });
    original.drawn = quads;
}

/// While the original's HUD is drawn, hide the placeholder's drawings.
fn hide_placeholder(
    original: Res<OriginalLayout>,
    mut parts: Query<&mut Visibility, With<PlaceholderPart>>,
) {
    let want = if original.working && !original.drawn.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut v in &mut parts {
        if *v != want {
            *v = want;
        }
    }
}

/// The original HUD's pictures on screen now (screen fractions), for the
/// script's colour check. Empty when the placeholder is shown.
pub fn layout_drawn(w: &World) -> Vec<Quad> {
    w.get_resource::<OriginalLayout>()
        .filter(|o| o.working)
        .map(|o| o.drawn.clone())
        .unwrap_or_default()
}

/// Once the HUD is up, add the picture beside the bars, in the same
/// centred point (their parent). It goes when the HUD goes.
fn spawn_picture(
    mut commands: Commands,
    hud: Res<OriginalHud>,
    crosses: Query<&ChildOf, With<CrossBar>>,
    pictures: Query<(), With<CrossPicture>>,
) {
    if hud.library.is_none() || !pictures.is_empty() {
        return;
    }
    let Some(centre) = crosses.iter().next().map(|c| c.parent()) else {
        return;
    };
    let picture = commands
        .spawn((
            CrossPicture,
            ImageNode::default(),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();
    commands.entity(centre).add_child(picture);
}

fn load_picture(
    library: &mut rc_menus::Library,
    path: &str,
    images: &mut Assets<Image>,
    log: &GameLog,
) -> Picture {
    match library.texture(path) {
        Ok(rgba) => {
            let size = Vec2::new(rgba.width as f32, rgba.height as f32);
            log.line(format!(
                "original hud: crosshair {path}: {}x{}",
                rgba.width, rgba.height
            ));
            let image = Image::new(
                Extent3d {
                    width: rgba.width as u32,
                    height: rgba.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba.pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            Some((images.add(image), size))
        }
        Err(e) => {
            log.line(format!(
                "original hud: crosshair {path}: {e}; keeping the placeholder crosshair"
            ));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_crosshair(
    mut hud: ResMut<OriginalHud>,
    weapon: Res<WeaponReadout>,
    theme: Res<Theme>,
    mut images: ResMut<Assets<Image>>,
    mut pictures: Query<(&mut ImageNode, &mut Node, &mut Visibility), With<CrossPicture>>,
    mut bars: Query<&mut Visibility, (With<CrossBar>, Without<CrossPicture>)>,
    log: Res<GameLog>,
) {
    let hud = &mut *hud;
    let Ok((mut image, mut node, mut shown)) = pictures.single_mut() else {
        return;
    };
    let (Some(plan), Some(library)) = (&hud.plan, hud.library.as_mut()) else {
        return;
    };
    let path = plan.reticle_for(&weapon.name).to_string();
    let key = path.to_ascii_lowercase();
    let picture = hud
        .pictures
        .entry(key)
        .or_insert_with(|| load_picture(library, &path, &mut images, &log))
        .clone();
    if hud.showing.as_deref() != Some(&path) {
        hud.showing = Some(path.clone());
        if picture.is_some() {
            log.line(format!(
                "original hud: crosshair for {:?} is {path}",
                weapon.name
            ));
        }
    }
    let use_picture = picture.is_some();
    for mut v in &mut bars {
        let want = if use_picture {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want {
            *v = want;
        }
    }
    let Some((handle, size)) = picture else {
        if *shown != Visibility::Hidden {
            *shown = Visibility::Hidden;
        }
        return;
    };
    *shown = Visibility::Inherited;
    if image.image != handle {
        image.image = handle;
    }
    // The original turns the crosshair red over an enemy.
    image.color = if weapon.on_target {
        theme.crosshair_on_target
    } else {
        Color::WHITE
    };
    // Sized by screen height, so it is the same size at any screen shape.
    let s = plan.crosshair.scale * 100.0 / BASE_LINES;
    let (w, h) = (Val::Vh(size.x * s), Val::Vh(size.y * s));
    if node.width != w || node.height != h {
        node.width = w;
        node.height = h;
        node.left = Val::Vh(-size.x * s / 2.0);
        node.top = Val::Vh(-size.y * s / 2.0);
    }
}

/// Whether the original's reticle picture is on screen now (for the
/// script's check).
pub fn reticle_showing(w: &mut World) -> bool {
    let mut q = w.query_filtered::<&Visibility, With<CrossPicture>>();
    q.iter(w).any(|v| *v != Visibility::Hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_hud_file_reads() {
        let p = HudPlan::parse(include_str!("../../../data/ui/hud.toml")).unwrap();
        assert!(p.crosshair.enabled);
        assert!(p.crosshair.scale > 0.0);
        assert_eq!(p.reticle_for("No Such Gun"), p.crosshair.default);
        assert_ne!(p.reticle_for("dc-15s pistol"), p.crosshair.default);
    }

    /// With a picture loaded, the bars hide and the picture shows, centred
    /// and sized by screen height; on target it turns red.
    #[test]
    fn picture_replaces_the_bars() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .insert_resource(GameLog::terminal())
            .insert_resource(Theme::default())
            .insert_resource(WeaponReadout {
                name: "DC-17m Blaster".into(),
                ..default()
            })
            .add_systems(Update, update_crosshair);
        let plan = HudPlan::parse(include_str!("../../../data/ui/hud.toml")).unwrap();
        let path = plan.crosshair.default.to_ascii_lowercase();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let mut hud = OriginalHud {
            plan: Some(plan),
            library: Some(rc_menus::Library::new(std::path::Path::new("no-game"))),
            ..default()
        };
        hud.pictures
            .insert(path, Some((handle.clone(), Vec2::new(48.0, 24.0))));
        app.insert_resource(hud);
        let bar = app
            .world_mut()
            .spawn((CrossBar(Vec2::X), Visibility::Inherited))
            .id();
        let pic = app
            .world_mut()
            .spawn((
                CrossPicture,
                ImageNode::default(),
                Node::default(),
                Visibility::Hidden,
            ))
            .id();
        app.update();
        let w = app.world();
        assert_eq!(w.get::<Visibility>(bar), Some(&Visibility::Hidden));
        assert_eq!(w.get::<Visibility>(pic), Some(&Visibility::Inherited));
        assert_eq!(w.get::<ImageNode>(pic).unwrap().image, handle);
        let n = w.get::<Node>(pic).unwrap();
        assert_eq!(n.width, Val::Vh(10.0)); // 48 of 480 lines
        assert_eq!(n.height, Val::Vh(5.0));
        assert_eq!(n.left, Val::Vh(-5.0));
        app.world_mut().resource_mut::<WeaponReadout>().on_target = true;
        app.update();
        let red = app.world().resource::<Theme>().crosshair_on_target;
        assert_eq!(app.world().get::<ImageNode>(pic).unwrap().color, red);
    }

    /// A picture that can't be read leaves the bars as they are.
    #[test]
    fn missing_picture_keeps_the_bars() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .insert_resource(GameLog::terminal())
            .insert_resource(Theme::default())
            .init_resource::<WeaponReadout>()
            .insert_resource(OriginalHud {
                plan: Some(HudPlan::parse(include_str!("../../../data/ui/hud.toml")).unwrap()),
                library: Some(rc_menus::Library::new(std::path::Path::new("no-game"))),
                ..default()
            })
            .add_systems(Update, update_crosshair);
        let bar = app
            .world_mut()
            .spawn((CrossBar(Vec2::X), Visibility::Inherited))
            .id();
        let pic = app
            .world_mut()
            .spawn((
                CrossPicture,
                ImageNode::default(),
                Node::default(),
                Visibility::Hidden,
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(bar),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().get::<Visibility>(pic),
            Some(&Visibility::Hidden)
        );
    }
}
