//! rc_squad_sim: a test run of Delta Squad on a real map, with no window.
//!
//! Usage: cargo run -p rc_squad_plugin --example rc_squad_sim [MAP]
//! (default geo_01a; a name from GameData/Maps or a path to a .ctm file).
//! The game folder is RC_GAME_DIR or the default Steam path.
//!
//! It runs the real `SquadPlugin` and `rc_combat::CombatPlugin` in a Bevy
//! app without a window, with a scripted player (the same `rc_movement`
//! body as the walker) who walks, aims and presses the squad order buttons.
//! For the fight it puts stand-in battle droids in sight of the squad:
//! real combat targets (health, team, body size of the game's droid) that
//! stand still and don't shoot back, so the squad's shooting, hits and
//! kills can be counted on their own. Then it knocks squad mates and the
//! player down and checks who revives whom, and how long it takes. After each part it checks the
//! squad's numbers, logs everything to `logs/rc_squad_sim.log`, and ends
//! with `SIM DONE: N checks passed, M failed` (exit code 1 if any failed).
//! Time runs in fixed 1/60 s steps, so every run gives the same numbers.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{Damage, DamageKind, Dead, Health, Hittable, Team};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    Action, ActionState, CorePlugin, GameLog, GamePaths, GameSet, GameState, LevelCollision,
    Player, PlayerCamera, SquadReadout, SquadStatus,
};
use rc_movement::{MoveInput, MoveParams, Pawn, TriangleWorld, Vec3 as UVec3};
use rc_squad::{MarkerState, MemberState};
use rc_squad_plugin::{
    yaw_rotation, SquadFightStats, SquadMapData, SquadMate, SquadPlugin, SquadSet, SquadState,
};
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
    /// Take all of this squad mate's health (as a hit would).
    Hurt(usize),
    /// Knock a squad mate down with a real hit through the combat part.
    KnockDown(usize),
    /// Knock the player down.
    KnockDownPlayer,
    /// Put this many stand-in droids in sight of the player.
    SpawnDroids(usize),
    /// Stand next to this (downed) squad mate.
    GoTo(usize),
    /// Hold the Use button for this long (seconds), standing still.
    HoldUse(f32),
    /// Set how far squad mates go to revive someone (uu); 0 = they don't.
    ReviveRange(f32),
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
    /// Where the hurt squad mate was when hurt.
    hurt_at: Option<(usize, UVec3)>,
    /// The squad panel: health fraction and whether shown as down.
    readout: Vec<(f32, bool)>,
    /// The squad's fight numbers.
    fight: SquadFightStats,
    /// Stand-in droids spawned, and how many are still standing.
    droids: Vec<Entity>,
    droids_alive: usize,
    /// The player: down, and health.
    player_down: bool,
    player_health: f32,
    /// Which squad mates are down.
    down: Vec<bool>,
    /// Seconds Use is still held.
    use_left: f32,
    world: Arc<TriangleWorld>,
}

