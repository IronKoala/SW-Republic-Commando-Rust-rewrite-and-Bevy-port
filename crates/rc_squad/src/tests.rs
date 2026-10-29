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

/// A battle droid standing at `feet` (80 uu half-height, eyes 60 above
/// its middle, as in data/enemies/battle_droid.toml).
fn threat(id: u64, feet: Vec3) -> Threat {
    Threat {
        id,
        feet,
        centre: feet + Vec3::new(0.0, 0.0, 80.0),
        eye: feet + Vec3::new(0.0, 0.0, 140.0),
    }
}

fn shots(events: &[SquadEvent]) -> Vec<(usize, Vec3, Vec3, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            SquadEvent::Fire {
                index,
                origin,
                direction,
                target,
                ..
            } => Some((*index, *origin, *direction, *target)),
            _ => None,
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
    squad.threats = vec![threat(1, droid)];
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
    // Nobody goes to revive him in this test.
    squad.settings.revive.range = 0.0;
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

#[test]
fn squad_mates_shoot_a_droid_they_can_see() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    // A droid ahead and to the right, clear of the leader's line.
    let droid = threat(7, Vec3::new(900.0, 600.0, 0.0));
    squad.threats = vec![droid];
    let mut events = Vec::new();
    let mut first_shot = None;
    let mut t = 0.0;
    while t < 4.0 {
        let ev = squad.update(DT, start, &world, &StraightLine);
        t += DT;
        if first_shot.is_none() && !shots(&ev).is_empty() {
            first_shot = Some(t);
        }
        events.extend(ev);
    }
    let fired = shots(&events);
    // Every member who isn't blocked by a friend shoots, and only at it.
    let shooters: std::collections::HashSet<usize> = fired.iter().map(|s| s.0).collect();
    assert!(shooters.len() >= 2, "shooters {shooters:?}");
    assert!(fired.iter().all(|s| s.3 == 7));
    // Not before the reaction time (plus up to one think).
    let first = first_shot.unwrap();
    assert!((0.5..0.85).contains(&first), "first shot at {first}");
    // Bursts of 5 at 0.1 s, a 0.6-1.2 s rest: 4 s gives roughly 2-4 bursts each.
    for i in &shooters {
        let n = fired.iter().filter(|s| s.0 == *i).count();
        assert!((8..=25).contains(&n), "member {i} fired {n}");
    }
    // Aimed at the droid's middle, off by at most (1 - 0.9) x 10 degrees.
    for (_, origin, dir, _) in &fired {
        let want = (droid.centre - *origin).normalize_or_zero();
        let angle = want.dot(*dir).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(angle <= 1.0 * 1.2 + 0.1, "{angle} deg off");
    }
}

#[test]
fn squad_mates_ignore_enemies_out_of_sight_range() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    squad.threats = vec![threat(1, Vec3::new(3900.0, 1900.0, 6000.0))];
    let events = run(&mut squad, start, 2.0, &world);
    assert!(shots(&events).is_empty());
}

#[test]
fn nobody_shoots_through_the_player() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    // Scorch stands straight behind the leader; the droid is straight ahead.
    let scorch = squad.members[2].pawn.feet();
    assert!(scorch.y.abs() < 5.0 && scorch.x < -100.0, "{scorch:?}");
    squad.threats = vec![threat(3, Vec3::new(1500.0, 0.0, 0.0))];
    let events = run(&mut squad, start, 3.0, &world);
    assert!(
        shots(&events).iter().all(|s| s.0 != 2),
        "Scorch fired through the player"
    );
    assert!(squad.members[2].fight.held_fire > 0);
    assert!(events
        .iter()
        .any(|e| matches!(e, SquadEvent::HeldFire { friend, .. } if friend == "the player")));
}

