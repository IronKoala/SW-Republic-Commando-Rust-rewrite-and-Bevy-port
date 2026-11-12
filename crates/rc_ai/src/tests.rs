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

/// A clone trooper's settings: a battle droid that fights for the republic.
fn trooper_params() -> Arc<AiParams> {
    Arc::new(
        AiParams::battle_droid()
            .with_overrides("[type]\nid = \"clone_trooper\"\nside = \"republic\"\n")
            .unwrap(),
    )
}

#[test]
fn clone_troopers_and_droids_fight_each_other_not_the_player() {
    let world = flat_floor(vec![]);
    let mut b = one_droid_battle();
    // A clone trooper 1500 uu away, facing the droid; the player is
    // nearer the trooper than the droid is.
    b.fighters.push(Fighter::new(
        2,
        trooper_params(),
        Vec3::new(1500.0, 0.0, 0.0),
        std::f32::consts::PI,
        StartState::StationaryAttack,
    ));
    b.made_by.push(0);
    let player = target_at(2500.0, 2500.0);
    let mut droid_dirs = Vec::new();
    let mut trooper_dirs = Vec::new();
    for _ in 0..90 {
        for e in b.step_with(1.0 / 30.0, &world, Some(player), &[]) {
            if let BattleEvent::Ai {
                id,
                event: AiEvent::Fire { direction, .. },
            } = e
            {
                if id == 1 {
                    droid_dirs.push(direction);
                } else {
                    trooper_dirs.push(direction);
                }
            }
        }
    }
    assert!(!droid_dirs.is_empty(), "the droid never fired");
    assert!(!trooper_dirs.is_empty(), "the trooper never fired");
    for d in &droid_dirs {
        assert!(d.x > 0.9, "the droid fired at {d:?}, not the trooper");
    }
    for d in &trooper_dirs {
        assert!(d.x < -0.9, "the trooper fired at {d:?}, not the droid");
    }
    // Clone troopers on their own don't make it a fight for the music.
    let mut alone = Battle::new(trooper_params(), NavGraph::default(), vec![]);
    alone.fighters.push(Fighter::new(
        1,
        trooper_params(),
        Vec3::ZERO,
        0.0,
        StartState::StationaryAttack,
    ));
    alone.made_by.push(0);
    for _ in 0..60 {
        let ev = alone.step_with(1.0 / 30.0, &world, Some(target_at(800.0, 0.0)), &[]);
        assert!(
            fire_directions(&ev).is_empty(),
            "a trooper fired at the player"
        );
    }
    assert!(!alone.in_fight());
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
            // The clone troopers hold their RequiredEquipment's model.
            let held = match params.kind.id.as_str() {
                "clone_trooper" => "DC15BlasterClone",
                "clone_trooper_tm" => "DC15BlasterCloneTM",
                _ => "",
            };
            assert_eq!(params.look.held_weapon, held, "{}", params.kind.id);
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
    // The clone troopers fight on the player's side.
    assert_eq!(who("CloneTrooper").as_deref(), Some("clone_trooper"));
    assert_eq!(who("CloneTrooperTM").as_deref(), Some("clone_trooper_tm"));
    for t in &types {
        assert_eq!(t.republic(), t.id.starts_with("clone_trooper"), "{}", t.id);
    }
    // Geonosian warriors: the unarmed one placed in geo_01a has its own.
    assert_eq!(who("GeonosianWarrior").as_deref(), Some("geonosian"));
    assert_eq!(
        who("GeonosianWarriorNoWeap").as_deref(),
        Some("geonosian_no_weapon")
    );
    // Every Trandoshan kind the campaign's maps place has exactly one file.
    for (class, id) in [
        ("TrandoshanMercenary", "trandoshan"),
        ("TrandoshanMercenaryConc", "trandoshan"),
        ("TrandoshanMercenaryGrenadier", "trandoshan_grenadier"),
        ("TrandoshanMercGrenYYYUp", "trandoshan_grenadier"),
        ("TrandoshanMercGrenYYYLow", "trandoshan_grenadier"),
        ("TrandoshanMercYYYUp", "trandoshan_kashyyyk"),
        ("TrandoshanMercYYYLow", "trandoshan_kashyyyk"),
        ("TrandoshanSlaver", "trandoshan_slaver"),
        ("TrandoshanSlaverBerserker", "trandoshan_slaver_berserker"),
        ("TrandoshanSlaverGrenadier", "trandoshan_slaver_grenadier"),
        ("TrandoshanHeavy", "trandoshan_heavy"),
        ("TrandoshanScav", "trandoshan_scavenger"),
        ("TrandoshanScavNoWookiee", "trandoshan_scavenger"),
    ] {
        assert_eq!(who(class).as_deref(), Some(id), "{class}");
    }
}

