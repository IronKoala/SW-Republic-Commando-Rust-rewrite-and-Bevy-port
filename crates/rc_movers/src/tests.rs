use super::*;
use rc_level::actors::Placement;
use rc_level::movers::{Sounds, MAX_KEYS};
use rc_movement::{MoveInput, MoveParams, Mode};

const DT: f32 = 1.0 / 60.0;

/// The 12 triangles of a box.
fn box_tris(lo: Vec3, hi: Vec3) -> Vec<[Vec3; 3]> {
    let c = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let faces = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    faces
        .iter()
        .flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
        .collect()
}

fn floor() -> TriangleWorld {
    TriangleWorld::new(box_tris(Vec3::new(-3000.0, -3000.0, -20.0), Vec3::new(3000.0, 3000.0, 0.0)))
}

fn mover(state: &str, keys: &[[f32; 3]], move_time: f32, stay: f32) -> MoverInfo {
    let mut key_pos = [[0.0; 3]; MAX_KEYS];
    for (k, p) in keys.iter().enumerate() {
        key_pos[k] = *p;
    }
    MoverInfo {
        place: Placement {
            export: 7,
            class: "Mover".into(),
            name: "Mover0".into(),
            location: [0.0; 3],
            rotation: [0; 3],
            draw_scale: 1.0,
            draw_scale_3d: [1.0; 3],
            pre_pivot: [0.0; 3],
            static_mesh: 0,
            skins: vec![],
            hidden: false,
            static_mesh_instance: 0,
        },
        chain: vec!["Mover".into()],
        base_pos: [0.0; 3],
        base_rot: [0; 3],
        key_pos,
        key_rot: [[0; 3]; MAX_KEYS],
        num_keys: keys.len() as u8,
        key_num: 0,
        move_times: [move_time; MAX_KEYS],
        stay_open_time: stay,
        delay_time: 0.0,
        glide: Glide::Smooth,
        encroach: Encroach::Return,
        state: state.into(),
        tag: "Door1".into(),
        opening_event: String::new(),
        opened_event: String::new(),
        closing_event: String::new(),
        closed_event: String::new(),
        auto_door: false,
        trigger_once_only: false,
        sounds: Sounds {
            opening: Some("Sounds.Door.Open".into()),
            opened: None,
            closing: Some("Sounds.Door.Close".into()),
            closed: None,
            moving: None,
        },
        defaulted: vec![],
    }
}

fn set_of(info: MoverInfo, tris: Vec<[Vec3; 3]>) -> MoverSet {
    let map = MapMovers {
        movers: vec![info],
        triggers: vec![],
        failures: vec![],
    };
    MoverSet::new(map, Settings::default(), move |_| tris.clone())
}

fn far_player() -> Pawn {
    Pawn::new(MoveParams::player_commando(), Vec3::new(2000.0, 2000.0, 0.0))
}

#[test]
fn triggered_door_opens_waits_and_closes_in_the_map_times() {
    let door = box_tris(Vec3::new(-10.0, -100.0, 0.0), Vec3::new(10.0, 100.0, 200.0));
    let mut set = set_of(mover("TriggerOpenTimed", &[[0.0; 3], [0.0, 0.0, 190.0]], 1.5, 2.0), door);
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    assert_eq!(set.send("door1", &mut ev), 1);
    assert!(ev.contains(&MoverEvent::Sound { mover: 0, sound: "Sounds.Door.Open".into() }));
    let mut opened_at = None;
    let mut closed = None;
    for f in 1..=600 {
        for e in set.step(DT, &fixed, &mut p) {
            match e {
                MoverEvent::Opened { secs, .. } => opened_at = Some((f as f32 * DT, secs)),
                MoverEvent::Closed { secs, .. } => closed = Some((f as f32 * DT, secs)),
                _ => {}
            }
        }
    }
    let (at, secs) = opened_at.expect("opened");
    assert!((secs - 1.5).abs() < 0.05, "open took {secs}");
    let (cat, csecs) = closed.expect("closed");
    assert!((csecs - 1.5).abs() < 0.05, "close took {csecs}");
    // Waits StayOpenTime between.
    assert!((cat - at - 1.5 - 2.0).abs() < 0.05, "{at} {cat}");
    assert!(set.movers[0].is_closed());
}

