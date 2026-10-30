//! rc_droidview: watch battle droids fight in a real level, in a window.
//!
//! Usage: rc_droidview [MAP] [--seconds N] [--check]
//!
//! The game itself (`rc_game`) starts you where the map does, and on
//! geo_01a that area stays walled in until level scripts (not run yet) open
//! it. So this program uses a stand-in player instead: it starts on a path
//! 2500-4500 uu from a droid spawner's walk-in trigger, walks into it, and
//! shoots back with the DC-17 at the nearest droid it can see. You see
//! through its eyes. The droids, their shots, damage and deaths are the
//! real plugins (`EnemiesPlugin`, `CombatPlugin`), as in the game.
//!
//! `--check` also tests the picture: once a droid has been fighting for a
//! second and is in view, play pauses, the window is captured with the
//! droids shown and again with them hidden, and the pixels that changed
//! (the droids) are counted and their colour measured. Saved as
//! `logs/droidview_with.png`, `logs/droidview_without.png` and
//! `logs/droidview_droids.png`. Ends with CHECK lines and exits 1 if any
//! failed.
//!
//! Map: a name from GameData/Maps (default geo_01a). Game folder:
//! RC_GAME_DIR or the default Steam path. Log: logs/rc_droidview.log.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use rc_ai::Mode;
use rc_combat::{CombatSet, Dead, FireShot};
use rc_enemies::{Droid, EnemiesPlugin, Fight, Looks};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    playing, CorePlugin, CurrentLevel, GameLog, GamePaths, GameSet, GameState, LevelCollision,
    Play, Player, PlayerCamera,
};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::{draw, materials, scene};
use rc_movement::{MoveParams, Pawn, TriangleWorld, Vec3 as UVec3};
use rc_nav::PathFollower;
use rc_package::Package;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const SHOT_INTERVAL: f32 = 0.3;
/// The stand-in waits this long after the first droid sees it before
/// shooting back, so the droids get to fight first.
const HOLD_FIRE: f32 = 3.0;

#[derive(Resource)]
struct Options {
    seconds: f32,
    check: bool,
}

/// The stand-in player's body and route.
#[derive(Component)]
struct StandIn {
    pawn: Pawn,
    path: PathFollower,
    yaw: f32,
    next_shot: f32,
    /// Where it is looking, Unreal units.
    look_at: Option<UVec3>,
}

#[derive(Resource, Default)]
struct Check {
    stage: u32,
    frames: u32,
    with: Option<Vec<u8>>,
    without: Option<Vec<u8>>,
    size: (usize, usize),
    pending: bool,
    failed: u32,
    done: bool,
}

fn main() -> AppExit {
    let mut args = std::env::args().skip(1);
    let mut map = "geo_01a".to_string();
    let mut seconds = 40.0;
    let mut check = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(seconds),
            "--check" => check = true,
            other => map = other.to_string(),
        }
    }
    let log = match GameLog::create(Path::new("logs/rc_droidview.log")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't create the log: {e}");
            return AppExit::from_code(1);
        }
    };
    let game =
        PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    log.line(format!(
        "rc_droidview: map {map}, game folder {}",
        game.display()
    ));
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Battle droids (rc_droidview)".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((CorePlugin, rc_combat::CombatPlugin, EnemiesPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(log)
        .insert_resource(GamePaths::new(game, vec![PathBuf::from("data")]))
        .insert_resource(CurrentLevel { map: Some(map) })
        .insert_resource(Options { seconds, check })
        .init_resource::<Check>()
        .add_systems(Startup, |mut next: ResMut<NextState<GameState>>| {
            next.set(GameState::Loading)
        })
        .add_systems(OnEnter(GameState::Loading), load_level)
        .add_systems(Update, place_player.run_if(in_state(GameState::Loading)))
        .add_systems(
            Update,
            stand_in
                .in_set(GameSet::Simulate)
                .before(CombatSet::Fire)
                .run_if(playing),
        )
        .add_systems(
            Update,
            (picture_check, finish).run_if(in_state(GameState::InGame)),
        )
        .run()
}

