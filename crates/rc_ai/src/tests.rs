use crate::*;
use rc_movement::{Brush, BrushWorld, Vec3};
use rc_nav::NavGraph;
use std::collections::HashSet;
use std::sync::Arc;

fn flat_floor(extra: Vec<Brush>) -> BrushWorld {
    let mut b = vec![Brush::cuboid(
        Vec3::new(-5000.0, -5000.0, -16.0),
        Vec3::new(5000.0, 5000.0, 0.0),
    )];
    b.extend(extra);
    BrushWorld { brushes: b }
}

fn target_at(x: f32, y: f32) -> Target {
    Target {
        centre: Vec3::new(x, y, 84.0),
        eye: Vec3::new(x, y, 144.0),
    }
}

/// Run a fighter for `seconds` against a still target; returns all events.
fn run(
    f: &mut Fighter,
    world: &BrushWorld,
    target: Option<Target>,
    seconds: f32,
) -> Vec<(f32, AiEvent)> {
    let nav = NavGraph::default();
    let taken = HashSet::new();
    let dt = 1.0 / 30.0;
    let mut out = Vec::new();
    let mut t = 0.0;
    while t < seconds {
        let view = View {
            world,
            nav: &nav,
            time: t,
            target,
            cover_taken: &taken,
            allies: &[],
        };
        for e in f.tick(dt, &view) {
            out.push((t, e));
        }
        t += dt;
    }
    out
}

fn droid(yaw: f32) -> Fighter {
    Fighter::new(
        1,
        Arc::new(AiParams::battle_droid()),
        Vec3::ZERO,
        yaw,
        StartState::StationaryAttack,
    )
}

#[test]
fn settings_file_matches_built_in_values() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/enemies/battle_droid.toml"
    ))
    .unwrap();
    let from_file = AiParams::battle_droid().with_overrides(&text).unwrap();
    assert_eq!(from_file, AiParams::battle_droid());
}

#[test]
fn sound_lines_by_damage_kind() {
    let p = AiParams::battle_droid()
        .with_overrides("[sound]\nhurt = [\"A\"]\nhurt_explosion = [\"B\"]\n")
        .unwrap();
    assert_eq!(p.sounds.for_damage("hurt", "explosion"), ["B".to_string()]);
    assert_eq!(p.sounds.for_damage("hurt", "blaster"), ["A".to_string()]);
    // The file's [sound] section replaced the built-in by-kind lines.
    assert_eq!(
        p.sounds.for_damage("death", "electrical"),
        ["BD1_DieSmallArms".to_string()]
    );
    let built_in = AiParams::battle_droid();
    assert_eq!(
        built_in.sounds.for_damage("death", "electrical"),
        ["BD1_DieElectricity".to_string()]
    );
    assert_eq!(
        built_in.sounds.for_damage("death", "sonic"),
        ["BD1_DieSmallArms".to_string()]
    );
    let e = AiParams::battle_droid()
        .with_overrides("[sound]\nhurt_lazer = [\"B\"]\n")
        .unwrap_err();
    assert!(e.contains("line 2") && e.contains("hurt_lazer"), "{e}");
}

#[test]
fn settings_errors_name_the_line() {
    let e = AiParams::battle_droid()
        .with_overrides("[combat]\naccuracy = 0.5\nacuracy = 1\n")
        .unwrap_err();
    assert!(e.contains("line 3") && e.contains("acuracy"), "{e}");
    let e = AiParams::battle_droid()
        .with_overrides("[movement]\n\nground_sped = 1\n")
        .unwrap_err();
    assert!(e.contains("line 3"), "{e}");
    let p = AiParams::battle_droid()
        .with_overrides("[combat]\naccuracy = 0.9 # sharper\nweapon = \"x#y\"\n")
        .unwrap();
    assert_eq!(p.combat.accuracy, 0.9);
    assert_eq!(p.combat.weapon, "x#y");
}

#[test]
fn spots_reacts_then_fires_in_bursts() {
    let world = flat_floor(vec![]);
    let mut f = droid(0.0);
    let ev = run(&mut f, &world, Some(target_at(2000.0, 0.0)), 10.0);
    let spotted = ev
        .iter()
        .find(|(_, e)| matches!(e, AiEvent::Spotted { .. }))
        .map(|(t, _)| *t);
    let first_shot = ev
        .iter()
        .find(|(_, e)| matches!(e, AiEvent::Fire { .. }))
        .map(|(t, _)| *t);
    assert_eq!(spotted, Some(0.0));
    let first_shot = first_shot.expect("no shot");
    // Reaction time 0.5 s, decided at the next frame.
    assert!(
        first_shot >= 0.5 && first_shot < 0.7,
        "first shot at {first_shot}"
    );
    let shots: Vec<f32> = ev
        .iter()
        .filter(|(_, e)| matches!(e, AiEvent::Fire { .. }))
        .map(|(t, _)| *t)
        .collect();
    // Bursts of 3, 0.1 s apart, 1-2 s rest: 9.5 s of fighting gives
    // between 3 x 9.5/2.2 and 3 x 9.5/1.2 shots.
    assert!(
        shots.len() >= 12 && shots.len() <= 24,
        "{} shots",
        shots.len()
    );
    assert_eq!(f.shots_fired as usize, shots.len());
    // Aim error never over (1 - 0.4) x 10 = 6 degrees, and every shot
    // heads roughly at the target.
    for (_, e) in &ev {
        if let AiEvent::Fire {
            direction,
            aim_error,
            ..
        } = e
        {
            assert!(*aim_error <= 6.0 + 1e-3);
            assert!(direction.x > 0.99);
        }
    }
}

