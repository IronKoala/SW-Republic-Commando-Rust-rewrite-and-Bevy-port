//! Patrol routes: the list of points a character walks (`PatrolRoute`),
//! how it goes through them (`PatrolMode`) and how much it wants to
//! (`PatrolPriority`).
//!
//! From the original (read only; described in our own words):
//! - Engine.u `Pawn`: `struct PatrolPoint { Node, PauseTime, Animation,
//!   NumAnimLoops, Event, RunToNode, ShootWhileMoving, OrientToNode,
//!   MakeInvulnerable, PatrolPriorityOverride }`, `enum EPatrolMode {
//!   PM_Loop, PM_PingPong, PM_Random, PM_Once }`, and the class default
//!   `PatrolPriority = 0.3`.
//! - CTGame.u `PawnFactory.SetupNewActor`: a character made at the
//!   factory gets the factory's `PatrolRouteFromFactory`,
//!   `PatrolModeFromFactory` and `PatrolPriorityFromFactory` (default
//!   0.3); one made at a spawn point gets the spawn point's `PatrolRoute`,
//!   `PatrolMode` and `PatrolPriority`. A character placed in the map has
//!   its own.
//! - Which point comes next is decided in compiled code (ctgame.dll
//!   `StatePatrol`, decompiled for reference): see [`Patrol::next`].

use rc_level::scripts::{Val, Vals};

/// How a character goes through its patrol points (`EPatrolMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatrolMode {
    /// First to last, then from the first again.
    #[default]
    Loop,
    /// First to last, then back to the first, and so on.
    PingPong,
    /// A random point each time (never the last one: see [`Patrol::next`]).
    Random,
    /// First to last, once.
    Once,
}

impl PatrolMode {
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => PatrolMode::PingPong,
            2 => PatrolMode::Random,
            3 => PatrolMode::Once,
            _ => PatrolMode::Loop,
        }
    }
}

/// One point of a patrol route (`PatrolPoint`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatrolPoint {
    /// The navigation point to walk to, by actor name. `None`: no walk.
    pub node: Option<String>,
    /// Seconds to wait there.
    pub pause_time: f32,
    /// An animation to play there. `None`: none.
    pub animation: Option<String>,
    pub num_anim_loops: u32,
    /// Sent when it moves on from this point.
    pub event: Option<String>,
    pub run_to_node: bool,
    pub shoot_while_moving: bool,
    pub orient_to_node: bool,
    pub make_invulnerable: bool,
    /// Above 0: the character's patrol priority from here on.
    pub priority_override: f32,
}

/// A character's patrol: its route, mode and priority.
#[derive(Debug, Clone, PartialEq)]
pub struct Patrol {
    pub points: Vec<PatrolPoint>,
    pub mode: PatrolMode,
    /// How much it wants to patrol, against its other states' wishes.
    pub priority: f32,
}

/// `PatrolPriority`'s class default (Engine.u `Pawn`, and CTGame.u
/// `PawnFactory.PatrolPriorityFromFactory`).
pub const DEFAULT_PATROL_PRIORITY: f32 = 0.3;

fn name(v: Option<&Val>) -> Option<String> {
    match v {
        Some(Val::Text(t)) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => Some(t.clone()),
        _ => None,
    }
}

fn point(v: &Vals) -> PatrolPoint {
    let float = |k: &str| match v.get(k) {
        Some(Val::Float(x)) => *x,
        _ => 0.0,
    };
    let flag = |k: &str| matches!(v.get(k), Some(Val::Bool(true)));
    PatrolPoint {
        node: name(v.get("Node")),
        pause_time: float("PauseTime"),
        animation: name(v.get("Animation")),
        num_anim_loops: match v.get("NumAnimLoops") {
            Some(Val::Int(n)) => (*n).max(0) as u32,
            _ => 0,
        },
        event: name(v.get("Event")),
        run_to_node: flag("RunToNode"),
        shoot_while_moving: flag("ShootWhileMoving"),
        orient_to_node: flag("OrientToNode"),
        make_invulnerable: flag("MakeInvulnerable"),
        priority_override: float("PatrolPriorityOverride"),
    }
}

impl Patrol {
    /// Read a patrol from an actor's values: `route`, `mode` and
    /// `priority` are the property names (`PatrolRoute`, `PatrolMode`,
    /// `PatrolPriority` or the factory's `...FromFactory`). `None` when it
    /// has no route.
    pub fn from_values(values: &Vals, route: &str, mode: &str, priority: &str) -> Option<Patrol> {
        let points: Vec<PatrolPoint> = match values.get(route) {
            Some(Val::Structs(list)) => list.iter().map(point).collect(),
            _ => Vec::new(),
        };
        if points.is_empty() {
            return None;
        }
        Some(Patrol {
            points,
            mode: match values.get(mode) {
                Some(Val::Byte(b)) => PatrolMode::from_byte(*b),
                _ => PatrolMode::Loop,
            },
            priority: match values.get(priority) {
                Some(Val::Float(x)) => *x,
                _ => DEFAULT_PATROL_PRIORITY,
            },
        })
    }

    /// The point after `index` (ctgame.dll `StatePatrol`'s update, as
    /// decompiled): Loop adds one and wraps to 0; PingPong adds `*step`
    /// and turns round at either end; Random picks `random % (count - 1)`
    /// (0 when there is one point), so it never picks the last point;
    /// Once adds one (past the end means done). `random` is a fresh
    /// random number.
    pub fn next(&self, index: usize, step: &mut i32, random: u32) -> usize {
        let len = self.points.len();
        match self.mode {
            PatrolMode::Loop => {
                if index + 1 >= len {
                    0
                } else {
                    index + 1
                }
            }
            PatrolMode::PingPong => {
                if *step == 0 {
                    *step = 1;
                }
                let n = index as i32 + *step;
                if n < 0 || n >= len as i32 {
                    *step = -*step;
                    (index as i32 + *step).clamp(0, len.saturating_sub(1) as i32) as usize
                } else {
                    n as usize
                }
            }
            PatrolMode::Random => {
                if len <= 1 {
                    0
                } else {
                    random as usize % (len - 1)
                }
            }
            PatrolMode::Once => index + 1,
        }
    }
}