/// Draw the map and make it solid.
fn load_level(world: &mut World) {
    let log = world.resource::<GameLog>().clone();
    let paths = world.resource::<GamePaths>().clone();
    let map = world
        .resource::<CurrentLevel>()
        .map
        .clone()
        .unwrap_or_default();
    let path = paths.map_path(&map);
    let t = std::time::Instant::now();
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
    log.line(format!(
        "level: {} collision triangles in {:.0} ms",
        coll.triangles.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
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
                        let mut commands = world.commands();
                        draw::spawn_level(
                            &mut commands,
                            &mut meshes,
                            &mut images,
                            &mut mats,
                            &sc,
                            &settings,
                        );
                    })
                })
            });
            world.flush();
        }
        Err(e) => log.line(format!("level: not drawn ({e})")),
    }
}

/// Once the fight is loaded: put the stand-in near a spawner and start.
fn place_player(
    mut commands: Commands,
    fight: Option<Res<Fight>>,
    collision: Option<Res<LevelCollision>>,
    mut next: ResMut<NextState<GameState>>,
    log: Res<GameLog>,
) {
    let (Some(fight), Some(_)) = (fight, collision) else {
        return;
    };
    let b = &fight.battle;
    let Some((node, goal, spawner)) = b.approach() else {
        log.line("no droid spawner trigger can be reached on this map");
        commands.write_message(AppExit::from_code(1));
        return;
    };
    let params = MoveParams::player_commando();
    let start = b.nav.nodes[node].location;
    let feet = start - UVec3::new(0.0, 0.0, params.collision_height);
    let route = b
        .nav
        .route(start, goal)
        .expect("approach() checked the route");
    log.line(format!(
        "stand-in: starts at path node {} ({:.0}, {:.0}, {:.0}), walks {:.0} uu to the walk-in trigger of {}",
        b.nav.nodes[node].name,
        start.x,
        start.y,
        start.z,
        route.length,
        b.spawners[spawner].factory.name
    ));
    let first = route.points.first().copied().unwrap_or(goal);
    let yaw = (first.y - start.y).atan2(first.x - start.x);
    commands.spawn((
        Player,
        StandIn {
            pawn: Pawn::new(params, feet),
            path: PathFollower::new(route.points),
            yaw,
            next_shot: 0.0,
            look_at: None,
        },
        Transform::from_translation(to_bevy(feet)),
    ));
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Tonemapping::None,
        Exposure::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.05,
            far: 5000.0,
            fov: 70f32.to_radians(),
            ..default()
        }),
        Transform::default(),
    ));
    next.set(GameState::InGame);
}

/// Walk the route; look at, and later shoot, the nearest droid in sight.
#[allow(clippy::too_many_arguments)]
fn stand_in(
    time: Res<Time>,
    fight: Res<Fight>,
    collision: Res<LevelCollision>,
    mut me: Query<(Entity, &mut StandIn, &mut Transform), (With<Player>, Without<Dead>)>,
    mut camera: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
    mut fire: MessageWriter<FireShot>,
) {
    let Ok((entity, mut s, mut t)) = me.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.1);
    let world = &*collision.0;
    let s = &mut *s;
    let mut input = s.path.update(&s.pawn, dt).unwrap_or_default();
    let eye = s.pawn.eye();
    let target = fight
        .battle
        .alive()
        .filter(|f| (f.pawn.location - eye).length() < 6000.0)
        .filter(|f| rc_ai::brain::clear_line(world, eye, f.eye()))
        .min_by(|a, b| {
            (a.pawn.location - eye)
                .length()
                .total_cmp(&(b.pawn.location - eye).length())
        });
    if let Some(f) = target {
        let aim = f.pawn.location;
        s.look_at = Some(aim);
        // Stop and fight.
        input = Default::default();
        let now = fight.battle.time;
        let spotted = fight.stats.first_spotted.unwrap_or(f32::INFINITY);
        if now >= s.next_shot && now > spotted + HOLD_FIRE {
            s.next_shot = now + SHOT_INTERVAL;
            fire.write(FireShot {
                shooter: entity,
                origin: to_bevy(eye),
                direction: to_bevy(aim - eye),
                weapon: "dc17m_blaster".into(),
            });
        }
    } else {
        s.look_at = s.path.target();
    }
    if let Some(p) = s.look_at {
        s.yaw = (p.y - eye.y).atan2(p.x - eye.x);
    }
    input.yaw = s.yaw;
    s.pawn.step(dt, &input, world);
    t.translation = to_bevy(s.pawn.feet());
    if let Ok(mut c) = camera.single_mut() {
        c.translation = to_bevy(s.pawn.eye());
        let look = s.look_at.map(to_bevy).unwrap_or(c.translation + Vec3::X);
        if (look - c.translation).length() > 0.01 {
            c.look_at(look, Vec3::Y);
        }
    }
}

