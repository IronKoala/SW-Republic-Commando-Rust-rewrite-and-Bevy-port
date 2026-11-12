//! The objective tracker: the tilted ring with an arrow and a distance at
//! the bottom of the screen that points the player at the next goal, and
//! the four pulsing corner arrows round the goal once it is in view.
//!
//! A level script turns it on with `ACTION_ShowActorObjectiveTracker`
//! (ctgame.u, UnrealScript source in the package): it finds the first
//! actor whose `Tag` is `ActorTag` (`AllActors` order, which is the map's
//! actor list) and calls `CTPlayer.ShowActorObjectiveTracker`, which hands
//! it to the helmet (engine.u `Helmet.ShowActorObjectiveTracker`: sets
//! `ObjectiveActor`, `bShowObjectiveTracker`, and makes the next sight
//! check happen at once). When no actor has the tag, or the tag is None,
//! it calls `HideObjectiveTracker` instead. When the actor is destroyed
//! the helmet hides the tracker too (Engine.dll `UHelmet::ActorDestroyed`).
//!
//! The rest is native code, read from Engine.dll (`UHelmet::DrawHelmet`,
//! 0x10437ada to 0x104389a6, decompiled locally for reference; nothing of
//! it is in this repository). Each frame while the helmet draws:
//!
//! 1. The goal's place is its actor's `Location`, followed as it moves.
//!    Its size is the radius of the actor's static mesh's bounding sphere
//!    (`StaticMesh->GetRenderBoundingSphere`, which is the mesh's own
//!    sphere, not scaled by `DrawScale`), or 0 with no static mesh.
//! 2. `dir` is the unit line from the eye (pawn location plus eye height)
//!    to the goal. If the view direction (`Rotation.Vector()` of the
//!    player controller) is within 45 degrees of it (dot product at least
//!    cos(pi/4)), the goal may be in view: a line is checked through the
//!    level and its movers (`SingleLineCheck`, flags 0x286) from the eye
//!    to 250 units short of the goal. That check runs at once when the
//!    goal was not in view last frame, then every `VisCheckFrequency`
//!    (0.25 s, engine.u Helmet defaults) while it stays clear.
//! 3. In view and clear (and `bBracketVisibleObjective`, true by default):
//!    four corner arrows (see [`bracket_corners`]). Otherwise: the ring
//!    (see [`ring_angle`]) and the distance from the pawn's `Location`
//!    (the centre of its collision box) to the goal, in hundreds of
//!    units, one decimal (`"%.1f"`).
//!
//! This part does the rules; `rc_game` draws [`ObjectiveTracker::view`].

use crate::PART;
use bevy::prelude::*;
use rc_combat::{Dead, Hittable};
use rc_game_core::units::to_unreal;
use rc_game_core::{GameLog, GamePaths, LevelCollision, PlayerCamera, PlayerPawn, ScriptTags};
use rc_level::scripts::{MapScripts, ScriptObject};
use rc_package::properties::Value;
use rc_package::Package;
use std::collections::HashMap;

/// How far above its feet a character's `Location` is when the game has no
/// size for it (a cutscene character): the commando's collision half
/// height, as `sight.rs`.
const DEFAULT_CENTRE_UP: f32 = 84.0;

/// Engine.dll's angle unit: 65536 to a full turn.
pub const FULL_TURN: i32 = 65536;

/// `FVector::Rotation` (Core.dll): `appAtan2(...) * 10430.218905280077`
/// (65535 / 2 pi), truncated to a whole number by `_ftol`.
const UNITS_PER_RADIAN: f64 = 10430.218905280077;

/// The numbers the tracker is drawn and run with. The built-in values are
/// the original's (Engine.dll and engine.u, see the module notes); the
/// file `data/ui/objective_tracker.toml` holds the same values so mods
/// can change them.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TrackerLook {
    /// `Helmet.TexObjectiveRing`.
    pub ring_texture: String,
    /// Ring centre, in the original's 800x600 screen units.
    pub ring_centre: [f32; 2],
    /// Ring picture size (units) and the part of the picture used (texels).
    pub ring_size: [f32; 2],
    pub ring_source: [f32; 4],
    /// How far the ring is tipped back, in Engine.dll angle units (12288
    /// is 67.5 degrees: the ring is drawn this much flatter).
    pub ring_tilt: i32,
    /// Its colour (red green blue), and the shadow drawn first: this many
    /// units lower, at this opacity, then the ring itself at full opacity.
    pub ring_colour: [u8; 3],
    pub ring_shadow_down: f32,
    pub ring_shadow_alpha: u8,
    /// `Helmet.Orbit8Font`; the distance is drawn centred on the ring's
    /// centre in this colour (red green blue opacity).
    pub distance_font: String,
    pub distance_colour: [u8; 4],
    /// Unreal units to the number shown.
    pub distance_scale: f32,
    /// `Helmet.LargeVisorIcons`, and the arrow's part of it (texels).
    pub bracket_texture: String,
    pub bracket_source: [f32; 4],
    /// Arrow size (units).
    pub bracket_size: f32,
    /// The box the arrows sit on is never smaller than this many screen
    /// pixels across (the arrow size, twice, but at least 40).
    pub bracket_min_pixels: f32,
    /// Each corner's turn (Engine.dll units): bottom right, top right,
    /// top left, bottom left.
    pub bracket_turns: [i32; 4],
    /// Opacity pulse: whole part of sin(time * speed) * amount + base,
    /// kept to its lowest 8 bits as the original's byte is.
    pub pulse_speed: f32,
    pub pulse_amount: f32,
    pub pulse_base: f32,
    /// The arrow is drawn three times: at size 1 with the pulse's opacity,
    /// then at these sizes with this part of it.
    pub glow_sizes: [f32; 2],
    pub glow_opacity: f32,
    /// `Helmet.VisCheckFrequency`, seconds.
    pub sight_check_seconds: f32,
    /// The sight line stops this many units short of the goal.
    pub sight_short: f32,
    /// The goal counts as ahead within this angle of the view, degrees.
    pub ahead_degrees: f32,
    /// `Helmet.bBracketVisibleObjective`.
    pub bracket_visible: bool,
}

