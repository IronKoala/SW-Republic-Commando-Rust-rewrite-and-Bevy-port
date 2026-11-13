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
//!
//! The advisor's words (`subtitles.rs`) and the current objective are
//! drawn as the original's helmet draws them (Engine.dll `UHelmet`
//! `DrawSubtitleText` and `DrawCurrentObjectiveText`): in one box near the
//! top, the words winning over everything else in it. Elements marked
//! `glass` are moved where the helmet's glass shows them, as the original
//! draws its HUD onto the glass (`rc_menus::helmet::GlassMap`).
#![allow(deprecated)]

use crate::helmet::{Helmet, HelmetReadout, HudPage};
use crate::hud::{CrossBar, PlaceholderPart};
use crate::menu::GamePathsFound;
use crate::settings::Settings;
use crate::subtitles::Spoken;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use rc_game_core::{
    pickups::PickedUp, GameLog, GamePaths, GameSet, GameState, HudMessage, PlayerVitals,
    SquadReadout, SquadStatus, WeaponReadout,
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
    /// The use panel round the crosshair (bacta dispensers).
    #[serde(default)]
    pub use_panel: Option<UsePanelPlan>,
    /// Read by `rc_menus::hud` (see `HudLayout`).
    #[serde(default)]
    layout: Option<toml::Value>,
    #[serde(default)]
    element: Option<toml::Value>,
    #[serde(default)]
    pickup_message: Option<toml::Value>,
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
    /// Its tint, red green blue opacity: normally, over a friend and over
    /// an enemy.
    #[serde(default = "plain_colour")]
    pub colour: [u8; 4],
    #[serde(default = "friend_colour")]
    pub friend_colour: [u8; 4],
    #[serde(default = "enemy_colour")]
    pub enemy_colour: [u8; 4],
}

fn plain_colour() -> [u8; 4] {
    [200, 240, 255, 110]
}

fn friend_colour() -> [u8; 4] {
    [130, 255, 130, 220]
}

fn enemy_colour() -> [u8; 4] {
    [255, 130, 130, 220]
}

/// `[use_panel]`: the original's use panel at the crosshair while the
/// player aims at something they use by holding Use (a bacta
/// dispenser): its icon, and round it a ring that lights up in twelve
/// steps. See `data/ui/hud.toml` and [`ring_pixels`].
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UsePanelPlan {
    /// The ring picture, and its size (1 = one picture pixel per line of
    /// a 480-line screen).
    pub ring: String,
    pub ring_scale: f32,
    /// The ring's colour (sRGB 0 to 255), how opaque its lit pieces are
    /// and how opaque the whole ring is drawn underneath (0 to 255).
    pub ring_colour: [u8; 3],
    pub ring_alpha: u8,
    pub ring_alpha_unlit: u8,
    /// The sheet of icons, its square size in pixels, how many squares
    /// across and down it counts, and which square (counted from the
    /// bottom right of that grid, as the game's `HUDIconX`/`Y`).
    pub icon_sheet: String,
    pub icon_cell: u32,
    pub icon_grid: [u32; 2],
    pub icon: [u32; 2],
    pub icon_scale: f32,
    /// The icon's colour at 0 (empty) and at 1 (full); in between it is
    /// mixed in a straight line. Its opacity (0 to 255).
    pub icon_colour_empty: [u8; 3],
    pub icon_colour_full: [u8; 3],
    pub icon_alpha: u8,
    /// The prompt above ("PRESS AND HOLD 'F' TO HEAL") breaks onto a
    /// second line after about this many letters.
    pub prompt_wrap: usize,
}

/// Break `text` into at most two lines of about `wrap` letters, at spaces.
pub fn wrap_prompt(text: &str, wrap: usize) -> Vec<String> {
    let mut first = String::new();
    let mut words = text.split_whitespace().peekable();
    while let Some(w) = words.peek() {
        let len = first.chars().count() + usize::from(!first.is_empty()) + w.chars().count();
        if !first.is_empty() && len > wrap {
            break;
        }
        if !first.is_empty() {
            first.push(' ');
        }
        first.push_str(w);
        words.next();
    }
    let rest: Vec<&str> = words.collect();
    let mut out = vec![first];
    if !rest.is_empty() {
        out.push(rest.join(" "));
    }
    out
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
    /// What was just picked up and its seconds left (one at a time: a new
    /// one replaces the last, as in the original).
    pickup: Option<(String, f32)>,
    /// The objective shown and for how long (it fades in when it changes).
    objective: (String, f32),
    /// The use prompt drawn last frame and for how long it has been up
    /// (it fades in again whenever it comes back or its words change).
    prompt: Option<(Vec<String>, f32)>,
    /// The helmet glass's map for the screen shape it was made for.
    glass: Option<(u32, rc_menus::helmet::GlassMap)>,
    /// Where the compass strip has slid to (it follows the view's turn).
    compass: Option<f32>,
    /// The shields last frame (the original helmet's `LastShield`) and
    /// the seconds of hit flash left (`ShieldFlashTime`).
    last_shield: Option<f32>,
    shield_flash: f32,
    /// The start-up sequence's clock (a new level or a loaded game
    /// starts it again).
    clock: StartClock,
    /// The tactical visor's arcs and ring.
    tactical: crate::helmet_start::Tactical,
}

