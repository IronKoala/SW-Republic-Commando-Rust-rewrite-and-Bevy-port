//! The automatic test run (`--script`): a fixed list of moves that stands in
//! for the keyboard and mouse, with a check after each one. The checks use
//! numbers worked out by hand from the game's values (see docs/DESIGN.md).

use rc_movement::room::*;
use rc_movement::{Event, Mode, MoveInput, Pawn, Vec3};

/// What has happened since the last `Place`, for the checks.
#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub start_feet_z: f32,
    pub peak_feet_z: f32,
    pub events: Vec<Event>,
}

impl Stats {
    /// The hardest landing (spawning drops the player a hair onto the
    /// floor too, which also counts as a landing).
    fn landing(&self) -> Option<(f32, f32)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Event::Landed { speed, damage, .. } => Some((*speed, *damage)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
    }
}

type Measure = fn(&Pawn, &Stats) -> f32;

pub enum Step {
    /// Move the player: feet position and facing (radians).
    Place { feet: Vec3, yaw: f32 },
    /// Hold these keys for this long.
    Hold { secs: f32, input: MoveInput },
    /// Compare a measurement with what we expect.
    Check { name: &'static str, measure: Measure, want: f32, tolerance: f32 },
}

fn hold(secs: f32, input: MoveInput) -> Step {
    Step::Hold { secs, input }
}

fn check(name: &'static str, measure: Measure, want: f32, tolerance: f32) -> Step {
    Step::Check { name, measure, want, tolerance }
}

fn place(x: f32, y: f32) -> Step {
    Step::Place { feet: Vec3::new(x, y, 0.0), yaw: 0.0 }
}

fn keys(forward: f32, strafe: f32) -> MoveInput {
    MoveInput { forward, strafe, ..Default::default() }
}

fn speed(p: &Pawn, _: &Stats) -> f32 {
    p.velocity.length_2d()
}

fn feet_z(p: &Pawn, _: &Stats) -> f32 {
    p.feet().z
}

fn jump_height(_: &Pawn, s: &Stats) -> f32 {
    s.peak_feet_z - s.start_feet_z
}

fn yes(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

pub fn steps() -> Vec<Step> {
    let run = keys(1.0, 0.0);
    let idle = MoveInput::default();
    vec![
        // Open floor at the start.
        place(0.0, 0.0),
        hold(0.5, idle),
        check("standing eye above feet", |p, _| p.eye().z - p.feet().z, 144.0, 0.5),
        hold(1.0, run),
        check("run speed", speed, 450.0, 1.0),
        hold(0.3, idle),
        check("stopped after 0.3 s", speed, 0.0, 10.0),
        hold(1.0, MoveInput { forward: 1.0, walk: true, ..Default::default() }),
        check("walk speed (Shift)", speed, 225.0, 1.0),
        hold(1.0, keys(0.0, 1.0)),
        check("strafe speed", speed, 337.5, 1.0),
        hold(1.0, keys(-1.0, 0.0)),
        check("backwards speed", speed, 270.0, 1.0),
        hold(1.0, MoveInput { forward: 1.0, crouch: true, ..Default::default() }),
        check("crouched speed", speed, 225.0, 1.0),
        check("crouched eye above centre", |p, _| p.eye_height, 44.8, 0.5),
        hold(0.5, idle),
        place(0.0, 0.0),
        hold(0.3, idle),
        hold(1.2, MoveInput { jump: true, ..Default::default() }),
        check("jump height", jump_height, 102.6, 1.5),
        check("back on the ground", |p, _| yes(p.mode == Mode::Walking), 1.0, 0.0),
        // Lanes. Each starts 300 short of its obstacle.
        place(700.0, LANE_STEP_30),
        hold(1.5, run),
        check("30-high step: feet on top", feet_z, 30.0, 0.5),
        place(700.0, LANE_STEP_40),
        hold(1.5, run),
        check("40-high step: still on floor", feet_z, 0.0, 0.5),
        hold(0.6, MoveInput { forward: 1.0, jump: true, ..Default::default() }),
        hold(0.6, run),
        check("40-high step: on top after a jump", feet_z, 40.0, 0.5),
        place(700.0, LANE_STAIRS),
        hold(2.0, run),
        check("stairs: feet on landing", feet_z, 128.0, 0.5),
        place(700.0, LANE_RAMP_30),
        hold(1.6, run),
        check("30 degree ramp: feet on top", feet_z, 200.0, 0.5),
        place(700.0, LANE_RAMP_50),
        hold(3.0, run),
        check("50 degree ramp: can't climb", |p, _| yes(p.feet().z < 35.0), 1.0, 0.0),
        place(700.0, LANE_CEILING),
        hold(1.5, run),
        check("low ceiling: blocked standing", |p, _| p.location.x + 40.0, LANES_X, 0.5),
        hold(1.8, MoveInput { forward: 1.0, crouch: true, ..Default::default() }),
        hold(0.3, idle),
        check("low ceiling: stays crouched", |p, _| yes(p.crouched), 1.0, 0.0),
        hold(2.0, run),
        check("low ceiling: stands up after", |p, _| yes(!p.crouched), 1.0, 0.0),
        // Walk off the 700-high tower.
        Step::Place { feet: TOWER_TOP + Vec3::new(150.0, 0.0, 0.0), yaw: 0.0 },
        hold(2.5, run),
        check("700 fall: landing speed", |_, s| s.landing().map_or(0.0, |l| l.0), 1241.0, 2.0),
        check("700 fall: damage", |_, s| s.landing().map_or(-1.0, |l| l.1), 3.4, 0.3),
    ]
}