impl Default for TrackerLook {
    fn default() -> Self {
        TrackerLook {
            ring_texture: "HUDTextures.Icons.ObjectiveRing".into(),
            ring_centre: [400.0, 463.0],
            ring_size: [256.0, 256.0],
            ring_source: [0.0, 0.0, 256.0, 256.0],
            ring_tilt: 12288,
            ring_colour: [200, 240, 255],
            ring_shadow_down: 5.0,
            ring_shadow_alpha: 63,
            distance_font: "OrbitFonts.OrbitBold8".into(),
            distance_colour: [200, 240, 255, 255],
            distance_scale: 0.01,
            bracket_texture: "HUDTextures.Icons.LargeVisorIcons".into(),
            bracket_source: [0.0, 360.0, 39.0, 399.0],
            bracket_size: 39.0,
            bracket_min_pixels: 78.0,
            bracket_turns: [24575, 8191, -8193, -24577],
            pulse_speed: 2.5,
            pulse_amount: 200.0,
            pulse_base: 55.0,
            glow_sizes: [1.25, 0.75],
            glow_opacity: 0.25,
            sight_check_seconds: 0.25,
            sight_short: 250.0,
            ahead_degrees: 45.0,
            bracket_visible: true,
        }
    }
}

impl TrackerLook {
    pub fn parse(text: &str) -> Result<TrackerLook, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// Engine.dll's sine: a 16384-entry table (`GMath`), looked up with
/// `(angle >> 2) & 0x3fff`, so angles step by 4 units.
pub fn table_sin(angle: i32) -> f32 {
    let i = (angle >> 2) & 0x3fff;
    (i as f64 * std::f64::consts::TAU / 16384.0).sin() as f32
}

pub fn table_cos(angle: i32) -> f32 {
    table_sin(angle.wrapping_add(16384))
}

/// `FVector::Rotation` (Core.dll): (pitch, yaw), each truncated.
pub fn rotation_of(v: [f32; 3]) -> (i32, i32) {
    let [x, y, z] = v.map(|c| c as f64);
    let yaw = (y.atan2(x) * UNITS_PER_RADIAN) as i32;
    let pitch = (z.atan2((x * x + y * y).sqrt()) * UNITS_PER_RADIAN) as i32;
    (pitch, yaw)
}

/// The port's yaw (radians, Unreal's direction) as a controller's whole
/// number yaw. (The port keeps radians, so this rounding is ours.)
pub fn yaw_units(yaw: f32) -> i32 {
    (yaw as f64 * FULL_TURN as f64 / std::f64::consts::TAU).round() as i32
}

/// The ring's turn (Engine.dll units, clockwise on screen): 65535 minus
/// (the view's yaw minus the goal's yaw seen from the pawn).
pub fn ring_angle(view_yaw: i32, goal_yaw: i32) -> i32 {
    0xffff_i32.wrapping_sub(view_yaw.wrapping_sub(goal_yaw))
}

/// Where a corner of a rotated, tipped picture lands, relative to its
/// centre (`FCanvasUtil::DrawRotatedTile`, checked by running it): turned
/// clockwise by `angle`, then flattened by the cosine of `tilt`.
pub fn ring_point(p: [f32; 2], angle: i32, tilt: i32) -> [f32; 2] {
    let (s, c) = (table_sin(angle), table_cos(angle));
    let x = p[0] * c - p[1] * s;
    let y = p[0] * s + p[1] * c;
    [x, y * table_cos(tilt)]
}

/// `FMatrix(FRotator(pitch, yaw, 0))` (Core.dll): its Y (right) and Z (up)
/// axes, with the table sine.
fn side_axes(pitch: i32, yaw: i32) -> ([f32; 3], [f32; 3]) {
    let (sp, cp) = (table_sin(pitch), table_cos(pitch));
    let (sy, cy) = (table_sin(yaw), table_cos(yaw));
    ([-sy, cy, 0.0], [-sp * cy, -sp * sy, cp])
}

/// The four corners round the goal, in the world (Unreal units): half its
/// radius to the side and up or down, square to the line from the eye.
/// Order: right-down, right-up, left-up, left-down (as `bracket_turns`).
pub fn bracket_corners(goal: [f32; 3], dir: [f32; 3], radius: f32) -> [[f32; 3]; 4] {
    let (pitch, yaw) = rotation_of(dir);
    let (y, z) = side_axes(pitch, yaw);
    let h = radius * 0.5;
    [(h, -h), (h, h), (-h, h), (-h, -h)]
        .map(|(a, b)| [0, 1, 2].map(|i| goal[i] + a * y[i] + b * z[i]))
}

/// The corners on screen (pixels, x right y down, same order), pushed out
/// so the box is at least `min` across each way: around the middle of
/// corner 0 and 2 across, and of corner 1 and 3 down, as the original
/// does it (Engine.dll 0x1043873f).
pub fn widen_box(mut p: [[f32; 2]; 4], min: f32) -> [[f32; 2]; 4] {
    let half = min * 0.5;
    let across = (p[0][0] - p[2][0]).abs();
    if across < min {
        let mid = across * 0.5 + p[2][0];
        p[0][0] = mid + half;
        p[1][0] = mid + half;
        p[2][0] = mid - half;
        p[3][0] = mid - half;
    }
    let down = (p[3][1] - p[1][1]).abs();
    if down < min {
        let mid = down * 0.5 + p[1][1];
        p[0][1] = mid + half;
        p[3][1] = mid + half;
        p[1][1] = mid - half;
        p[2][1] = mid - half;
    }
    p
}

/// The arrows' opacity at `time` seconds of level time: the whole part of
/// sin(time * speed) * amount + base (toward zero), kept to its lowest 8
/// bits (the original stores it in a byte, so below zero wraps round).
pub fn pulse_alpha(look: &TrackerLook, time: f32) -> u8 {
    let v = ((time as f64 * look.pulse_speed as f64).sin() * look.pulse_amount as f64
        + look.pulse_base as f64) as i32;
    v as u8
}

/// The glow copies' opacity: the pulse's byte times `glow_opacity`, whole.
pub fn glow_alpha(look: &TrackerLook, alpha: u8) -> u8 {
    ((alpha as f32 * look.glow_opacity) as i32) as u8
}

/// The distance as the original prints it.
pub fn distance_text(look: &TrackerLook, from: [f32; 3], to: [f32; 3]) -> String {
    let d = (0..3)
        .map(|i| (to[i] - from[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    format!("{:.1}", d * look.distance_scale)
}

/// A level script asks for the tracker (`ACTION_ShowActorObjectiveTracker`).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct TrackerAsk {
    /// `ActorTag`; "None" hides it.
    pub tag: String,
    /// Which script step, for the log.
    pub from: String,
}

/// If this script step is the tracker's, its `ActorTag`.
pub fn asked(action: &ScriptObject) -> Option<String> {
    action
        .class
        .eq_ignore_ascii_case("ACTION_ShowActorObjectiveTracker")
        .then(|| action.text("ActorTag").unwrap_or("None").to_string())
}

/// One actor placed in the map that a tracker step names.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    /// The actor's export index in the map.
    pub export: usize,
    pub class: String,
    pub name: String,
    /// Unreal units.
    pub location: [f32; 3],
    /// Its static mesh's bounding sphere radius (0 without one).
    pub radius: f32,
    /// A character (its class comes from `Pawn`): the game's character
    /// with this tag is followed while it is there.
    pub pawn: bool,
    /// Where the radius came from, for the log.
    pub why: String,
}

/// The first placed actor with each tag the map's tracker steps name,
/// lower-case tag to spot.
#[derive(Resource, Debug, Clone, Default)]
pub struct TrackerTargets {
    pub spots: HashMap<String, Spot>,
}

/// The tags this map's tracker steps name (not None).
pub fn tags_in(map: &MapScripts) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in &map.actors {
        for act in &a.actions {
            if let Some(t) = asked(act) {
                if !t.eq_ignore_ascii_case("None")
                    && !out.iter().any(|o| o.eq_ignore_ascii_case(&t))
                {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Script packages a placed actor's class may live in, for its defaults
/// (static mesh) and whether it is a character.
const CLASS_PACKAGES: [&str; 8] = [
    "GameData/System/Core.u",
    "GameData/System/Engine.u",
    "GameData/System/Gameplay.u",
    "GameData/System/CTGame.u",
    "GameData/System/CTInventory.u",
    "GameData/System/CTCharacters.u",
    "GameData/System/CTMarkers.u",
    "GameData/Properties/MarkerProperties.u",
];

fn class_library(paths: &GamePaths) -> (rc_package::class_library::ClassLibrary, Vec<String>) {
    let mut lib = rc_package::class_library::ClassLibrary::new();
    let mut missing = Vec::new();
    let mut rels: Vec<String> = CLASS_PACKAGES.iter().map(|s| s.to_string()).collect();
    // Every other properties package too (props, karma props...).
    if let Ok(dir) = std::fs::read_dir(paths.game_file("GameData/Properties")) {
        let mut more: Vec<String> = dir
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.to_ascii_lowercase()
                    .ends_with(".u")
                    .then(|| format!("GameData/Properties/{n}"))
            })
            .filter(|r| !rels.iter().any(|x| x.eq_ignore_ascii_case(r)))
            .collect();
        more.sort();
        rels.extend(more);
    }
    for rel in rels {
        let name = rel
            .rsplit('/')
            .next()
            .unwrap_or(&rel)
            .trim_end_matches(".u")
            .trim_end_matches(".U")
            .to_string();
        match std::fs::read(paths.game_file(&rel)) {
            Ok(d) => {
                if let Err(e) = lib.add_package(&name, d) {
                    missing.push(format!("{rel}: {e}"));
                }
            }
            Err(e) => missing.push(format!("{rel}: {e}")),
        }
    }
    (lib, missing)
}

/// The radius of a static mesh's bounding sphere, by its full path
/// (`Package.Group.Name`): from `GameData/StaticMeshes/Package.usx`, or
/// from the map itself when the package is the map's own (`myLevel`).
fn mesh_radius(
    paths: &GamePaths,
    map: (&Package, &[u8]),
    path: &str,
    cache: &mut HashMap<String, Option<(Package, Vec<u8>)>>,
) -> Result<f32, String> {
    let mut parts = path.split('.');
    let package = parts.next().unwrap_or("").to_ascii_lowercase();
    let name = path.rsplit('.').next().unwrap_or(path);
    let find = |pkg: &Package, data: &[u8]| -> Option<Result<f32, String>> {
        let e = pkg.exports.iter().find(|e| {
            pkg.name(e.object_name).eq_ignore_ascii_case(name)
                && pkg.export_class_name(e).eq_ignore_ascii_case("StaticMesh")
        })?;
        Some(
            rc_package::static_mesh::read_static_mesh(pkg, data, e)
                .map(|m| m.bounding_sphere[3])
                .map_err(|e| e.to_string()),
        )
    };
    if package == "mylevel" {
        return find(map.0, map.1).unwrap_or_else(|| Err(format!("{path} not in the map")));
    }
    let loaded = cache.entry(package.clone()).or_insert_with(|| {
        let data =
            std::fs::read(paths.game_file(&format!("GameData/StaticMeshes/{package}.usx"))).ok()?;
        let pkg = Package::parse(&data).ok()?;
        Some((pkg, data))
    });
    match loaded {
        Some((pkg, data)) => {
            find(pkg, data).unwrap_or_else(|| Err(format!("{path}: no such static mesh")))
        }
        None => Err(format!("{path}: package {package}.usx not readable")),
    }
}

/// Find the first placed actor for each tag the map's tracker steps name.
pub fn read_targets(
    pkg: &Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
    map: &MapScripts,
    paths: &GamePaths,
    log: &GameLog,
) -> TrackerTargets {
    let tags = tags_in(map);
    let mut out = TrackerTargets::default();
    if tags.is_empty() {
        return out;
    }
    let (lib, missing) = class_library(paths);
    for m in &missing {
        log.line(format!("{PART}: tracker: class package {m}"));
    }
    let mut meshes = HashMap::new();
    for &a in level.actors.iter().flatten() {
        if out.spots.len() == tags.len() {
            break;
        }
        let class = pkg.export_class_name(&pkg.exports[a]).to_string();
        let Ok(props) = rc_level::actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let find = |n: &str| rc_package::properties::find(pkg, &props, n).map(|p| &p.value);
        // UE2: an actor without a Tag of its own is tagged with its class.
        let tag = match find("Tag") {
            Some(Value::Name(n)) => pkg.name(*n).to_string(),
            _ => class.clone(),
        };
        let Some(want) = tags.iter().find(|t| t.eq_ignore_ascii_case(&tag)) else {
            continue;
        };
        let key = want.to_ascii_lowercase();
        if out.spots.contains_key(&key) {
            continue;
        }
        let location = match find("Location") {
            Some(Value::Raw(b)) if b.len() == 12 => {
                [0, 4, 8].map(|i| f32::from_le_bytes(b[i..i + 4].try_into().unwrap()))
            }
            _ => [0.0; 3],
        };
        // Its static mesh: the map's own setting, else its class's.
        let mesh = match find("StaticMesh") {
            Some(Value::Object(o)) => match rc_package::ObjectRef::from_index(*o) {
                rc_package::ObjectRef::None => None,
                rc_package::ObjectRef::Export(_) => Some(format!(
                    "myLevel.{}",
                    rc_package::class::object_path(pkg, *o)
                )),
                rc_package::ObjectRef::Import(_) => Some(rc_package::class::object_path(pkg, *o)),
            },
            _ => lib.object_path(&class, "StaticMesh").flatten(),
        };
        let (radius, why) = match &mesh {
            None => (0.0, "no static mesh".to_string()),
            Some(p) => match mesh_radius(paths, (pkg, data), p, &mut meshes) {
                Ok(r) => (r, format!("static mesh {p}")),
                Err(e) => (0.0, format!("static mesh unreadable ({e}); radius 0")),
            },
        };
        let chain = lib.chain(&class);
        let pawn = chain.iter().any(|c| c.eq_ignore_ascii_case("Pawn"));
        out.spots.insert(
            key,
            Spot {
                export: a,
                class: class.clone(),
                name: pkg.name(pkg.exports[a].object_name).to_string(),
                location,
                radius,
                pawn,
                why,
            },
        );
    }
    for t in &tags {
        match out.spots.get(&t.to_ascii_lowercase()) {
            Some(s) => log.line(format!(
                "{PART}: tracker target {t}: {} {} at ({:.0}, {:.0}, {:.0}), radius {:.1} ({}){}",
                s.class,
                s.name,
                s.location[0],
                s.location[1],
                s.location[2],
                s.radius,
                s.why,
                if s.pawn { ", a character" } else { "" }
            )),
            None => log.line(format!(
                "{PART}: tracker target {t}: no placed actor has this tag (only a character the game spawns can match it)"
            )),
        }
    }
    out
}

/// What the tracker follows.
#[derive(Debug, Clone, PartialEq)]
pub struct Followed {
    pub tag: String,
    /// A character of the game's, followed while it exists (the tracker
    /// hides when it goes, as the original's does when its actor is
    /// destroyed).
    pub character: Option<Entity>,
    /// Otherwise the placed actor (export index in the map): a door or
    /// lift is followed where it is now; anything else stays at `place`.
    pub export: Option<usize>,
    /// Its place as the map puts it (Unreal units).
    pub place: [f32; 3],
    pub radius: f32,
}

/// What to draw this frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TrackerView {
    #[default]
    Hidden,
    /// The ring, turned by `angle` (Engine.dll units), and the distance.
    Ring { angle: i32, distance: String },
    /// The arrows round the goal: the four corners in the world (Unreal
    /// units), to be put on screen with the player's camera.
    Brackets { corners: [[f32; 3]; 4] },
}

/// The tracker's state (the helmet's `ObjectiveActor`,
/// `bShowObjectiveTracker` and `mCachedObjectiveVisInfo`).
#[derive(Resource, Debug, Clone, Default)]
pub struct ObjectiveTracker {
    pub look: TrackerLook,
    pub followed: Option<Followed>,
    /// Seconds since the last sight check, and what it found.
    pub since_check: f32,
    pub visible: bool,
    pub view: TrackerView,
    /// Sight checks made (for the self-test and the log).
    pub checks: u32,
}

impl ObjectiveTracker {
    /// `ShowActorObjectiveTracker`: follow this, and check sight at once.
    pub fn show(&mut self, f: Followed) {
        self.followed = Some(f);
        self.since_check = self.look.sight_check_seconds;
    }

    /// `HideObjectiveTracker` (and `ActorDestroyed`).
    pub fn hide(&mut self) {
        self.followed = None;
        self.since_check = self.look.sight_check_seconds;
        self.view = TrackerView::Hidden;
    }

    /// One frame of the helmet's rules (see the module notes). `eye` and
    /// `pawn` are Unreal units; `forward` is the view's unit direction;
    /// `clear` checks the sight line from the eye to a point.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        goal: [f32; 3],
        eye: [f32; 3],
        forward: [f32; 3],
        pawn: [f32; 3],
        view_yaw: i32,
        clear: &mut dyn FnMut([f32; 3], [f32; 3]) -> bool,
    ) {
        let Some(f) = &self.followed else {
            self.view = TrackerView::Hidden;
            return;
        };
        let radius = f.radius;
        let to = [goal[0] - eye[0], goal[1] - eye[1], goal[2] - eye[2]];
        let len = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let dir = if len > 0.0 { to.map(|c| c / len) } else { to };
        let dot = dir[0] * forward[0] + dir[1] * forward[1] + dir[2] * forward[2];
        let ahead = dot >= self.look.ahead_degrees.to_radians().cos();
        let mut in_view = false;
        if ahead {
            self.since_check += dt;
            if self.since_check >= self.look.sight_check_seconds {
                let short = self.look.sight_short;
                let from = [0, 1, 2].map(|i| goal[i] - dir[i] * short);
                self.visible = clear(eye, from);
                self.since_check = 0.0;
                self.checks += 1;
            }
            in_view = self.visible;
        }
        if in_view {
            self.view = if self.look.bracket_visible {
                TrackerView::Brackets {
                    corners: bracket_corners(goal, dir, radius),
                }
            } else {
                TrackerView::Hidden
            };
            return;
        }
        // Not ahead, or the line was blocked: check again at once next
        // time it is ahead.
        self.since_check = self.look.sight_check_seconds;
        let (_, goal_yaw) = rotation_of([goal[0] - pawn[0], goal[1] - pawn[1], goal[2] - pawn[2]]);
        self.view = TrackerView::Ring {
            angle: ring_angle(view_yaw, goal_yaw),
            distance: distance_text(&self.look, pawn, goal),
        };
    }
}

/// Load `data/ui/objective_tracker.toml` into the tracker (the built-in
/// values when it is missing or wrong, and the log says so).
pub fn load_look(mut tracker: ResMut<ObjectiveTracker>, paths: Res<GamePaths>, log: Res<GameLog>) {
    tracker.look = match paths.data_file("ui/objective_tracker.toml") {
        None => {
            log.line(format!(
                "{PART}: no ui/objective_tracker.toml; built-in tracker values"
            ));
            TrackerLook::default()
        }
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| TrackerLook::parse(&t))
        {
            Ok(l) => l,
            Err(e) => {
                log.line(format!(
                    "{PART}: ERROR in {}: {e}; built-in tracker values",
                    p.display()
                ));
                TrackerLook::default()
            }
        },
    };
}

type Characters<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ScriptTags,
        &'static GlobalTransform,
        Option<&'static Hittable>,
        Has<Dead>,
    ),