/// The helmet's start-up clock (`InitTime`), the level's time last
/// frame, and the shields as drawn last frame, unrounded, for their
/// sounds.
#[derive(Default)]
struct StartClock {
    init_time: f32,
    level_time: Option<f32>,
    last_charge_shield: Option<f32>,
    /// The start-up lines shown so far this level (for the script's check).
    lines: Vec<String>,
}

/// The system that works out and draws the HUD each frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DrawLayout;

/// The visor frame's alpha at low health (Engine.dll `UHelmet::DrawHUD`
/// 0x10431b9a): at health level 1 or 0 it is `150 + 105 * sin(t)^2` with
/// `t` the engine's time times 2.5, cut to a whole number; above that the
/// frame doesn't pulse.
pub fn frame_pulse(health: f32, time: f32) -> Option<u8> {
    if rc_menus::hud::health_level(health) > 1 {
        return None;
    }
    let s = (time * 2.5).sin();
    Some((150.0 + 105.0 * s * s) as u8)
}

/// The original helmet's `ShieldFlashDuration` (engine.u Helmet default).
const SHIELD_FLASH_DURATION: f32 = 0.5;

/// The shield hit flash, as Engine.dll `UHelmet::DrawHUD` keeps it: when
/// the shields drop the flash starts again at `ShieldFlashDuration` and runs
/// down with time. While it runs the bars swell by `int(left * 6)` and,
/// unless the shields are charging, lose `int(left / duration * 155)` of
/// their green and blue. Returns (swell, red) and keeps `last`/`left`.
pub fn shield_flash(last: &mut Option<f32>, left: &mut f32, shield: f32, dt: f32) -> (u32, u8) {
    let before = last.unwrap_or(shield);
    if before > shield {
        *left = SHIELD_FLASH_DURATION;
    }
    *left = (*left - dt).max(0.0);
    let swell = (*left * 6.0) as u32;
    let red = if *left > 0.0 && before >= shield {
        (*left / SHIELD_FLASH_DURATION * 155.0) as u8
    } else {
        0
    };
    *last = Some(shield);
    (swell, red)
}

#[derive(Component)]
struct LayoutRoot;

#[derive(Component)]
struct CrossPicture;

/// One picture of the use panel.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum UsePart {
    Icon,
    Ring,
}

/// The use panel's pictures: the ring's own pixels (to redraw it filled
/// to a new amount) and the image it is drawn into, and the icon.
#[derive(Resource, Default)]
struct UsePanelPictures {
    tried: bool,
    ring: Option<(rc_image::Rgba, Handle<Image>)>,
    /// How many of its twelve steps the ring image shows lit now.
    ring_fill: Option<u32>,
    icon: Option<(Handle<Image>, Vec2)>,
}

pub struct OriginalHudPlugin;