#[test]
fn smooth_glide_is_halfway_at_half_time_and_slow_at_the_start() {
    let mut set = set_of(mover("TriggerToggle", &[[0.0; 3], [0.0, 0.0, 100.0]], 1.0, 0.0), vec![]);
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    set.send("Door1", &mut ev);
    for _ in 0..15 {
        set.step(DT, &fixed, &mut p);
    }
    // a = 0.25: 3a^2 - 2a^3 = 0.15625
    assert!((set.movers[0].pose.pos.z - 15.625).abs() < 0.5, "{}", set.movers[0].pose.pos.z);
    for _ in 0..15 {
        set.step(DT, &fixed, &mut p);
    }
    assert!((set.movers[0].pose.pos.z - 50.0).abs() < 0.5);
}

#[test]
fn multi_key_mover_takes_move_time_per_key() {
    let keys = [[0.0; 3], [0.0, 0.0, 100.0], [0.0, 100.0, 100.0]];
    let mut set = set_of(mover("TriggerToggle", &keys, 0.5, 0.0), vec![]);
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    set.send("Door1", &mut ev);
    let mut secs = None;
    for _ in 0..120 {
        for e in set.step(DT, &fixed, &mut p) {
            if let MoverEvent::Opened { secs: s, .. } = e {
                secs = Some(s);
            }
        }
    }
    assert!((secs.unwrap() - 1.0).abs() < 0.05);
    assert_eq!(set.movers[0].pose.pos, Vec3::new(0.0, 100.0, 100.0));
    // Toggle: a second trigger takes it back.
    set.send("Door1", &mut ev);
    for _ in 0..120 {
        set.step(DT, &fixed, &mut p);
    }
    assert_eq!(set.movers[0].pose.pos, Vec3::ZERO);
}

#[test]
fn lift_carries_the_player_up_and_back_down() {
    // A 300 x 300 platform, top at z = 10, rising 400 in 2 s.
    let plat = box_tris(Vec3::new(-150.0, -150.0, 0.0), Vec3::new(150.0, 150.0, 10.0));
    let mut set = set_of(mover("StandOpenTimed", &[[0.0; 3], [0.0, 0.0, 400.0]], 2.0, 1.0), plat);
    let fixed = floor();
    assert!(set.movers[0].is_lift(40.0));
    let spot = set.movers[0].top_spot(Vec3::new(40.0, 40.0, 84.0)).unwrap();
    assert!((spot.z - 10.0).abs() < 0.5 && spot.x.abs() < 0.5, "{spot:?}");
    let mut p = Pawn::new(MoveParams::player_commando(), Vec3::new(0.0, 0.0, 10.5));
    let input = MoveInput::default();
    let mut top = f32::MIN;
    let mut boarded = false;
    for _ in 0..(60 * 8) {
        for e in set.step(DT, &fixed, &mut p) {
            boarded |= matches!(e, MoverEvent::Boarded { mover: 0 });
        }
        let w = set.world(&fixed);
        p.step(DT, &input, &w);
        top = top.max(p.feet().z);
        let lift_top = set.movers[0].pose.pos.z + 10.0;
        // Never sinks into the platform, never floats far above it.
        assert!(p.feet().z > lift_top - 1.0, "feet {} lift {}", p.feet().z, lift_top);
    }
    assert!(boarded);
    assert!((top - 410.0).abs() < 3.0, "top {top}");
    // Back down after StayOpenTime, still on it.
    assert!((p.feet().z - 10.0).abs() < 3.0, "feet {}", p.feet().z);
    assert_eq!(p.mode, Mode::Walking);
    assert!(set.movers[0].stats.carried_frames > 200);
}

