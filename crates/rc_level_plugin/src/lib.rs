//! The level part of the game. `LevelPlugin` loads the map the menus chose
//! (`CurrentLevel`) when the game enters `GameState::Loading`, and hands the
//! rest of the game what it needs:
//!
//! - `LevelCollision`: the map's BSP, every placed prop and door (also the
//!   ones whose model comes from their class) and the invisible walls
//!   (blocking volumes), loaded by `rc_level::map_collision`;
//! - `PlayerStart`: the feet and facing of the map's first `PlayerStart`;
//! - the map's look, drawn with the level viewer's code
//!   (`rc_level_viewer::scene` and `draw`), and its sky
//!   (`rc_level_viewer::sky`): a second camera inside the sky box that turns
//!   with the player's camera;
//! - `LevelInfo`: counts for the log and the self-tests.
//!
//! Everything it spawns is removed when the game leaves `InGame`.
//!
//! It never reads the keyboard or moves the player, and it only touches the
//! player's camera to let the sky show through it (`SkyViewer` and no
//! clearing). A VR mod's camera gets the sky the same way: give it
//! `PlayerCamera`.

use bevy::camera::ClearColorConfig;
use bevy::prelude::*;
use rc_game_core::{
    CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, LoadingGate, PlayerCamera,
    PlayerStart,
};
use rc_level_viewer::draw::{self, BACKGROUND};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::sky::{self, SkyViewer};
use rc_level_viewer::{materials, scene};
use rc_movement::{MoveParams, TriangleWorld, Vec3};
use rc_package::Package;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// For other parts and self-tests: the sky camera's marker, and the marker
/// the player's camera gets.
pub use rc_level_viewer::sky::{SkyCamera, SkyViewer as SkyFollows};

/// The name this part uses in the loading gate and the log.
pub const PART: &str = "level";

/// The lighting file a mod (or the player) can put in a data folder to
/// change brightness without touching code. Optional.
pub const LIGHTING_FILE: &str = "levels/lighting.cfg";

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(sky::SkyPlugin)
            .add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(Update, load.run_if(in_state(GameState::Loading)))
            .add_systems(
                Update,
                let_sky_through.run_if(in_state(GameState::InGame)),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

/// What was loaded, for the log and self-tests. Present from the end of
/// loading until the game leaves `InGame`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelInfo {
    pub map: String,
    pub collision_triangles: usize,
    pub props: usize,
    /// Props and doors whose model comes from their class.
    pub class_props: usize,
    pub unreadable_props: usize,
    pub blocking_volumes: usize,
    pub drawn_triangles: usize,
    /// The sky box's position (Bevy units), if the map has one.
    pub sky: Option<[f32; 3]>,
    /// Whether the map has a PlayerStart (if not, the player starts above
    /// the middle of the map).
    pub has_player_start: bool,
    pub load_ms: f64,
    pub draw_ms: f64,
}

/// Frames to wait in the loading screen before the slow load, so the
/// loading screen is drawn first.
#[derive(Resource)]
struct Countdown(u32);

fn begin(mut commands: Commands, mut gate: ResMut<LoadingGate>) {
    gate.wait(PART);
    commands.insert_resource(Countdown(3));
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelInfo>();
}