impl Sim {
    fn total(v: &[u32]) -> f32 {
        v.iter().sum::<u32>() as f32
    }
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
        // Sev goes down: the nearest squad mate goes and revives him.
        Hurt(0),
        Hold(0.5, idle),
        Check(
            "down: Sev shown down on the squad panel",
            |s| yes(s.readout.first().is_some_and(|r| r.1 && r.0 == 0.0)),
            1.0,
            1.0,
        ),
        Check(
            "down: the others shown up",
            |s| yes(s.readout.iter().skip(1).all(|r| !r.1 && r.0 == 1.0)),
            1.0,
            1.0,
        ),
        Hold(8.0, idle),
        Check(
            "revive: Sev back up within 8.5 s",
            |s| yes(!s.down[0]),
            1.0,
            1.0,
        ),
        Check(
            "revive: Sev back with half health on the panel",
            |s| s.readout[0].0,
            0.49,
            0.51,
        ),
        Check(
            "revive: done by a squad mate",
            |s| Sim::total(&s.fight.revives),
            1.0,
            1.0,
        ),
        // The fight: droids in sight. Search and destroy so the squad
        // spreads out ahead.
        Press(Action::SquadFormUp),
        Hold(3.0, idle),
        SpawnDroids(3),
        Hold(20.0, idle),
        Check(
            "fight: squad shots fired",
            |s| Sim::total(&s.fight.shots),
            20.0,
            1e9,
        ),
        Check(
            "fight: squad shots that hit a droid",
            |s| Sim::total(&s.fight.hits),
            5.0,
            1e9,
        ),
        Check(
            "fight: droids killed by the squad",
            |s| Sim::total(&s.fight.kills),
            1.0,
            3.0,
        ),
        Check(
            "fight: every droid down after 20 s",
            |s| s.droids_alive as f32,
            0.0,
            0.0,
        ),
        Check(
            "fight: the squad never hit the player or each other (player health)",
            |s| s.player_health,
            200.0,
            200.0,
        ),
        // The player goes down: a squad mate comes to revive them.
        KnockDownPlayer,
        Hold(0.5, idle),
        Check("player down", |s| yes(s.player_down), 1.0, 1.0),
        Hold(8.0, idle),
        Check(
            "player revived within 8.5 s",
            |s| yes(!s.player_down),
            1.0,
            1.0,
        ),
        Check(
            "player back with half health",
            |s| s.player_health,
            99.0,
            101.0,
        ),
        // Scorch goes down and the player revives him by holding Use. The
        // others are told not to, or the nearest would get there first.
        ReviveRange(0.0),
        KnockDown(2),
        Hold(0.2, idle),
        GoTo(2),
        HoldUse(3.3),
        Hold(0.5, idle),
        Check(
            "player revive: Scorch back up",
            |s| yes(!s.down[2]),
            1.0,
            1.0,
        ),
        Check(
            "player revive: counted for the player",
            |s| s.fight.player_revives as f32,
            1.0,
            1.0,
        ),
        ReviveRange(4000.0),
        Hold(3.0, idle),
        Check(
            "nobody fell out of the map (lowest member feet z)",
            |s| s.lowest_member_z,
            -1e9,
            1e9,
        ),
    ]
}

/// Up to `n` spots a droid can stand in sight of the player, 600 to 1400
/// uu ahead, spread across the view.
fn droid_spots(world: &TriangleWorld, pawn: &Pawn, yaw: f32, n: usize) -> Vec<UVec3> {
    use rc_movement::World;
    let eye = pawn.eye();
    let ext = UVec3::new(35.0, 35.0, 80.0);
    let mut out: Vec<UVec3> = Vec::new();
    for dist in [1000.0f32, 800.0, 1300.0, 600.0] {
        for deg in [-25.0f32, 0.0, 25.0, -45.0, 45.0, -12.0, 12.0] {
            if out.len() >= n {
                return out;
            }
            let a = yaw + deg.to_radians();
            let want = pawn.feet() + UVec3::new(a.cos() * dist, a.sin() * dist, 0.0);
            // The floor under it (from a bit above eye height).
            let top = UVec3::new(want.x, want.y, eye.z + 100.0);
            let bottom = top - UVec3::new(0.0, 0.0, 1200.0);
            let Some(h) = world.sweep(ext, top, bottom) else {
                continue;
            };
            if h.start_solid {
                continue;
            }
            let centre = top + (bottom - top) * h.time;
            let feet = centre - UVec3::new(0.0, 0.0, 80.0);
            let droid_eye = centre + UVec3::new(0.0, 0.0, 60.0);
            if world.sweep(UVec3::ZERO, eye, droid_eye).is_some() {
                continue;
            }
            if out.iter().any(|o| (*o - feet).length() < 200.0) {
                continue;
            }
            out.push(feet);
        }
    }
    out
}

fn view_rotation(yaw: f32, pitch: f32) -> Quat {
    yaw_rotation(yaw) * Quat::from_rotation_x(pitch)
}

