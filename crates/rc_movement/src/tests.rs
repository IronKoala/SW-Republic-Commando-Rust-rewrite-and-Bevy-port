//! Movement checked against the numbers worked out from the game's values.

use super::*;
use crate::room::*;

const DT: f32 = 1.0 / 60.0;

fn player_at(feet: Vec3) -> Pawn {
    let mut p = Pawn::new(MoveParams::player_commando(), feet);
    // Settle onto the floor.
    let w = test_room();
    p.step(0.2, &MoveInput::default(), &w);
    assert_eq!(p.mode, Mode::Walking, "should be standing at {feet:?}");
    p
}

/// Run `input` for `secs`, returning every event.
fn run(p: &mut Pawn, w: &dyn World, input: MoveInput, secs: f32) -> Vec<Event> {
    let mut ev = Vec::new();
    let n = (secs / DT).round() as usize;
    for _ in 0..n {
        ev.extend(p.step(DT, &input, w));
    }
    ev
}

fn forward() -> MoveInput {
    MoveInput { forward: 1.0, ..Default::default() }
}

#[test]
fn standing_height_and_eye() {
    let p = player_at(Vec3::ZERO);
    assert!((p.feet().z - 0.1).abs() < 0.05, "feet {}", p.feet().z);
    // Eye 84 + 60 = 144 above the feet.
    assert!((p.eye().z - p.feet().z - 144.0).abs() < 0.01);
}

#[test]
fn run_speed_and_time_to_full_speed() {
    let w = test_room();
    let mut p = player_at(Vec3::ZERO);
    let mut t = 0.0;
    while p.velocity.length_2d() < 449.0 {
        p.step(DT, &forward(), &w);
        t += DT;
        assert!(t < 2.0);
    }
    run(&mut p, &w, forward(), 0.5);
    assert!((p.velocity.length_2d() - 450.0).abs() < 0.5, "{}", p.velocity.length_2d());
    // 1024 uu/s² gets to 450 in about 0.44 s.
    assert!(t > 0.40 && t < 0.50, "took {t}");
}

#[test]
fn walk_crouch_strafe_back_speeds() {
    let w = test_room();
    let cases = [
        (MoveInput { forward: 1.0, walk: true, ..Default::default() }, 225.0),
        (MoveInput { forward: 1.0, crouch: true, ..Default::default() }, 225.0),
        (MoveInput { forward: 1.0, crouch: true, walk: true, ..Default::default() }, 112.5),
        (MoveInput { strafe: 1.0, ..Default::default() }, 337.5),
        (MoveInput { forward: -1.0, ..Default::default() }, 270.0),
    ];
    for (input, want) in cases {
        let mut p = player_at(Vec3::ZERO);
        run(&mut p, &w, input, 1.0);
        let got = p.velocity.length_2d();
        assert!((got - want).abs() < 0.5, "{input:?}: {got}, want {want}");
    }
}

#[test]
fn stopping() {
    let w = test_room();
    let mut p = player_at(Vec3::ZERO);
    run(&mut p, &w, forward(), 1.0);
    let x0 = p.location.x;
    let mut t = 0.0;
    while p.velocity.length_2d() > 10.0 {
        p.step(DT, &MoveInput::default(), &w);
        t += DT;
        assert!(t < 2.0);
    }
    let slide = p.location.x - x0;
    // Friction 8 brings a 450 run under 10 uu/s in about a quarter of a
    // second, sliding about 28 units (450 / 16).
    assert!(t < 0.3 && slide > 15.0 && slide < 35.0, "t {t}, slide {slide}");
}