>;

fn character_centre(t: &GlobalTransform, hit: Option<&Hittable>) -> [f32; 3] {
    let p = to_unreal(t.translation());
    let up = hit.map_or(DEFAULT_CENTRE_UP, |h| h.center_up);
    [p.x, p.y, p.z + up]
}

/// Take the scripts' requests, then run one frame of the tracker.
#[allow(clippy::too_many_arguments)]
pub fn track(
    time: Res<Time>,
    mut asks: MessageReader<TrackerAsk>,
    targets: Option<Res<TrackerTargets>>,
    mut tracker: ResMut<ObjectiveTracker>,
    characters: Characters,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    pawn: Res<PlayerPawn>,
    (collision, through, movers): (
        Option<Res<LevelCollision>>,
        Option<Res<rc_level_plugin::LineThroughVolumes>>,
        Option<Res<rc_movers_plugin::LevelMovers>>,
    ),
    log: Res<GameLog>,
) {
    let tracker = &mut *tracker;
    for ask in asks.read() {
        let key = ask.tag.to_ascii_lowercase();
        let spot = targets.as_ref().and_then(|t| t.spots.get(&key));
        // The game's character with this tag: the placed character itself
        // (the map placed a character), or one the game spawned (no placed
        // actor has the tag). A living one first.
        let live = || {
            let mut best: Option<(Entity, bool)> = None;
            for (e, tags, _, _, dead) in &characters {
                if tags
                    .tag
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case(&ask.tag))
                    && best.is_none_or(|(_, was_dead)| was_dead && !dead)
                {
                    best = Some((e, dead));
                }
            }
            best.map(|(e, _)| e)
        };
        let followed = if key == "none" {
            None
        } else {
            match spot {
                Some(s) => {
                    let character = if s.pawn { live() } else { None };
                    Some(Followed {
                        tag: ask.tag.clone(),
                        character,
                        export: Some(s.export),
                        place: s.location,
                        radius: s.radius,
                    })
                }
                None => live().map(|e| Followed {
                    tag: ask.tag.clone(),
                    character: Some(e),
                    export: None,
                    place: [0.0; 3],
                    radius: 0.0,
                }),
            }
        };
        match followed {
            Some(f) => {
                log.line(format!(
                    "{PART}: {}: objective tracker on {} ({})",
                    ask.from,
                    ask.tag,
                    match (&f.character, spot) {
                        (Some(_), _) => "following the character".to_string(),
                        (None, Some(s)) => format!(
                            "{} {} at ({:.0}, {:.0}, {:.0})",
                            s.class, s.name, s.location[0], s.location[1], s.location[2]
                        ),
                        (None, None) => String::new(),
                    }
                ));
                tracker.show(f);
            }
            None => {
                log.line(format!(
                    "{PART}: {}: objective tracker hidden ({})",
                    ask.from,
                    if key == "none" {
                        "tag None".to_string()
                    } else {
                        format!("nothing has the tag {}", ask.tag)
                    }
                ));
                tracker.hide();
            }
        }
    }
    // Where the goal is now.
    let goal = match &tracker.followed {
        None => {
            tracker.view = TrackerView::Hidden;
            return;
        }
        Some(Followed {
            character: Some(e),
            tag,
            ..
        }) => match characters.get(*e) {
            Ok((_, _, t, hit, _)) => character_centre(t, hit),
            Err(_) => {
                log.line(format!(
                    "{PART}: objective tracker: {tag} is gone; tracker hidden"
                ));
                tracker.hide();
                return;
            }
        },
        // A door or lift: where it has moved to (`Location` follows the
        // mover in the original too).
        Some(f) => f
            .export
            .and_then(|x| {
                let set = movers.as_ref()?.read();
                let m = set.movers.iter().find(|m| m.info.place.export == x)?;
                Some([m.pose.pos.x, m.pose.pos.y, m.pose.pos.z])
            })
            .unwrap_or(f.place),
    };
    let Ok(cam) = camera.single() else {
        return;
    };
    let eye = to_unreal(cam.translation());
    let f = cam.forward();
    let forward = [f.x, f.z, f.y];
    let before = std::mem::discriminant(&tracker.view);
    let mut clear = |a: [f32; 3], b: [f32; 3]| -> bool {
        collision
            .as_ref()
            .is_none_or(|c| line_clear(c.0.as_ref(), through.as_ref().map(|t| t.0.as_ref()), a, b))
    };
    tracker.step(
        time.delta_secs().min(0.25),
        goal,
        [eye.x, eye.y, eye.z],
        forward,
        pawn.centre,
        yaw_units(pawn.yaw),
        &mut clear,
    );
    if std::mem::discriminant(&tracker.view) != before {
        log.line(format!(
            "{PART}: objective tracker shows {}",
            match &tracker.view {
                TrackerView::Hidden => "nothing".to_string(),
                TrackerView::Ring { angle, distance } =>
                    format!("the ring (turn {angle}, distance {distance})"),
                TrackerView::Brackets { .. } => "the arrows round the goal".to_string(),
            }
        ));
    }
}