#[test]
fn does_not_see_through_walls_or_behind() {
    // A wall between droid and target.
    let wall = Brush::cuboid(
        Vec3::new(900.0, -500.0, 0.0),
        Vec3::new(1000.0, 500.0, 400.0),
    );
    let world = flat_floor(vec![wall]);
    let mut f = droid(0.0);
    let ev = run(&mut f, &world, Some(target_at(2000.0, 0.0)), 3.0);
    assert!(ev
        .iter()
        .all(|(_, e)| !matches!(e, AiEvent::Spotted { .. } | AiEvent::Fire { .. })));
    assert_eq!(f.mode, Mode::Idle);

    // Behind it (facing away), beyond the 200 uu "notice anyway" radius.
    let world = flat_floor(vec![]);
    let mut f = droid(std::f32::consts::PI);
    let ev = run(&mut f, &world, Some(target_at(2000.0, 0.0)), 2.0);
    assert!(ev
        .iter()
        .all(|(_, e)| !matches!(e, AiEvent::Spotted { .. })));
    // ...but within 200 uu it notices.
    let mut f = droid(std::f32::consts::PI);
    let ev = run(&mut f, &world, Some(target_at(150.0, 0.0)), 1.0);
    assert!(ev.iter().any(|(_, e)| matches!(e, AiEvent::Spotted { .. })));
}

#[test]
fn turns_to_face_a_target_it_hears_about() {
    let world = flat_floor(vec![]);
    let mut f = droid(std::f32::consts::PI); // facing away
    let mut ev = Vec::new();
    f.hear(Vec3::new(2000.0, 0.0, 84.0), 0.0, &mut ev);
    assert_eq!(f.mode, Mode::Alerted);
    let ev = run(&mut f, &world, Some(target_at(2000.0, 0.0)), 3.0);
    // Alerted, it keeps track of the enemy without the view cone, but can
    // only shoot within 60 degrees of its facing: turning 120 degrees at
    // 110 degrees/s takes about 1.1 s.
    let first_shot = ev
        .iter()
        .find(|(_, e)| matches!(e, AiEvent::Fire { .. }))
        .map(|(t, _)| *t)
        .unwrap();
    assert!(
        first_shot > 1.0 && first_shot < 1.4,
        "first shot at {first_shot}"
    );
    // Too far to hear.
    let mut f = droid(0.0);
    let mut ev = Vec::new();
    f.hear(Vec3::new(3000.0, 0.0, 84.0), 0.0, &mut ev);
    assert_eq!(f.mode, Mode::Idle);
}

/// A droid shot at in the open walks to a free cover point that faces the
/// enemy, then crouches between bursts and stands to fire.
#[test]
fn takes_cover_when_shot_in_the_open() {
    use rc_nav::{reach, Link, NavNode, NodeKind};
    let world = flat_floor(vec![]);
    let node = |name: &str, x: f32, kind: NodeKind| NavNode {
        name: name.into(),
        class: "x".into(),
        tag: name.into(),
        location: Vec3::new(x, 0.0, 80.0),
        export: 0,
        kind,
    };
    let mut nav = NavGraph {
        nodes: vec![
            node("Here", 0.0, NodeKind::Path),
            node(
                "Cover",
                400.0,
                NodeKind::Cover {
                    strafe_left: 100.0,
                    strafe_right: 100.0,
                    distance_to_cover: 50.0,
                    facing: 0.0,
                },
            ),
            node(
                "BackwardsCover",
                -400.0,
                NodeKind::Cover {
                    strafe_left: 100.0,
                    strafe_right: 100.0,
                    distance_to_cover: 50.0,
                    facing: std::f32::consts::PI,
                },
            ),
        ],
        links: vec![Vec::new(); 3],
        ..Default::default()
    };
    for (a, b) in [(0, 1), (1, 0), (0, 2), (2, 0)] {
        nav.links[a].push(Link {
            to: b,
            distance: 400.0,
            flags: reach::WALK,
        });
    }
    let mut f = Fighter::new(
        1,
        Arc::new(AiParams::battle_droid()),
        Vec3::ZERO,
        0.0,
        StartState::StationaryAttack,
    );
    let taken = HashSet::new();
    let target = Some(target_at(2500.0, 0.0));
    let dt = 1.0 / 30.0;
    let mut t = 0.0;
    let mut events = Vec::new();
    let mut crouched_frames = 0;
    let mut shots_in_cover = 0;
    while t < 12.0 {
        if (t - 2.0f32).abs() < dt / 2.0 {
            f.on_damaged(Some(Vec3::new(2500.0, 0.0, 84.0)), t, &mut events);
        }
        let view = View {
            world: &world,
            nav: &nav,
            time: t,
            target,
            cover_taken: &taken,
            allies: &[],
        };
        for e in f.tick(dt, &view) {
            if matches!(e, AiEvent::Fire { .. }) && matches!(f.maneuver, Maneuver::InCover(_)) {
                shots_in_cover += 1;
            }
            events.push(e);
        }
        if matches!(f.maneuver, Maneuver::InCover(_)) && f.pawn.crouched {
            crouched_frames += 1;
        }
        t += dt;
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AiEvent::ManeuverChanged { to: "to cover", .. })),
        "{events:?}"
    );
    assert_eq!(
        f.maneuver,
        Maneuver::InCover(1),
        "picked the cover facing the enemy"
    );
    assert!(
        (f.pawn.location.x - 400.0).abs() < 40.0,
        "at {:?}",
        f.pawn.location
    );
    assert!(crouched_frames > 30, "crouched {crouched_frames} frames");
    assert!(shots_in_cover >= 3, "{shots_in_cover} shots from cover");
}