#[test]
fn closing_door_goes_back_when_the_player_is_in_the_way() {
    // A door that slides down from above (open) to the floor (closed).
    let door = box_tris(Vec3::new(-10.0, -100.0, 0.0), Vec3::new(10.0, 100.0, 200.0));
    let mut info = mover("TriggerOpenTimed", &[[0.0; 3], [0.0, 0.0, 250.0]], 1.0, 0.5);
    info.encroach = Encroach::Return;
    let mut set = set_of(info, door);
    let fixed = floor();
    let mut ev = Vec::new();
    set.send("Door1", &mut ev);
    let mut p = far_player();
    for _ in 0..60 {
        set.step(DT, &fixed, &mut p);
    }
    // Stand in the doorway (pushing straight down is impossible: the floor).
    p = Pawn::new(MoveParams::player_commando(), Vec3::new(0.0, 0.0, 0.5));
    let mut blocked = 0;
    for _ in 0..120 {
        for e in set.step(DT, &fixed, &mut p) {
            blocked += matches!(e, MoverEvent::Blocked { .. }) as u32;
        }
    }
    assert!(blocked >= 1);
    // It never got down into the player.
    assert!(set.movers[0].pose.pos.z > p.location.z + p.extent().z - 1.0);
}

#[test]
fn bump_door_opens_when_walked_into() {
    let door = box_tris(Vec3::new(-10.0, -100.0, 0.0), Vec3::new(10.0, 100.0, 200.0));
    let mut set = set_of(mover("BumpOpenTimed", &[[0.0; 3], [0.0, 0.0, 190.0]], 1.0, 2.0), door);
    let fixed = floor();
    let mut p = Pawn::new(MoveParams::player_commando(), Vec3::new(-200.0, 0.0, 0.5));
    let input = MoveInput { forward: 1.0, ..Default::default() };
    let mut passed = false;
    for _ in 0..(60 * 4) {
        set.step(DT, &fixed, &mut p);
        let w = set.world(&fixed);
        p.step(DT, &input, &w);
        passed |= p.location.x > 60.0;
    }
    assert!(set.movers[0].stats.opened >= 1);
    assert!(passed, "player x {}", p.location.x);
}

#[test]
fn turning_mover_moves_points_with_it() {
    let mut info = mover("TriggerToggle", &[[0.0; 3], [0.0; 3]], 1.0, 0.0);
    info.key_rot[1] = [0, 16384, 0];
    let m = Mover::new(info, vec![], &Settings::default());
    let a = key_pose(&m.info, 0);
    let b = key_pose(&m.info, 1);
    let mo = Motion::new(a, b);
    let p = mo.apply(Vec3::new(100.0, 0.0, 5.0));
    assert!((p.x).abs() < 1e-3 && (p.y - 100.0).abs() < 1e-3 && (p.z - 5.0).abs() < 1e-3, "{p:?}");
    let back = mo.undo(p);
    assert!((back.x - 100.0).abs() < 1e-3);
    let (mat, shift) = mo.matrix();
    let q = Vec3::new(
        mat[0][0] * 100.0 + mat[0][2] * 5.0 + shift.x,
        mat[1][0] * 100.0 + mat[1][2] * 5.0 + shift.y,
        mat[2][0] * 100.0 + mat[2][2] * 5.0 + shift.z,
    );
    assert!((q.y - 100.0).abs() < 1e-3);
}

#[test]
fn per_key_times_and_chained_events() {
    // Two keys' worth of moves with different times, and a second mover
    // that the first sets off when it's open.
    let keys = [[0.0; 3], [0.0, 0.0, 100.0], [0.0, 0.0, 300.0]];
    let mut a = mover("TriggerToggle", &keys, 0.5, 0.0);
    a.move_times[1] = 1.5;
    a.opened_event = "Lift2".into();
    let mut b = mover("TriggerToggle", &[[0.0; 3], [100.0, 0.0, 0.0]], 1.0, 0.0);
    b.tag = "Lift2".into();
    b.place.export = 8;
    assert!((a.open_time() - 2.0).abs() < 1e-6);
    let map = MapMovers { movers: vec![a, b], triggers: vec![], failures: vec![] };
    let mut set = MoverSet::new(map, Settings::default(), |_| Vec::new());
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    set.send("Door1", &mut ev);
    let (mut a_open, mut b_open) = (None, None);
    for f in 1..=240 {
        for e in set.step(DT, &fixed, &mut p) {
            if let MoverEvent::Opened { mover, secs } = e {
                if mover == 0 { a_open = Some(secs) } else { b_open = Some(f as f32 * DT) }
            }
        }
        if f == 30 {
            // Half a second in: at key 1.
            assert!((set.movers[0].pose.pos.z - 100.0).abs() < 1.0);
        }
    }
    assert!((a_open.unwrap() - 2.0).abs() < 0.05);
    // B starts when A is open (2 s) and takes 1 s.
    assert!((b_open.unwrap() - 3.0).abs() < 0.05, "{b_open:?}");
}

