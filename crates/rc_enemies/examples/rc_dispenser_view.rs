//! rc_dispenser_view: watch one droid dispenser on a real map, in a window.
//!
//! `RC_GAME_DIR=<game folder> cargo run --release -p rc_enemies --example rc_dispenser_view [MAP] [N] [--level]`
//!
//! Makes the map solid (as rc_droidview does; with `--level` it draws it
//! too, otherwise only the characters and the dispenser are drawn, so
//! nothing hides them) and runs the game's own enemies
//! part (`EnemiesPlugin`, with `CombatPlugin`). A stand-in player stands
//! still 1200 uu in front of where dispenser `N` (default 0, counting the
//! map's battle droid and super battle droid dispensers) ends up after its
//! intro, looking at it, and doesn't shoot. Then this program stands in
//! for the level: it sends the dispenser's tag (and its online tag after
//! the intro, if it has one). At 16 s it hurts the dispenser three times
//! (to 60 %, 30 % and 0 of its health), as the player's explosives would.
//!
//! Pictures go to `logs/dispenser_view_<seconds>.png` along the way
//! (`--shots 1.5,6,7.2` picks other moments), and
//! the game's log to `logs/rc_dispenser_view.log`. Ends with CHECK lines
//! about what the dispenser did and exits 1 if any failed.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::render::view::Tonemapping;
use rc_combat::Health;
use rc_enemies::dispensers::{DispenserBody, DispenserStats};
use rc_enemies::{EnemiesPlugin, Fight};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    playing, CorePlugin, CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, Player,
};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::{draw, materials, prop_light, scene};
use rc_movement::{MoveParams, TriangleWorld, Vec3 as UVec3};
use rc_package::Package;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
/// Seconds after its tag when it is hurt, and when the program ends.
const HURT_AT: f32 = 16.0;
const END_AT: f32 = 22.0;
/// Pictures at these seconds after its tag.
const SHOTS: &[f32] = &[0.2, 0.8, 1.4, 2.5, 4.0, 5.5, 8.0, 12.0, 16.5, 18.0, 21.0];

#[derive(Resource)]
struct DrawLevel(bool);

#[derive(Resource)]
struct Watch {
    which: usize,
    spawner: Option<usize>,
    started: Option<f32>,
    online_sent: bool,
    hurt: u32,
    shots_taken: usize,
    shots: Vec<f32>,
    done: bool,
}

fn main() -> AppExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let draw_level = args.iter().any(|a| a == "--level");
    let shots: Vec<f32> = match args.iter().position(|a| a == "--shots") {
        Some(i) => args
            .get(i + 1)
            .map(|l| l.split(',').filter_map(|t| t.trim().parse().ok()).collect())
            .unwrap_or_default(),
        None => SHOTS.to_vec(),
    };
    let args: Vec<String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in args {
            if skip {
                skip = false;
            } else if a == "--shots" {
                skip = true;
            } else if a != "--level" {
                out.push(a);
            }
        }
        out
    };
    let map = args.first().cloned().unwrap_or_else(|| "yyy_35b".into());
    let which = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let _ = std::fs::create_dir_all("logs");
    let log = match GameLog::create(Path::new("logs/rc_dispenser_view.log")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't create the log: {e}");
            return AppExit::from_code(1);
        }
    };
    let game =
        PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    log.line(format!(
        "rc_dispenser_view: map {map}, dispenser {which}, game folder {}",
        game.display()
    ));
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Droid dispenser (rc_dispenser_view)".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((CorePlugin, rc_combat::CombatPlugin, EnemiesPlugin))
        .add_plugins(rc_effects_plugin::EffectsPlugin)
        .add_plugins(prop_light::PropLightPlugin)
        .insert_resource(ClearColor(Color::srgb(0.2, 0.25, 0.3)))
        .insert_resource(log)
        .insert_resource(GamePaths::new(game, vec![PathBuf::from("data")]))
        .insert_resource(CurrentLevel { map: Some(map) })
        .insert_resource(DrawLevel(draw_level))
        .insert_resource(Watch {
            which,
            spawner: None,
            started: None,
            online_sent: false,
            hurt: 0,
            shots_taken: 0,
            shots,
            done: false,
        })
        .add_systems(Startup, |mut next: ResMut<NextState<GameState>>| {
            next.set(GameState::Loading)
        })
        .add_systems(OnEnter(GameState::Loading), load_level)
        .add_systems(Update, place_player.run_if(in_state(GameState::Loading)))
        .add_systems(Update, run.run_if(playing).run_if(resource_exists::<Fight>))
        .run()
}

