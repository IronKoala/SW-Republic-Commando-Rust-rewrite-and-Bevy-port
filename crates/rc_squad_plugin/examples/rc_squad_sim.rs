//! rc_squad_sim: a test run of Delta Squad on a real map, with no window.
//!
//! Usage: cargo run -p rc_squad_plugin --example rc_squad_sim [MAP]
//! (default geo_01a; a name from GameData/Maps or a path to a .ctm file).
//! The game folder is RC_GAME_DIR or the default Steam path.
//!
//! It runs the real `SquadPlugin` in a Bevy app without a window, with a
//! scripted player (the same `rc_movement` body as the walker) who walks,
//! aims and presses the squad order buttons. After each part it checks the
//! squad's numbers, logs everything to `logs/rc_squad_sim.log`, and ends
//! with `SIM DONE: N checks passed, M failed` (exit code 1 if any failed).
//! Time runs in fixed 1/60 s steps, so every run gives the same numbers.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_game_core::units::to_bevy;
use rc_game_core::{
    Action, ActionState, CorePlugin, GameLog, GameSet, GameState, LevelCollision, Player,
    PlayerCamera,
};
use rc_movement::{MoveInput, MoveParams, Pawn, TriangleWorld, Vec3 as UVec3};
use rc_squad::{MarkerState, MemberState};
use rc_squad_plugin::{yaw_rotation, SquadMapData, SquadPlugin, SquadSet, SquadState};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DT: f32 = 1.0 / 60.0;

