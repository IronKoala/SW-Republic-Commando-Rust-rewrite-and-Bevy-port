//! Squad tests in the movement test room (flat floor at z = 0, open from
//! x -2000 to 1000 around the start). Each runs the squad for a few
//! simulated seconds and checks distances, the way the log is checked.

use super::*;
use rc_movement::room::test_room;
use rc_movement::{BrushWorld, MoveParams, Vec3};

const DT: f32 = 1.0 / 60.0;

fn leader_at(x: f32, y: f32, yaw_deg: f32) -> Leader {
    Leader {
        feet: Vec3::new(x, y, 0.0),
        yaw: yaw_deg.to_radians(),
    }
}

fn new_squad(world: &BrushWorld, leader: Leader) -> Squad {
    Squad::new(
        SquadSettings::default(),
        MoveParams::player_commando(),
        leader,
        world,
    )
    .0
}

/// Run for `secs`, returning every event.
fn run(squad: &mut Squad, leader: Leader, secs: f32, world: &BrushWorld) -> Vec<SquadEvent> {
    let mut events = Vec::new();
    let mut t = 0.0;
    while t < secs {
        events.extend(squad.update(DT, leader, world, &StraightLine));
        t += DT;
    }
    events
}

fn distances(squad: &Squad, p: Vec3) -> Vec<f32> {
    squad
        .members
        .iter()
        .map(|m| {
            let f = m.pawn.feet();
            Vec3::new(f.x - p.x, f.y - p.y, 0.0).length()
        })
        .collect()
}

fn all_idle(squad: &Squad) -> bool {
    squad.members.iter().all(|m| m.state == MemberState::Idle)
}

#[test]
fn shipped_settings_file_matches_built_in_values() {
    let text = include_str!("../../../data/squad/squad.toml");
    let s = SquadSettings::default().with_overrides(text).unwrap();
    assert_eq!(s, SquadSettings::default());
}

#[test]
fn settings_typo_names_the_line() {
    let e = SquadSettings::default()
        .with_overrides("[stance.form_up]\nlp_activat = 5\n")
        .unwrap_err();
    assert!(e.contains("line 2") && e.contains("lp_activat"), "{e}");
    let e = SquadSettings::default()
        .with_overrides("[squad]\nspawn = \"maybe\"\n")
        .unwrap_err();
    assert!(e.contains("line 2"), "{e}");
}

#[test]
fn settings_override_one_value_and_members() {
    let s = SquadSettings::default()
        .with_overrides(
            "[stance.form_up]\nlp_activate = 300\n[member.boss]\nname = \"Boss\"\nslot = 1\n",
        )
        .unwrap();
    assert_eq!(s.form_up.lp_activate, 300.0);
    assert_eq!(s.form_up.lp_deactivate, 192.0);
    assert_eq!(s.members.len(), 1);
    assert_eq!(s.members[0].name, "Boss");
}

#[test]
fn spawns_in_formation_and_settles_on_the_floor() {
    let world = test_room();
    let leader = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, leader);
    run(&mut squad, leader, 1.0, &world);
    for (m, d) in squad.members.iter().zip(distances(&squad, leader.feet)) {
        assert!((150.0..200.0).contains(&d), "{} {d}", m.settings.name);
        assert!(
            m.pawn.feet().z.abs() < 1.0,
            "{} z {}",
            m.settings.name,
            m.pawn.feet().z
        );
        // Behind the leader (who faces +X).
        assert!(m.pawn.feet().x < -100.0);
    }
    assert!(all_idle(&squad));
}

#[test]
fn form_up_waits_until_the_leader_is_far_then_catches_up() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    // 500 away is inside the 768 tether: nobody moves.
    let near = leader_at(500.0, 0.0, 0.0);
    let ev = run(&mut squad, near, 2.0, &world);
    assert!(
        !ev.iter().any(|e| matches!(e, SquadEvent::SetOff { .. })),
        "{ev:?}"
    );
    // 1000 away is outside it: everyone sets off and closes in.
    let far = leader_at(1000.0, 0.0, 0.0);
    let ev = run(&mut squad, far, 6.0, &world);
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(
                e,
                SquadEvent::SetOff {
                    by_order: false,
                    ..
                }
            ))
            .count(),
        3
    );
    assert!(all_idle(&squad));
    for d in distances(&squad, far.feet) {
        assert!(d <= 260.0, "{d}");
    }
}

#[test]
fn direct_form_up_goes_to_the_slots() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    let moved = leader_at(400.0, 300.0, 90.0);
    squad.command(Order::FormUp, moved, &world);
    run(&mut squad, moved, 5.0, &world);
    assert!(all_idle(&squad));
    for m in &squad.members {
        let [f, r] = squad.settings.formation[m.settings.slot - 1];
        let (fw, rt) = rc_movement::axes(moved.yaw);
        let slot = moved.feet + fw * f + rt * r;
        let d = (m.pawn.feet() - slot).length_2d();
        assert!(d < 60.0, "{} is {d} from its slot", m.settings.name);
    }
}