#[test]
fn a_downed_squad_mate_is_revived_by_the_nearest_other() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    squad.set_down(0, true);
    let mut done_at = None;
    let mut reviver = String::new();
    let mut t = 0.0;
    while t < 10.0 && done_at.is_none() {
        for e in squad.update(DT, start, &world, &StraightLine) {
            match e {
                SquadEvent::ReviveSetOff { member, .. } => reviver = member,
                SquadEvent::ReviveDone {
                    patient: Patient::Mate(0),
                    member,
                    ..
                } => {
                    assert_eq!(member, reviver);
                    done_at = Some(t);
                }
                _ => {}
            }
        }
        t += DT;
    }
    // Scorch stands 121 uu from Sev (Fixer 220): a short walk plus the 3 s
    // revive.
    assert_eq!(reviver, "Scorch");
    let t = done_at.expect("Sev was never revived");
    assert!((3.0..6.0).contains(&t), "revived after {t} s");
    // The game gives the health back; then he's up and the reviver free.
    assert!(matches!(
        squad.set_down(0, false),
        Some(SquadEvent::Revived { .. })
    ));
    run(&mut squad, start, 0.5, &world);
    assert!(squad.members.iter().all(|m| m.revive.is_none()));
}

#[test]
fn squad_mates_revive_the_player() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    run(&mut squad, start, 0.5, &world);
    squad.leader_down = true;
    let mut events = Vec::new();
    let mut t = 0.0;
    while t < 6.0 {
        let ev = squad.update(DT, start, &world, &StraightLine);
        // The game gives the player's health back.
        if ev.iter().any(|e| {
            matches!(
                e,
                SquadEvent::ReviveDone {
                    patient: Patient::Leader,
                    ..
                }
            )
        }) {
            squad.leader_down = false;
        }
        events.extend(ev);
        t += DT;
    }
    let done: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                SquadEvent::ReviveDone {
                    patient: Patient::Leader,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(done.len(), 1, "{events:?}");
    // Only one goes.
    let set_off = events
        .iter()
        .filter(|e| matches!(e, SquadEvent::ReviveSetOff { .. }))
        .count();
    assert_eq!(set_off, 1);
}

#[test]
fn the_player_revives_a_squad_mate_by_holding_use() {
    let world = test_room();
    let start = leader_at(0.0, 0.0, 0.0);
    let mut squad = new_squad(&world, start);
    squad.settings.revive.range = 0.0; // only the player revives here
    run(&mut squad, start, 0.5, &world);
    squad.set_down(2, true);
    let body = squad.members[2].pawn.feet();
    // Too far: nothing.
    assert!(squad.player_use(1.0, start, true).is_empty());
    let beside = Leader {
        feet: body + Vec3::new(60.0, 0.0, 0.0),
        yaw: 0.0,
    };
    // Let go after 1 s: it stops and starts over.
    let mut ev = Vec::new();
    for _ in 0..60 {
        ev.extend(squad.player_use(DT, beside, true));
    }
    ev.extend(squad.player_use(DT, beside, false));
    assert!(matches!(
        ev.first(),
        Some(SquadEvent::PlayerReviveStarted { .. })
    ));
    assert!(matches!(
        ev.last(),
        Some(SquadEvent::PlayerReviveStopped { .. })
    ));
    // Hold for the full 3 s.
    let mut t = 0.0;
    let mut done = None;
    while t < 4.0 && done.is_none() {
        for e in squad.player_use(DT, beside, true) {
            if let SquadEvent::ReviveDone { patient, .. } = e {
                done = Some((patient, t));
            }
        }
        t += DT;
    }
    let (patient, t) = done.expect("not revived");
    assert_eq!(patient, Patient::Mate(2));
    assert!((2.9..3.1).contains(&t), "{t}");
}

#[test]
fn revive_and_fight_settings_read_from_the_file() {
    let s = SquadSettings::default()
        .with_overrides("[revive]\ntime = 5\nrevive_player = \"no\"\n[fight]\nweapon = \"dc15s_pistol\"\nmax_aim_yaw = 30\n")
        .unwrap();
    assert_eq!(s.revive.time, 5.0);
    assert!(!s.revive.revive_player);
    assert_eq!(s.fight.weapon, "dc15s_pistol");
    assert!((s.fight.max_aim_yaw - 30f32.to_radians()).abs() < 1e-6);
    let e = SquadSettings::default()
        .with_overrides("[revive]\nrevive_player = \"maybe\"\n")
        .unwrap_err();
    assert!(e.contains("line 2"), "{e}");
}
