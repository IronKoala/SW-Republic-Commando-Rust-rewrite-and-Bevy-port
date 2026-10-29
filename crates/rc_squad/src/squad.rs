//! The squad itself: three bodies that follow the leader and obey orders.
//!
//! Each squad mate is an `rc_movement::Pawn`, the same body and physics as
//! the player, steered by a `MoveInput` the way a player would press keys.
//! How they decide where to go is described in docs/DESIGN.md ("Delta
//! Squad"). In short: every order ("stance") ties each member to something
//! (the leader, the ordered spot, or where they stood), and they only set
//! off when they've drifted too far from it, then stop near it again. The
//! distances are the original game's. Where each one stands (its slot) is
//! our own choice.

use crate::map::MarkerSpot;
use crate::nav::Navigator;
use crate::settings::{MemberSettings, SquadSettings, StanceSettings, Tether};
use rc_movement::{axes, MoveInput, MoveParams, Pawn, Vec3, World};
use std::f32::consts::{PI, TAU};
use std::fmt;

/// The squad leader (the player), as the squad sees them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leader {
    pub feet: Vec3,
    /// Facing, radians (Unreal's: 0 is +X, a quarter turn is +Y).
    pub yaw: f32,
}

/// An order from the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Order {
    /// F2 in the original: follow me.
    FormUp,
    /// F3, aimed at a spot: go there and hold it.
    SecurePosition(Vec3),
    /// F1: spread out ahead and fight anything near.
    SearchAndDestroy,
    /// Stay where you are (no key in the PC version).
    Hold,
    /// F4: drop every job and form up.
    Cancel,
    /// Aimed at a marked spot: one squad mate goes and does that job.
    Marker(usize),
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Order::FormUp => write!(f, "form up"),
            Order::SecurePosition(p) => {
                write!(f, "secure position at ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z)
            }
            Order::SearchAndDestroy => write!(f, "search and destroy"),
            Order::Hold => write!(f, "hold"),
            Order::Cancel => write!(f, "cancel (recall)"),
            Order::Marker(i) => write!(f, "marker #{i}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    FormUp,
    SecurePosition,
    SearchAndDestroy,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemberState {
    /// Standing at (or near enough to) their spot.
    Idle,
    /// On the way. `urgent` means too far from what they're tied to: they
    /// run all the way. `by_order` means a direct order sent them, so they
    /// go all the way to their spot rather than stopping once close enough.
    Moving { urgent: bool, by_order: bool },
    /// Doing a marked job; seconds left.
    Working { left: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerState {
    Open,
    Assigned(usize),
    Done,
}

pub struct Member {
    pub settings: MemberSettings,
    pub pawn: Pawn,
    /// Facing, radians.
    pub yaw: f32,
    pub state: MemberState,
    /// Where they're heading (or standing).
    pub goal: Vec3,
    /// Where they stood when told to hold.
    pub hold_point: Vec3,
    /// The marked job they're on, if any.
    pub marker: Option<usize>,
    /// Threat they're moving to fight (search and destroy), if any.
    pub engaging: Option<usize>,
    /// Out of the fight (health at 0). Squad mates can't die in the
    /// original: they lie where they fell until revived. A downed member
    /// takes no orders and doesn't move on its own.
    pub down: bool,
    path: Vec<Vec3>,
    planned_for: Option<Vec3>,
    think_in: f32,
    input: MoveInput,
    slow_time: f32,
    tried_jump: bool,
    stuck_reported: bool,
}

/// Something that happened, for the log.
#[derive(Debug, Clone, PartialEq)]
pub enum SquadEvent {
    Spawned {
        member: String,
        feet: Vec3,
    },
    OrderGiven {
        order: String,
        stance: String,
    },
    OrderIgnored {
        order: String,
        why: String,
    },
    SetOff {
        member: String,
        tether_distance: f32,
        urgent: bool,
        by_order: bool,
        goal: Vec3,
    },
    Arrived {
        member: String,
        leader_distance: f32,
        tether_distance: f32,
    },
    MarkerAssigned {
        member: String,
        marker: String,
        job: String,
        distance: f32,
    },
    MarkerStarted {
        member: String,
        marker: String,
    },
    MarkerDone {
        member: String,
        marker: String,
    },
    MarkerCancelled {
        member: String,
        marker: String,
    },
    Stuck {
        member: String,
        at: Vec3,
        goal: Vec3,
        next: Vec3,
        path_points: usize,
    },
    /// A route through the map's path network (only logged when it isn't
    /// a straight line).
    Route {
        member: String,
        points: Vec<Vec3>,
    },
    /// Stuck on the way somewhere close enough: stopped there instead.
    StoppedShort {
        member: String,
        short_by: f32,
    },
    /// Health ran out: the member is down until revived.
    Down {
        member: String,
        at: Vec3,
    },
    Revived {
        member: String,
    },
}

impl fmt::Display for SquadEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let p = |v: &Vec3| format!("({:.0}, {:.0}, {:.0})", v.x, v.y, v.z);
        match self {
            SquadEvent::Spawned { member, feet } => write!(f, "{member} spawned, feet {}", p(feet)),
            SquadEvent::OrderGiven { order, stance } => write!(f, "ORDER {order} -> stance {stance}"),
            SquadEvent::OrderIgnored { order, why } => write!(f, "order {order} ignored: {why}"),
            SquadEvent::SetOff { member, tether_distance, urgent, by_order, goal } => write!(
                f,
                "{member} sets off for {} ({}; {:.0} uu from tether{})",
                p(goal),
                if *by_order { "ordered" } else { "drifted too far" },
                tether_distance,
                if *urgent { ", urgent" } else { "" }
            ),
            SquadEvent::Arrived { member, leader_distance, tether_distance } => write!(
                f,
                "{member} arrived: {leader_distance:.0} uu from the leader, {tether_distance:.0} uu from tether"
            ),
            SquadEvent::MarkerAssigned { member, marker, job, distance } => {
                write!(f, "{member} assigned to {marker} ({job}), {distance:.0} uu away")
            }
            SquadEvent::MarkerStarted { member, marker } => write!(f, "{member} working at {marker}"),
            SquadEvent::MarkerDone { member, marker } => write!(f, "{member} finished {marker}"),
            SquadEvent::MarkerCancelled { member, marker } => write!(f, "{member} left {marker} (cancelled)"),
            SquadEvent::Stuck { member, at, goal, next, path_points } => write!(
                f,
                "{member} STUCK at {} trying to reach {} (next path point {}, {path_points} points left)",
                p(at),
                p(goal),
                p(next)
            ),
            SquadEvent::Route { member, points } => {
                write!(f, "{member} route, {} points:", points.len())?;
                for q in points.iter().take(64) {
                    write!(f, " {}", p(q))?;
                }
                if points.len() > 64 {
                    write!(f, " ...")?;
                }
                Ok(())
            }
            SquadEvent::StoppedShort { member, short_by } => {
                write!(f, "{member} can't get closer; stopped {short_by:.0} uu short of the spot")
            }
            SquadEvent::Down { member, at } => write!(f, "{member} is DOWN at {}", p(at)),
            SquadEvent::Revived { member } => write!(f, "{member} is back up"),
        }
    }
}

/// One squad mate, for the display and the log.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberStatus {
    pub name: String,
    pub number: String,
    pub feet: Vec3,
    pub yaw: f32,
    /// "Form up", "Secure area", or the marked job.
    pub doing: String,
    pub state: MemberState,
    pub leader_distance: f32,
    pub goal_distance: f32,
    pub speed: f32,
    pub on_ground: bool,
    pub down: bool,
}

pub struct Squad {
    pub settings: SquadSettings,
    pub members: Vec<Member>,
    pub stance: Stance,
    /// The ordered spot (secure position).
    pub objective: Option<Vec3>,
    /// The way the leader faced the spot when ordering it.
    objective_yaw: f32,
    pub markers: Vec<(MarkerSpot, MarkerState)>,
    /// Enemies the squad knows about this frame, set by the game.
    pub threats: Vec<Vec3>,
    pub time: f32,
}

fn dist_2d(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, a.y - b.y, 0.0).length()
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

/// A point `forward` ahead and `right` to the side of `origin`, for a
/// facing `yaw`.
fn offset(origin: Vec3, yaw: f32, forward: f32, right: f32) -> Vec3 {
    let (f, r) = axes(yaw);
    origin + f * forward + r * right
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

/// Put a body down at `want`, unless a wall is in the way between
/// `from` and there, then as far as it fits.
fn free_spot(world: &dyn World, extent: Vec3, from: Vec3, want: Vec3) -> Vec3 {
    match world.sweep(extent, from, want) {
        None => want,
        Some(h) if h.start_solid => from,
        Some(h) => {
            let d = want - from;
            let len = d.length().max(1e-3);
            let t = ((h.time * len - 2.0) / len).max(0.0);
            from + d * t
        }
    }
}

impl Squad {
    /// The squad, standing in their form-up slots around the leader.
    pub fn new(
        settings: SquadSettings,
        params: MoveParams,
        leader: Leader,
        world: &dyn World,
    ) -> (Self, Vec<SquadEvent>) {
        let mut events = Vec::new();
        let lift = Vec3::new(0.0, 0.0, params.collision_height + 0.5);
        let extent = Vec3::new(
            params.collision_radius,
            params.collision_radius,
            params.collision_height,
        );
        let members = settings
            .members
            .iter()
            .map(|m| {
                let [f, r] = settings.formation[m.slot - 1];
                let want = offset(leader.feet, leader.yaw, f, r);
                let centre = free_spot(world, extent, leader.feet + lift, want + lift);
                let feet = centre - lift;
                events.push(SquadEvent::Spawned {
                    member: m.name.clone(),
                    feet,
                });
                Member {
                    settings: m.clone(),
                    pawn: Pawn::new(params.clone(), feet),
                    yaw: leader.yaw,
                    state: MemberState::Idle,
                    goal: feet,
                    hold_point: feet,
                    marker: None,
                    engaging: None,
                    down: false,
                    path: Vec::new(),
                    planned_for: None,
                    think_in: 0.0,
                    input: MoveInput::default(),
                    slow_time: 0.0,
                    tried_jump: false,
                    stuck_reported: false,
                }
            })
            .collect();
        let squad = Squad {
            settings,
            members,
            stance: Stance::FormUp,
            objective: None,
            objective_yaw: leader.yaw,
            markers: Vec::new(),
            threats: Vec::new(),
            time: 0.0,
        };
        (squad, events)
    }

    pub fn stance_settings(&self) -> &StanceSettings {
        match self.stance {
            Stance::FormUp => &self.settings.form_up,
            Stance::SecurePosition => &self.settings.secure_position,
            Stance::SearchAndDestroy => &self.settings.search_and_destroy,
            Stance::Hold => &self.settings.hold,
        }
    }

    /// Give an order. Members not on a marked job set off at once.
    pub fn command(&mut self, order: Order, leader: Leader, world: &dyn World) -> Vec<SquadEvent> {
        let mut events = Vec::new();
        match order {
            Order::FormUp => self.stance = Stance::FormUp,
            Order::SearchAndDestroy => self.stance = Stance::SearchAndDestroy,
            Order::SecurePosition(p) => {
                self.stance = Stance::SecurePosition;
                self.objective = Some(p);
                let d = p - leader.feet;
                if d.length_2d() > 1.0 {
                    self.objective_yaw = d.y.atan2(d.x);
                }
            }
            Order::Hold => {
                self.stance = Stance::Hold;
                for m in &mut self.members {
                    m.hold_point = m.pawn.feet();
                }
            }
            Order::Cancel => {
                for i in 0..self.members.len() {
                    if let Some(k) = self.members[i].marker.take() {
                        self.markers[k].1 = MarkerState::Open;
                        events.push(SquadEvent::MarkerCancelled {
                            member: self.members[i].settings.name.clone(),
                            marker: self.markers[k].0.name.clone(),
                        });
                        self.members[i].state = MemberState::Idle;
                    }
                }
                self.stance = Stance::FormUp;
            }
            Order::Marker(k) => {
                let Some((spot, state)) = self.markers.get(k) else {
                    events.push(SquadEvent::OrderIgnored {
                        order: order.to_string(),
                        why: "no such marker".into(),
                    });
                    return events;
                };
                if *state != MarkerState::Open {
                    events.push(SquadEvent::OrderIgnored {
                        order: format!("{} ({})", spot.name, spot.job),
                        why: format!("already {:?}", state),
                    });
                    return events;
                }
                let stand = spot.stand;
                let best = self
                    .members
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.marker.is_none() && !m.down)
                    .min_by(|a, b| {
                        dist(a.1.pawn.feet(), stand).total_cmp(&dist(b.1.pawn.feet(), stand))
                    });
                let Some((i, m)) = best else {
                    events.push(SquadEvent::OrderIgnored {
                        order: format!("{} ({})", spot.name, spot.job),
                        why: "everyone is busy".into(),
                    });
                    return events;
                };
                events.push(SquadEvent::MarkerAssigned {
                    member: m.settings.name.clone(),
                    marker: spot.name.clone(),
                    job: spot.job.clone(),
                    distance: dist(m.pawn.feet(), stand),
                });
                self.markers[k].1 = MarkerState::Assigned(i);
                let m = &mut self.members[i];
                m.marker = Some(k);
                m.state = MemberState::Idle;
                m.think_in = 0.0;
                self.set_off_now(i, leader, world, &mut events);
                events.insert(
                    0,
                    SquadEvent::OrderGiven {
                        order: format!("{} ({})", self.markers[k].0.name, self.markers[k].0.job),
                        stance: self.stance_settings().name.clone(),
                    },
                );
                return events;
            }
        }
        events.insert(
            0,
            SquadEvent::OrderGiven {
                order: order.to_string(),
                stance: self.stance_settings().name.clone(),
            },
        );
        for i in 0..self.members.len() {
            if self.members[i].marker.is_none() && !self.members[i].down {
                self.set_off_now(i, leader, world, &mut events);
            }
        }
        events
    }

    /// A direct order: head for the new spot now, unless already there.
    fn set_off_now(
        &mut self,
        i: usize,
        leader: Leader,
        world: &dyn World,
        events: &mut Vec<SquadEvent>,
    ) {
        let goal = self.goal_for(i, leader, world);
        let tether = self.tether_point(i, leader);
        let st = self.stance_settings().clone();
        let arrive = self.settings.arrive_radius;
        let m = &mut self.members[i];
        m.goal = goal;
        m.think_in = self.settings.think_interval;
        let feet = m.pawn.feet();
        if dist_2d(feet, goal) <= arrive && m.marker.is_none() {
            m.state = MemberState::Idle;
            return;
        }
        let td = dist(feet, tether);
        m.state = MemberState::Moving {
            urgent: td > st.hp_activate,
            by_order: true,
        };
        m.planned_for = None;
        events.push(SquadEvent::SetOff {
            member: m.settings.name.clone(),
            tether_distance: td,
            urgent: td > st.hp_activate,
            by_order: true,
            goal,
        });
    }

    /// What member `i` is tied to under the current order.
    fn tether_point(&self, i: usize, leader: Leader) -> Vec3 {
        let m = &self.members[i];
        if let Some(k) = m.marker {
            return self.markers[k].0.stand;
        }
        match self.stance_settings().tether {
            Tether::Leader => leader.feet,
            Tether::Objective => self.objective.unwrap_or(leader.feet),
            Tether::SelfPos => m.hold_point,
        }
    }

    /// Where member `i` wants to stand.
    fn goal_for(&self, i: usize, leader: Leader, world: &dyn World) -> Vec3 {
        let m = &self.members[i];
        if let Some(k) = m.marker {
            return self.markers[k].0.stand;
        }
        let slot = m.settings.slot - 1;
        // A slot behind a wall or over a drop moves to where a body fits,
        // on the floor, seen from what the slot is measured from.
        let fit = |from: Vec3, want: Vec3| place_on_floor(world, &m.pawn, from, want);
        match self.stance {
            Stance::FormUp => {
                let [f, r] = self.settings.formation[slot];
                fit(leader.feet, offset(leader.feet, leader.yaw, f, r))
            }
            Stance::SearchAndDestroy => {
                if let Some(t) = self.engage_target(i, leader) {
                    // Close in to about half the search radius.
                    let threat = self.threats[t];
                    let from = m.pawn.feet() - threat;
                    let d = from.length_2d().max(1.0);
                    let keep = (self.settings.search_radius * 0.5).min(d);
                    return threat + Vec3::new(from.x / d * keep, from.y / d * keep, 0.0);
                }
                let [f, r] = self.settings.formation_search[slot];
                fit(leader.feet, offset(leader.feet, leader.yaw, f, r))
            }
            Stance::SecurePosition => {
                let p = self.objective.unwrap_or(leader.feet);
                // Spread round the spot, the first on the far side.
                let a = self.objective_yaw + slot as f32 * TAU / 3.0;
                let r = self.settings.secure_radius;
                fit(p, p + Vec3::new(a.cos() * r, a.sin() * r, 0.0))
            }
            Stance::Hold => m.hold_point,
        }
    }

    /// Search and destroy: the nearest known enemy within reach.
    fn engage_target(&self, i: usize, leader: Leader) -> Option<usize> {
        let anchor = leader.feet;
        let reach = self
            .settings
            .search_radius
            .max(self.stance_settings().prevent_range);
        let feet = self.members[i].pawn.feet();
        self.threats
            .iter()
            .enumerate()
            .filter(|(_, t)| dist(**t, anchor) <= reach)
            .min_by(|a, b| dist(*a.1, feet).total_cmp(&dist(*b.1, feet)))
            .map(|(k, _)| k)
    }

    /// The way an idle member faces.
    fn facing_for(&self, i: usize, leader: Leader) -> f32 {
        let m = &self.members[i];
        if let Some(k) = m.marker {
            return self.markers[k].0.yaw;
        }
        match self.stance {
            Stance::SecurePosition => {
                // Look outward from the spot.
                let d = m.pawn.feet() - self.objective.unwrap_or(leader.feet);
                if d.length_2d() > 1.0 {
                    d.y.atan2(d.x)
                } else {
                    self.objective_yaw
                }
            }
            Stance::Hold => m.yaw,
            _ => leader.yaw,
        }
    }

    /// Re-decide where member `i` should be (every `think_interval`).
    fn think(&mut self, i: usize, leader: Leader, world: &dyn World, events: &mut Vec<SquadEvent>) {
        let goal = self.goal_for(i, leader, world);
        let tether = self.tether_point(i, leader);
        let st = self.stance_settings().clone();
        let arrive = self.settings.arrive_radius;
        let engaging =
            if self.stance == Stance::SearchAndDestroy && self.members[i].marker.is_none() {
                self.engage_target(i, leader)
            } else {
                None
            };
        let m = &mut self.members[i];
        m.goal = goal;
        m.engaging = engaging;
        let feet = m.pawn.feet();
        let td = dist(feet, tether);
        let to_goal = dist_2d(feet, goal);
        let close_height = (feet.z - goal.z).abs() < 150.0;
        match m.state {
            MemberState::Working { .. } => {}
            MemberState::Idle => {
                if m.marker.is_some() && !(to_goal <= arrive && close_height) {
                    m.state = MemberState::Moving {
                        urgent: false,
                        by_order: true,
                    };
                } else if td > st.lp_activate || (engaging.is_some() && to_goal > arrive) {
                    let urgent = td > st.hp_activate;
                    m.state = MemberState::Moving {
                        urgent,
                        by_order: false,
                    };
                    m.planned_for = None;
                    events.push(SquadEvent::SetOff {
                        member: m.settings.name.clone(),
                        tether_distance: td,
                        urgent,
                        by_order: false,
                        goal,
                    });
                }
            }
            MemberState::Moving { urgent, by_order } => {
                let at_goal = to_goal <= arrive && close_height;
                let close_enough =
                    !by_order && m.marker.is_none() && engaging.is_none() && td <= st.lp_deactivate;
                if at_goal || close_enough {
                    m.state = MemberState::Idle;
                    m.slow_time = 0.0;
                    m.tried_jump = false;
                    m.stuck_reported = false;
                    events.push(SquadEvent::Arrived {
                        member: m.settings.name.clone(),
                        leader_distance: dist(feet, leader.feet),
                        tether_distance: td,
                    });
                    if let Some(k) = m.marker {
                        if at_goal {
                            m.state = MemberState::Working {
                                left: self.settings.marker_work_time,
                            };
                            events.push(SquadEvent::MarkerStarted {
                                member: m.settings.name.clone(),
                                marker: self.markers[k].0.name.clone(),
                            });
                        }
                    }
                } else {
                    // Urgent until back inside the high-priority stop distance.
                    let urgent = if urgent {
                        td > st.hp_deactivate
                    } else {
                        td > st.hp_activate
                    };
                    m.state = MemberState::Moving { urgent, by_order };
                }
            }
        }
    }

    /// Advance the squad by `dt` seconds: think, steer, and move every body.
    pub fn update(
        &mut self,
        dt: f32,
        leader: Leader,
        world: &dyn World,
        nav: &dyn Navigator,
    ) -> Vec<SquadEvent> {
        let mut events = Vec::new();
        self.time += dt;
        for i in 0..self.members.len() {
            // Marked jobs count down.
            if let MemberState::Working { left } = self.members[i].state {
                let left = left - dt;
                if left <= 0.0 {
                    let k = self.members[i]
                        .marker
                        .take()
                        .expect("working members have a marker");
                    self.markers[k].1 = MarkerState::Done;
                    events.push(SquadEvent::MarkerDone {
                        member: self.members[i].settings.name.clone(),
                        marker: self.markers[k].0.name.clone(),
                    });
                    self.members[i].state = MemberState::Idle;
                    self.members[i].think_in = 0.0;
                } else {
                    self.members[i].state = MemberState::Working { left };
                }
            }
            if self.members[i].down {
                // Lies where they fell; gravity still applies.
                let m = &mut self.members[i];
                m.input = MoveInput::default();
                m.pawn.step(dt, &m.input, world);
                continue;
            }
            self.members[i].think_in -= dt;
            if self.members[i].think_in <= 0.0 {
                self.members[i].think_in += self.settings.think_interval;
                if self.members[i].think_in <= 0.0 {
                    self.members[i].think_in = self.settings.think_interval;
                }
                self.think(i, leader, world, &mut events);
            }
            let facing = self.facing_for(i, leader);
            let near_tether = dist(self.members[i].pawn.feet(), self.tether_point(i, leader))
                <= self.stance_settings().lp_activate;
            self.steer(i, dt, facing, near_tether, world, nav, &mut events);
            let m = &mut self.members[i];
            m.pawn.step(dt, &m.input, world);
        }
        events
    }

    /// Turn the member's state into this frame's `MoveInput`.
    #[allow(clippy::too_many_arguments)]
    fn steer(
        &mut self,
        i: usize,
        dt: f32,
        facing: f32,
        near_tether: bool,
        world: &dyn World,
        nav: &dyn Navigator,
        events: &mut Vec<SquadEvent>,
    ) {
        let s = &self.settings;
        let m = &mut self.members[i];
        let feet = m.pawn.feet();
        let MemberState::Moving { urgent, .. } = m.state else {
            // Standing: turn smoothly to face the right way.
            let d = angle_diff(facing, m.yaw);
            let step = s.turn_rate * dt;
            m.yaw = (m.yaw + d.clamp(-step, step)).rem_euclid(TAU);
            m.input = MoveInput {
                yaw: m.yaw,
                ..Default::default()
            };
            m.slow_time = 0.0;
            return;
        };
        let replan = match m.planned_for {
            None => true,
            Some(p) => dist(p, m.goal) > s.replan_distance || m.path.is_empty(),
        };
        if replan {
            m.path = if clear_line(world, &m.pawn, m.goal) {
                vec![m.goal]
            } else {
                let p = nav.find_path(feet, m.goal).unwrap_or_else(|| vec![m.goal]);
                if p.len() > 1 {
                    events.push(SquadEvent::Route {
                        member: m.settings.name.clone(),
                        points: p.clone(),
                    });
                }
                p
            };
            m.planned_for = Some(m.goal);
        }
        // The last point is the goal itself, which may have moved a little
        // since the plan; keep it current.
        if let Some(last) = m.path.last_mut() {
            *last = m.goal;
        }
        while m.path.len() > 1 && dist_2d(feet, m.path[0]) < s.waypoint_radius {
            m.path.remove(0);
        }
        let next = m.path.first().copied().unwrap_or(m.goal);
        let d = next - feet;
        let to_goal = dist_2d(feet, m.goal);
        if d.length_2d() > 1.0 {
            m.yaw = d.y.atan2(d.x).rem_euclid(TAU);
        }
        let walk = !urgent && to_goal < s.autorun_distance;
        // Ease off on the last stretch so they don't overshoot their spot.
        let forward = if to_goal < s.arrive_radius { 0.0 } else { 1.0 };
        // Stuck: slow while trying to move. Jump once, then report it.
        let speed = m.pawn.velocity.length_2d();
        let mut jump = false;
        if speed < 30.0 && forward > 0.0 {
            m.slow_time += dt;
            if m.slow_time > 0.6 && !m.tried_jump {
                m.tried_jump = true;
                jump = true;
            }
            if m.slow_time > 3.0 && !m.stuck_reported {
                m.stuck_reported = true;
                m.planned_for = None;
                events.push(SquadEvent::Stuck {
                    member: m.settings.name.clone(),
                    at: feet,
                    goal: m.goal,
                    next,
                    path_points: m.path.len(),
                });
                // Close enough to what they're tied to: stand here rather
                // than push at the obstacle. (A marked job keeps trying
                // until `marker_give_up` below.)
                if near_tether && m.marker.is_none() {
                    events.push(SquadEvent::StoppedShort {
                        member: m.settings.name.clone(),
                        short_by: to_goal,
                    });
                    m.state = MemberState::Idle;
                    m.input = MoveInput {
                        yaw: m.yaw,
                        ..Default::default()
                    };
                    m.slow_time = 0.0;
                    m.tried_jump = false;
                    m.stuck_reported = false;
                    return;
                }
            }
            if m.slow_time > MARKER_GIVE_UP {
                if let Some(k) = m.marker.take() {
                    self.markers[k].1 = MarkerState::Open;
                    events.push(SquadEvent::MarkerCancelled {
                        member: m.settings.name.clone(),
                        marker: format!("{} (can't get there)", self.markers[k].0.name),
                    });
                    m.state = MemberState::Idle;
                    m.slow_time = 0.0;
                    m.tried_jump = false;
                    m.stuck_reported = false;
                }
            }
        } else {
            m.slow_time = 0.0;
            m.tried_jump = false;
        }
        m.input = MoveInput {
            forward,
            strafe: 0.0,
            yaw: m.yaw,
            jump,
            crouch: false,
            walk,
        };
    }

    /// Mark member `i` down (health ran out) or back up. A downed member
    /// drops any marked job (the spot opens again) and stops where it is.
    /// `None` when nothing changed.
    pub fn set_down(&mut self, i: usize, down: bool) -> Option<SquadEvent> {
        let m = self.members.get_mut(i)?;
        if m.down == down {
            return None;
        }
        m.down = down;
        m.think_in = 0.0;
        m.path.clear();
        m.planned_for = None;
        m.slow_time = 0.0;
        m.tried_jump = false;
        m.stuck_reported = false;
        let name = m.settings.name.clone();
        if !down {
            m.state = MemberState::Idle;
            return Some(SquadEvent::Revived { member: name });
        }
        m.state = MemberState::Idle;
        m.engaging = None;
        let at = m.pawn.feet();
        if let Some(k) = m.marker.take() {
            self.markers[k].1 = MarkerState::Open;
        }
        Some(SquadEvent::Down { member: name, at })
    }

    /// Every member's state, for the display and the log.
    pub fn status(&self, leader: Leader) -> Vec<MemberStatus> {
        self.members
            .iter()
            .map(|m| {
                let feet = m.pawn.feet();
                let doing = match m.marker {
                    Some(k) => self.markers[k].0.job.clone(),
                    None => self.stance_settings().name.clone(),
                };
                MemberStatus {
                    name: m.settings.name.clone(),
                    number: m.settings.number.clone(),
                    feet,
                    yaw: m.yaw,
                    doing,
                    state: m.state,
                    leader_distance: dist(feet, leader.feet),
                    goal_distance: dist_2d(feet, m.goal),
                    speed: m.pawn.velocity.length_2d(),
                    on_ground: m.pawn.mode == rc_movement::Mode::Walking,
                    down: m.down,
                }
            })
            .collect()
    }

    /// The marked spot the player is aiming at: within `max_angle`
    /// (radians) of the aim and `reach` uu of the eye, nearest the aim.
    pub fn marker_in_aim(&self, eye: Vec3, aim: Vec3, reach: f32, max_angle: f32) -> Option<usize> {
        let aim = aim.normalize_or_zero();
        self.markers
            .iter()
            .enumerate()
            .filter(|(_, (_, st))| *st == MarkerState::Open)
            .filter_map(|(k, (spot, _))| {
                let to = spot.aim - eye;
                let d = to.length();
                if d > reach || d < 1.0 {
                    return None;
                }
                let angle = (to.dot(aim) / d).clamp(-1.0, 1.0).acos();
                (angle <= max_angle).then_some((k, angle))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }
}

/// Seconds stuck before a member gives up on a marked job. Our own.
const MARKER_GIVE_UP: f32 = 10.0;

/// Where a body can stand for a wanted spot `want`, measured from `from`
/// (both feet positions): pulled back if a wall is in the way, then
/// dropped onto the floor below (up to a storey down). If there's no floor
/// under it, the spot stays where it was.
fn place_on_floor(world: &dyn World, pawn: &Pawn, from: Vec3, want: Vec3) -> Vec3 {
    let e = pawn.extent();
    let lift = Vec3::new(0.0, 0.0, e.z + pawn.params.max_step_height);
    let centre = free_spot(world, e, from + lift, want + lift);
    let down = centre - Vec3::new(0.0, 0.0, 400.0 + pawn.params.max_step_height);
    match world.sweep(e, centre, down) {
        Some(h) if !h.start_solid => centre + (down - centre) * h.time - Vec3::new(0.0, 0.0, e.z),
        _ => centre - lift,
    }
}

/// Nothing solid between the body and a spot at the same height: it can
/// walk straight there. (Slightly slimmer than the body, and lifted by a
/// step's height, so floors, small steps and gentle slopes don't count.)
fn clear_line(world: &dyn World, pawn: &Pawn, goal: Vec3) -> bool {
    let e = pawn.extent();
    let lift = Vec3::new(0.0, 0.0, pawn.params.max_step_height);
    let start = pawn.location + lift;
    let end = Vec3::new(goal.x, goal.y, goal.z + e.z + 0.5) + lift;
    let slim = Vec3::new(
        e.x * 0.8,
        e.y * 0.8,
        e.z - pawn.params.max_step_height * 0.5,
    );
    world.sweep(slim, start, end).is_none()
}

/// Where an aim line from `eye` along `dir` meets the world, within
/// `reach` uu, moved back a little toward the eye and dropped to the floor
/// so a body can stand there. `None` if it meets nothing in reach.
pub fn aimed_spot(
    world: &dyn World,
    eye: Vec3,
    dir: Vec3,
    reach: f32,
    params: &MoveParams,
) -> Option<Vec3> {
    let dir = dir.normalize_or_zero();
    let end = eye + dir * reach;
    let thin = Vec3::new(1.0, 1.0, 1.0);
    let hit = world.sweep(thin, eye, end)?;
    if hit.start_solid {
        return None;
    }
    let point = eye + dir * (reach * hit.time);
    // Step back off the surface by a body's width, then find the floor.
    let back =
        point - Vec3::new(dir.x, dir.y, 0.0).normalize_or_zero() * (params.collision_radius + 10.0);
    let extent = Vec3::new(
        params.collision_radius,
        params.collision_radius,
        params.collision_height,
    );
    let top = back + Vec3::new(0.0, 0.0, params.collision_height + 40.0);
    let bottom = top - Vec3::new(0.0, 0.0, 2000.0);
    match world.sweep(extent, top, bottom) {
        Some(h) if !h.start_solid => {
            let centre = top + (bottom - top) * h.time;
            Some(centre - Vec3::new(0.0, 0.0, params.collision_height))
        }
        _ => Some(back),
    }
}