/// The scripted player: run the steps, move the body, place the camera.
#[allow(clippy::too_many_arguments)]
fn drive_player(
    mut sim: ResMut<Sim>,
    mut actions: ResMut<ActionState>,
    mut player: Query<&mut Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
    mut squad: ResMut<SquadState>,
    readout: Res<SquadReadout>,
    mut mates: Query<(Entity, &SquadMate, &mut Health, Has<Dead>), Without<Player>>,
    players: Query<(Entity, Option<&Health>, Has<Dead>), With<Player>>,
    droids: Query<
        (),
        (
            With<Team>,
            Without<Dead>,
            Without<SquadMate>,
            Without<Player>,
        ),
    >,
    stats: Res<SquadFightStats>,
    mut damage: MessageWriter<Damage>,
    mut commands: Commands,
    log: Res<GameLog>,
    mut exit: MessageWriter<AppExit>,
) {
    let s = &mut *sim;
    s.fight = stats.clone();
    s.droids_alive = s.droids.iter().filter(|e| droids.contains(**e)).count();
    let player_entity = players.single().ok().map(|p| p.0);
    if let Ok((_, h, dead)) = players.single() {
        s.player_down = dead;
        s.player_health = h.map_or(0.0, |h| h.current);
    }
    let mut down = vec![false; 3];
    for (_, m, _, dead) in &mates {
        if let Some(d) = down.get_mut(m.index) {
            *d = dead;
        }
    }
    s.down = down;
    s.readout = readout
        .members
        .iter()
        .map(|m| (m.health, m.status == SquadStatus::Incapacitated))
        .collect();
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
    if s.use_left > 0.0 {
        s.use_left -= DT;
        pressed.insert(Action::Use);
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
            Step::KnockDown(i) => {
                for (e, mate, _, _) in &mates {
                    if mate.index == i {
                        damage.write(Damage {
                            target: e,
                            amount: 10_000.0,
                            kind: DamageKind::Other,
                            instigator: None,
                            point: Vec3::ZERO,
                            direction: Vec3::ZERO,
                        });
                        log.line(format!("sim: {} knocked down", mate.name));
                    }
                }
            }
            Step::KnockDownPlayer => {
                if let Some(e) = player_entity {
                    damage.write(Damage {
                        target: e,
                        amount: 10_000.0,
                        kind: DamageKind::Other,
                        instigator: None,
                        point: Vec3::ZERO,
                        direction: Vec3::ZERO,
                    });
                    log.line("sim: the player is knocked down");
                }
            }
            Step::SpawnDroids(n) => {
                let spots = droid_spots(&s.world, &s.pawn, s.yaw, n);
                for (k, feet) in spots.iter().enumerate() {
                    let e = commands
                        .spawn((
                            Name::new(format!("stand-in droid {k}")),
                            Health::new(100.0),
                            Team::Separatist,
                            Hittable {
                                radius: 35.0,
                                half_height: 80.0,
                                center_up: 80.0,
                            },
                            Transform::from_translation(to_bevy(*feet)),
                        ))
                        .id();
                    s.droids.push(e);
                    log.line(format!(
                        "sim: stand-in droid {k} at ({:.0}, {:.0}, {:.0}), {:.0} uu from the player",
                        feet.x,
                        feet.y,
                        feet.z,
                        (*feet - s.pawn.feet()).length()
                    ));
                }
                if spots.len() < n {
                    log.line(format!("sim: only {} droid spots in sight", spots.len()));
                }
            }
            Step::GoTo(i) => {
                if let Some(m) = s.members.get(i) {
                    let feet = m.1;
                    let to = s.pawn.feet() - feet;
                    let side = if to.length_2d() > 1.0 {
                        to.flat().normalize_or_zero()
                    } else {
                        UVec3::new(1.0, 0.0, 0.0)
                    };
                    let stand = feet + side * 70.0;
                    s.pawn = Pawn::new(s.pawn.params.clone(), stand + UVec3::new(0.0, 0.0, 10.0));
                    log.line(format!(
                        "sim: the player stands beside {} at ({:.0}, {:.0}, {:.0})",
                        m.0, stand.x, stand.y, stand.z
                    ));
                }
            }
            Step::ReviveRange(r) => {
                if let Some(sq) = squad.squad.as_mut() {
                    sq.settings.revive.range = r;
                }
                log.line(format!("sim: squad mates revive within {r:.0} uu"));
            }
            Step::HoldUse(secs) => {
                s.use_left = secs;
                s.hold_left = secs;
                s.input = MoveInput::default();
                log.line(format!("sim: hold Use for {secs:.1} s"));
            }
            Step::Hurt(i) => {
                for (_, mate, mut h, _) in &mut mates {
                    if mate.index == i {
                        h.current = 0.0;
                        log.line(format!("sim: {} loses all health", mate.name));
                    }
                }
                if let Some(m) = s.members.get(i) {
                    s.hurt_at = Some((i, m.1));
                }
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
        rc_combat::CombatPlugin,
        SquadPlugin,
    ))
    .insert_resource(GamePaths::new(game.clone(), vec![PathBuf::from("data")]))
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
        hurt_at: None,
        readout: vec![],
        fight: SquadFightStats::default(),
        droids: vec![],
        droids_alive: 0,
        player_down: false,
        player_health: 0.0,
        down: vec![false; 3],
        use_left: 0.0,
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