#[test]
fn jump_height_and_air_time() {
    let w = test_room();
    let mut p = player_at(Vec3::ZERO);
    let z0 = p.feet().z;
    let mut peak: f32 = 0.0;
    let mut t = 0.0;
    let jump = MoveInput { jump: true, ..Default::default() };
    let ev = p.step(DT, &jump, &w);
    assert!(matches!(ev[0], Event::Jumped { .. }));
    t += DT;
    loop {
        let ev = p.step(DT, &jump, &w);
        t += DT;
        peak = peak.max(p.feet().z - z0);
        if ev.iter().any(|e| matches!(e, Event::Landed { .. })) {
            break;
        }
        assert!(t < 2.0);
    }
    // 475² / (2 * 1100) = 102.6 uu up; 2 * 475 / 1100 = 0.864 s.
    assert!((peak - 102.6).abs() < 1.0, "peak {peak}");
    assert!((t - 0.864).abs() < 0.03, "air time {t}");
    // Holding the key doesn't jump again.
    let ev = run(&mut p, &w, jump, 0.5);
    assert!(!ev.iter().any(|e| matches!(e, Event::Jumped { .. })));
}

#[test]
fn step_30_climbs_step_40_blocks() {
    let w = test_room();
    let mut p = player_at(Vec3::new(700.0, LANE_STEP_30, 0.0));
    let ev = run(&mut p, &w, forward(), 1.5);
    assert!(ev.iter().any(|e| matches!(e, Event::SteppedUp { .. })), "{ev:?}");
    assert!((p.feet().z - 30.0).abs() < 0.5, "feet {}", p.feet().z);
    assert!(p.location.x > LANES_X + 100.0);

    let mut p = player_at(Vec3::new(700.0, LANE_STEP_40, 0.0));
    run(&mut p, &w, forward(), 1.5);
    assert!(p.feet().z < 1.0, "feet {}", p.feet().z);
    // Stopped against the step: front of the box at the step's face.
    assert!((p.location.x + 40.0 - LANES_X).abs() < 1.0, "x {}", p.location.x);
    assert!(p.velocity.length_2d() < 1.0);
}

#[test]
fn stairs() {
    let w = test_room();
    let mut p = player_at(Vec3::new(700.0, LANE_STAIRS, 0.0));
    run(&mut p, &w, forward(), 2.0);
    assert!((p.feet().z - 128.0).abs() < 0.5, "feet {}", p.feet().z);
    assert_eq!(p.mode, Mode::Walking);
}

#[test]
fn ramps() {
    let w = test_room();
    let mut p = player_at(Vec3::new(700.0, LANE_RAMP_30, 0.0));
    // 1.6 s is enough to reach the top but not to run off its far end.
    run(&mut p, &w, forward(), 1.6);
    assert!((p.feet().z - 200.0).abs() < 0.5, "30°: feet {}", p.feet().z);

    let mut p = player_at(Vec3::new(700.0, LANE_RAMP_50, 0.0));
    run(&mut p, &w, forward(), 3.0);
    assert!(p.feet().z < 36.0, "50°: feet {}", p.feet().z);
}

#[test]
fn crouch_under_ceiling() {
    let w = test_room();
    // Standing: blocked by the ceiling's edge.
    let mut p = player_at(Vec3::new(700.0, LANE_CEILING, 0.0));
    run(&mut p, &w, forward(), 1.5);
    assert!(p.location.x + 40.0 <= LANES_X + 0.5, "x {}", p.location.x);

    // Crouched: goes under. Letting go underneath keeps it crouched.
    let crouch = MoveInput { forward: 1.0, crouch: true, ..Default::default() };
    let mut p = player_at(Vec3::new(700.0, LANE_CEILING, 0.0));
    run(&mut p, &w, crouch, 1.8);
    assert!(p.location.x > LANES_X + 50.0 && p.location.x < LANES_X + 400.0, "x {}", p.location.x);
    let ev = run(&mut p, &w, MoveInput::default(), 0.2);
    assert!(p.crouched);
    assert!(ev.contains(&Event::CantStand));
    // Eye eases down to 44.8 above the centre.
    run(&mut p, &w, MoveInput { crouch: true, ..Default::default() }, 1.0);
    assert!((p.eye_height - 44.8).abs() < 0.1, "eye {}", p.eye_height);
    // Out from under, it stands.
    let ev = run(&mut p, &w, forward(), 2.0);
    assert!(ev.contains(&Event::StoodUp));
    assert!(!p.crouched);
}