/// Draw the map and make it solid (as rc_droidview).
fn load_level(world: &mut World) {
    let log = world.resource::<GameLog>().clone();
    let paths = world.resource::<GamePaths>().clone();
    let map = world
        .resource::<CurrentLevel>()
        .map
        .clone()
        .unwrap_or_default();
    let path = paths.map_path(&map);
    let coll = match rc_level::map_collision::load(
        &path,
        &paths.game_data(),
        MoveParams::player_commando().collision_height,
    ) {
        Ok(c) => c,
        Err(e) => {
            log.line(format!("ERROR loading {}: {e}", path.display()));
            world.write_message(AppExit::from_code(1));
            return;
        }
    };
    let v = |p: [f32; 3]| UVec3::new(p[0], p[1], p[2]);
    let tris = coll
        .triangles
        .iter()
        .map(|t| [v(t[0]), v(t[1]), v(t[2])])
        .collect();
    world.insert_resource(LevelCollision(Arc::new(TriangleWorld::new(tris))));
    if !world.resource::<DrawLevel>().0 {
        return;
    }
    let built = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|data| {
            let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
            let mut mats = materials::Materials::new(&paths.game_data());
            scene::build(Rc::new((pkg, data)), &mut mats)
        });
    match built {
        Ok(sc) => {
            let (settings, _) = LightingSettings::load(Path::new("lighting.cfg"));
            world.resource_scope(|world, mut meshes: Mut<Assets<Mesh>>| {
                world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
                    world.resource_scope(|world, mut mats: Mut<Assets<StandardMaterial>>| {
                        world.resource_scope(
                            |world, mut props: Mut<Assets<prop_light::PropMaterial>>| {
                                let mut commands = world.commands();
                                draw::spawn_level(
                                    &mut commands,
                                    &mut meshes,
                                    &mut images,
                                    &mut mats,
                                    None,
                                    &mut props,
                                    &sc,
                                    &settings,
                                );
                            },
                        )
                    })
                })
            });
            world.flush();
        }
        Err(e) => log.line(format!("level: not drawn ({e})")),
    }
}

/// Once the fight and its dispensers are loaded: stand in front of the
/// chosen one, looking at where it will stand, and start.
fn place_player(
    mut commands: Commands,
    fight: Option<Res<Fight>>,
    collision: Option<Res<LevelCollision>>,
    bodies: Query<&DispenserBody>,
    mut watch: ResMut<Watch>,
    mut next: ResMut<NextState<GameState>>,
    log: Res<GameLog>,
) {
    let (Some(fight), Some(_)) = (fight, collision) else {
        return;
    };
    if bodies.is_empty() {
        return;
    }
    let b = &fight.battle;
    let list: Vec<usize> = (0..b.spawners.len())
        .filter(|&i| b.spawners[i].motion.is_some())
        .collect();
    let Some(&i) = list.get(watch.which) else {
        log.line(format!(
            "CHECK FAIL: this map has {} drawn dispensers, no number {}",
            list.len(),
            watch.which
        ));
        commands.write_message(AppExit::from_code(1));
        return;
    };
    watch.spawner = Some(i);
    let sp = &b.spawners[i];
    let m = sp.motion.as_ref().unwrap();
    let d = sp.factory.dispenser.as_ref().unwrap();
    let yaw = sp.factory.yaw;
    let along = m.intro_travel.last().map_or(UVec3::ZERO, |p| p.1);
    let (s, c) = yaw.sin_cos();
    let landed = sp.factory.location
        + UVec3::new(
            along.x * c - along.y * s,
            along.x * s + along.y * c,
            along.z,
        );
    let forward = UVec3::new(c, s, 0.0);
    let bottom = landed - UVec3::new(0.0, 0.0, d.collision_height);
    let eye = bottom + forward * 1100.0 + UVec3::new(0.0, 0.0, 450.0);
    log.line(format!(
        "watching {} ({}), tag {}: lands at ({:.0}, {:.0}, {:.0}); eye at ({:.0}, {:.0}, {:.0})",
        sp.factory.name, d.class, sp.factory.tag, landed.x, landed.y, landed.z, eye.x, eye.y, eye.z
    ));
    // The stand-in: something for the droids to look at, standing on the
    // floor in front of the machine.
    commands.spawn((
        Player,
        Transform::from_translation(to_bevy(bottom + forward * 1100.0)),
    ));
    // A stand-in pawn where the machine lands: an intro that crushes
    // (`entranceSky`, ...) should kill it (`DroidDispenser.Bump`).
    commands.spawn((
        Name::new("crush stand-in"),
        CrushStandIn,
        Transform::from_translation(to_bevy(bottom)),
        Health::new(100.0),
        rc_combat::Hittable::COMMANDO,
    ));
    // A plain camera (not the player's, which would carry the first-person
    // arms).
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Exposure::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.05,
            far: 5000.0,
            fov: 80f32.to_radians(),
            ..default()
        }),
        Transform::from_translation(to_bevy(eye))
            .looking_at(to_bevy(bottom + UVec3::new(0.0, 0.0, 120.0)), Vec3::Y),
    ));
    next.set(GameState::InGame);
}

/// The pawn standing where the machine lands.
#[derive(Component)]
struct CrushStandIn;