#[test]
fn secure_position_holds_the_spot_when_the_leader_leaves() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    let spot = Vec3::new(700.0, 500.0, 0.0);
    squad.command(Order::SecurePosition(spot), start, &world);
    run(&mut squad, start, 5.0, &world);
    assert!(all_idle(&squad));
    for d in distances(&squad, spot) {
        assert!((60.0..170.0).contains(&d), "{d}");
    }
    // The leader walks off; the squad stays.
    let away = leader_at(-1200.0, -900.0, 0.0);
    let ev = run(&mut squad, away, 3.0, &world);
    assert!(
        !ev.iter().any(|e| matches!(e, SquadEvent::SetOff { .. })),
        "{ev:?}"
    );
    for d in distances(&squad, spot) {
        assert!(d < 170.0, "{d}");
    }
    // Cancel brings them back.
    squad.command(Order::Cancel, away, &world);
    assert_eq!(squad.stance, Stance::FormUp);
    run(&mut squad, away, 8.0, &world);
    for d in distances(&squad, away.feet) {
        assert!(d <= 260.0, "{d}");
    }
}

#[test]
fn hold_stays_put() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    let before: Vec<Vec3> = squad.members.iter().map(|m| m.pawn.feet()).collect();
    squad.command(Order::Hold, start, &world);
    let away = leader_at(900.0, 0.0, 0.0);
    run(&mut squad, away, 3.0, &world);
    for (m, b) in squad.members.iter().zip(before) {
        assert!((m.pawn.feet() - b).length_2d() < 5.0);
    }
}

#[test]
fn marker_sends_the_nearest_member_who_does_the_job() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    squad.markers.push((
        MarkerSpot {
            name: "SquadMarkerSpot0".into(),
            job: "Bacta".into(),
            anchor: "BactaDispenserGEO0".into(),
            anchor_class: "BactaDispenserGEO".into(),
            stand: Vec3::new(-300.0, 600.0, 0.0),
            yaw: 0.0,
            aim: Vec3::new(-260.0, 640.0, 60.0),
        },
        MarkerState::Open,
    ));
    // Aiming at it from the leader's eyes picks it.
    let eye = start.feet + Vec3::new(0.0, 0.0, 144.0);
    let k = squad.marker_in_aim(eye, Vec3::new(-260.0, 640.0, 60.0) - eye, 2048.0, 0.15);
    assert_eq!(k, Some(0));
    let ev = squad.command(Order::Marker(0), start, &world);
    // Fixer (slot 2, on the leader's right, +Y) is nearest.
    assert!(
        ev.iter()
            .any(|e| matches!(e, SquadEvent::MarkerAssigned { member, .. } if member == "Fixer")),
        "{ev:?}"
    );
    let ev = run(&mut squad, start, 8.0, &world);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SquadEvent::MarkerStarted { .. })),
        "{ev:?}"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, SquadEvent::MarkerDone { .. })),
        "{ev:?}"
    );
    assert_eq!(squad.markers[0].1, MarkerState::Done);
    // A finished marker can't be ordered again.
    let ev = squad.command(Order::Marker(0), start, &world);
    assert!(matches!(ev[0], SquadEvent::OrderIgnored { .. }));
}

#[test]
fn search_and_destroy_closes_on_a_known_enemy() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    squad.command(Order::SearchAndDestroy, start, &world);
    run(&mut squad, start, 3.0, &world);
    // Without enemies they spread out ahead of the leader.
    for m in &squad.members {
        assert!(
            m.pawn.feet().x > 100.0,
            "{} x {}",
            m.settings.name,
            m.pawn.feet().x
        );
    }
    // Within reach (1024 uu) of the leader.
    let droid = Vec3::new(700.0, -500.0, 0.0);
    squad.threats = vec![droid];
    run(&mut squad, start, 5.0, &world);
    for d in distances(&squad, droid) {
        // Half the search radius, plus the stopping distance.
        assert!(d <= 384.0 + 60.0, "{d}");
    }
    assert!(squad.members.iter().all(|m| m.engaging == Some(0)));
}

#[test]
fn aimed_spot_finds_the_floor_in_front() {
    let world = test_room();
    let p = MoveParams::player_commando();
    let eye = Vec3::new(0.0, 0.0, 144.0);
    // Looking down at 20 degrees hits the floor about 400 ahead.
    let dir = Vec3::new(20f32.to_radians().cos(), 0.0, -20f32.to_radians().sin());
    let spot = aimed_spot(&world, eye, dir, 4000.0, &p).unwrap();
    assert!(spot.z.abs() < 1.0, "{spot:?}");
    assert!((300.0..420.0).contains(&spot.x), "{spot:?}");
    // Looking up into the open sky meets nothing.
    assert!(aimed_spot(&world, eye, Vec3::new(0.0, 0.0, 1.0), 4000.0, &p).is_none());
}

#[test]
fn a_downed_member_stays_put_and_skips_orders() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    let fell_at = squad.members[0].pawn.feet();
    let ev = squad.set_down(0, true);
    assert!(matches!(ev, Some(SquadEvent::Down { .. })), "{ev:?}");
    assert_eq!(squad.set_down(0, true), None);
    // The leader walks off and orders a form up: the others follow, Sev
    // doesn't.
    let away = leader_at(1000.0, 0.0, 0.0);
    squad.command(Order::FormUp, away, &world);
    run(&mut squad, away, 6.0, &world);
    let d = distances(&squad, away.feet);
    assert!(d[1] < 300.0 && d[2] < 300.0, "{d:?}");
    let moved = (squad.members[0].pawn.feet() - fell_at).length();
    assert!(moved < 1.0, "Sev moved {moved}");
    assert!(squad.status(away)[0].down);
    // Back up: Sev catches up.
    assert!(matches!(
        squad.set_down(0, false),
        Some(SquadEvent::Revived { .. })
    ));
    run(&mut squad, away, 6.0, &world);
    let d = distances(&squad, away.feet);
    assert!(d[0] < 300.0, "{d:?}");
}