#[test]
fn fall_from_tower() {
    let w = test_room();
    let mut p = player_at(TOWER_TOP + Vec3::new(150.0, 0.0, 0.0));
    let ev = run(&mut p, &w, forward(), 3.0);
    let landed = ev.iter().find_map(|e| match e {
        Event::Landed { speed, damage, .. } => Some((*speed, *damage)),
        _ => None,
    });
    let (speed, damage) = landed.expect("should land");
    // Dropping 700 under gravity 1100: sqrt(2 * 1100 * 700) = 1241 uu/s,
    // just over the 1200 that hurts: 100 * 41 / 1200 = 3.4 damage.
    assert!((speed - 1241.0).abs() < 2.0, "speed {speed}");
    assert!((damage - 3.4).abs() < 0.3, "damage {damage}");
}


#[test]
fn shipped_settings_file_matches_built_in_values() {
    let text = include_str!("../../../data/movement/player_commando.toml");
    let built_in = MoveParams::player_commando();
    // Start from zeros, so every value must come from the file.
    let mut zero = built_in.clone();
    for (name, _) in built_in.fields() {
        zero = zero.with_overrides(&format!("{name} = 0")).unwrap();
    }
    let from_file = zero.with_overrides(text).unwrap();
    assert_eq!(from_file, built_in);
}

#[test]
fn full_jump_while_pushing_into_a_wall() {
    // Found on geo_01a: jumping while pressed against a wall at a shallow
    // angle used to stall the rise (peak 87 instead of 103), because the
    // slide along the wall kept "hitting" the same wall. The wall here is
    // the outer +Y wall of the test room, pushed into at about 5 degrees.
    let w = test_room();
    let mut p = player_at(Vec3::new(0.0, 1955.0, 0.0));
    let input = MoveInput { forward: 1.0, yaw: 5f32.to_radians(), ..Default::default() };
    run(&mut p, &w, input, 0.5);
    let z0 = p.feet().z;
    let mut peak = z0;
    let jump = MoveInput { jump: true, ..input };
    for _ in 0..50 {
        p.step(DT, &jump, &w);
        peak = peak.max(p.feet().z);
    }
    assert!((peak - z0 - 102.6).abs() < 1.0, "peak {}", peak - z0);
}

/// Run forward for `secs` at `fps` frames a second.
fn run_at(p: &mut Pawn, w: &dyn World, input: MoveInput, secs: f32, fps: f32) {
    for _ in 0..(secs * fps).round() as usize {
        p.step(1.0 / fps, &input, w);
    }
}

/// Fast PCs run 240 frames a second or more, where each frame's move is a
/// unit or two. Ramps, stairs and steps must work the same as at 60:
/// before, the player slowed to a stop on ramps at 240.
#[test]
fn high_frame_rates_climb_like_60() {
    let w = test_room();
    for fps in [144.0, 240.0, 500.0, 1000.0] {
        let mut p = player_at(Vec3::new(700.0, LANE_RAMP_30, 0.0));
        run_at(&mut p, &w, forward(), 1.6, fps);
        assert!((p.feet().z - 200.0).abs() < 0.5, "{fps} fps, 30° ramp: feet {}", p.feet().z);

        let mut p = player_at(Vec3::new(700.0, LANE_STAIRS, 0.0));
        run_at(&mut p, &w, forward(), 2.0, fps);
        assert!((p.feet().z - 128.0).abs() < 0.5, "{fps} fps, stairs: feet {}", p.feet().z);

        let mut p = player_at(Vec3::new(700.0, LANE_STEP_30, 0.0));
        run_at(&mut p, &w, forward(), 1.5, fps);
        assert!((p.feet().z - 30.0).abs() < 0.5, "{fps} fps, step: feet {}", p.feet().z);
    }
}