#[test]
fn forgets_after_memory_time() {
    let world = flat_floor(vec![]);
    let mut f = droid(0.0);
    run(&mut f, &world, Some(target_at(2000.0, 0.0)), 1.0);
    assert_eq!(f.mode, Mode::Attack);
    // Target gone: after 15 s it investigates, then (no paths here) idles.
    run(&mut f, &world, None, 1.0);
    assert_eq!(f.mode, Mode::Attack);
    let ev = run(&mut f, &world, None, 17.0);
    assert!(ev.iter().any(|(_, e)| matches!(
        e,
        AiEvent::ModeChanged {
            to: Mode::Investigate,
            ..
        }
    )));
}

#[test]
fn scatter_stays_in_the_cone() {
    let mut rng = brain::Rng::new(3);
    let dir = Vec3::new(0.0, 1.0, 0.0);
    let mut biggest = 0.0f32;
    for _ in 0..2000 {
        let (d, a) = brain::scatter(dir, 6.0, &mut rng);
        let real = d.dot(dir).clamp(-1.0, 1.0).acos().to_degrees();
        assert!((real - a).abs() < 0.01);
        assert!((d.length() - 1.0).abs() < 1e-4);
        biggest = biggest.max(real);
    }
    assert!(biggest > 5.5 && biggest <= 6.0001, "{biggest}");
}

#[test]
fn holds_fire_when_a_friend_is_in_the_way() {
    let world = flat_floor(vec![]);
    let nav = NavGraph::default();
    let taken = HashSet::new();
    let dt = 1.0 / 30.0;
    // A friend 500 uu ahead, right on the line to the target.
    let friend = Vec3::new(500.0, 0.0, 80.0);
    let mut f = droid(0.0);
    let mut shots = 0;
    let mut t = 0.0;
    while t < 3.0 {
        let allies = [f.pawn.location, friend];
        let view = View {
            world: &world,
            nav: &nav,
            time: t,
            target: Some(target_at(2000.0, 0.0)),
            cover_taken: &taken,
            allies: &allies,
        };
        shots += f
            .tick(dt, &view)
            .iter()
            .filter(|e| matches!(e, AiEvent::Fire { .. }))
            .count();
        t += dt;
    }
    assert_eq!(shots, 0, "fired through a friend");
    // The friend steps 200 uu aside: it fires.
    let friend = Vec3::new(500.0, 200.0, 80.0);
    while t < 6.0 {
        let allies = [friend];
        let view = View {
            world: &world,
            nav: &nav,
            time: t,
            target: Some(target_at(2000.0, 0.0)),
            cover_taken: &taken,
            allies: &allies,
        };
        shots += f
            .tick(dt, &view)
            .iter()
            .filter(|e| matches!(e, AiEvent::Fire { .. }))
            .count();
        t += dt;
    }
    assert!(shots >= 3, "{shots} shots once the way was clear");
}

#[test]
fn droids_on_the_same_spot_are_pushed_apart() {
    let world = flat_floor(vec![]);
    let params = Arc::new(AiParams::battle_droid());
    let mut b = Battle::new(params.clone(), NavGraph::default(), vec![]);
    for id in 1..=2 {
        b.fighters.push(Fighter::new(
            id,
            params.clone(),
            Vec3::new(0.0, 0.0, 0.0),
            0.0,
            StartState::StationaryAttack,
        ));
        b.made_by.push(0);
    }
    for _ in 0..30 {
        b.step(1.0 / 30.0, &world, None);
    }
    let gap = (b.fighters[0].pawn.location - b.fighters[1].pawn.location).length();
    assert!(gap >= 69.0, "still overlapping: {gap:.1} uu apart");
}

/// A battle with one stationary droid at the origin, facing +X.
fn one_droid_battle() -> Battle {
    let params = Arc::new(AiParams::battle_droid());
    let mut b = Battle::new(params.clone(), NavGraph::default(), vec![]);
    b.fighters.push(Fighter::new(
        1,
        params,
        Vec3::ZERO,
        0.0,
        StartState::StationaryAttack,
    ));
    b.made_by.push(0);
    b
}

fn fire_directions(events: &[BattleEvent]) -> Vec<Vec3> {
    events
        .iter()
        .filter_map(|e| match e {
            BattleEvent::Ai {
                event: AiEvent::Fire { direction, .. },
                ..
            } => Some(*direction),
            _ => None,
        })
        .collect()
}

#[test]
fn shoots_a_squad_mate_when_the_player_is_hidden() {
    // The player stands behind a wall; a squad mate is in the open.
    let wall = Brush::cuboid(
        Vec3::new(900.0, 1400.0, 0.0),
        Vec3::new(1100.0, 2600.0, 400.0),
    );
    let world = flat_floor(vec![wall]);
    let mut b = one_droid_battle();
    let player = target_at(2000.0, 2000.0);
    let mate = target_at(1500.0, -300.0);
    let mut dirs = Vec::new();
    for _ in 0..90 {
        dirs.extend(fire_directions(&b.step_with(
            1.0 / 30.0,
            &world,
            Some(player),
            &[mate],
        )));
    }
    assert!(!dirs.is_empty(), "never fired");
    let to_mate = (mate.centre - Vec3::new(0.0, 0.0, 60.0)).normalize_or_zero();
    for d in &dirs {
        assert!(d.dot(to_mate) > 0.95, "fired at {d:?}, not the mate");
    }
    assert_eq!(b.fighters[0].target_index, Some(1));
}