/// What the scripted player does next.
#[derive(Clone, Copy)]
enum Step {
    /// Hold these keys for this long.
    Hold(f32, MoveInput),
    /// Turn by degrees (positive turns right).
    Turn(f32),
    /// Look up (+) or down (-) to this many degrees.
    Pitch(f32),
    /// Press a button for one frame.
    Press(Action),
    /// Stand near the squad marker nearest the player, then aim at it and
    /// press.
    AimAtNearestMarker,
    /// Aim at the marker picked by `AimAtMarker` from where the player is.
    FaceMarker,
    /// A check on the squad: name, measurement, lowest and highest allowed.
    Check(&'static str, fn(&Sim) -> f32, f32, f32),
}

/// The sim's own state: the scripted player and the run's numbers.
#[derive(Resource)]
struct Sim {
    pawn: Pawn,
    yaw: f32,
    pitch: f32,
    steps: Vec<Step>,
    next: usize,
    hold_left: f32,
    input: MoveInput,
    press: Option<Action>,
    passed: u32,
    failed: u32,
    lowest_member_z: f32,
    /// Copies of the squad's numbers, refreshed every frame for the checks.
    members: Vec<(String, UVec3, MemberState, f32)>,
    stance: String,
    secure_spot: Option<UVec3>,
    markers: Vec<(String, UVec3, UVec3, MarkerState)>,
    marker_target: Option<usize>,
    world: Arc<TriangleWorld>,
}

impl Sim {
    fn max_leader_distance(&self) -> f32 {
        self.members.iter().map(|m| m.3).fold(0.0, f32::max)
    }
    fn max_distance_to(&self, p: UVec3) -> f32 {
        self.members
            .iter()
            .map(|m| (m.1 - p).length_2d())
            .fold(0.0, f32::max)
    }
}

fn yes(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn keys(forward: f32) -> MoveInput {
    MoveInput {
        forward,
        ..Default::default()
    }
}

fn steps() -> Vec<Step> {
    use Step::*;
    let idle = MoveInput::default();
    vec![
        // The PlayerStart is up in the (not yet solid) gunship: drop.
        Hold(4.0, idle),
        Check("all three spawned", |s| s.members.len() as f32, 3.0, 3.0),
        Check(
            "everyone on the ground near the leader (uu)",
            |s| s.max_leader_distance(),
            0.0,
            400.0,
        ),
        Press(Action::SquadFormUp),
        Hold(5.0, idle),
        Check("form up: stance", |s| yes(s.stance == "Form up"), 1.0, 1.0),
        Check(
            "form up: furthest member from the leader (uu)",
            |s| s.max_leader_distance(),
            0.0,
            260.0,
        ),
        Check(
            "form up: everyone standing",
            |s| yes(s.members.iter().all(|m| m.2 == MemberState::Idle)),
            1.0,
            1.0,
        ),
        // Run forward, then wait: they follow once 768 uu behind.
        Hold(3.0, keys(1.0)),
        Hold(10.0, idle),
        Check(
            "follow: furthest member from the leader after the run (uu)",
            |s| s.max_leader_distance(),
            0.0,
            300.0,
        ),
        Check(
            "follow: everyone standing",
            |s| yes(s.members.iter().all(|m| m.2 == MemberState::Idle)),
            1.0,
            1.0,
        ),
        // Aim at the floor ahead and order secure position (F3).
        Pitch(-25.0),
        Press(Action::SquadSecurePosition),
        Pitch(0.0),
        Hold(8.0, idle),
        Check(
            "secure: stance",
            |s| yes(s.stance == "Secure area"),
            1.0,
            1.0,
        ),
        Check(
            "secure: furthest member from the spot (uu)",
            |s| s.secure_spot.map_or(9e9, |p| s.max_distance_to(p)),
            0.0,
            200.0,
        ),
        // Walk away; they hold the spot.
        Turn(180.0),
        Hold(2.5, keys(1.0)),
        Hold(3.0, idle),
        Check(
            "secure: still at the spot after the leader left (uu)",
            |s| s.secure_spot.map_or(9e9, |p| s.max_distance_to(p)),
            0.0,
            200.0,
        ),
        Check(
            "secure: leader really left (uu)",
            |s| {
                s.secure_spot
                    .map_or(0.0, |p| (s.pawn.feet() - p).length_2d())
            },
            800.0,
            1e9,
        ),
        // Cancel (F4): back to the leader.
        Press(Action::SquadCancel),
        Hold(12.0, idle),
        Check(
            "cancel: stance back to form up",
            |s| yes(s.stance == "Form up"),
            1.0,
            1.0,
        ),
        Check(
            "cancel: furthest member from the leader (uu)",
            |s| s.max_leader_distance(),
            0.0,
            260.0,
        ),
        // Search and destroy (F1): spread out ahead.
        Press(Action::SquadSearchAndDestroy),
        Hold(5.0, idle),
        Check(
            "search and destroy: stance",
            |s| yes(s.stance == "Search and destroy"),
            1.0,
            1.0,
        ),
        Check(
            "search and destroy: furthest member from the leader (uu)",
            |s| s.max_leader_distance(),
            0.0,
            500.0,
        ),
        Press(Action::SquadFormUp),
        Hold(4.0, idle),
        // The nearest marked job: go there, aim at it and order it. The
        // squad is left behind and has to cross the map to get there.
        AimAtNearestMarker,
        Hold(45.0, idle),
        Check(
            "marker: job done",
            |s| {
                s.marker_target
                    .map_or(0.0, |k| yes(s.markers[k].3 == MarkerState::Done))
            },
            1.0,
            1.0,
        ),
        Hold(15.0, idle),
        Check(
            "after the marker: everyone back near the leader (uu)",
            |s| s.max_leader_distance(),
            0.0,
            400.0,
        ),
        Check(
            "nobody fell out of the map (lowest member feet z)",
            |s| s.lowest_member_z,
            -1e9,
            1e9,
        ),
    ]
}

fn view_rotation(yaw: f32, pitch: f32) -> Quat {
    yaw_rotation(yaw) * Quat::from_rotation_x(pitch)
}

/// The scripted player: run the steps, move the body, place the camera.
fn drive_player(
    mut sim: ResMut<Sim>,
    mut actions: ResMut<ActionState>,
    mut player: Query<&mut Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
    squad: Res<SquadState>,
    log: Res<GameLog>,
    mut exit: MessageWriter<AppExit>,
) {
    let s = &mut *sim;
    // Copy the squad's numbers for the checks.
    if let Some(sq) = squad.squad.as_ref() {
        let leader = rc_squad::Leader {
            feet: s.pawn.feet(),
            yaw: s.yaw,
        };
        s.members = sq
            .status(leader)
            .into_iter()
            .map(|m| (m.name, m.feet, m.state, m.leader_distance))
            .collect();
        for m in &s.members {
            s.lowest_member_z = s.lowest_member_z.min(m.1.z);
        }
        s.stance = sq.stance_settings().name.clone();
        s.secure_spot = sq.objective;
        s.markers = sq
            .markers
            .iter()
            .map(|(m, st)| (m.name.clone(), m.stand, m.aim, *st))
            .collect();
    }
    let mut pressed = HashSet::new();
    if let Some(a) = s.press.take() {
        pressed.insert(a);
    }
    while s.hold_left <= 0.0 {
        let Some(&step) = s.steps.get(s.next) else {
            log.line(format!(
                "SIM DONE: {} checks passed, {} failed",
                s.passed, s.failed
            ));
            exit.write(if s.failed == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        s.next += 1;
        match step {
            Step::Hold(secs, input) => {
                s.hold_left = secs;
                s.input = input;
                log.line(format!("sim: hold {secs:.1} s, forward {}", input.forward));
            }
            Step::Turn(d) => {
                s.yaw = (s.yaw + d.to_radians()).rem_euclid(std::f32::consts::TAU);
                log.line(format!(
                    "sim: turn {d:.0}, now facing {:.0} deg",
                    s.yaw.to_degrees()
                ));
            }
            Step::Pitch(d) => s.pitch = d.to_radians(),
            Step::Press(a) => {
                // Pressed next frame, after the camera has moved.
                s.press = Some(a);
                log.line(format!("sim: press {}", a.name()));
                s.hold_left = DT;
            }
            Step::AimAtNearestMarker => {
                let here = s.pawn.feet();
                let nearest = (0..s.markers.len()).min_by(|&a, &b| {
                    (s.markers[a].1 - here)
                        .length()
                        .total_cmp(&(s.markers[b].1 - here).length())
                });
                let Some(k) = nearest else {
                    log.line("sim: no marker to aim at");
                    s.failed += 1;
                    continue;
                };
                let (name, stand, aim, _) = s.markers[k].clone();
                s.marker_target = Some(k);
                // Stand 500 uu from the marker's anchor, on the side of
                // its stand spot, and face it.
                let away = (stand - aim).flat().normalize_or_zero();
                let away = if away.length() < 0.5 {
                    UVec3::new(1.0, 0.0, 0.0)
                } else {
                    away
                };
                let feet = UVec3::new(aim.x, aim.y, stand.z - 40.0) + away * 500.0;
                s.pawn = Pawn::new(s.pawn.params.clone(), feet + UVec3::new(0.0, 0.0, 20.0));
                log.line(format!(
                    "sim: teleport to ({:.0}, {:.0}, {:.0}) near marker {name} at ({:.0}, {:.0}, {:.0})",
                    feet.x, feet.y, feet.z, aim.x, aim.y, aim.z
                ));
                // Let the body land, then aim and press.
                s.steps
                    .insert(s.next, Step::Hold(1.0, MoveInput::default()));
                s.steps.insert(s.next + 1, Step::FaceMarker);
                s.steps
                    .insert(s.next + 2, Step::Hold(DT, MoveInput::default()));
                s.steps
                    .insert(s.next + 3, Step::Press(Action::SquadCommand));
            }
            Step::FaceMarker => {
                let Some(k) = s.marker_target else { continue };
                let aim = s.markers[k].2;
                let d = aim - s.pawn.eye();
                s.yaw = d.y.atan2(d.x);
                s.pitch = d.z.atan2(d.length_2d());
                log.line(format!(
                    "sim: aim at marker {} from eye ({:.0}, {:.0}, {:.0}): yaw {:.1}, pitch {:.1} deg",
                    s.markers[k].0,
                    s.pawn.eye().x,
                    s.pawn.eye().y,
                    s.pawn.eye().z,
                    s.yaw.to_degrees(),
                    s.pitch.to_degrees()
                ));
            }
            Step::Check(name, f, lo, hi) => {
                let got = f(s);
                let ok = got >= lo && got <= hi;
                if ok {
                    s.passed += 1;
                } else {
                    s.failed += 1;
                }
                log.line(format!(
                    "CHECK {name}: got {got:.1}, want {lo:.1} to {hi:.1} -> {}",
                    if ok { "PASS" } else { "FAIL" }
                ));
            }
        }
    }
    s.hold_left -= DT;
    let input = MoveInput {
        yaw: s.yaw,
        ..s.input
    };
    let world = s.world.clone();
    s.pawn.step(DT, &input, &*world);
    actions.update(pressed, Vec2::ZERO, Vec2::ZERO);
    if let Ok(mut t) = player.single_mut() {
        t.translation = to_bevy(s.pawn.feet());
        t.rotation = yaw_rotation(s.yaw);
    }
    if let Ok(mut t) = camera.single_mut() {
        t.translation = to_bevy(s.pawn.eye());
        t.rotation = view_rotation(s.yaw, s.pitch);
    }
}

fn main() -> AppExit {
    let map = std::env::args().nth(1).unwrap_or_else(|| "geo_01a".into());
    let game =
        PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    let game_data = game.join("GameData");
    let path = if map.to_ascii_lowercase().ends_with(".ctm") {
        PathBuf::from(&map)
    } else {
        game_data.join("Maps").join(format!("{map}.ctm"))
    };
    let log = match GameLog::create(std::path::Path::new("logs/rc_squad_sim.log")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't create logs/rc_squad_sim.log: {e}");
            return AppExit::from_code(1);
        }
    };
    let params = MoveParams::player_commando();
    let col = match rc_level::map_collision::load(&path, &game_data, params.collision_height) {
        Ok(c) => c,
        Err(e) => {
            log.line(format!("ERROR loading {}: {e}", path.display()));
            return AppExit::from_code(1);
        }
    };
    let v = |p: [f32; 3]| UVec3::new(p[0], p[1], p[2]);
    let world = Arc::new(TriangleWorld::new(
        col.triangles
            .iter()
            .map(|t| [v(t[0]), v(t[1]), v(t[2])])
            .collect(),
    ));
    let (feet, yaw) = col
        .start
        .map(|(f, y)| (v(f), y))
        .unwrap_or((UVec3::ZERO, 0.0));
    let lowest = world.bounds().0.z;
    log.line(format!(
        "rc_squad_sim on {}: {} collision triangles, start feet ({:.0}, {:.0}, {:.0}) facing {:.0} deg, lowest z {:.0}",
        path.display(),
        world.triangles.len(),
        feet.x,
        feet.y,
        feet.z,
        yaw.to_degrees(),
        lowest
    ));
    let map_data = SquadMapData::load(&path);
    for n in &map_data.notes {
        log.line(format!("squad: {n}"));
    }

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        CorePlugin,
        SquadPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        DT,
    )))
    .insert_resource(log)
    .insert_resource(LevelCollision(world.clone()))
    .insert_resource(map_data)
    .insert_resource(Sim {
        pawn: Pawn::new(params, feet),
        yaw,
        pitch: 0.0,
        steps: steps(),
        next: 0,
        hold_left: 0.0,
        input: MoveInput::default(),
        press: None,
        passed: 0,
        failed: 0,
        lowest_member_z: f32::INFINITY,
        members: vec![],
        stance: String::new(),
        secure_spot: None,
        markers: vec![],
        marker_target: None,
        world,
    })
    .add_systems(
        Update,
        drive_player.in_set(GameSet::Simulate).before(SquadSet),
    );
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    app.world_mut().spawn((
        Player,
        Transform::from_translation(to_bevy(feet)).with_rotation(yaw_rotation(yaw)),
    ));
    app.world_mut().spawn((PlayerCamera, Transform::default()));
    app.run()
}