/// A gentle slope in a level far from the middle of the map, run across at
/// an angle, like the first ramp in geo_01a: at 240 frames a second the
/// player kept full speed. Before, rounding left the box touching the slope
/// and it slowed to a crawl, or the slide along the slope turned it
/// downhill.
#[test]
fn slope_at_an_angle_keeps_speed_at_240() {
    // 16 degrees, rising along +X, at x = 4000, y = -8000.
    let (x0, y0) = (4000.0, -8000.0);
    let rise = 400.0;
    let run = rise / 16f32.to_radians().tan();
    let a = Vec3::new(x0, y0 - 2000.0, -2000.0);
    let b = Vec3::new(x0 + run, y0 - 2000.0, -2000.0 + rise);
    let c = Vec3::new(x0 + run, y0 + 2000.0, -2000.0 + rise);
    let d = Vec3::new(x0, y0 + 2000.0, -2000.0);
    let floor_lo = Vec3::new(x0 - 3000.0, y0 - 2000.0, -2000.0);
    let floor_hi = Vec3::new(x0 - 3000.0, y0 + 2000.0, -2000.0);
    let w = TriangleWorld::new(vec![[a, b, c], [a, c, d], [floor_lo, a, d], [floor_lo, d, floor_hi]]);
    let start = Vec3::new(x0 - 200.0, y0, -2000.0);
    // 30 degrees off straight up the slope.
    let input = MoveInput { forward: 1.0, yaw: 30f32.to_radians(), ..Default::default() };
    let mut p = Pawn::new(MoveParams::player_commando(), start);
    run_at(&mut p, &w, MoveInput::default(), 0.2, 240.0);
    run_at(&mut p, &w, input, 1.5, 240.0);
    let moved = (p.location - Vec3::new(start.x, start.y, p.location.z)).length_2d();
    // Full speed is 450 uu/s, reached in under half a second.
    assert!(moved > 550.0, "moved only {moved} in 1.5 s");
    let heading = (p.location.y - start.y).atan2(p.location.x - start.x).to_degrees();
    assert!((heading - 30.0).abs() < 3.0, "drifted to {heading} degrees");
    assert_eq!(p.mode, Mode::Walking);
}

/// A low lip (3 high) with a sloped front, too steep to stand on: at 240
/// frames a second each frame's move ended on the sloped front, so the
/// step up failed every frame and the player stopped dead.
#[test]
fn low_lip_with_sloped_front_at_240() {
    let mut tris = vec![];
    let quad = |t: &mut Vec<[Vec3; 3]>, p: [Vec3; 4]| {
        t.push([p[0], p[1], p[2]]);
        t.push([p[0], p[2], p[3]]);
    };
    let v = Vec3::new;
    // Floor to x = 100, a front sloping up to 3 high by x = 101, a top on.
    quad(&mut tris, [v(-1000.0, -500.0, 0.0), v(100.0, -500.0, 0.0), v(100.0, 500.0, 0.0), v(-1000.0, 500.0, 0.0)]);
    quad(&mut tris, [v(100.0, -500.0, 0.0), v(101.0, -500.0, 3.0), v(101.0, 500.0, 3.0), v(100.0, 500.0, 0.0)]);
    quad(&mut tris, [v(101.0, -500.0, 3.0), v(1000.0, -500.0, 3.0), v(1000.0, 500.0, 3.0), v(101.0, 500.0, 3.0)]);
    let w = TriangleWorld::new(tris);
    for fps in [60.0, 240.0, 1000.0] {
        let mut p = Pawn::new(MoveParams::player_commando(), v(0.0, 0.0, 0.0));
        run_at(&mut p, &w, MoveInput::default(), 0.2, fps);
        // Walk (not run) up to it, so each frame's move is short.
        let walk = MoveInput { forward: 1.0, walk: true, ..Default::default() };
        run_at(&mut p, &w, walk, 1.5, fps);
        assert!(p.location.x > 250.0, "{fps} fps: stopped at x = {}", p.location.x);
        assert!((p.feet().z - 3.0).abs() < 0.5, "{fps} fps: feet {}", p.feet().z);
    }
}