#[test]
fn prefers_the_nearest_visible_enemy_and_keeps_it() {
    let world = flat_floor(vec![]);
    let mut b = one_droid_battle();
    let player = target_at(3000.0, 0.0);
    let mate = target_at(1000.0, 200.0);
    for _ in 0..30 {
        b.step_with(1.0 / 30.0, &world, Some(player), &[mate]);
    }
    assert_eq!(b.fighters[0].target_index, Some(1), "nearest is the mate");
    // The player walks closer, but the droid keeps its target while it
    // can still see it.
    let near_player = target_at(800.0, 0.0);
    b.step_with(1.0 / 30.0, &world, Some(near_player), &[mate]);
    assert_eq!(b.fighters[0].target_index, Some(1));
}

#[test]
fn a_stunned_droid_does_nothing() {
    let world = flat_floor(vec![]);
    let mut b = one_droid_battle();
    b.stun(1, 2.0);
    let mut shots = 0;
    for _ in 0..55 {
        shots += fire_directions(&b.step(1.0 / 30.0, &world, Some(target_at(1500.0, 0.0)))).len();
    }
    assert_eq!(shots, 0, "fired while stunned");
    for _ in 0..60 {
        shots += fire_directions(&b.step(1.0 / 30.0, &world, Some(target_at(1500.0, 0.0)))).len();
    }
    assert!(shots > 0, "never fired after the stun wore off");
}

#[test]
fn every_enemy_file_reads_and_claims_its_classes() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/enemies");
    let mut types = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "toml") {
            let text = std::fs::read_to_string(&p).unwrap();
            let params = AiParams::battle_droid()
                .with_overrides(&text)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert_eq!(
                params.kind.id,
                p.file_stem().unwrap().to_string_lossy(),
                "[type] id should be the file's name"
            );
            types.push(params.kind);
        }
    }
    assert!(types.len() >= 4);
    let who = |class: &str| {
        let m: Vec<_> = types
            .iter()
            .filter(|t| t.matches(class))
            .map(|t| t.id.as_str())
            .collect();
        assert!(m.len() <= 1, "{class} claimed by {m:?}");
        m.first().map(|s| s.to_string())
    };
    assert_eq!(who("BattleDroidTM").as_deref(), Some("battle_droid"));
    assert_eq!(who("battledroid").as_deref(), Some("battle_droid"));
    assert_eq!(
        who("SuperBattleDroid").as_deref(),
        Some("super_battle_droid")
    );
    assert_eq!(who("CloneCommando"), None);
}

#[test]
fn spawners_make_their_own_enemy_type() {
    let world = flat_floor(vec![]);
    let base = Arc::new(AiParams::battle_droid());
    let mut big = AiParams::battle_droid();
    big.kind.id = "big".into();
    big.movement.collision_height = 120.0;
    let big = Arc::new(big);
    let factory = |name: &str, x: f32| Factory {
        name: name.into(),
        tag: name.into(),
        prototype: name.into(),
        location: Vec3::new(x, 0.0, 200.0),
        yaw: 0.0,
        spots: vec![],
        capacity: 1,
        max_alive: 1,
        generate_forever: false,
        spawn_mode: 2, // at the factory: it has no spawn points
        spawn_interval: 0.1,
        offspring: Offspring {
            start_state: StartState::StationaryAttack,
            ..Default::default()
        },
        start_state: StartState::StationaryAttack,
        triggers: vec![],
        ..Default::default()
    };
    let mut b = Battle::with_types(
        base.clone(),
        NavGraph::default(),
        vec![(factory("a", 500.0), base), (factory("b", -500.0), big)],
    );
    for _ in 0..20 {
        b.step(1.0 / 30.0, &world, Some(target_at(0.0, 2000.0)));
    }
    assert_eq!(b.fighters.len(), 2);
    let ids: Vec<_> = b
        .fighters
        .iter()
        .map(|f| f.params.kind.id.as_str())
        .collect();
    assert!(
        ids.contains(&"battle_droid") && ids.contains(&"big"),
        "{ids:?}"
    );
    // Placed with its own height: feet at 200 - 120 for the big one.
    let bigf = b
        .fighters
        .iter()
        .find(|f| f.params.kind.id == "big")
        .unwrap();
    assert!(bigf.pawn.feet().z < 81.0, "{:?}", bigf.pawn.feet());
}

fn factory(x: f32) -> Factory {
    Factory {
        name: format!("factory at {x}"),
        tag: String::new(),
        prototype: "BattleDroidTM".into(),
        location: Vec3::new(x, 0.0, 60.0),
        yaw: 0.0,
        spots: vec![],
        capacity: 2,
        max_alive: 2,
        generate_forever: false,
        spawn_mode: 2, // at the factory: it has no spawn points
        spawn_interval: 1.0,
        offspring: Offspring {
            start_state: StartState::StationaryAttack,
            ..Default::default()
        },
        start_state: StartState::StationaryAttack,
        triggers: vec![],
        ..Default::default()
    }
}