/// The loading itself. It blocks for as long as the map takes (about 2 s for
/// geo_01a in a debug build); the loading screen stays up meanwhile.
fn load(world: &mut World) {
    {
        let Some(mut c) = world.get_resource_mut::<Countdown>() else {
            return;
        };
        if c.0 > 0 {
            c.0 -= 1;
            return;
        }
    }
    world.remove_resource::<Countdown>();
    let log = world.resource::<GameLog>().clone();
    let paths = world.resource::<GamePaths>().clone();
    let Some(map) = world.resource::<CurrentLevel>().map.clone() else {
        world.resource_mut::<LoadingGate>().fail(PART, "no map chosen");
        return;
    };
    let path = paths.map_path(&map);
    let game_data = paths.game_data();

    // Collision and the player start, in Unreal units. The pawn's centre
    // stands at the PlayerStart, so the feet are one collision height lower.
    let height = MoveParams::player_commando().collision_height;
    let t = Instant::now();
    let lvl = match rc_level::map_collision::load(&path, &game_data, height) {
        Ok(l) => l,
        Err(e) => {
            log.line(format!("{PART}: ERROR loading {}: {e}", path.display()));
            world.resource_mut::<LoadingGate>().fail(PART, e);
            return;
        }
    };
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    log.line(format!(
        "{PART}: {} loaded: {} collision triangles, {} props ({} with their class's model, {} unreadable), {} blocking volumes ({} unreadable), {:.0} ms",
        path.display(),
        lvl.triangles.len(),
        lvl.mesh_actors,
        lvl.class_mesh_actors,
        lvl.mesh_failures,
        lvl.blocking_volumes,
        lvl.volume_failures,
        load_ms
    ));
    let mut info = LevelInfo {
        map: map.clone(),
        collision_triangles: lvl.triangles.len(),
        props: lvl.mesh_actors,
        class_props: lvl.class_mesh_actors,
        unreadable_props: lvl.mesh_failures,
        blocking_volumes: lvl.blocking_volumes,
        has_player_start: lvl.start.is_some(),
        load_ms,
        ..default()
    };
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    let tw = TriangleWorld::new(lvl.triangles.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect());
    let (feet, yaw) = match lvl.start {
        Some((f, yaw)) => (v(f), yaw),
        None => {
            let (lo, hi) = tw.bounds();
            log.line(format!("{PART}: no PlayerStart, starting above the middle"));
            (Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z), 0.0)
        }
    };
    log.line(format!(
        "{PART}: player start feet ({:.1}, {:.1}, {:.1}) facing {:.0} deg",
        feet.x,
        feet.y,
        feet.z,
        yaw.to_degrees()
    ));
    world.insert_resource(PlayerStart { feet, yaw });
    world.insert_resource(LevelCollision(Arc::new(tw)));

    // The look. If it can't be built the level is still solid, just
    // invisible, so the game goes on and says so in the log.
    let t = Instant::now();
    match build_look(&path, &game_data, &paths) {
        Ok((sc, settings, notes)) => {
            for n in notes {
                log.line(format!("{PART}: lighting: {n}"));
            }
            let report = world.resource_scope(|world, mut meshes: Mut<Assets<Mesh>>| {
                world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
                    world.resource_scope(|world, mut mats: Mut<Assets<StandardMaterial>>| {
                        let mut commands = world.commands();
                        draw::spawn_level(&mut commands, &mut meshes, &mut images, &mut mats, &sc, &settings)
                    })
                })
            });
            let mut spawned = report.entities.clone();
            if let Some(s) = &sc.sky {
                let bg = Color::srgb_u8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]);
                let mut commands = world.commands();
                let cam = sky::spawn_sky_camera(&mut commands, s, bg);
                // Under the player's camera (order 0) and the menus' and
                // display's camera (order 1).
                commands.entity(cam).insert(Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(bg),
                    ..default()
                });
                spawned.push(cam);
                info.sky = Some(s.position);
                log.line(format!(
                    "{PART}: sky camera inside the sky box at {} ({} sky batches)",
                    s.from, report.sky_batches
                ));
            } else {
                log.line(format!("{PART}: this map has no SkyZoneInfo, so no sky"));
            }
            world.flush();
            for e in spawned {
                if let Ok(mut e) = world.get_entity_mut(e) {
                    e.insert(DespawnOnExit(GameState::InGame));
                }
            }
            info.drawn_triangles = report.triangles;
            info.draw_ms = t.elapsed().as_secs_f64() * 1000.0;
            log.line(format!(
                "{PART}: drawn in {:.0} ms: {} triangles, {} textured batches, {} lightmapped, {} lights",
                info.draw_ms,
                report.triangles,
                report.textured,
                report.lightmapped,
                report.lights.point + report.lights.spot + report.lights.directional
            ));
        }
        Err(e) => log.line(format!("{PART}: map look not built ({e}); it will be invisible")),
    }
    world.insert_resource(info);
    world.resource_mut::<LoadingGate>().done(PART);
}

fn build_look(
    map_path: &Path,
    game_data: &Path,
    paths: &GamePaths,
) -> Result<(scene::Scene, LightingSettings, Vec<String>), String> {
    let (settings, notes) = match paths.data_file(LIGHTING_FILE) {
        Some(p) => LightingSettings::load(&p),
        None => (LightingSettings::default(), Vec::new()),
    };
    let data = std::fs::read(map_path).map_err(|e| e.to_string())?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut mats = materials::Materials::new(game_data);
    let sc = scene::build(Rc::new((pkg, data)), &mut mats)?;
    Ok((sc, settings, notes))
}

/// When the map has a sky, the player's camera must not clear the picture
/// the sky camera drew, and the sky camera must turn with it.
fn let_sky_through(
    mut commands: Commands,
    info: Option<Res<LevelInfo>>,
    mut views: Query<(Entity, &mut Camera), (With<PlayerCamera>, Without<SkyViewer>)>,
    log: Res<GameLog>,
) {
    if info.is_none_or(|i| i.sky.is_none()) {
        return;
    }
    for (e, mut cam) in &mut views {
        cam.clear_color = ClearColorConfig::None;
        commands.entity(e).insert(SkyViewer);
        log.line(format!("{PART}: the player's camera now shows the sky behind the level"));
    }
}