fn rgba(img: &Image) -> Option<(Vec<u8>, usize, usize)> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let bgra = matches!(
        img.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let data = img.data.as_ref()?;
    let mut out = Vec::with_capacity(w * h * 4);
    for p in data.chunks_exact(4) {
        out.extend(if bgra {
            [p[2], p[1], p[0], 255]
        } else {
            [p[0], p[1], p[2], 255]
        });
    }
    Some((out, w, h))
}

fn captured(shot: On<ScreenshotCaptured>, mut check: ResMut<Check>) {
    check.pending = false;
    if let Some((px, w, h)) = rgba(&shot.image) {
        check.size = (w, h);
        if check.with.is_none() {
            check.with = Some(px);
        } else {
            check.without = Some(px);
        }
    }
}

fn save(name: &str, w: usize, h: usize, pixels: Vec<u8>) {
    let _ = std::fs::write(
        format!("logs/{name}.png"),
        rc_image::png::encode(&rc_image::Rgba {
            width: w,
            height: h,
            pixels,
        }),
    );
}

/// The picture test (only with --check).
#[allow(clippy::too_many_arguments)]
fn picture_check(
    mut commands: Commands,
    opts: Res<Options>,
    fight: Option<Res<Fight>>,
    mut check: ResMut<Check>,
    mut droids: Query<&mut Visibility, With<Droid>>,
    camera: Query<&Transform, With<PlayerCamera>>,
    mut next_play: ResMut<NextState<Play>>,
    looks_ready: Query<(), (With<Droid>, With<Mesh3d>)>,
    looks: Res<Looks>,
    log: Res<GameLog>,
) {
    if !opts.check || check.done {
        return;
    }
    let Some(fight) = fight else { return };
    check.frames += 1;
    let verdict = |check: &mut Check, ok: bool, what: String| {
        log.line(format!(
            "CHECK {}: {what}",
            if ok { "PASS" } else { "FAIL" }
        ));
        if !ok {
            check.failed += 1;
        }
    };
    match check.stage {
        // Wait for a fighting droid in front of the camera.
        0 => {
            let Some(spotted) = fight.stats.first_spotted else {
                return;
            };
            let Ok(cam) = camera.single() else { return };
            let fwd = cam.forward();
            let in_view = fight
                .battle
                .alive()
                .filter(|f| f.mode == Mode::Attack)
                .any(|f| {
                    let to = to_bevy(f.pawn.location) - cam.translation;
                    to.length() < 4000.0 / 50.0 && to.normalize().dot(*fwd) > 0.9
                });
            if fight.battle.time > spotted + 1.0 && in_view {
                verdict(
                    &mut check,
                    !looks_ready.is_empty(),
                    format!("droids have meshes ({} drawn)", looks_ready.iter().count()),
                );
                next_play.set(Play::Paused);
                check.stage = 1;
                check.frames = 0;
            }
        }
        // Paused: capture with droids, hide them, capture without.
        1 | 3 => {
            if check.frames == 4 && !check.pending {
                check.pending = true;
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(captured);
            }
            let have = if check.stage == 1 {
                check.with.is_some()
            } else {
                check.without.is_some()
            };
            if have {
                if check.stage == 1 {
                    for mut v in &mut droids {
                        *v = Visibility::Hidden;
                    }
                }
                check.stage += 2;
                check.frames = 0;
            } else if check.frames > 300 {
                verdict(&mut check, false, "screenshot captured".into());
                check.done = true;
            }
        }
        // Compare.
        5 => {
            for mut v in &mut droids {
                *v = Visibility::Inherited;
            }
            next_play.set(Play::Running);
            let (w, h) = check.size;
            let (Some(a), Some(b)) = (check.with.take(), check.without.take()) else {
                return;
            };
            let mut mask = vec![0u8; w * h * 4];
            // `n`: pixels the droids changed. `solid`: pixels they changed a
            // lot (their bodies, not edges blended with the background),
            // used for the colour.
            let (mut n, mut solid, mut sum, mut magenta) = (0u32, 0u32, [0f64; 3], 0u32);
            for i in 0..w * h {
                let (pa, pb) = (&a[i * 4..i * 4 + 3], &b[i * 4..i * 4 + 3]);
                let diff: i32 = (0..3).map(|k| (pa[k] as i32 - pb[k] as i32).abs()).sum();
                if diff > 24 {
                    n += 1;
                    if pa[0] > 150 && pa[2] > 150 && pa[1] < 80 {
                        magenta += 1;
                    }
                    mask[i * 4..i * 4 + 4].copy_from_slice(&[pa[0], pa[1], pa[2], 255]);
                }
                if diff > 60 {
                    solid += 1;
                    for k in 0..3 {
                        sum[k] += pa[k] as f64;
                    }
                }
            }
            let mean = sum.map(|s| s / solid.max(1) as f64);
            let skin = looks
                .by_type
                .get("battle_droid")
                .and_then(|l| l.look.skin.as_ref())
                .map(|(_, img)| img.mean());
            log.line(format!(
                "picture: {n} pixels changed when the droids were hidden ({:.2}% of the window), {solid} of them strongly; their average colour R {:.0} G {:.0} B {:.0}; {magenta} magenta; skin texture average {:?}",
                n as f64 * 100.0 / (w * h) as f64,
                mean[0],
                mean[1],
                mean[2],
                skin
            ));
            save("droidview_with", w, h, a);
            save("droidview_without", w, h, b);
            save("droidview_droids", w, h, mask);
            verdict(
                &mut check,
                n >= 300,
                format!("droids are drawn ({n} pixels)"),
            );
            verdict(
                &mut check,
                (magenta as f32) < 0.05 * n as f32,
                format!("droids are not magenta ({magenta} of {n})"),
            );
            // Brightness: ambient 0.5 alone gives about 0.73 of the skin
            // picture's brightness on screen; the level's lights add to
            // that. Between 0.4 and 2 times the picture means neither black
            // nor washed out.
            let bright = (mean[0] + mean[1] + mean[2]) / 3.0;
            let skin_bright = skin.map_or(80.0, |[r, g, b, _]| (r + g + b) as f64 / 3.0);
            verdict(
                &mut check,
                bright > 0.4 * skin_bright && bright < 2.0 * skin_bright,
                format!("droids are lit, neither black nor washed out (brightness {bright:.0}, skin picture {skin_bright:.0})"),
            );
            // Same tint as the skin picture: its red-to-blue balance, within
            // a quarter either way (light on the level can tint it a little).
            if let Some([r, _, b, _]) = skin {
                let want = r as f64 / (b as f64).max(1.0);
                let got = mean[0] / mean[2].max(1.0);
                verdict(
                    &mut check,
                    (got / want - 1.0).abs() < 0.25,
                    format!("droids have the skin's tint (red/blue {got:.2}, skin {want:.2})"),
                );
            }
            check.done = true;
        }
        _ => {}
    }
}