#[test]
fn a_battle_carries_on_from_a_snapshot() {
    let world = flat_floor(vec![]);
    let params = Arc::new(AiParams::battle_droid());
    let make = || {
        Battle::new(
            params.clone(),
            NavGraph::default(),
            vec![factory(0.0), factory(50_000.0)],
        )
    };
    let mut b = make();
    // The player stands by the first spawner (it switches on) until it has
    // made its droids, then one droid is destroyed.
    let player = Some(target_at(200.0, 0.0));
    for _ in 0..120 {
        b.step(1.0 / 30.0, &world, player);
    }
    assert_eq!(b.spawners[0].made, 2);
    let first = b.fighters[0].id;
    b.killed(first);
    let snap = b.snapshot();
    assert_eq!(snap.fighters.len(), 1, "the dead droid isn't saved");
    assert!(snap.spawners[0].active && !snap.spawners[1].active);

    let mut fresh = make();
    let events = fresh.restore(&snap).unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BattleEvent::Spawned { .. }));
    let again = fresh.snapshot();
    assert_eq!(again.spawners, snap.spawners);
    assert_eq!(again.time, snap.time);
    assert_eq!(again.fighters.len(), 1);
    let (a, z) = (&snap.fighters[0], &again.fighters[0]);
    assert_eq!((a.id, a.spawner), (z.id, z.spawner));
    assert!(
        (a.feet - z.feet).length() < 0.5,
        "{:?} vs {:?}",
        a.feet,
        z.feet
    );
    // A droid that was fighting is fighting again.
    if a.mode != Mode::Idle {
        assert_eq!(z.mode, Mode::Alerted);
    }
    // The spawner doesn't make the destroyed droid again.
    for _ in 0..120 {
        fresh.step(1.0 / 30.0, &world, player);
    }
    assert_eq!(fresh.spawners[0].made, 2);
    assert_eq!(fresh.alive().count(), 1);
    // A snapshot from another map is refused.
    let mut other = Battle::new(params.clone(), NavGraph::default(), vec![factory(0.0)]);
    assert!(other.restore(&snap).is_err());
}

// Spawners run by the level's scripts (see `spawns` for the rules).

fn scripted_battle(factories: Vec<Factory>) -> Battle {
    let params = Arc::new(AiParams::battle_droid());
    let mut b = Battle::new(params, NavGraph::default(), factories);
    b.scripted = true;
    b
}

fn tagged_factory(tag: &str, x: f32) -> Factory {
    Factory {
        name: format!("{tag} factory"),
        tag: tag.into(),
        prototype: "BattleDroidTM".into(),
        location: Vec3::new(x, 0.0, 60.0),
        capacity: 1,
        offspring: Offspring {
            tag: Some(format!("{tag}Droids")),
            event: Some(format!("{tag}DroidDead")),
            first_damaged_event: Some(format!("{tag}DroidHurt")),
            start_state: StartState::StationaryAttack,
            team: None,
        },
        ..Default::default()
    }
}

fn steps(b: &mut Battle, n: usize) -> Vec<BattleEvent> {
    let world = flat_floor(vec![]);
    let mut out = Vec::new();
    for _ in 0..n {
        // The player far away, so nothing fights.
        out.extend(b.step(1.0 / 30.0, &world, Some(target_at(-4500.0, -4500.0))));
    }
    out
}