impl Plugin for OriginalHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OriginalHud>()
            .init_resource::<OriginalLayout>()
            .init_resource::<UsePanelPictures>()
            .add_systems(Startup, read_plan)
            .add_systems(OnExit(GameState::InGame), forget_drawn)
            .add_systems(
                Update,
                (
                    spawn_picture,
                    update_crosshair,
                    update_use_panel,
                    collect_messages,
                    draw_layout.in_set(DrawLayout),
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
    original.pickup = None;
    original.objective = Default::default();
}

/// Keep the short messages for as long as they asked, and the pickup
/// message for as long as `[pickup_message]` says.
fn collect_messages(
    mut original: ResMut<OriginalLayout>,
    mut incoming: MessageReader<HudMessage>,
    mut picked: MessageReader<PickedUp>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    let seconds = original
        .layout
        .as_ref()
        .map_or(3.0, |l| l.pickup_message.seconds);
    if let Some(p) = &mut original.pickup {
        p.1 -= dt;
        if p.1 <= 0.0 {
            original.pickup = None;
        }
    }
    for p in picked.read() {
        if !p.text.is_empty() {
            original.pickup = Some((p.text.clone(), seconds));
        }
    }
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

/// Where the compass strip should slide to, in picture widths, for a view
/// facing `yaw` (radians, Unreal's): Engine.dll `UHelmet::DrawHUD` takes
/// the yaw in Unreal's units (65536 a turn), modulo 65535, times 0.0001.
fn compass_target(yaw: f32) -> f32 {
    let units = (yaw / std::f32::consts::TAU * 65536.0) as i32;
    (units % 65535) as f32 * 0.0001
}

/// The strip follows its target at 3 picture widths a second (so a turn
/// past due north slides it all the way back, as in the original).
fn compass_follow(now: f32, target: f32, dt: f32) -> f32 {
    let step = 3.0 * dt;
    if target > now {
        (now + step).min(target)
    } else {
        (now - step).max(target)
    }
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
            .filter(|m| m.in_squad)
            .map(|m| SquadLine {
                number: m.number.clone(),
                name: m.name.clone(),
                health: frac(m.health, 1.0),
                down: m.status == SquadStatus::Incapacitated,
                order: m.order.clone(),
                ..Default::default()
            })
            .collect(),
        messages: messages.iter().map(|m| m.0.clone()).collect(),
        ..Default::default()
    }
}

/// The helmet's start-up sequence (Engine.dll `UHelmet::DrawHUD`): the
/// "SCANNING HEALTH" / "CHARGING SHIELDS" / "INIT SQUAD LINK" lines, the
/// health meter and shield bars filling from empty in their phases, the
/// squad panels coming in one by one, and the shield charging sounds'
/// steps (worked out from the bars as drawn). True when the level's
/// clock started again.
fn start_up(
    clock: &mut StartClock,
    now: &mut HudValues,
    vitals: &PlayerVitals,
    start: &crate::helmet_start::HelmetStart,
    charge: &mut crate::helmet_start::ChargeSounds,
    script: Option<&rc_scripts_plugin::LevelScript>,
    time: &Time,
    hud_hidden: bool,
) -> bool {
    use crate::helmet_start::{charge_step, fill, squad_slot, text_alpha, Slot};
    // The level's own clock: it starts again with a new level or a
    // loaded game, and so does the sequence.
    let level = script.map(|s| s.director.time);
    let mut restarted = false;
    let dt = match (level, clock.level_time) {
        (Some(t), Some(before)) if t >= before => t - before,
        (Some(_), _) => {
            *clock = StartClock::default();
            restarted = true;
            0.0
        }
        (None, _) => time.delta_secs(),
    };
    clock.level_time = level;
    // All of this is in the original's `UHelmet::DrawHUD`, which isn't
    // run while the HUD is hidden (a scene with `bDrawHUD` off): the
    // clock and the sounds wait for the HUD to come back.
    if hud_hidden {
        return restarted;
    }
    let t = level.unwrap_or(time.elapsed_secs());
    let raw = if vitals.max_shield > 0.0 {
        (vitals.shield / vitals.max_shield).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut drawn_shield = raw;
    if let Some(plan) = &start.plan {
        clock.init_time = plan.advance(clock.init_time, dt);
        let init = clock.init_time;
        if let Some(f) = fill(&plan.shields, init) {
            now.shield = ((now.shield * f) * 200.0).round() / 200.0;
            drawn_shield = raw * f;
        }
        now.health_meter =
            fill(&plan.health, init).map(|f| ((now.health * f) * 200.0).round() / 200.0);
        if let Some(text) = plan.text(init, !now.squad.is_empty()) {
            now.start_up = text.to_string();
            now.start_up_alpha = text_alpha(t);
            if !clock.lines.iter().any(|l| l == text) {
                clock.lines.push(text.to_string());
            }
        }
        for (i, m) in now.squad.iter_mut().enumerate() {
            match squad_slot(&plan.squad, init, i + 1, t) {
                Slot::Hidden => m.hidden = true,
                Slot::Blinking(a) => m.start_up_alpha = Some(a),
                Slot::Normal => {}
            }
        }
    }
    let last = clock.last_charge_shield.unwrap_or(drawn_shield);
    if let Some(step) = charge_step(last, drawn_shield, charge.playing) {
        charge.steps.push(step);
        // As the helmet's `PlayingShieldChargeSound`.
        charge.playing = step == crate::helmet_start::ChargeStep::Start;
    }
    clock.last_charge_shield = Some(drawn_shield);
    restarted
}

/// What the helmet's text box shows (Engine.dll `UHelmet::DrawTextInfo`):
/// the advisor's words win over everything else in the box; the short
/// messages and the use prompt over the objective.
fn text_box(now: &mut HudValues, layout: &HudLayout, spoken: &Spoken, objective: &(String, f32)) {
    let l = &layout.layout;
    if !spoken.text.is_empty() {
        now.subtitle = spoken.text.clone();
        now.subtitle_fade = l.faded_in(spoken.age);
        now.messages.clear();
        now.prompt.clear();
    } else if now.messages.is_empty() && now.prompt.is_empty() && !objective.0.is_empty() {
        now.objective = objective.0.clone();
        now.objective_fade = l.faded_in(objective.1);
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
    use_item: Option<Res<rc_game_core::UseItem>>,
    hud: Res<OriginalHud>,
    (spoken, objectives, time, script): (
        Option<Res<Spoken>>,
        Option<Res<rc_scripts_plugin::Objectives>>,
        Res<Time>,
        Option<Res<rc_scripts_plugin::LevelScript>>,
    ),
    (helmet, helmet_readout, bodies, page): (
        Res<Helmet>,
        Res<HelmetReadout>,
        Query<&crate::player::Body, With<rc_game_core::Player>>,
        Option<Res<HudPage>>,
    ),
    (start, mut charge, visor, cutscene): (
        Res<crate::helmet_start::HelmetStart>,
        ResMut<crate::helmet_start::ChargeSounds>,
        Option<Res<rc_visor::VisorState>>,
        Option<Res<rc_game_core::Cutscene>>,
    ),
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
    // The objective fades in again whenever it changes.
    let objective = objectives.and_then(|o| o.shown()).unwrap_or_default();
    if objective == original.objective.0 {
        original.objective.1 += time.delta_secs();
    } else {
        original.objective = (objective, 0.0);
    }
    let mut now = values(&vitals, &weapon, &squad, &original.messages);
    if start_up(
        &mut original.clock,
        &mut now,
        &vitals,
        &start,
        &mut charge,
        script.as_deref(),
        &time,
        cutscene.is_some_and(|c| c.hide_hud),
    ) {
        original.last_shield = None;
        original.shield_flash = 0.0;
    }
    if let Some(on) = visor.map(|v| v.mode == rc_visor::VisorMode::Tactical) {
        let dt = time.delta_secs();
        if let Some((shown, turn)) = original.tactical.step(on, dt) {
            // Rounded so a grown-in arc doesn't redraw; the ring turns
            // every frame anyway.
            now.tactical = Some((shown * 200.0).round() / 200.0);
            now.tactical_spin = turn;
        }
    }
    let (swell, red) = shield_flash(
        &mut original.last_shield,
        &mut original.shield_flash,
        now.shield,
        time.delta_secs(),
    );
    if swell > 0
        && now.shield_hit_steps == 0
        && original
            .drawn_for
            .as_ref()
            .is_some_and(|d| d.0.shield_hit_steps == 0)
    {
        log.line(format!(
            "original hud: shields hit, now {:.3}: bars flash",
            now.shield
        ));
    }
    now.shield_hit_steps = swell;
    now.shield_hit_red = red;
    now.frame_pulse = frame_pulse(now.health, time.elapsed_secs());
    // The parts the level's scripts blink, all with the opacity the
    // original works out from the level's time.
    if let Some(s) = &script {
        let a = rc_menus::hud::blink_alpha(s.director.time);
        now.blinking = s
            .hud_blink
            .now()
            .into_iter()
            .map(|k| (k.to_string(), a))
            .collect();
    }
    if let Ok(b) = bodies.single() {
        let c = original.compass.get_or_insert(compass_target(b.yaw));
        *c = compass_follow(*c, compass_target(b.yaw), time.delta_secs());
        // Rounded so standing still doesn't redraw.
        now.compass = (*c * 2000.0).round() / 2000.0;
    }
    if let Some((text, left)) = &original.pickup {
        now.pickup = text.clone();
        now.pickup_fade = layout.pickup_message.strength(*left);
    }
    // The use prompt ("PRESS AND HOLD 'F' TO HEAL"), in its own place
    // near the top (the `prompt` elements).
    if let Some(p) = use_item.as_ref().and_then(|u| u.prompt.as_ref()) {
        let wrap = hud
            .plan
            .as_ref()
            .and_then(|p| p.use_panel.as_ref())
            .map_or(usize::MAX, |u| u.prompt_wrap);
        now.prompt = wrap_prompt(p, wrap);
    }
    text_box(
        &mut now,
        layout,
        spoken.as_deref().unwrap_or(&Spoken::default()),
        &original.objective,
    );
    // The prompt fades in as the original's does (Engine.dll
    // `UHelmet::DrawActivateText`): its clock starts again when it wasn't
    // drawn the frame before (bActivateTextLastFrame) or its words differ
    // from LastActivateText; the first frame is at opacity 0.
    original.prompt = if now.prompt.is_empty() {
        None
    } else {
        match original.prompt.take() {
            Some((lines, age)) if lines == now.prompt => {
                let older = age + time.delta_secs();
                if layout.layout.prompt_faded_in(age) < 1.0
                    && layout.layout.prompt_faded_in(older) >= 1.0
                {
                    log.line(format!(
                        "original hud: prompt fully faded in after {older:.2} s"
                    ));
                }
                Some((lines, older))
            }
            _ => {
                log.line(format!(
                    "original hud: prompt {:?} fading in from opacity 0",
                    now.prompt.join(" ")
                ));
                Some((now.prompt.clone(), 0.0))
            }
        }
    };
    if let Some((_, age)) = &original.prompt {
        now.prompt_fade = layout.layout.prompt_faded_in(*age);
    }
    // Through the helmet's glass while it is drawn.
    let glass_model = match (&helmet.model, helmet_readout.shown) {
        (Some(Ok(m)), true) if !m.glass.is_empty() => Some(m.clone()),
        _ => None,
    };
    let aspect_key = (w / h).to_bits();
    match &glass_model {
        Some(m) if original.glass.as_ref().is_none_or(|g| g.0 != aspect_key) => {
            original.glass = Some((
                aspect_key,
                rc_menus::helmet::GlassMap::new(m, &helmet.plan, w / h),
            ));
        }
        None => original.glass = None,
        _ => {}
    }
    let max_aspect = settings.video.hud_max_aspect;
    let glass_on = original.glass.is_some() as u32;
    let key = (
        now,
        [w as u32, h as u32],
        max_aspect.to_bits() ^ (glass_on << 31),
    );
    if original.drawn_for.as_ref() == Some(&key) && !roots.is_empty() {
        return;
    }
    let first = original.drawn_for.is_none();
    let glass = original.glass.as_ref().map(|g| &g.1);
    // With the helmet on, the glass parts go on the page laid on its
    // glass (`helmet::HudPage`); the rest, or everything without it, on
    // the screen.
    let on_page = glass.is_some() && page.is_some();
    let quads = rc_menus::hud::hud_quads_of(layout, &key.0, w, h, max_aspect, assets, None, &|e| {
        !(on_page && e.glass)
    });
    let page_quads = match (&page, on_page) {
        (Some(p), true) => rc_menus::hud::hud_quads_of(
            layout,
            &key.0,
            p.size.x as f32,
            p.size.y as f32,
            4.0 / 3.0,
            assets,
            None,
            &|e| e.glass,
        ),
        _ => Vec::new(),
    };
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
    for q in quads.iter().chain(&page_quads) {
        let k = picture_key(q);
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
    spawn_quads(&mut commands, images, &quads, None);
    if let (Some(p), false) = (&page, page_quads.is_empty()) {
        spawn_quads(&mut commands, images, &page_quads, Some(p.camera));
    }
    // For checks: where the page's parts show on the screen.
    let mut drawn = quads;
    if let Some(g) = glass {
        drawn.extend(page_quads.into_iter().map(|mut q| {
            let a = g.place([q.dst[0], q.dst[1]]);
            let b = g.place([q.dst[2], q.dst[3]]);
            q.dst = [a[0], a[1], b[0], b[1]];
            q
        }));
    }
    original.drawn = drawn;
}

/// Which copy of a picture a quad needs: by texture, blend and tint. A
/// see-through picture's copy is made at full alpha and its alpha given
/// when drawn, so a pulsing alpha doesn't make a copy per step.
fn picture_key(q: &Quad) -> (String, gpu::Variant, [u8; 4]) {
    let v = gpu::variant_over_scene(q.render_style);
    let mut c = q.color;
    if v == gpu::Variant::OverScene {
        c[3] = 255;
    }
    (q.texture.to_ascii_lowercase(), v, c)
}

/// One HUD root holding `quads` (screen fractions of its target): on the
/// screen, or on the helmet's page when `page` names its camera.
fn spawn_quads(
    commands: &mut Commands,
    images: &HashMap<(String, gpu::Variant, [u8; 4]), Handle<Image>>,
    quads: &[Quad],
    page: Option<Entity>,
) {
    let mut root = commands.spawn((
        LayoutRoot,
        crate::cutscene::HudRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        // Over the placeholder (1), under the menus (10).
        GlobalZIndex(2),
        DespawnOnExit(GameState::InGame),
    ));
    if let Some(cam) = page {
        root.insert(UiTargetCamera(cam));
    }
    root.with_children(|root| {
        for q in quads {
            let Some(image) = images.get(&picture_key(q)) else {
                continue;
            };
            // See-through pictures keep their alpha here, not baked in.
            let alpha = match gpu::variant_over_scene(q.render_style) {
                gpu::Variant::OverScene => q.color[3] as f32 / 255.0,
                _ => 1.0,
            };
            // A mirrored part has its left edge after its right.
            let (x0, x1) = (q.src[0].min(q.src[2]), q.src[0].max(q.src[2]));
            // A flipped part has its top edge below its bottom.
            let (y0, y1) = (q.src[1].min(q.src[3]), q.src[1].max(q.src[3]));
            root.spawn((
                ImageNode {
                    image: image.clone(),
                    rect: Some(Rect::new(x0, y0, x1, y1)),
                    flip_x: q.src[0] > q.src[2],
                    flip_y: q.src[1] > q.src[3],
                    // The tint is baked into the picture (gpu.rs),
                    // all but a see-through picture's alpha.
                    color: Color::srgba(1.0, 1.0, 1.0, alpha),
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
                // Turned round its middle, clockwise as the engine's
                // `DrawRotatedIcon`.
                UiTransform::from_rotation(Rot2::radians(q.turn * std::f32::consts::TAU)),
            ));
        }
    });
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

/// The helmet's start-up clock (`InitTime`) and the lines it has shown
/// this level, for the script's check.
pub fn start_up_seen(w: &World) -> (f32, Vec<String>) {
    w.get_resource::<OriginalLayout>()
        .map(|o| (o.clock.init_time, o.clock.lines.clone()))
        .unwrap_or_default()
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
    for part in [UsePart::Ring, UsePart::Icon] {
        let p = commands
            .spawn((
                part,
                ImageNode::default(),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                UiTransform::IDENTITY,
                Visibility::Hidden,
            ))
            .id();
        commands.entity(centre).add_child(p);
    }
}

/// The ring picture cut into a 4 by 4 grid of pieces: which pieces light
/// up at each of the twelve steps, as (column, row) from the top left.
/// The ring lights from just left of the top, down the left side, along
/// the bottom and up the right side; the last piece (just right of the
/// top) only shows when the ring is full, when the whole picture lights.
const RING_STEPS: [&[(usize, usize)]; 11] = [
    &[(1, 0)],
    &[(0, 0), (1, 1)],
    &[(0, 1)],
    &[(0, 2)],
    &[(0, 3), (1, 2)],
    &[(1, 3)],
    &[(2, 3)],
    &[(3, 3), (2, 2)],
    &[(3, 2)],
    &[(3, 1)],
    &[(3, 0), (2, 1)],
];

/// How many of the ring's twelve steps are lit at `fill` (0 to 1): one
/// more for each full twelfth.
pub fn ring_steps(fill: f32) -> usize {
    (1..=12).take_while(|k| fill >= *k as f32 / 12.0).count()
}

/// The ring drawn at `fill` (0 to 1), as the original draws it: the whole
/// ring faint (`unlit` opacity) and on top of it the pieces lit so far
/// (`lit` opacity), going up in twelfths. See [`RING_STEPS`].
pub fn ring_pixels(src: &rc_image::Rgba, fill: f32, lit: u8, unlit: u8) -> Vec<u8> {
    let (w, h) = (src.width, src.height);
    let steps = ring_steps(fill);
    let mut on = [[steps >= 12; 4]; 4];
    for pieces in RING_STEPS.iter().take(steps.min(11)) {
        for &(c, r) in *pieces {
            on[r][c] = true;
        }
    }
    let (lit, unlit) = (lit as f32 / 255.0, unlit as f32 / 255.0);
    let mut out = src.pixels.clone();
    for y in 0..h {
        for x in 0..w {
            let (c, r) = ((x * 4 / w.max(1)).min(3), (y * 4 / h.max(1)).min(3));
            let a = &mut out[(y * w + x) * 4 + 3];
            let base = *a as f32 / 255.0;
            let under = base * unlit;
            let alpha = if on[r][c] {
                // The lit piece drawn over the faint whole ring.
                let over = base * lit;
                over + under * (1.0 - over)
            } else {
                under
            };
            *a = (alpha * 255.0).round() as u8;
        }
    }
    out
}

/// The icon's colour at `fill` (0 to 1): from `empty` to `full` in a
/// straight line, each part cut to a whole number.
pub fn icon_colour(empty: [u8; 3], full: [u8; 3], fill: f32) -> [u8; 3] {
    let f = fill.clamp(0.0, 1.0);
    let mut out = [0; 3];
    for i in 0..3 {
        let v = empty[i] as f32 + (full[i] as f32 - empty[i] as f32) * f;
        out[i] = v.clamp(0.0, 255.0) as u8;
    }
    out
}

/// The use panel at the crosshair: the icon, and round it the ring filled
/// to the use's progress. See [`UsePanelPlan`].
#[allow(clippy::type_complexity)]
fn update_use_panel(
    mut hud: ResMut<OriginalHud>,
    mut pics: ResMut<UsePanelPictures>,
    use_item: Res<rc_game_core::UseItem>,
    mut images: ResMut<Assets<Image>>,
    mut parts: Query<(&UsePart, &mut ImageNode, &mut Node, &mut Visibility)>,
    log: Res<GameLog>,
    mut last: Local<Option<rc_game_core::UsePanel>>,
) {
    use rc_game_core::UsePanel;
    let hud = &mut *hud;
    let (Some(plan), Some(library)) = (&hud.plan, hud.library.as_mut()) else {
        return;
    };
    let Some(panel) = plan.use_panel.clone() else {
        return;
    };
    let state = use_item.panel;
    let changed =
        |a: &UsePanel, b: &UsePanel| std::mem::discriminant(a) != std::mem::discriminant(b);
    if last.as_ref().is_none_or(|l| changed(l, &state)) {
        if last.is_some() || state != UsePanel::Hidden {
            log.line(format!("original hud: use panel {state:?}"));
        }
        *last = Some(state);
    }
    if state != UsePanel::Hidden && !pics.tried {
        pics.tried = true;
        match library.texture(&panel.ring) {
            Ok(src) => {
                let image = Image::new(
                    Extent3d {
                        width: src.width as u32,
                        height: src.height as u32,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    src.pixels.clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::all(),
                );
                log.line(format!(
                    "original hud: use ring {}: {}x{}",
                    panel.ring, src.width, src.height
                ));
                pics.ring = Some((src, images.add(image)));
            }
            Err(e) => log.line(format!(
                "original hud: use ring {}: {e}; not drawn",
                panel.ring
            )),
        }
        match library.texture(&panel.icon_sheet) {
            Ok(sheet) => {
                let c = panel.icon_cell as usize;
                let (ix, iy) = (panel.icon[0] as usize, panel.icon[1] as usize);
                let (cols, rows) = (panel.icon_grid[0] as usize, panel.icon_grid[1] as usize);
                if ix < cols
                    && iy < rows
                    && cols * c <= sheet.width
                    && rows * c <= sheet.height
                    && c > 0
                {
                    let x0 = (cols - 1 - ix) * c;
                    let y0 = (rows - 1 - iy) * c;
                    let mut px = Vec::with_capacity(c * c * 4);
                    for y in y0..y0 + c {
                        let i = (y * sheet.width + x0) * 4;
                        px.extend_from_slice(&sheet.pixels[i..i + c * 4]);
                    }
                    let image = Image::new(
                        Extent3d {
                            width: c as u32,
                            height: c as u32,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        px,
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    log.line(format!(
                        "original hud: use icon {} square {:?} ({c} px, from the bottom right of {cols}x{rows})",
                        panel.icon_sheet, panel.icon
                    ));
                    pics.icon = Some((images.add(image), Vec2::splat(c as f32)));
                } else {
                    log.line(format!(
                        "original hud: use icon square {:?} is outside {}",
                        panel.icon, panel.icon_sheet
                    ));
                }
            }
            Err(e) => log.line(format!(
                "original hud: use icon {}: {e}; not drawn",
                panel.icon_sheet
            )),
        }
    }
    let fill = match state {
        UsePanel::Ready(f) | UsePanel::Working(f) => Some(f),
        _ => None,
    };
    let icon_fill = match state {
        UsePanel::Ready(f) | UsePanel::Working(f) | UsePanel::Icon(f) => f,
        UsePanel::Hidden => 0.0,
    };
    // Redraw the ring when its fill changes.
    if let (Some(f), Some((src, handle))) = (fill, pics.ring.as_ref()) {
        let step = ring_steps(f) as u32;
        if pics.ring_fill != Some(step) {
            let px = ring_pixels(src, f, panel.ring_alpha, panel.ring_alpha_unlit);
            if let Some(mut img) = images.get_mut(handle) {
                img.data = Some(px);
            }
            pics.ring_fill = Some(step);
        }
    }
    let size_of = |picture_px: Vec2, scale: f32| picture_px * scale * 100.0 / BASE_LINES;
    let rgba = |c: [u8; 3], a: u8| Color::srgba_u8(c[0], c[1], c[2], a);
    for (part, mut image, mut node, mut shown) in &mut parts {
        let drawn = match part {
            UsePart::Ring => fill.and_then(|_| {
                pics.ring.as_ref().map(|(src, h)| {
                    (
                        h.clone(),
                        size_of(
                            Vec2::new(src.width as f32, src.height as f32),
                            panel.ring_scale,
                        ),
                        // Opacity is in the picture (see ring_pixels).
                        rgba(panel.ring_colour, 255),
                    )
                })
            }),
            UsePart::Icon => (state != UsePanel::Hidden)
                .then(|| pics.icon.clone())
                .flatten()
                .map(|(h, px)| {
                    let colour =
                        icon_colour(panel.icon_colour_empty, panel.icon_colour_full, icon_fill);
                    (
                        h,
                        size_of(px, panel.icon_scale),
                        rgba(colour, panel.icon_alpha),
                    )
                }),
        };
        let Some((handle, size, colour)) = drawn else {
            if *shown != Visibility::Hidden {
                *shown = Visibility::Hidden;
            }
            continue;
        };
        if *shown != Visibility::Inherited {
            *shown = Visibility::Inherited;
        }
        if image.image != handle {
            image.image = handle;
        }
        if image.color != colour {
            image.color = colour;
        }
        let (w, h) = (Val::Vh(size.x), Val::Vh(size.y));
        if node.width != w || node.height != h {
            node.width = w;
            node.height = h;
            node.left = Val::Vh(-size.x / 2.0);
            node.top = Val::Vh(-size.y / 2.0);
        }
    }
}

/// Which use panel pictures are on screen now (for the script's check).
pub fn use_panel_showing(w: &mut World) -> Vec<String> {
    let mut q = w.query::<(&UsePart, &Visibility)>();
    q.iter(w)
        .filter(|(_, v)| **v != Visibility::Hidden)
        .map(|(p, _)| format!("{p:?}"))
        .collect()
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
                "original hud: picture {path}: {}x{}",
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
                "original hud: picture {path}: {e}; not drawn (the crosshair keeps its placeholder)"
            ));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_crosshair(
    mut hud: ResMut<OriginalHud>,
    weapon: Res<WeaponReadout>,
    mut images: ResMut<Assets<Image>>,
    mut pictures: Query<(&mut ImageNode, &mut Node, &mut Visibility), With<CrossPicture>>,
    mut bars: Query<&mut Visibility, (With<CrossBar>, Without<CrossPicture>)>,
    log: Res<GameLog>,
    use_item: Option<Res<rc_game_core::UseItem>>,
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
    // The use panel takes the crosshair's place, as in the original.
    let want = if use_item.is_some_and(|u| u.panel != rc_game_core::UsePanel::Hidden) {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if *shown != want {
        *shown = want;
    }
    if image.image != handle {
        image.image = handle;
    }
    // The original tints the crosshair by what it is over.
    let c = &plan.crosshair;
    let [r, g, b, a] = if weapon.on_target {
        c.enemy_colour
    } else if weapon.on_friend {
        c.friend_colour
    } else {
        c.colour
    };
    let tint = Color::srgba_u8(r, g, b, a);
    if image.color != tint {
        image.color = tint;
    }
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
    use crate::theme::Theme;

    #[test]
    fn frame_pulses_only_below_half_health() {
        assert_eq!(frame_pulse(0.6, 1.0), None);
        assert_eq!(frame_pulse(0.4, 0.0), Some(150));
        // sin(2.5 * 0.6283) = sin(pi / 2) = 1: the brightest.
        assert_eq!(frame_pulse(0.4, std::f32::consts::PI / 5.0), Some(255));
    }

    #[test]
    fn shield_flash_runs_as_the_original() {
        let (mut last, mut left) = (None, 0.0);
        // Steady: nothing.
        assert_eq!(shield_flash(&mut last, &mut left, 1.0, 0.016), (0, 0));
        // Hit: starts at 0.5 less this frame, swells 2 (0.484 * 6), red 150.
        assert_eq!(shield_flash(&mut last, &mut left, 0.8, 0.016), (2, 150));
        // A quarter second on: 0.234 left, swell 1, red 72.
        assert_eq!(shield_flash(&mut last, &mut left, 0.8, 0.25), (1, 72));
        // Charging while it runs: swells but no red.
        assert_eq!(shield_flash(&mut last, &mut left, 0.81, 0.016), (1, 0));
        // Ended.
        assert_eq!(shield_flash(&mut last, &mut left, 0.82, 1.0), (0, 0));
    }

    #[test]
    fn compass_slides_with_the_turn_as_the_original() {
        // A quarter turn right is 16384 units: 1.6384 picture widths.
        assert!((compass_target(std::f32::consts::FRAC_PI_2) - 1.6384).abs() < 1e-4);
        // Turning left gives a negative slide (C's remainder keeps the sign).
        assert!(compass_target(-std::f32::consts::FRAC_PI_2) < 0.0);
        // It follows at 3 a second, and stops on its target.
        assert!((compass_follow(0.0, 1.0, 0.1) - 0.3).abs() < 1e-6);
        assert_eq!(compass_follow(0.9, 1.0, 0.1), 1.0);
        assert!((compass_follow(1.0, 0.0, 0.1) - 0.7).abs() < 1e-6);
    }

    /// The ring lights in twelfths, piece by piece of a 4 by 4 grid, from
    /// just left of the top round the left side; the rest stays faint.
    #[test]
    fn ring_lights_in_twelve_steps() {
        assert_eq!(ring_steps(0.0), 0);
        assert_eq!(ring_steps(0.08), 0);
        assert_eq!(ring_steps(1.0 / 12.0), 1);
        assert_eq!(ring_steps(0.5), 6);
        assert_eq!(ring_steps(0.99), 11);
        assert_eq!(ring_steps(1.0), 12);
        let src = rc_image::Rgba {
            width: 4,
            height: 4,
            pixels: vec![255; 64],
        };
        let alpha = |px: &[u8], x: usize, y: usize| px[(y * 4 + x) * 4 + 3];
        // One step: only the piece just left of the top is lit.
        let one = ring_pixels(&src, 0.1, 255, 92);
        assert_eq!(alpha(&one, 1, 0), 255);
        assert_eq!(alpha(&one, 0, 0), 92);
        assert_eq!(alpha(&one, 2, 0), 92);
        // Half: the left side and the bottom left are lit.
        let half = ring_pixels(&src, 0.5, 255, 92);
        for (x, y) in [
            (0, 0),
            (1, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 3),
            (1, 1),
            (1, 2),
        ] {
            assert_eq!(alpha(&half, x, y), 255, "{x},{y}");
        }
        for (x, y) in [(2, 0), (3, 0), (3, 3), (2, 3), (2, 2)] {
            assert_eq!(alpha(&half, x, y), 92, "{x},{y}");
        }
        // Eleven steps: all but the piece just right of the top.
        let most = ring_pixels(&src, 0.95, 255, 92);
        assert_eq!(alpha(&most, 2, 0), 92);
        assert_eq!(alpha(&most, 3, 0), 255);
        let full = ring_pixels(&src, 1.0, 255, 92);
        assert!((0..16).all(|i| alpha(&full, i % 4, i / 4) == 255));
    }

    /// The icon goes from red-pink when empty to green when full.
    #[test]
    fn icon_colour_follows_the_fill() {
        let (e, f) = ([255, 140, 180], [140, 255, 180]);
        assert_eq!(icon_colour(e, f, 0.0), [255, 140, 180]);
        assert_eq!(icon_colour(e, f, 1.0), [140, 255, 180]);
        assert_eq!(icon_colour(e, f, 0.5), [197, 197, 180]);
    }

    /// The prompt breaks where the original's does.
    #[test]
    fn prompt_breaks_like_the_original() {
        assert_eq!(
            wrap_prompt("PRESS AND HOLD 'F' TO HEAL", 21),
            vec!["PRESS AND HOLD 'F' TO".to_string(), "HEAL".to_string()]
        );
        assert_eq!(wrap_prompt("SHORT", 21), vec!["SHORT".to_string()]);
    }

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
        // 48 pixels of the original's 600 lines: 8% of the screen height.
        let vh = |v: Val| match v {
            Val::Vh(x) => x,
            _ => f32::NAN,
        };
        assert!((vh(n.width) - 8.0).abs() < 1e-3, "{:?}", n.width);
        assert!((vh(n.height) - 4.0).abs() < 1e-3, "{:?}", n.height);
        let tint = |app: &App| app.world().get::<ImageNode>(pic).unwrap().color;
        assert_eq!(tint(&app), Color::srgba_u8(200, 240, 255, 110));
        app.world_mut().resource_mut::<WeaponReadout>().on_friend = true;
        app.update();
        assert_eq!(tint(&app), Color::srgba_u8(130, 255, 130, 220));
        app.world_mut().resource_mut::<WeaponReadout>().on_target = true;
        app.update();
        assert_eq!(tint(&app), Color::srgba_u8(255, 130, 130, 220));
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