/// Send its events, hurt it, take pictures, and finish with the checks.
#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    mut fight: ResMut<Fight>,
    mut watch: ResMut<Watch>,
    mut bodies: Query<(&DispenserBody, &mut Health), Without<CrushStandIn>>,
    stand_in: Query<&Health, With<CrushStandIn>>,
    stats: Res<DispenserStats>,
    mut exit: MessageWriter<AppExit>,
    log: Res<GameLog>,
) {
    let Some(i) = watch.spawner else { return };
    let now = fight.battle.time;
    let fight = &mut *fight;
    let Some(start) = watch.started else {
        let tag = fight.battle.spawners[i].factory.tag.clone();
        log.line(format!("t={now:.1}s sending {tag}"));
        let ev = fight.battle.trigger(&tag);
        fight.queued.extend(ev);
        watch.started = Some(now);
        return;
    };
    let t = now - start;
    let online_tag = fight.battle.spawners[i]
        .factory
        .dispenser
        .as_ref()
        .and_then(|d| d.online_tag.clone());
    if let Some(ot) = online_tag {
        let waiting = matches!(
            fight.battle.spawners[i]
                .disp
                .as_ref()
                .map(|s| s.phase.clone()),
            Some(rc_ai::Phase::WaitOnline)
        );
        if waiting && !watch.online_sent {
            watch.online_sent = true;
            log.line(format!("t={now:.1}s sending its online tag {ot}"));
            let ev = fight.battle.trigger(&ot);
            fight.queued.extend(ev);
        }
    }
    if watch.hurt < 3 && t >= HURT_AT + watch.hurt as f32 * 0.5 {
        for (b, mut h) in &mut bodies {
            if b.spawner == i {
                h.current = h.max * [0.6, 0.3, 0.0][watch.hurt as usize];
                log.line(format!(
                    "t={now:.1}s the dispenser is hurt to {:.0}",
                    h.current
                ));
            }
        }
        watch.hurt += 1;
    }
    if watch.shots_taken < watch.shots.len() && t >= watch.shots[watch.shots_taken] {
        let path = format!(
            "logs/dispenser_view_{:04.1}.png",
            watch.shots[watch.shots_taken]
        );
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        watch.shots_taken += 1;
    }
    if t >= END_AT && !watch.done {
        watch.done = true;
        let body = bodies.iter().find(|(b, _)| b.spawner == i).map(|(b, _)| b);
        let mut failed = 0;
        let mut check = |ok: bool, what: String| {
            log.line(format!(
                "CHECK {}: {what}",
                if ok { "PASS" } else { "FAIL" }
            ));
            if !ok {
                failed += 1;
            }
        };
        let played = body.map(|b| b.played.clone()).unwrap_or_default();
        check(
            !played.is_empty(),
            format!("its animations played: {played:?}"),
        );
        check(
            played.iter().any(|a| a.starts_with("Dispense")),
            "it played a slot's dispense animation".into(),
        );
        check(
            stats.released > 0,
            format!("{} droids let go", stats.released),
        );
        check(
            stats.unfolded > 0,
            format!("{} droids unfolded", stats.unfolded),
        );
        check(
            stats.destroyed == 1,
            format!(
                "destroyed ({}), effects: {:?}",
                stats.destroyed, stats.effects
            ),
        );
        check(
            body.is_some_and(|b| {
                b.skin.ends_with("_Dest") || b.skin.to_ascii_lowercase().contains("dest")
            }),
            format!(
                "shows its destroyed skin ({:?})",
                body.map(|b| b.skin.clone())
            ),
        );
        // The intro's crushing and landing effects, when it has them.
        let intro = played.first().cloned().unwrap_or_default();
        let crushes = ["entranceSky", "EntranceSky704", "EntranceSky384"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&intro));
        let left = stand_in.single().map(|h| h.current).unwrap_or(f32::NAN);
        if crushes {
            check(
                left <= 0.0,
                format!("its intro {intro} crushed the pawn where it landed (health {left})"),
            );
        } else {
            log.line(format!(
                "its intro {intro:?} doesn't crush; the pawn where it landed has {left} health"
            ));
        }
        let landing: Vec<String> = body
            .and_then(|b| b.model.effects.get(&intro))
            .map(|l| l.iter().map(|(_, e)| e.class.clone()).collect())
            .unwrap_or_default();
        if !landing.is_empty() {
            check(
                landing
                    .iter()
                    .all(|c| stats.effects.iter().any(|e| e.ends_with(c.as_str()))),
                format!(
                    "its intro started {landing:?} (effects: {:?})",
                    stats.effects
                ),
            );
        }
        log.line(format!(
            "heard: {:?}",
            body.map(|b| b.heard.clone()).unwrap_or_default()
        ));
        log.line(format!("DONE: {failed} checks failed"));
        exit.write(if failed == 0 {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
    }
}