/// After the time limit: the numbers.
fn finish(
    opts: Res<Options>,
    fight: Option<Res<Fight>>,
    check: Res<Check>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
    log: Res<GameLog>,
) {
    let Some(fight) = fight else { return };
    if *done || fight.battle.time < opts.seconds {
        return;
    }
    *done = true;
    let s = &fight.stats;
    let mut failed = check.failed;
    let mut verdict = |ok: bool, what: String| {
        log.line(format!(
            "CHECK {}: {what}",
            if ok { "PASS" } else { "FAIL" }
        ));
        if !ok {
            failed += 1;
        }
    };
    log.line(format!("results: {s:?}"));
    verdict(s.spawned > 0, format!("droids spawned ({})", s.spawned));
    verdict(s.first_spotted.is_some(), "a droid saw the stand-in".into());
    verdict(s.shots > 0, format!("droids fired ({} shots)", s.shots));
    verdict(
        s.hits_on_player > 0 && s.hits_on_player < s.shots,
        format!(
            "some droid shots hit, not all ({} of {})",
            s.hits_on_player, s.shots
        ),
    );
    verdict(
        s.bolts_on_droids * 20 <= s.shots,
        format!(
            "droids rarely shoot each other ({} of {} bolts hit a droid)",
            s.bolts_on_droids, s.shots
        ),
    );
    verdict(
        s.droids_hurt > 0,
        format!(
            "the stand-in's shots hurt droids ({} hits, {} destroyed)",
            s.droids_hurt, s.droids_destroyed
        ),
    );
    if opts.check {
        verdict(check.done, "picture check ran".into());
    }
    log.line(format!("DONE: {failed} checks failed"));
    exit.write(if failed == 0 {
        AppExit::Success
    } else {
        AppExit::from_code(1)
    });
}