#[test]
fn kashyyyk_mercenaries_have_a_shield() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/enemies/trandoshan_kashyyyk.toml"
    ))
    .unwrap();
    let p = AiParams::battle_droid().with_overrides(&text).unwrap();
    assert_eq!(p.combat.health, 100.0);
    assert_eq!(p.combat.shields, 100.0);
    assert_eq!(p.combat.shield_recharge_rate, 10.0);
    // Nothing else has one unless its file says so.
    assert_eq!(AiParams::battle_droid().combat.shields, 0.0);
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
    // A droid that was fighting is fighting again: one that was attacking
    // carries straight on, one that had only just seen you is alerted.
    match a.mode {
        Mode::Idle => {}
        Mode::Attack => assert_eq!(z.mode, Mode::Attack),
        _ => assert_eq!(z.mode, Mode::Alerted),
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
            script: Some(format!("{tag}AI")),
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

/// A level script's orders: walk to a point and turn there, turn to the
/// player, hold still until a time; each ends with `ScriptDone`, and the
/// droid doesn't shoot while it follows them.
#[test]
fn follows_script_orders() {
    let world = flat_floor(vec![]);
    let mut f = droid(0.0);
    let to = Vec3::new(600.0, 300.0, 84.0);
    f.follow_script(ScriptGoal::MoveTo {
        to,
        walk: false,
        tolerance: 0.0,
        face: Some(std::f32::consts::PI),
    });
    let ev = run(&mut f, &world, None, 8.0);
    let done: Vec<f32> = ev
        .iter()
        .filter(|(_, e)| *e == AiEvent::ScriptDone)
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(done.len(), 1, "{ev:?}");
    assert!(f.script.is_none());
    let at = f.pawn.location;
    assert!((at - to).length_2d() < 60.0, "ended at {at:?}");
    assert!(
        brain::angle_diff(f.facing, std::f32::consts::PI).abs() < 0.1,
        "facing {} {ev:?}",
        f.facing
    );
    // With the player in plain sight the script still comes first: no
    // shots until it has arrived.
    let mut g = droid(0.0);
    g.follow_script(ScriptGoal::MoveTo {
        to,
        walk: false,
        tolerance: 0.0,
        face: Some(std::f32::consts::PI),
    });
    let ev = run(&mut g, &world, Some(target_at(-800.0, 0.0)), 8.0);
    let done = ev
        .iter()
        .find(|(_, e)| *e == AiEvent::ScriptDone)
        .map(|(t, _)| *t)
        .unwrap();
    assert!(!ev
        .iter()
        .any(|(t, e)| matches!(e, AiEvent::Fire { .. }) && *t < done));
    // ... and fights after (it ends facing the player).
    assert!(ev
        .iter()
        .any(|(t, e)| matches!(e, AiEvent::Fire { .. }) && *t > done));

    // Turn to the player, at (600, -500): straight "down" in y.
    f.follow_script(ScriptGoal::TurnToTarget);
    let ev = run(&mut f, &world, Some(target_at(600.0, -500.0)), 2.0);
    assert!(ev.iter().any(|(_, e)| *e == AiEvent::ScriptDone));
    assert!(brain::angle_diff(f.facing, -std::f32::consts::FRAC_PI_2).abs() < 0.1);

    // Hold until t = 1.0 (the test's clock starts at 0 each run).
    f.follow_script(ScriptGoal::Hold { until: 1.0 });
    let ev = run(&mut f, &world, None, 2.0);
    let t = ev
        .iter()
        .find(|(_, e)| *e == AiEvent::ScriptDone)
        .map(|(t, _)| *t)
        .unwrap();
    assert!((1.0..1.1).contains(&t), "{t}");

    // Being hurt ends a hold at once.
    f.follow_script(ScriptGoal::Hold { until: 100.0 });
    let mut ev = Vec::new();
    f.on_damaged(Some(Vec3::new(0.0, 0.0, 84.0)), 0.0, &mut ev);
    assert!(ev.contains(&AiEvent::ScriptDone));
    assert!(f.script.is_none());
}

/// A scripted point well above it: a character that can leap
/// (`leap_speed`) leaps there; a battle droid (can't) doesn't.
#[test]
fn leaps_to_high_script_points() {
    let world = flat_floor(vec![]);
    let to = Vec3::new(500.0, 0.0, 600.0);
    let order = ScriptGoal::MoveTo {
        to,
        walk: false,
        tolerance: 0.0,
        face: None,
    };
    let mut f = droid(0.0);
    std::sync::Arc::make_mut(&mut f.params).combat.leap_speed = 900.0;
    f.follow_script(order.clone());
    let ev = run(&mut f, &world, None, 4.0);
    assert!(ev.iter().any(|(_, e)| *e == AiEvent::Leapt), "{ev:?}");
    assert!(ev.iter().any(|(_, e)| *e == AiEvent::ScriptDone));
    assert!((f.pawn.location - to).length_2d() < 60.0);

    let mut g = droid(0.0);
    g.follow_script(order);
    let ev = run(&mut g, &world, None, 4.0);
    assert!(!ev.iter().any(|(_, e)| *e == AiEvent::Leapt));
}

// Droid dispensers (see `battle::dispense` for the rules).

fn bd_dispenser(tag: &str) -> Factory {
    let (d, proto, capacity) = Dispenser::class_defaults("BDDispenser").unwrap();
    let mut d = d;
    d.damaged.event = Some(format!("{tag}Destroyed"));
    let spot = |x: f32, y: f32| SpawnSpot {
        location: Vec3::new(x, y, 60.0),
        yaw: 0.0,
        offspring: Offspring {
            tag: Some(format!("{tag}Droid")),
            start_state: StartState::StationaryAttack,
            ..Default::default()
        },
    };
    Factory {
        name: format!("{tag} dispenser"),
        tag: tag.into(),
        prototype: proto.into(),
        location: Vec3::new(0.0, 0.0, 125.0),
        spots: vec![spot(0.0, -400.0), spot(400.0, 0.0), spot(-400.0, 0.0)],
        capacity,
        max_alive: 1,
        generate_forever: true,
        start_timer_after_death: true,
        spawn_mode: 0,
        spawn_interval: 3.0,
        exhausted_event: Some(format!("{tag}Gone")),
        spawn_event: Some(format!("{tag}Made")),
        dispenser: Some(d),
        placed: false,
        ..Default::default()
    }
}

// Characters placed straight in the map (see `spawns::read_placed`).

fn placed(name: &str, x: f32) -> Factory {
    Factory {
        name: name.into(),
        prototype: "BattleDroidTM".into(),
        location: Vec3::new(x, 0.0, 60.0),
        capacity: 1,
        offspring: Offspring {
            tag: Some(name.into()),
            event: Some(format!("{name}Dead")),
            first_damaged_event: None,
            start_state: StartState::Idle,
            team: None,
            script: Some(format!("{name}AI")),
        },
        placed: true,
        ..Default::default()
    }
}

/// Made-up animation numbers: intro 1.5 s, each slot's animation 2 s with
/// the droid let go at 0.25 (0.5 s), its bone moving 100 uu out along -Y
/// from 92 uu up; the unfold takes 1.5 s and moves the droid 120 forward
/// and 80 down.
fn test_motion() -> DispenserMotion {
    let slot = |dir: Vec3| SlotMotion {
        seconds: 2.0,
        release: 0.25,
        path: vec![
            BoneSample {
                seconds: 0.0,
                at: dir * 100.0 + Vec3::new(0.0, 0.0, 92.0),
                x: Vec3::new(1.0, 0.0, 0.0),
                z: Vec3::new(0.0, 0.0, 1.0),
            },
            BoneSample {
                seconds: 0.5,
                at: dir * 200.0 + Vec3::new(0.0, 0.0, 92.0),
                x: Vec3::new(1.0, 0.0, 0.0),
                z: Vec3::new(0.0, 0.0, 1.0),
            },
        ],
    };
    DispenserMotion {
        intro_seconds: 1.5,
        slots: vec![
            slot(Vec3::new(0.0, -1.0, 0.0)),
            slot(Vec3::new(1.0, 0.0, 0.0)),
            slot(Vec3::new(-1.0, 0.0, 0.0)),
        ],
        unfold: Unfold {
            seconds: 1.5,
            travel: vec![(0.0, Vec3::ZERO), (1.5, Vec3::new(120.0, 0.0, -80.0))],
        },
        intro_travel: Vec::new(),
    }
}

fn dispenser_battle() -> Battle {
    let mut b = scripted_battle(vec![bd_dispenser("Disp")]);
    b.set_dispenser_motion(0, test_motion());
    b
}

/// Step until `secs` of battle time, returning (time, event) pairs.
fn steps_until(b: &mut Battle, secs: f32) -> Vec<(f32, BattleEvent)> {
    let world = flat_floor(vec![]);
    let mut out = Vec::new();
    while b.time < secs {
        let t = b.time;
        for e in b.step(1.0 / 30.0, &world, Some(target_at(-4500.0, -4500.0))) {
            out.push((t, e));
        }
    }
    out
}

fn dispenser_steps(ev: &[(f32, BattleEvent)]) -> Vec<(f32, DispenserStep)> {
    ev.iter()
        .filter_map(|(t, e)| match e {
            BattleEvent::Dispenser { step, .. } => Some((*t, step.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn dispenser_class_defaults_are_the_originals() {
    let (bd, proto, cap) = Dispenser::class_defaults("BDDispenser").unwrap();
    assert_eq!((proto, cap), ("BattleDroid", 3));
    assert_eq!((bd.num_dispensers, bd.num_init_droids), (3, 3));
    assert_eq!((bd.max_health, bd.warm_up_time), (5000.0, 1.0));
    assert_eq!(bd.intro_anim.as_deref(), Some("entranceSky"));
    assert_eq!(
        bd.online.material.as_deref(),
        Some("DispensersTextures.BDDispenser.BDDispenser_On")
    );
    assert_eq!(bd.damaged.anim.as_deref(), Some("Destroyed"));
    assert_eq!(bd.damaged.effect_offset, Vec3::new(0.0, 0.0, -50.0));
    let (sbd, proto, cap) = Dispenser::class_defaults("SBDDispenser").unwrap();
    assert_eq!((proto, cap), ("SuperBattleDroid", 4));
    assert_eq!((sbd.num_dispensers, sbd.num_init_droids), (4, 0));
    assert_eq!((sbd.collision_radius, sbd.collision_height), (219.0, 125.0));
    assert!(sbd.intro_anim.is_none());
}

#[test]
fn a_dispenser_waits_for_its_event_then_drops_in_warms_up_and_dispenses() {
    let mut b = dispenser_battle();
    let ev = steps_until(&mut b, 3.0);
    assert!(b.fighters.is_empty(), "nothing before its event");
    assert!(dispenser_steps(&ev).is_empty());
    assert!(b.trigger("Disp").iter().any(|e| matches!(
        e,
        BattleEvent::Dispenser { step: DispenserStep::Intro { anim: Some(a) }, .. } if a == "entranceSky"
    )));
    let start = b.time;
    let ev = steps_until(&mut b, start + 3.0);
    let st = dispenser_steps(&ev);
    // Intro 1.5 s, then warm-up 1 s (WarmUpTime), then online and a droid
    // at once.
    let at = |want: &dyn Fn(&DispenserStep) -> bool| {
        st.iter().find(|(_, s)| want(s)).map(|(t, _)| t - start)
    };
    let warm = at(&|s| matches!(s, DispenserStep::Look { which: "warm_up" })).unwrap();
    let online = at(&|s| matches!(s, DispenserStep::Look { which: "online" })).unwrap();
    let first = at(&|s| matches!(s, DispenserStep::Dispense { .. })).unwrap();
    // (To the step: 1/30 s.)
    assert!((warm - 1.5).abs() < 0.04, "warm-up at {warm}");
    assert!((online - 2.5).abs() < 0.04, "online at {online}");
    assert!((first - online).abs() < 0.04, "first droid at {first}");
    // It used spawn point 0 (its tag) on slot 1: the slot is the next
    // spawn point's number.
    let (id, slot) = st
        .iter()
        .find_map(|(_, s)| match s {
            DispenserStep::Dispense { id, slot } => Some((*id, *slot)),
            _ => None,
        })
        .unwrap();
    assert_eq!(slot, 1);
    assert_eq!(b.offspring(id).unwrap().tag.as_deref(), Some("DispDroid"));
}

#[test]
fn a_dispensed_droid_rides_its_slot_then_unfolds_then_fights() {
    let mut b = dispenser_battle();
    b.trigger("Disp");
    let ev = steps_until(&mut b, 2.6);
    let id = b.fighters[0].id;
    assert!(b.is_dispensing(id));
    // On slot 1, whose bone goes out along +X from 100 to 200 uu, 92 up;
    // the dispenser's model stands on its cylinder's bottom (z 0), and the
    // droid's centre is its own half-height above the bone.
    let half = b.fighters[0].params.movement.collision_height;
    let p = b.fighters[0].pawn.location;
    assert!(p.x > 99.0 && p.x < 130.0, "{p:?}");
    assert!((p.z - (92.0 + half)).abs() < 0.5, "{p:?}");
    assert!(!ev
        .iter()
        .any(|(_, e)| matches!(e, BattleEvent::Ai { id: i, .. } if *i == id)));
    // Let go at 0.5 s, at 200 uu, with its SpawnEvent.
    let ev = steps_until(&mut b, 3.15);
    assert!(names(&ev.iter().map(|e| e.1.clone()).collect::<Vec<_>>())
        .contains(&"DispMade".to_string()));
    let p = b.fighters[0].pawn.location;
    assert!(p.x > 199.0 && p.x < 210.0, "{p:?}");
    // Unfolds over 1.5 s: 120 forward (its facing is +X) and 80 down.
    let ev = steps_until(&mut b, 4.7);
    assert!(dispenser_steps(&ev)
        .iter()
        .any(|(_, s)| *s == DispenserStep::Unfolded { id }));
    assert!(!b.is_dispensing(id));
    let p = b.fighters[0].home;
    assert!((p.x - 320.0).abs() < 2.0, "{p:?}");
    assert!((p.z - (12.0 + half)).abs() < 2.0, "{p:?}");
}

#[test]
fn a_dispenser_makes_its_first_three_then_waits_for_a_death() {
    // MaxAlive 3, as most maps set it.
    let mut f = bd_dispenser("Disp");
    f.max_alive = 3;
    let mut b = scripted_battle(vec![f]);
    b.set_dispenser_motion(0, test_motion());
    b.trigger("Disp");
    // Online at 2.5 s; NumInitDroids 3 with a 3 s interval: 2.5, 5.5, 8.5.
    steps_until(&mut b, 20.0);
    assert_eq!(b.alive().count(), 3);
    assert!(matches!(
        b.spawners[0].disp.as_ref().unwrap().phase,
        Phase::Waiting { .. }
    ));
    // Slots in turn after the first: spawn points 0, 1, 2 give slots 1, 2, 0.
    // One dies: seen within a second, then its 3 s interval, then a new one
    // at once.
    let first = b.fighters[0].id;
    b.killed(first);
    let t0 = b.time;
    let ev = steps_until(&mut b, t0 + 6.0);
    let made: Vec<f32> = dispenser_steps(&ev)
        .iter()
        .filter(|(_, s)| matches!(s, DispenserStep::Dispense { .. }))
        .map(|(t, _)| t - t0)
        .collect();
    assert_eq!(made.len(), 1, "{made:?}");
    assert!(made[0] > 3.0 && made[0] <= 4.1, "{made:?}");
    // Back to 3: no more until another dies.
    steps_until(&mut b, t0 + 30.0);
    assert_eq!(b.alive().count(), 3);
    assert_eq!(b.spawners[0].made, 4);
}

#[test]
fn after_its_first_droids_a_dispenser_keeps_to_max_alive() {
    // MaxAlive 1 (the class default): NumInitDroids (3) counts only at
    // the start; afterwards it waits until none of its droids is left.
    let mut b = dispenser_battle();
    b.trigger("Disp");
    steps_until(&mut b, 20.0);
    assert_eq!(b.alive().count(), 3);
    let ids: Vec<u32> = b.alive().map(|f| f.id).collect();
    b.killed(ids[0]);
    b.killed(ids[1]);
    steps_until(&mut b, 40.0);
    assert_eq!(b.alive().count(), 1, "one left: MaxAlive is 1");
    b.killed(ids[2]);
    steps_until(&mut b, 50.0);
    assert_eq!(b.alive().count(), 1, "a new one");
    assert_eq!(b.spawners[0].made, 4);
}

#[test]
fn a_dispenser_is_damaged_in_steps_and_destroyed() {
    let mut b = dispenser_battle();
    b.trigger("Disp");
    steps_until(&mut b, 2.6);
    let rider = b.fighters[0].id;
    assert!(b.dispensing(rider).unwrap().released.is_none());
    let ev = b.dispenser_hurt(0, 3000.0);
    assert!(matches!(
        &ev[..],
        [BattleEvent::Dispenser {
            step: DispenserStep::Look {
                which: "slightly_damaged"
            },
            ..
        }]
    ));
    assert!(b.dispenser_hurt(0, 2900.0).is_empty(), "each step once");
    let ev = b.dispenser_hurt(0, 1500.0);
    assert!(matches!(
        &ev[..],
        [BattleEvent::Dispenser {
            step: DispenserStep::Look {
                which: "heavily_damaged"
            },
            ..
        }]
    ));
    let ev = b.dispenser_hurt(0, -10.0);
    // Destroyed: its look and event, and the droid still on its slot goes
    // with it.
    assert!(ev.iter().any(|e| matches!(
        e,
        BattleEvent::Dispenser {
            step: DispenserStep::Look { which: "damaged" },
            ..
        }
    )));
    assert!(names(&ev).contains(&"DispDestroyed".to_string()));
    assert!(ev.contains(&BattleEvent::Dispenser {
        spawner: 0,
        step: DispenserStep::KilledOnSlot { id: rider }
    }));
    assert_eq!(b.alive().count(), 0);
    // No more droids; with all of them dead, its ExhaustedEvent once.
    let ev = steps_until(&mut b, 30.0);
    assert!(b.fighters.len() == 1);
    let ev: Vec<BattleEvent> = ev.into_iter().map(|e| e.1).collect();
    assert_eq!(names(&ev), vec!["DispGone".to_string()]);
}

#[test]
fn a_dispenser_with_an_online_tag_waits_for_it_after_its_intro() {
    let mut f = bd_dispenser("Disp");
    f.dispenser.as_mut().unwrap().online_tag = Some("StartDispensing".into());
    let mut b = scripted_battle(vec![f]);
    b.set_dispenser_motion(0, test_motion());
    b.trigger("Disp");
    steps_until(&mut b, 10.0);
    assert!(b.fighters.is_empty());
    assert_eq!(
        b.spawners[0].disp.as_ref().unwrap().phase,
        Phase::WaitOnline
    );
    // Its old tag does nothing now; the online tag starts it.
    b.trigger("Disp");
    assert_eq!(
        b.spawners[0].disp.as_ref().unwrap().phase,
        Phase::WaitOnline
    );
    b.trigger("StartDispensing");
    steps_until(&mut b, 10.2);
    assert_eq!(b.fighters.len(), 1);
}

#[test]
fn a_dispenser_is_saved_and_carries_on() {
    let mut b = dispenser_battle();
    b.trigger("Disp");
    steps_until(&mut b, 12.0);
    b.dispenser_hurt(0, 3000.0);
    let snap = b.snapshot();
    let mut fresh = dispenser_battle();
    fresh.restore(&snap).unwrap();
    assert_eq!(fresh.snapshot().spawners, snap.spawners);
    assert_eq!(
        fresh.spawners[0].disp,
        b.spawners[0].disp.as_ref().map(|d| DispState {
            riding: None,
            ..d.clone()
        })
    );
}

#[test]
fn a_dispenser_is_carried_by_its_intro_and_its_droids_come_out_where_it_landed() {
    // As entranceSky: the machine starts 3000 uu up and its intro's root
    // bone brings it down to the floor (PHYS_RootMotion).
    let mut b = scripted_battle(vec![bd_dispenser("Disp")]);
    let mut m = test_motion();
    m.intro_travel = vec![(0.0, Vec3::ZERO), (1.5, Vec3::new(0.0, 0.0, -3000.0))];
    b.set_dispenser_motion(0, m);
    let placed = b.spawners[0].factory.location;
    b.spawners[0].factory.location = placed + Vec3::new(0.0, 0.0, 3000.0);
    assert_eq!(
        b.dispenser_location(0),
        placed + Vec3::new(0.0, 0.0, 3000.0),
        "up there before its event"
    );
    b.trigger("Disp");
    let start = b.time;
    steps_until(&mut b, start + 0.75);
    let mid = b.dispenser_location(0);
    assert!(
        (mid.z - (placed.z + 1500.0)).abs() < 70.0,
        "half way down: {mid:?}"
    );
    steps_until(&mut b, start + 2.6);
    assert_eq!(b.dispenser_location(0), placed, "landed");
    // Its first droid rides slot 1 from where it landed (bone 92 up).
    let half = b.fighters[0].params.movement.collision_height;
    let p = b.fighters[0].pawn.location;
    assert!(
        (p.z - (placed.z
            - b.spawners[0]
                .factory
                .dispenser
                .as_ref()
                .unwrap()
                .collision_height
            + 92.0
            + half))
            .abs()
            < 0.5,
        "{p:?}"
    );
    // And stays down after a save.
    let snap = b.snapshot();
    let mut again = scripted_battle(vec![bd_dispenser("Disp")]);
    let mut m = test_motion();
    m.intro_travel = vec![(0.0, Vec3::ZERO), (1.5, Vec3::new(0.0, 0.0, -3000.0))];
    again.set_dispenser_motion(0, m);
    again.spawners[0].factory.location = placed + Vec3::new(0.0, 0.0, 3000.0);
    again.restore(&snap).unwrap();
    assert_eq!(again.dispenser_location(0), placed);
}

#[test]
fn a_dispenser_rides_its_base_and_its_droids_ride_with_it() {
    // `AttachTag`: based on a mover that has gone 1000 uu along X and
    // turned a quarter round (yaw +90 degrees) about the level's origin.
    let mut b = scripted_battle(vec![bd_dispenser("Disp")]);
    b.set_dispenser_motion(0, test_motion());
    let placed = b.spawners[0].factory.location;
    let yaw0 = b.dispenser_yaw(0);
    let carry = Carry {
        turn: [
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
        shift: Vec3::new(1000.0, 0.0, 0.0),
        yaw: std::f32::consts::FRAC_PI_2,
    };
    b.set_dispenser_carry(0, Some(carry));
    let want = Vec3::new(1000.0 - placed.y, placed.x, placed.z);
    assert!((b.dispenser_location(0) - want).length() < 0.01, "carried");
    assert!(
        (b.dispenser_yaw(0) - yaw0 - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "turned"
    );
    // Its droid on a slot is placed round the carried machine.
    b.trigger("Disp");
    let start = b.time;
    steps_until(&mut b, start + 2.6);
    let p = b.fighters[0].pawn.location;
    let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
    let d = (flat(p) - flat(want)).length();
    assert!(
        d > 50.0 && d < 300.0,
        "droid {p:?} is {d} from the carried machine"
    );
    assert!(
        (flat(p) - flat(placed)).length() > 700.0,
        "not where the map put it: {p:?}"
    );
    // No base: back where the map put it.
    b.set_dispenser_carry(0, None);
    assert_eq!(b.dispenser_location(0), placed);
}

#[test]
fn placed_characters_are_all_there_from_the_start() {
    // More than the cap on fighters alive at once, and no events: the
    // original has every placed character in the level as it starts.
    let n = AiParams::battle_droid().spawning.max_alive_total as usize + 3;
    let list: Vec<Factory> = (0..n)
        .map(|i| placed(&format!("Placed{i}"), i as f32 * 100.0))
        .collect();
    let mut b = scripted_battle(list);
    let ev = steps(&mut b, 1);
    let made = ev
        .iter()
        .filter(|e| matches!(e, BattleEvent::Spawned { .. }))
        .count();
    assert_eq!(made, n, "{ev:?}");
    // At its own place, feet on the floor, with its own tag and script.
    let f = &b.fighters[2];
    assert!((f.pawn.feet() - Vec3::new(200.0, 0.0, 0.0)).length_2d() < 1.0);
    assert!(f.pawn.feet().z < 60.0, "{:?}", f.pawn.feet());
    assert_eq!(b.tagged("Placed2"), vec![f.id]);
    assert_eq!(b.following("Placed2AI"), vec![f.id]);
    // Never made again, even after it dies.
    let id = f.id;
    let dead = b.killed(id);
    assert_eq!(names(&dead), vec!["Placed2Dead".to_string()]);
    steps(&mut b, 120);
    assert_eq!(b.fighters.len(), n);
    assert!(b.spawners.iter().all(|s| !s.active && s.made == 1));
}

#[test]
fn events_and_factory_orders_leave_placed_characters_alone() {
    // A placed spawner's tag is empty and events naming its character's
    // tag don't touch it.
    let mut b = scripted_battle(vec![placed("Rocky", 0.0)]);
    steps(&mut b, 1);
    assert!(b.trigger("Rocky").is_empty());
    assert!(b.trigger("").is_empty());
    assert!(b.stop("", true, true).is_empty());
    assert_eq!(b.reset(""), 0);
    assert_eq!(b.alive().count(), 1);
}

#[test]
fn saves_from_before_placed_characters_still_load() {
    let params = Arc::new(AiParams::battle_droid());
    let old = Battle::new(params.clone(), NavGraph::default(), vec![factory(0.0)]);
    let snap = old.snapshot();
    let mut now = Battle::new(
        params.clone(),
        NavGraph::default(),
        vec![factory(0.0), placed("Rocky", 500.0)],
    );
    now.scripted = true;
    now.restore(&snap).unwrap();
    // The placed character starts as on a fresh level.
    steps(&mut now, 1);
    assert_eq!(now.tagged("Rocky").len(), 1);
    // And a save made now carries on without making it twice.
    let snap = now.snapshot();
    let mut again = Battle::new(
        params,
        NavGraph::default(),
        vec![factory(0.0), placed("Rocky", 500.0)],
    );
    again.scripted = true;
    let back = again.restore(&snap).unwrap();
    assert_eq!(back.len(), 1);
    steps(&mut again, 30);
    assert_eq!(again.fighters.len(), 1);
}