fn names(events: &[BattleEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            BattleEvent::Event { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_class_defaults_are_the_originals() {
    let f = Factory::default();
    assert_eq!((f.capacity, f.max_alive, f.spawn_mode), (2, 1, 2));
    assert!(f.at_factory() && f.can_spawn());
}

#[test]
fn scripted_spawners_wait_for_their_event() {
    let mut b = scripted_battle(vec![tagged_factory("Bunker", 0.0)]);
    // Standing right next to it does nothing: only its event starts it.
    let world = flat_floor(vec![]);
    for _ in 0..60 {
        b.step(1.0 / 30.0, &world, Some(target_at(100.0, 0.0)));
    }
    assert_eq!(b.fighters.len(), 0);
    let on = b.trigger("bunker"); // event names ignore case
    assert!(
        matches!(
            &on[..],
            [BattleEvent::SpawnerOn {
                spawner: 0,
                by: SwitchedOn::Event(_)
            }]
        ),
        "{on:?}"
    );
    let ev = steps(&mut b, 2);
    assert_eq!(b.fighters.len(), 1, "made straight away: {ev:?}");
    // At the factory itself (the default), feet on the floor below it.
    let f = &b.fighters[0];
    assert!(
        (f.pawn.feet() - Vec3::new(0.0, 0.0, 60.0 - 50.0)).length_2d() < 1.0,
        "{:?}",
        f.pawn.feet()
    );
    assert_eq!(b.tagged("BunkerDroids"), vec![f.id]);
    assert_eq!(f.start_state, StartState::StationaryAttack);
}

#[test]
fn the_same_event_again_switches_a_spawner_off() {
    let mut f = tagged_factory("Wave", 0.0);
    f.generate_forever = true;
    let mut b = scripted_battle(vec![f]);
    b.trigger("Wave");
    steps(&mut b, 2);
    let off = b.trigger("Wave");
    assert!(
        matches!(&off[..], [BattleEvent::SpawnerOff { why: "event", .. }]),
        "{off:?}"
    );
    let id = b.fighters[0].id;
    b.killed(id);
    steps(&mut b, 90);
    assert_eq!(b.alive().count(), 0, "off: no replacement");
}

#[test]
fn droids_send_their_death_and_first_hurt_events() {
    let mut b = scripted_battle(vec![tagged_factory("Gunner", 0.0)]);
    b.trigger("Gunner");
    steps(&mut b, 2);
    let id = b.fighters[0].id;
    let hurt = names(&b.damaged(id, None));
    assert_eq!(hurt, vec!["GunnerDroidHurt".to_string()]);
    assert!(
        names(&b.damaged(id, None)).is_empty(),
        "only the first time"
    );
    assert_eq!(names(&b.killed(id)), vec!["GunnerDroidDead".to_string()]);
    assert!(names(&b.killed(id)).is_empty(), "only once");
}

#[test]
fn a_spent_spawner_sends_its_exhausted_event_once_all_are_gone() {
    let mut f = tagged_factory("Canyon", 0.0);
    f.capacity = 2;
    f.max_alive = 1;
    f.spawn_event = Some("OneMore".into());
    f.exhausted_event = Some("CanyonClear".into());
    let mut b = scripted_battle(vec![f]);
    b.trigger("Canyon");
    let mut ev = steps(&mut b, 2);
    assert_eq!(b.fighters.len(), 1);
    // The second waits for the first to die (one alive at once).
    ev.extend(steps(&mut b, 60));
    assert_eq!(b.fighters.len(), 1);
    let first = b.fighters[0].id;
    b.killed(first);
    ev.extend(steps(&mut b, 45));
    assert_eq!(b.fighters.len(), 2);
    assert!(
        !names(&ev).contains(&"CanyonClear".to_string()),
        "one still alive"
    );
    let second = b.fighters[1].id;
    b.killed(second);
    ev.extend(steps(&mut b, 45));
    let n = names(&ev);
    assert_eq!(n.iter().filter(|e| *e == "OneMore").count(), 2, "{n:?}");
    assert_eq!(n.iter().filter(|e| *e == "CanyonClear").count(), 1, "{n:?}");
    assert!(!b.spawners[0].active);
    // A script refills it; its event starts it again.
    assert_eq!(b.reset("canyon"), 1);
    b.trigger("Canyon");
    steps(&mut b, 2);
    assert_eq!(b.fighters.len(), 3);
}

#[test]
fn a_script_can_stop_a_spawner_for_good_and_remove_its_droids() {
    let mut f = tagged_factory("Trench", 0.0);
    f.generate_forever = true;
    f.max_alive = 2;
    f.spawn_interval = 0.2;
    let mut b = scripted_battle(vec![f]);
    b.trigger("Trench");
    steps(&mut b, 30);
    assert_eq!(b.alive().count(), 2);
    let out = b.stop("TRENCH", true, true);
    let removed = out
        .iter()
        .filter(|e| matches!(e, BattleEvent::Removed { .. }))
        .count();
    assert_eq!(removed, 2, "{out:?}");
    assert!(
        names(&out).is_empty(),
        "removed, not destroyed: no death events"
    );
    assert_eq!(b.alive().count(), 0);
    assert!(b.trigger("Trench").is_empty(), "off for good");
    steps(&mut b, 30);
    assert_eq!(b.alive().count(), 0);
    // Saved and put back, it stays off for good.
    let snap = b.snapshot();
    let mut again = scripted_battle(vec![tagged_factory("Trench", 0.0)]);
    again.restore(&snap).unwrap();
    assert!(again.spawners[0].disabled);
}

#[test]
fn spawn_points_take_turns_and_give_their_own_tags() {
    let spot = |x: f32, tag: &str| SpawnSpot {
        location: Vec3::new(x, 0.0, 60.0),
        yaw: 0.0,
        offspring: Offspring {
            tag: Some(tag.into()),
            event: Some(format!("{tag}Dead")),
            ..Default::default()
        },
    };
    let mut f = tagged_factory("Points", 0.0);
    f.spawn_mode = 0;
    f.capacity = 3;
    f.max_alive = 3;
    f.spawn_interval = 0.1;
    f.spots = vec![spot(1000.0, "A"), spot(2000.0, "B")];
    let mut b = scripted_battle(vec![f]);
    b.trigger("Points");
    steps(&mut b, 30);
    let xs: Vec<f32> = b.fighters.iter().map(|f| f.pawn.feet().x.round()).collect();
    assert_eq!(xs, vec![1000.0, 2000.0, 1000.0]);
    assert_eq!(b.tagged("A").len(), 2);
    let id = b.tagged("B")[0];
    assert_eq!(names(&b.killed(id)), vec!["BDead".to_string()]);
    // The spawn point is saved with the droid, so it keeps its tag.
    let snap = b.snapshot();
    let mut f2 = tagged_factory("Points", 0.0);
    f2.spawn_mode = 0;
    f2.spots = vec![spot(1000.0, "A"), spot(2000.0, "B")];
    let mut again = scripted_battle(vec![f2]);
    again.restore(&snap).unwrap();
    assert_eq!(again.tagged("A").len(), 2);
}

/// A battle droid that throws grenades every chance it gets.
fn grenadier(count: u32) -> Fighter {
    let mut p = AiParams::battle_droid();
    p.grenades.grenade = "thermal_detonator_enemy".into();
    p.grenades.count = count;
    p.grenades.probability = 1.0;
    Fighter::new(
        1,
        Arc::new(p),
        Vec3::ZERO,
        0.0,
        StartState::StationaryAttack,
    )
}

fn throws(ev: &[(f32, AiEvent)]) -> Vec<(f32, Vec3, Vec3, Vec3)> {
    ev.iter()
        .filter_map(|(t, e)| match e {
            AiEvent::Throw {
                origin,
                velocity,
                aim_at,
                ..
            } => Some((*t, *origin, *velocity, *aim_at)),
            _ => None,
        })
        .collect()
}

/// Where a throw comes back down to `z`, flying under `gravity`.
fn lands_at(origin: Vec3, v: Vec3, gravity: f32, z: f32) -> Vec3 {
    let dt = 1.0 / 1000.0;
    let (mut p, mut v) = (origin, v);
    loop {
        p = p + v * dt;
        v = v - Vec3::new(0.0, 0.0, gravity * dt);
        if v.z < 0.0 && p.z <= z {
            return p;
        }
    }
}

#[test]
fn a_throw_arc_lands_where_it_is_aimed() {
    let world = flat_floor(vec![]);
    let from = Vec3::new(0.0, 0.0, 120.0);
    for to in [
        Vec3::new(1500.0, 0.0, 24.0),
        Vec3::new(-400.0, 900.0, 24.0),
        Vec3::new(2400.0, 300.0, 300.0),
    ] {
        let v = brain::throw_arc(&world, from, to, 2200.0, 1100.0).expect("an arc");
        assert!(v.length() <= 2200.0 + 0.1, "{}", v.length());
        let landed = lands_at(from, v, 1100.0, to.z);
        assert!(
            (landed - to).length_2d() < 15.0,
            "aimed at {to:?}, lands at {landed:?}"
        );
    }
    // Too far for the fastest throw.
    assert!(brain::throw_arc(&world, from, Vec3::new(6000.0, 0.0, 24.0), 2200.0, 1100.0).is_none());
    // A low ceiling over the whole floor: no lob, but a flat throw fits.
    let low = flat_floor(vec![Brush::cuboid(
        Vec3::new(-5000.0, -5000.0, 400.0),
        Vec3::new(5000.0, 5000.0, 420.0),
    )]);
    let v = brain::throw_arc(&low, from, Vec3::new(1500.0, 0.0, 24.0), 2200.0, 1100.0)
        .expect("a flat arc under the ceiling");
    assert!(v.length() > 2199.0, "full speed: {}", v.length());
    // A tall wall in the way: nothing.
    let wall = flat_floor(vec![Brush::cuboid(
        Vec3::new(700.0, -5000.0, 0.0),
        Vec3::new(740.0, 5000.0, 3000.0),
    )]);
    assert!(brain::throw_arc(&wall, from, Vec3::new(1500.0, 0.0, 24.0), 2200.0, 1100.0).is_none());
}

#[test]
fn throws_grenades_by_the_rules() {
    let world = flat_floor(vec![]);
    // 1500 uu away: in range. It throws at its first decision in the
    // fight, then no sooner than min_time (10 s) after, until it has none.
    let mut f = grenadier(2);
    let ev = run(&mut f, &world, Some(target_at(1500.0, 0.0)), 30.0);
    let t = throws(&ev);
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t[0].0 < 1.0, "first throw at {}", t[0].0);
    assert!(t[1].0 - t[0].0 >= 10.0 - 1e-3, "{} then {}", t[0].0, t[1].0);
    assert_eq!(f.grenades_left, 0);
    for &(_, origin, v, aim) in &t {
        // Within the aim error of (1 - 0.4) x 10 = 6 degrees: 158 uu.
        assert!(
            (aim - Vec3::new(1500.0, 0.0, 24.0)).length_2d() <= 160.0,
            "{aim:?}"
        );
        let landed = lands_at(origin, v, 1100.0, aim.z);
        assert!((landed - aim).length_2d() < 15.0, "{landed:?} vs {aim:?}");
    }
    // No shots while throwing.
    let shots_near = ev.iter().filter(|(time, e)| {
        matches!(e, AiEvent::Fire { .. }) && *time >= t[0].0 && *time < t[0].0 + 0.99
    });
    assert_eq!(shots_near.count(), 0);

    // Too close (300 uu) or too far (3000 uu): no throw, but it shoots.
    for x in [300.0, 3000.0] {
        let mut f = grenadier(2);
        let ev = run(&mut f, &world, Some(target_at(x, 0.0)), 5.0);
        assert!(throws(&ev).is_empty(), "threw at {x} uu");
        assert!(ev.iter().any(|(_, e)| matches!(e, AiEvent::Fire { .. })));
    }
    // Battle droids carry none.
    let mut d = droid(0.0);
    let ev = run(&mut d, &world, Some(target_at(1500.0, 0.0)), 5.0);
    assert!(throws(&ev).is_empty());
}

#[test]
fn does_not_throw_onto_a_friend() {
    let world = flat_floor(vec![]);
    let nav = NavGraph::default();
    let taken = HashSet::new();
    let mut f = grenadier(2);
    // A friend standing 200 uu from the target.
    let allies = [Vec3::new(0.0, 0.0, 80.0), Vec3::new(1400.0, 150.0, 80.0)];
    let mut thrown = 0;
    let mut t = 0.0;
    while t < 5.0 {
        let view = View {
            world: &world,
            nav: &nav,
            time: t,
            target: Some(target_at(1500.0, 0.0)),
            cover_taken: &taken,
            allies: &allies,
        };
        thrown += f
            .tick(1.0 / 30.0, &view)
            .iter()
            .filter(|e| matches!(e, AiEvent::Throw { .. }))
            .count();
        t += 1.0 / 30.0;
    }
    assert_eq!(thrown, 0);
}

#[test]
fn grenades_left_are_saved() {
    let world = flat_floor(vec![]);
    let mut p = AiParams::battle_droid();
    p.grenades.grenade = "thermal_detonator_enemy".into();
    p.grenades.count = 2;
    p.grenades.probability = 1.0;
    let params = Arc::new(p);
    let make = || Battle::new(params.clone(), NavGraph::default(), vec![factory(0.0)]);
    let mut b = make();
    // The spawner switches on (the player is within 3000 uu), makes two,
    // and each throws once in the 9 s (the next waits 10 s).
    for _ in 0..270 {
        b.step(1.0 / 30.0, &world, Some(target_at(1500.0, 0.0)));
    }
    let snap = b.snapshot();
    let left: Vec<_> = snap.fighters.iter().map(|f| f.grenades_left).collect();
    assert_eq!(left, vec![Some(1), Some(1)], "one throw each");
    let mut fresh = make();
    fresh.restore(&snap).unwrap();
    let again: Vec<_> = fresh
        .snapshot()
        .fighters
        .iter()
        .map(|f| f.grenades_left)
        .collect();
    assert_eq!(again, left);
    // Battle droids' saves don't mention grenades.
    let plain = Battle::new(
        Arc::new(AiParams::battle_droid()),
        NavGraph::default(),
        vec![factory(0.0)],
    );
    assert!(plain
        .snapshot()
        .fighters
        .iter()
        .all(|f| f.grenades_left.is_none()));
}

/// A pike fighter like the Geonosian warrior: no gun, 350 uu reach.
fn warrior() -> Fighter {
    let mut p = AiParams::battle_droid();
    p.combat.weapon = String::new();
    p.combat.melee_range = 350.0;
    p.combat.melee_damage = 125.0;
    p.combat.melee_delay = 1.6;
    p.combat.lunge_distance = 800.0;
    p.movement.ground_speed = 450.0;
    Fighter::new(
        1,
        Arc::new(p),
        Vec3::ZERO,
        0.0,
        StartState::StationaryAttack,
    )
}

fn melee_times(ev: &[(f32, AiEvent)]) -> Vec<f32> {
    ev.iter()
        .filter(|(_, e)| matches!(e, AiEvent::Melee { .. }))
        .map(|(t, _)| *t)
        .collect()
}

#[test]
fn a_pike_fighter_rushes_in_and_hits() {
    let world = flat_floor(vec![]);
    let mut f = warrior();
    let ev = run(&mut f, &world, Some(target_at(1500.0, 0.0)), 8.0);
    assert!(
        !ev.iter().any(|(_, e)| matches!(e, AiEvent::Fire { .. })),
        "no gun, no shots"
    );
    let hits = melee_times(&ev);
    // About 1150 uu to run at 450 uu/s after 0.5 s of reacting: the first
    // hit by about 3.5 s, then one every 1.6 s.
    assert!(!hits.is_empty(), "never hit");
    assert!(hits[0] < 4.0, "first hit at {}", hits[0]);
    for w in hits.windows(2) {
        assert!(w[1] - w[0] >= 1.6 - 1e-3, "{hits:?}");
    }
    assert!(hits.len() >= 3, "{hits:?}");
    let gap = (Vec3::new(1500.0, 0.0, 0.0) - f.pawn.location).length_2d();
    assert!(gap <= 350.0 && gap > 100.0, "stands {gap:.0} uu away");
    for (_, e) in &ev {
        if let AiEvent::Melee {
            direction, range, ..
        } = e
        {
            assert!(direction.x > 0.9 && *range == 350.0, "{direction:?}");
        }
    }
}

#[test]
fn a_fighter_with_a_gun_only_lunges_when_close() {
    let world = flat_floor(vec![]);
    let make = || {
        let mut p = (*warrior().params).clone();
        p.combat.weapon = "e5_blaster".into();
        Fighter::new(
            1,
            Arc::new(p),
            Vec3::ZERO,
            0.0,
            StartState::StationaryAttack,
        )
    };
    // 1500 uu: beyond its lunge distance, it stays and shoots.
    let mut f = make();
    let ev = run(&mut f, &world, Some(target_at(1500.0, 0.0)), 5.0);
    assert!(melee_times(&ev).is_empty());
    assert!(ev.iter().any(|(_, e)| matches!(e, AiEvent::Fire { .. })));
    assert!(f.pawn.location.length_2d() < 50.0, "stayed put");
    // 700 uu: within 800, it rushes in and hits.
    let mut f = make();
    let ev = run(&mut f, &world, Some(target_at(700.0, 0.0)), 5.0);
    assert!(!melee_times(&ev).is_empty());
}

#[test]
fn a_pike_doesnt_reach_through_a_wall() {
    // A wall between them, too wide to walk round with no path network.
    let world = flat_floor(vec![Brush::cuboid(
        Vec3::new(140.0, -5000.0, 0.0),
        Vec3::new(160.0, 5000.0, 400.0),
    )]);
    let mut f = warrior();
    // It heard the enemy behind the wall.
    let mut ev = Vec::new();
    f.alert(Vec3::new(300.0, 0.0, 84.0), 0.0, &mut ev);
    let ev = run(&mut f, &world, Some(target_at(300.0, 0.0)), 4.0);
    assert!(melee_times(&ev).is_empty());
}