#[test]
fn trigger_loop_runs_until_triggered_again() {
    let mut set = set_of(mover("TriggerLoop", &[[0.0; 3], [0.0, 0.0, 100.0]], 0.5, 0.0), vec![]);
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    set.send("Door1", &mut ev);
    for _ in 0..(60 * 3) {
        set.step(DT, &fixed, &mut p);
    }
    assert!(set.movers[0].stats.opened >= 2);
    set.send("Door1", &mut ev);
    for _ in 0..(60 * 3) {
        set.step(DT, &fixed, &mut p);
    }
    assert!(set.movers[0].is_closed());
}

#[test]
fn snapshot_restores_a_door_half_open_and_it_carries_on_the_same() {
    let door = box_tris(Vec3::new(-10.0, -100.0, 0.0), Vec3::new(10.0, 100.0, 200.0));
    let info = mover("TriggerOpenTimed", &[[0.0; 3], [0.0, 0.0, 190.0]], 1.5, 2.0);
    let mut a = set_of(info.clone(), door.clone());
    let fixed = floor();
    let mut p = far_player();
    let mut ev = Vec::new();
    a.send("door1", &mut ev);
    for _ in 0..45 {
        a.step(DT, &fixed, &mut p);
    }
    let snap = a.snapshot();
    assert!(matches!(snap.movers[0].phase, Phase::Moving { .. }), "{snap:?}");

    // A fresh map, as after loading: the door is shut until restored.
    let mut b = set_of(info, door);
    assert!(b.movers[0].is_closed());
    let report = b.restore(&snap);
    assert_eq!(report.movers, 1);
    assert!(report.unknown.is_empty() && report.bad_keys.is_empty(), "{report:?}");
    assert_eq!(b.snapshot(), snap);
    assert!((b.movers[0].pose.pos.z - a.movers[0].pose.pos.z).abs() < 1e-3);

    // Both carry on the same way: open, wait, close.
    let mut q = far_player();
    for _ in 0..400 {
        let ea = a.step(DT, &fixed, &mut p);
        let eb = b.step(DT, &fixed, &mut q);
        assert_eq!(ea, eb);
        assert!((a.movers[0].pose.pos.z - b.movers[0].pose.pos.z).abs() < 1e-3);
    }
    assert_eq!(b.movers[0].stats.closed, 1);
    assert!(b.movers[0].is_closed());
}

#[test]
fn restore_skips_names_the_map_does_not_have() {
    let mut set = set_of(mover("TriggerToggle", &[[0.0; 3], [0.0, 0.0, 100.0]], 1.0, 0.0), vec![]);
    let mut snap = set.snapshot();
    snap.movers[0].name = "Mover99".into();
    snap.movers[0].key = 1;
    let report = set.restore(&snap);
    assert_eq!(report.unknown, vec!["Mover99".to_string()]);
    assert_eq!(set.movers[0].key, 0);
    // A key past the last one is clamped.
    let mut snap = set.snapshot();
    snap.movers[0].key = 9;
    let report = set.restore(&snap);
    assert_eq!(report.bad_keys, vec!["Mover0".to_string()]);
    assert_eq!(set.movers[0].key, 1);
    assert!((set.movers[0].pose.pos.z - 100.0).abs() < 1e-3);
}