/// Nothing in the level or its movers between two points (Unreal units).
/// The original's check (`SingleLineCheck` with flags 0x286: movers,
/// level, level geometry) doesn't stop at blocking volumes; the port's
/// collision has them in, so a hit on one of `through` (the faces lines
/// pass, see `rc_level_plugin::LineThroughVolumes`) is passed and the line
/// goes on beyond it.
fn line_clear(
    world: &(dyn rc_movement::World + Send + Sync),
    through: Option<&rc_movement::TriangleWorld>,
    a: [f32; 3],
    b: [f32; 3],
) -> bool {
    use rc_movement::{Vec3, World as _};
    let (a, b) = (Vec3::new(a[0], a[1], a[2]), Vec3::new(b[0], b[1], b[2]));
    let len = (b - a).length();
    if len < 1.0 {
        return true;
    }
    let dir = (b - a) * (1.0 / len);
    // Long lines in pieces, as `rc_scripts::sight::clear_line` does.
    let pieces = (len / 256.0).ceil().max(1.0) as usize;
    for i in 0..pieces {
        let mut from = a + (b - a) * (i as f32 / pieces as f32);
        let to = a + (b - a) * ((i + 1) as f32 / pieces as f32);
        // At most a few volume faces in one piece.
        for _ in 0..16 {
            let Some(h) = world.sweep(Vec3::ZERO, from, to) else {
                break;
            };
            if h.time >= 0.999 {
                break;
            }
            let at = from + (to - from) * h.time;
            // Is what it hit a volume's face the line passes through?
            let volume = through
                .and_then(|t| t.sweep(Vec3::ZERO, from, to))
                .is_some_and(|v| ((from + (to - from) * v.time) - at).length() < 0.5);
            if !volume {
                return false;
            }
            from = at + dir * 0.5;
            if (to - from).dot(dir) <= 0.0 {
                break;
            }
        }
    }
    true
}

/// A level ends: nothing to follow.
pub fn forget(mut tracker: ResMut<ObjectiveTracker>) {
    tracker.hide();
    tracker.checks = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_turns_toward_the_goal() {
        // Goal straight ahead: (almost) no turn.
        assert_eq!(ring_angle(0, 0), 65535);
        // A quarter turn to the right (Unreal yaw grows to the right):
        // the arrow turns a quarter clockwise.
        assert_eq!(ring_angle(0, 16384).rem_euclid(65536), 16383);
        let p = ring_point([0.0, -128.0], 16384, 0);
        assert!((p[0] - 128.0).abs() < 0.1 && p[1].abs() < 0.1, "{p:?}");
    }

    #[test]
    fn ring_corners_match_the_engine() {
        // From running Engine.dll's DrawRotatedTile: turn 8192, tilt
        // 12288, 256 square at (400, 463): corners (400, 393.7269),
        // (581.0193, 463), (400, 532.2731), (218.9807, 463).
        let want = [
            [400.0, 393.7269],
            [581.0193, 463.0],
            [400.0, 532.2731],
            [218.9807, 463.0],
        ];
        let corners = [
            [-128.0, -128.0],
            [128.0, -128.0],
            [128.0, 128.0],
            [-128.0, 128.0],
        ];
        for (c, w) in corners.iter().zip(want) {
            let p = ring_point(*c, 8192, 12288);
            assert!((p[0] + 400.0 - w[0]).abs() < 0.01, "{p:?} {w:?}");
            assert!((p[1] + 463.0 - w[1]).abs() < 0.01, "{p:?} {w:?}");
        }
    }

    #[test]
    fn rotation_truncates_like_core() {
        assert_eq!(rotation_of([1.0, 0.0, 0.0]), (0, 0));
        assert_eq!(rotation_of([0.0, 1.0, 0.0]).1, 16383);
        assert_eq!(rotation_of([0.0, -1.0, 0.0]).1, -16383);
        assert_eq!(rotation_of([1.0, 0.0, 1.0]).0, 8191);
    }

    #[test]
    fn corners_sit_square_to_the_view() {
        // Looking along +X: right is +Y, up is +Z.
        // Half the radius out each way.
        let c = bracket_corners([100.0, 0.0, 0.0], [1.0, 0.0, 0.0], 40.0);
        assert_eq!(c[0], [100.0, 20.0, -20.0]);
        assert_eq!(c[1], [100.0, 20.0, 20.0]);
        assert_eq!(c[2], [100.0, -20.0, 20.0]);
        assert_eq!(c[3], [100.0, -20.0, -20.0]);
    }

    #[test]
    fn small_boxes_widen_to_the_minimum() {
        let p = [[101.0, 101.0], [101.0, 99.0], [99.0, 99.0], [99.0, 101.0]];
        let w = widen_box(p, 78.0);
        assert_eq!(w[0], [139.0, 139.0]);
        assert_eq!(w[1], [139.0, 61.0]);
        assert_eq!(w[2], [61.0, 61.0]);
        assert_eq!(w[3], [61.0, 139.0]);
        // Big enough already: unchanged.
        let big = [[300.0, 300.0], [300.0, 0.0], [0.0, 0.0], [0.0, 300.0]];
        assert_eq!(widen_box(big, 78.0), big);
    }

    #[test]
    fn pulse_wraps_like_a_byte() {
        let l = TrackerLook::default();
        assert_eq!(pulse_alpha(&l, 0.0), 55);
        // sin = 1 (as near as a float time gets): 254 or 255.
        let t = std::f32::consts::FRAC_PI_2 / 2.5;
        assert!(pulse_alpha(&l, t) >= 254);
        // sin = -1: -145 (or -144 just short of it), which the byte keeps
        // as 111 (112): the dim end is not dark.
        let t = 3.0 * std::f32::consts::FRAC_PI_2 / 2.5;
        assert!((111..=112).contains(&pulse_alpha(&l, t)));
        assert_eq!(glow_alpha(&l, 255), 63);
    }

    #[test]
    fn distance_in_hundreds() {
        let l = TrackerLook::default();
        assert_eq!(distance_text(&l, [0.0; 3], [3770.0, 0.0, 0.0]), "37.7");
    }

    #[test]
    fn sight_check_runs_at_once_then_every_quarter_second() {
        let mut t = ObjectiveTracker::default();
        t.show(Followed {
            tag: "x".into(),
            character: None,
            export: None,
            place: [1000.0, 0.0, 0.0],
            radius: 0.0,
        });
        let mut n = 0;
        let mut clear = |_: [f32; 3], _: [f32; 3]| {
            n += 1;
            true
        };
        for _ in 0..10 {
            t.step(
                0.05,
                [1000.0, 0.0, 0.0],
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0; 3],
                0,
                &mut clear,
            );
        }
        // At once, then after 0.25 s (5 frames) and 0.5 s.
        assert_eq!(n, 2);
        assert!(matches!(t.view, TrackerView::Brackets { .. }));
        // Turned away: the ring, pointing behind.
        let mut never = |_: [f32; 3], _: [f32; 3]| -> bool { panic!("no check behind") };
        t.step(
            0.05,
            [1000.0, 0.0, 0.0],
            [0.0; 3],
            [-1.0, 0.0, 0.0],
            [0.0; 3],
            32768,
            &mut never,
        );
        match &t.view {
            TrackerView::Ring { angle, distance } => {
                assert_eq!(*angle, 65535 - 32768);
                assert_eq!(distance, "10.0");
            }
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn blocked_goal_checks_every_frame() {
        let mut t = ObjectiveTracker::default();
        t.show(Followed {
            tag: "x".into(),
            character: None,
            export: None,
            place: [1000.0, 0.0, 0.0],
            radius: 0.0,
        });
        let mut n = 0;
        let mut blocked = |_: [f32; 3], _: [f32; 3]| {
            n += 1;
            false
        };
        for _ in 0..4 {
            t.step(
                0.01,
                [1000.0, 0.0, 0.0],
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0; 3],
                0,
                &mut blocked,
            );
        }
        assert_eq!(n, 4);
        assert!(matches!(t.view, TrackerView::Ring { .. }));
    }

    /// A wall across the line at x = `x` (two triangles, big, facing
    /// the line's start).
    fn wall(x: f32) -> Vec<[rc_movement::Vec3; 3]> {
        let v = |y: f32, z: f32| rc_movement::Vec3::new(x, y, z);
        vec![
            [v(-1000.0, -1000.0), v(1000.0, 1000.0), v(1000.0, -1000.0)],
            [v(-1000.0, -1000.0), v(-1000.0, 1000.0), v(1000.0, 1000.0)],
        ]
    }

    #[test]
    fn sight_line_passes_blocking_volumes_but_not_walls() {
        use rc_movement::TriangleWorld;
        let (a, b) = ([0.0, 10.0, 20.0], [2000.0, 10.0, 20.0]);
        // A blocking volume's face at 500: in the collision, and in the
        // faces lines pass.
        let volume = TriangleWorld::new(wall(500.0));
        let through = TriangleWorld::new(wall(500.0));
        assert!(line_clear(&volume, Some(&through), a, b));
        assert!(!line_clear(&volume, None, a, b));
        // A real wall behind the volume still stops it.
        let mut both = wall(500.0);
        both.extend(wall(1500.0));
        let both = TriangleWorld::new(both);
        assert!(!line_clear(&both, Some(&through), a, b));
    }

    #[test]
    fn shipped_file_matches_the_built_in_values() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/ui/objective_tracker.toml"
        ))
        .unwrap();
        assert_eq!(TrackerLook::parse(&text).unwrap(), TrackerLook::default());
    }
}
