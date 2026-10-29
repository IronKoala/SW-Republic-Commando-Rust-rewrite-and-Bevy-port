//! Doors, lifts and hatches: a map's movers, moving the way the original
//! does, with collision that moves with them and carries whoever stands on
//! them.
//!
//! Plain Rust with no Bevy, like `rc_movement`, so it can be tested on its
//! own and used by the walker, the game and the droid tests. Everything is
//! in Unreal units with Z up.
//!
//! What comes from the game: every mover's keys, timing, glide, what it
//! does when blocked, what sets it off, its tag and event, and its sounds,
//! all read from the map and the class defaults (`rc_level::movers`).
//! How a mover runs through those values follows Unreal Engine 2's public
//! description of its `Mover` class; the exact engine code is compiled and
//! not read. Our own choices (how near counts as a bump, and so on) are in
//! `data/movers/movers.toml` (`settings.rs`). See docs/DESIGN.md, "Doors and
//! lifts (movers)".

pub mod settings;

use rc_level::movers::{Encroach, Glide, MapMovers, MoverInfo, TriggerInfo};
use rc_movement::{Hit, Pawn, TriangleWorld, Vec3, World};
pub use settings::Settings;

/// Gap kept between a carried or pushed player and what they touch, uu.
const SKIN: f32 = 0.1;

/// What a mover does, picked from its `InitialState` (see `Settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Never moves.
    Static,
    /// A trigger opens it; it waits `StayOpenTime`, then closes.
    TriggerOpenTimed,
    /// Each trigger opens it if closed, closes it if open.
    TriggerToggle,
    /// Open while the trigger is held, closes when it's let go.
    TriggerControl,
    /// Opens and closes over and over while the trigger is held.
    TriggerPound,
    /// A trigger starts it going back and forth through its keys; the next
    /// trigger lets it finish at key 0 and stop (Republic Commando's
    /// `TriggerLoop`, our reading of the name).
    TriggerLoop,
    /// Opens when someone walks into it, waits, closes.
    BumpOpenTimed,
    /// Opens when someone stands on it (a lift), waits, goes back.
    StandOpenTimed,
    /// Moves when bumped, comes back after a wait.
    BumpButton,
    /// Opens and closes for ever.
    Loop,
}

impl Behaviour {
    pub const ALL: [Behaviour; 10] = [
        Behaviour::Static,
        Behaviour::TriggerOpenTimed,
        Behaviour::TriggerToggle,
        Behaviour::TriggerControl,
        Behaviour::TriggerPound,
        Behaviour::TriggerLoop,
        Behaviour::BumpOpenTimed,
        Behaviour::StandOpenTimed,
        Behaviour::BumpButton,
        Behaviour::Loop,
    ];

    /// The name used in `movers.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Behaviour::Static => "static",
            Behaviour::TriggerOpenTimed => "trigger_open_timed",
            Behaviour::TriggerToggle => "trigger_toggle",
            Behaviour::TriggerControl => "trigger_control",
            Behaviour::TriggerPound => "trigger_pound",
            Behaviour::TriggerLoop => "trigger_loop",
            Behaviour::BumpOpenTimed => "bump_open_timed",
            Behaviour::StandOpenTimed => "stand_open_timed",
            Behaviour::BumpButton => "bump_button",
            Behaviour::Loop => "loop",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(s))
    }

    /// Goes back on its own after `StayOpenTime`.
    pub fn timed(self) -> bool {
        matches!(
            self,
            Behaviour::TriggerOpenTimed
                | Behaviour::BumpOpenTimed
                | Behaviour::StandOpenTimed
                | Behaviour::BumpButton
        )
    }
}

/// Where a mover is: position (uu) and turn (Unreal angle units, as
/// floats so they can be blended).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: [f32; 3],
}

impl Pose {
    fn lerp(a: Pose, b: Pose, t: f32) -> Pose {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Pose {
            pos: Vec3::new(l(a.pos.x, b.pos.x), l(a.pos.y, b.pos.y), l(a.pos.z, b.pos.z)),
            rot: [l(a.rot[0], b.rot[0]), l(a.rot[1], b.rot[1]), l(a.rot[2], b.rot[2])],
        }
    }

    /// Forward, right and up directions (Unreal's convention, as
    /// `rc_level::actors::rotation_axes`).
    pub fn axes(&self) -> [Vec3; 3] {
        let to_rad = std::f32::consts::TAU / 65536.0;
        let (sp, cp) = (self.rot[0] * to_rad).sin_cos();
        let (sy, cy) = (self.rot[1] * to_rad).sin_cos();
        let (sr, cr) = (self.rot[2] * to_rad).sin_cos();
        [
            Vec3::new(cp * cy, cp * sy, sp),
            Vec3::new(sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp),
            Vec3::new(-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp),
        ]
    }
}

/// The rigid move that takes the mover from one pose to another. Applied
/// to points, it moves them with the mover.
#[derive(Debug, Clone, Copy)]
pub struct Motion {
    from: Pose,
    from_axes: [Vec3; 3],
    to: Pose,
    to_axes: [Vec3; 3],
}

impl Motion {
    pub fn new(from: Pose, to: Pose) -> Self {
        Motion {
            from,
            from_axes: from.axes(),
            to,
            to_axes: to.axes(),
        }
    }

    /// Where a point that moved with the mover ends up.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        let d = p - self.from.pos;
        let mut out = self.to.pos;
        for k in 0..3 {
            out += self.to_axes[k] * d.dot(self.from_axes[k]);
        }
        out
    }

    /// The opposite of `apply`.
    pub fn undo(&self, p: Vec3) -> Vec3 {
        let d = p - self.to.pos;
        let mut out = self.from.pos;
        for k in 0..3 {
            out += self.from_axes[k] * d.dot(self.to_axes[k]);
        }
        out
    }

    /// Turn a direction with the mover.
    pub fn turn(&self, n: Vec3) -> Vec3 {
        let mut out = Vec3::ZERO;
        for k in 0..3 {
            out += self.to_axes[k] * n.dot(self.from_axes[k]);
        }
        out
    }

    /// The turn as a 3x3 matrix (row `r`, column `c`) and the shift, so
    /// that `apply(p) = matrix * p + shift`. For drawing.
    pub fn matrix(&self) -> ([[f32; 3]; 3], Vec3) {
        let mut m = [[0.0f32; 3]; 3];
        let unit = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
        for (c, e) in unit.iter().enumerate() {
            let col = self.turn(*e);
            m[0][c] = col.x;
            m[1][c] = col.y;
            m[2][c] = col.z;
        }
        let shift = self.apply(Vec3::ZERO);
        (m, shift)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Still at key `key`.
    Rest,
    /// Waiting `DelayTime` before starting to open.
    Delay { left: f32 },
    /// Going from one key to the next; `t` runs 0 to 1 over `MoveTime`.
    Moving { from: usize, to: usize, t: f32 },
    /// Open, waiting `StayOpenTime` before going back.
    Wait { left: f32 },
}

/// Counts and timings, for the log and the checks.
#[derive(Debug, Clone, Default)]
pub struct MoverStats {
    pub opened: u32,
    pub closed: u32,
    /// Times it ran into someone and went back (or stopped).
    pub blocked: u32,
    /// Seconds the last full opening took, from starting to open to open.
    pub last_open_secs: Option<f32>,
    pub last_close_secs: Option<f32>,
    /// Times the player was carried while standing on it.
    pub carried_frames: u32,
}

pub struct Mover {
    pub info: MoverInfo,
    pub behaviour: Behaviour,
    /// Whether `behaviour` came from a known state name.
    pub known_state: bool,
    /// Its collision triangles, placed where it was drawn (`built`).
    pub collision: Option<TriangleWorld>,
    /// The pose its triangles (and drawn mesh) were built at.
    pub built: Pose,
    pub pose: Pose,
    /// The key it is at, or last left.
    pub key: usize,
    pub phase: Phase,
    /// Heading toward the open end (the last key).
    pub opening: bool,
    pub stats: MoverStats,
    /// For `TriggerControl` and `TriggerPound`: the trigger is held.
    held: bool,
    /// Has been set off once (for `bTriggerOnceOnly`).
    used: bool,
    /// The player was bumping or standing on it last step: only a new
    /// touch sets it off.
    was_touched: bool,
    /// Its own events (`OpeningEvent` and so on) waiting to be sent.
    to_send: Vec<String>,
    /// When the current opening or closing run started, s.
    run_started: f32,
}

/// Something that happened to a mover, for the log and for sounds.
#[derive(Debug, Clone, PartialEq)]
pub enum MoverEvent {
    StartedOpening { mover: usize },
    Opened { mover: usize, secs: f32 },
    StartedClosing { mover: usize },
    Closed { mover: usize, secs: f32 },
    /// Ran into a player and did what its `MoverEncroachType` says.
    Blocked { mover: usize, what: Encroach },
    /// A sound the original plays here (not played yet).
    Sound { mover: usize, sound: String },
    /// A trigger was touched and sent its event.
    Triggered { trigger: usize, event: String, movers: usize },
    /// A mover sent one of its own events (opening, opened, closing,
    /// closed).
    Event { mover: usize, event: String, movers: usize },
    /// The player started or stopped standing on a mover.
    Boarded { mover: usize },
    LeftBoard { mover: usize },
}

impl Mover {
    pub fn new(info: MoverInfo, triangles: Vec<[Vec3; 3]>, settings: &Settings) -> Self {
        let (behaviour, known_state) = settings.behaviour(&info.state);
        let key = info.key_num as usize;
        let pose = key_pose(&info, key);
        // The drawn mesh and the collision sit where the map placed the
        // actor. That should be the starting key's pose; if the map says
        // otherwise, trust the actor's location for drawing.
        let built = Pose {
            pos: v3(info.place.location),
            rot: info.place.rotation.map(|r| r as f32),
        };
        Mover {
            behaviour,
            known_state,
            collision: (!triangles.is_empty()).then(|| TriangleWorld::new(triangles)),
            built,
            pose,
            key,
            phase: Phase::Rest,
            opening: false,
            stats: MoverStats::default(),
            held: false,
            used: false,
            was_touched: false,
            to_send: Vec::new(),
            run_started: 0.0,
            info,
        }
    }

    pub fn last_key(&self) -> usize {
        self.info.num_keys.max(1) as usize - 1
    }

    /// The move from where it was built to where it is now.
    pub fn motion(&self) -> Motion {
        Motion::new(self.built, self.pose)
    }

    /// A lift: it moves mostly up or down (at least 64 uu, more than it
    /// moves sideways), doesn't turn, and is wide enough to stand on for a
    /// box of half-width `radius`.
    pub fn is_lift(&self, radius: f32) -> bool {
        let info = &self.info;
        let Some(c) = &self.collision else { return false };
        if self.behaviour == Behaviour::Static || info.num_keys < 2 || info.turns() {
            return false;
        }
        let (a, b) = (info.key(0).0, info.key(info.num_keys as usize - 1).0);
        let rise = b[2] - a[2];
        let flat = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let (lo, hi) = c.bounds();
        rise.abs() >= 64.0 && flat <= rise.abs() && hi.x - lo.x >= 2.0 * radius && hi.y - lo.y >= 2.0 * radius
    }

    /// Where a player with box half-size `extent` can stand on top of it,
    /// in the middle: the feet position, or `None` if there's no floor
    /// there.
    pub fn top_spot(&self, extent: Vec3) -> Option<Vec3> {
        let (lo, hi) = self.collision.as_ref()?.bounds();
        let m = self.motion();
        let above = m.apply(Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z + 300.0));
        let drop = 300.0 + (hi.z - lo.z) + 10.0;
        let h = self.sweep(extent, above, above - Vec3::new(0.0, 0.0, drop))?;
        (!h.start_solid && h.normal.z > 0.7)
            .then(|| Vec3::new(above.x, above.y, above.z - drop * h.time - extent.z))
    }

    /// For a lift: where to stand on it (feet), and how much of its
    /// journey (0 to 1) a player standing there can make before the fixed
    /// level gets in the way (a ceiling over a door that slides up, say).
    /// `None` if there's nowhere to stand.
    pub fn ride_clearance(&self, fixed: &dyn World, extent: Vec3) -> Option<(Vec3, f32)> {
        let spot = self.top_spot(extent)?;
        let (a, b) = (self.info.key(0).0, self.info.key(self.last_key()).0);
        let shift = Vec3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        let centre = spot + Vec3::new(0.0, 0.0, extent.z + SKIN);
        let clear = match fixed.sweep(extent, centre, centre + shift) {
            None => 1.0,
            Some(h) if h.start_solid => 0.0,
            Some(h) => h.time,
        };
        Some((spot, clear))
    }

    /// Is it fully closed (at key 0 and still)?
    pub fn is_closed(&self) -> bool {
        self.key == 0 && self.phase == Phase::Rest
    }

    /// Sweep a box against the mover where it is now.
    pub fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        self.sweep_at(&self.motion(), extent, start, end)
    }

    fn sweep_at(&self, m: &Motion, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let world = self.collision.as_ref()?;
        // Move the sweep into the mover's built pose instead of moving the
        // triangles. Exact for sliding movers; for turning ones the box
        // keeps its world-aligned shape, which is close enough for doors.
        let h = world.sweep(extent, m.undo(start), m.undo(end))?;
        Some(Hit {
            normal: m.turn(h.normal),
            ..h
        })
    }

    fn overlaps_at(&self, m: &Motion, extent: Vec3, centre: Vec3) -> bool {
        self.sweep_at(m, extent, centre, centre)
            .is_some_and(|h| h.start_solid)
    }

    fn queue(&mut self, event: &str) {
        if !event.is_empty() {
            self.to_send.push(event.to_string());
        }
    }

    fn play(&self, me: usize, sound: &Option<String>, events: &mut Vec<MoverEvent>) {
        if let Some(s) = sound {
            events.push(MoverEvent::Sound {
                mover: me,
                sound: s.clone(),
            });
        }
    }

    /// Its last key is the same place as key 0 (a full circle, or a path
    /// that comes back where it started). Such a mover, when looping, goes
    /// on round instead of back: after the last key it carries on from key
    /// 0. Our reading of the game's maps: spinning antennas set key 1 a
    /// full turn from key 0, and searchlights go 0, 1, 2 with 2 = 0.
    pub fn is_circuit(&self) -> bool {
        let (a, b) = (key_pose(&self.info, 0), key_pose(&self.info, self.last_key()));
        let turn_same = (0..3).all(|c| ((a.rot[c] - b.rot[c]) % 65536.0).abs() < 1.0);
        self.last_key() > 0 && (a.pos - b.pos).length() < 1.0 && turn_same
    }

    fn start_opening(&mut self, me: usize, now: f32, events: &mut Vec<MoverEvent>) {
        let last = self.last_key();
        if last == 0 {
            return;
        }
        if self.phase == Phase::Rest && self.key >= last && self.is_circuit() {
            // Round again from key 0 (the same place).
            self.key = 0;
            self.pose = key_pose(&self.info, 0);
        }
        self.opening = true;
        match self.phase {
            Phase::Moving { from, to, t } if to < from => {
                // Closing: turn round where it is.
                self.phase = Phase::Moving { from: to, to: from, t: 1.0 - t };
            }
            Phase::Moving { .. } => {}
            Phase::Wait { .. } => {
                self.phase = Phase::Wait {
                    left: self.info.stay_open_time,
                };
                return;
            }
            Phase::Delay { .. } => return,
            Phase::Rest if self.key >= last => return,
            Phase::Rest if self.info.delay_time > 0.0 => {
                self.phase = Phase::Delay {
                    left: self.info.delay_time,
                };
                return;
            }
            Phase::Rest => {
                self.phase = Phase::Moving {
                    from: self.key,
                    to: self.key + 1,
                    t: 0.0,
                };
            }
        }
        self.run_started = now;
        events.push(MoverEvent::StartedOpening { mover: me });
        self.play(me, &self.info.sounds.opening, events);
        let e = self.info.opening_event.clone();
        self.queue(&e);
    }

    fn start_closing(&mut self, me: usize, now: f32, events: &mut Vec<MoverEvent>) {
        self.opening = false;
        match self.phase {
            Phase::Moving { from, to, t } if to > from => {
                self.phase = Phase::Moving { from: to, to: from, t: 1.0 - t };
            }
            Phase::Moving { .. } => return,
            Phase::Delay { .. } => {
                self.phase = Phase::Rest;
                return;
            }
            Phase::Rest if self.key == 0 => return,
            Phase::Rest | Phase::Wait { .. } => {
                self.phase = Phase::Moving {
                    from: self.key,
                    to: self.key - 1,
                    t: 0.0,
                };
            }
        }
        self.run_started = now;
        events.push(MoverEvent::StartedClosing { mover: me });
        self.play(me, &self.info.sounds.closing, events);
        let e = self.info.closing_event.clone();
        self.queue(&e);
    }

    /// A trigger with this mover's tag went off.
    fn trigger(&mut self, me: usize, now: f32, events: &mut Vec<MoverEvent>) {
        if self.info.trigger_once_only && self.used {
            return;
        }
        match self.behaviour {
            Behaviour::Static | Behaviour::Loop => return,
            Behaviour::TriggerToggle => {
                if self.opening || self.key == self.last_key() {
                    self.start_closing(me, now, events);
                } else {
                    self.start_opening(me, now, events);
                }
            }
            Behaviour::TriggerControl | Behaviour::TriggerPound => {
                self.held = true;
                self.start_opening(me, now, events);
            }
            Behaviour::TriggerLoop => {
                self.held = !self.held;
                if self.held {
                    self.start_opening(me, now, events);
                }
            }
            // Bump and stand movers can be triggered too in UE2; they
            // open the same way.
            _ => self.start_opening(me, now, events),
        }
        self.used = true;
    }

    fn untrigger(&mut self, me: usize, now: f32, events: &mut Vec<MoverEvent>) {
        if matches!(self.behaviour, Behaviour::TriggerControl | Behaviour::TriggerPound) {
            self.held = false;
            if self.behaviour == Behaviour::TriggerControl {
                self.start_closing(me, now, events);
            }
        }
    }

    /// Bumped or stood on by the player.
    fn touched(&mut self, me: usize, now: f32, standing: bool, events: &mut Vec<MoverEvent>) {
        let wants = match self.behaviour {
            Behaviour::BumpOpenTimed | Behaviour::BumpButton => true,
            Behaviour::StandOpenTimed => standing,
            _ => false,
        };
        if !wants || (self.info.trigger_once_only && self.used) {
            return;
        }
        // Like UE2, touching it while it's open or moving does nothing: it
        // keeps to its timer, and a player in the way is handled by its
        // `MoverEncroachType`.
        if self.is_closed() {
            self.used = true;
            self.start_opening(me, now, events);
        }
    }

    /// Advance the timers and the move by `dt`. Returns the new pose and
    /// the phase, key and direction to keep if the move is allowed.
    fn advance(&self, dt: f32) -> (Pose, Phase, usize) {
        match self.phase {
            Phase::Moving { from, to, t } => {
                let mut t = t + dt / self.info.segment_time(from, to).max(1e-3);
                // Adding up frame times leaves 0.9999997 instead of 1.
                if t > 1.0 - 1e-4 {
                    t = 1.0;
                }
                let a = match self.info.glide {
                    Glide::Linear => t,
                    // Eases in and out: no sudden start or stop.
                    Glide::Smooth => t * t * (3.0 - 2.0 * t),
                };
                let pose = Pose::lerp(key_pose(&self.info, from), key_pose(&self.info, to), a);
                (pose, Phase::Moving { from, to, t }, self.key)
            }
            _ => (self.pose, self.phase, self.key),
        }
    }

    /// Bookkeeping after a move was allowed: arriving at keys, waits,
    /// delays.
    fn settle(&mut self, me: usize, dt: f32, now: f32, events: &mut Vec<MoverEvent>) {
        match self.phase {
            Phase::Delay { left } => {
                if left - dt <= 0.0 {
                    self.phase = Phase::Rest;
                    let d = self.info.delay_time;
                    self.info.delay_time = 0.0;
                    self.start_opening(me, now, events);
                    self.info.delay_time = d;
                } else {
                    self.phase = Phase::Delay { left: left - dt };
                }
            }
            Phase::Wait { left } => {
                if left - dt <= 0.0 {
                    self.start_closing(me, now, events);
                } else {
                    self.phase = Phase::Wait { left: left - dt };
                }
            }
            Phase::Moving { from, to, t } if t >= 1.0 => {
                self.key = to;
                self.pose = key_pose(&self.info, to);
                let last = self.last_key();
                if to > from && to < last {
                    self.phase = Phase::Moving { from: to, to: to + 1, t: 0.0 };
                } else if to < from && to > 0 {
                    self.phase = Phase::Moving { from: to, to: to - 1, t: 0.0 };
                } else if to > from {
                    let secs = now - self.run_started;
                    self.stats.opened += 1;
                    self.stats.last_open_secs = Some(secs);
                    self.phase = Phase::Rest;
                    events.push(MoverEvent::Opened { mover: me, secs });
                    self.play(me, &self.info.sounds.opened, events);
                    let e = self.info.opened_event.clone();
                    self.queue(&e);
                    let pound = self.behaviour == Behaviour::TriggerPound;
                    let looping = self.behaviour == Behaviour::Loop
                        || (self.behaviour == Behaviour::TriggerLoop && self.held);
                    if self.behaviour.timed() {
                        self.phase = Phase::Wait {
                            left: self.info.stay_open_time,
                        };
                    } else if looping && self.is_circuit() {
                        self.start_opening(me, now, events);
                    } else if looping || pound {
                        self.start_closing(me, now, events);
                    } else if self.behaviour == Behaviour::TriggerLoop {
                        // Switched off part way: finish where it rests.
                        // A circuit's last key is key 0's pose, so it is
                        // already home; anything else goes back.
                        if self.is_circuit() {
                            self.key = 0;
                            self.opening = false;
                        } else {
                            self.start_closing(me, now, events);
                        }
                    }
                } else {
                    let secs = now - self.run_started;
                    self.stats.closed += 1;
                    self.stats.last_close_secs = Some(secs);
                    self.phase = Phase::Rest;
                    self.opening = false;
                    events.push(MoverEvent::Closed { mover: me, secs });
                    self.play(me, &self.info.sounds.closed, events);
                    let e = self.info.closed_event.clone();
                    self.queue(&e);
                    let again = matches!(self.behaviour, Behaviour::TriggerPound | Behaviour::TriggerLoop) && self.held;
                    if self.behaviour == Behaviour::Loop || again {
                        self.start_opening(me, now, events);
                    }
                }
            }
            _ => {}
        }
    }
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn key_pose(info: &MoverInfo, k: usize) -> Pose {
    let (p, r) = info.key(k);
    Pose {
        pos: v3(p),
        rot: r.map(|x| x as f32),
    }
}

struct TriggerState {
    info: TriggerInfo,
    touching: bool,
    used: bool,
    /// Time left before it may go off again, s.
    cooldown: f32,
}

/// Every mover and trigger in a map, run together.
pub struct MoverSet {
    pub movers: Vec<Mover>,
    triggers: Vec<TriggerState>,
    pub settings: Settings,
    /// Seconds since the start.
    pub time: f32,
    /// Which mover the player stood on last step.
    pub riding: Option<usize>,
    /// Triggers whose state we don't run (not `NormalTrigger`).
    pub unhandled_triggers: Vec<String>,
}

/// The fixed level plus the movers where they are now. `skip` leaves one
/// mover out (the one doing the pushing).
pub struct WithMovers<'a> {
    pub fixed: &'a dyn World,
    pub movers: &'a [Mover],
    pub skip: Option<usize>,
}

impl World for WithMovers<'_> {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let mut best = self.fixed.sweep(extent, start, end);
        for (i, m) in self.movers.iter().enumerate() {
            if Some(i) == self.skip {
                continue;
            }
            if let Some(h) = m.sweep(extent, start, end) {
                if best.is_none_or(|b| h.time < b.time || (h.start_solid && !b.start_solid)) {
                    best = Some(h);
                }
            }
        }
        best
    }
}

/// Move a player's box by `delta`, stopping short of anything in `world`.
/// Returns how much of the move was made (0 to 1).
fn shove(pawn: &mut Pawn, delta: Vec3, world: &dyn World) -> f32 {
    let len = delta.length();
    if len < 1e-5 {
        return 1.0;
    }
    match world.sweep(pawn.extent(), pawn.location, pawn.location + delta) {
        None => {
            pawn.location += delta;
            1.0
        }
        Some(h) if h.start_solid => 0.0,
        Some(h) => {
            let t = (h.time - SKIN / len).max(0.0);
            pawn.location += delta * t;
            t
        }
    }
}

impl MoverSet {
    /// `triangles(export)` gives each mover's collision triangles where it
    /// was placed.
    pub fn new(
        map: MapMovers,
        settings: Settings,
        mut triangles: impl FnMut(usize) -> Vec<[Vec3; 3]>,
    ) -> Self {
        let movers: Vec<Mover> = map
            .movers
            .into_iter()
            .map(|info| {
                let t = triangles(info.place.export);
                Mover::new(info, t, &settings)
            })
            .collect();
        let mut unhandled = Vec::new();
        let triggers = map
            .triggers
            .into_iter()
            .filter_map(|t| {
                let normal = t.state.is_empty() || t.state.eq_ignore_ascii_case("NormalTrigger");
                if !normal || t.event.is_empty() {
                    unhandled.push(format!(
                        "{} ({}): state {:?}, event {:?}",
                        t.name, t.class, t.state, t.event
                    ));
                    return None;
                }
                Some(TriggerState {
                    info: t,
                    touching: false,
                    used: false,
                    cooldown: 0.0,
                })
            })
            .collect();
        let mut set = MoverSet {
            movers,
            triggers,
            settings,
            time: 0.0,
            riding: None,
            unhandled_triggers: unhandled,
        };
        let mut ev = Vec::new();
        for i in 0..set.movers.len() {
            if set.movers[i].behaviour == Behaviour::Loop {
                set.movers[i].start_opening(i, 0.0, &mut ev);
            }
        }
        set
    }

    /// Triggers that run (touch sets off their event).
    pub fn active_triggers(&self) -> impl Iterator<Item = &TriggerInfo> {
        self.triggers.iter().map(|t| &t.info)
    }

    /// The world for the player to move in: the fixed level plus the
    /// movers.
    pub fn world<'a>(&'a self, fixed: &'a dyn World) -> WithMovers<'a> {
        WithMovers {
            fixed,
            movers: &self.movers,
            skip: None,
        }
    }

    /// Send `event` to every mover whose tag matches. Returns how many.
    pub fn send(&mut self, event: &str, events: &mut Vec<MoverEvent>) -> usize {
        let now = self.time;
        let mut n = 0;
        for (i, m) in self.movers.iter_mut().enumerate() {
            if !event.is_empty() && m.info.tag.eq_ignore_ascii_case(event) {
                m.trigger(i, now, events);
                n += 1;
            }
        }
        n
    }

    /// Start mover `i` opening whatever sets it off normally. For tests
    /// and the check tool.
    pub fn open(&mut self, i: usize) -> Vec<MoverEvent> {
        let mut events = Vec::new();
        let now = self.time;
        self.movers[i].start_opening(i, now, &mut events);
        self.send_queued(&mut events);
        events
    }

    /// Start mover `i` closing. For tests and the check tool.
    pub fn close(&mut self, i: usize) -> Vec<MoverEvent> {
        let mut events = Vec::new();
        let now = self.time;
        self.movers[i].start_closing(i, now, &mut events);
        self.send_queued(&mut events);
        events
    }

    /// Send the movers' own queued events (which can set off more movers,
    /// which can queue more; a chain stops after 16 rounds).
    fn send_queued(&mut self, events: &mut Vec<MoverEvent>) {
        for _ in 0..16 {
            let mut queued = Vec::new();
            for (i, m) in self.movers.iter_mut().enumerate() {
                for e in m.to_send.drain(..) {
                    queued.push((i, e));
                }
            }
            if queued.is_empty() {
                return;
            }
            for (i, e) in queued {
                let n = self.send(&e, events);
                events.push(MoverEvent::Event { mover: i, event: e, movers: n });
            }
        }
    }

    fn unsend(&mut self, event: &str, events: &mut Vec<MoverEvent>) {
        let now = self.time;
        for (i, m) in self.movers.iter_mut().enumerate() {
            if m.info.tag.eq_ignore_ascii_case(event) {
                m.untrigger(i, now, events);
            }
        }
    }

    /// Advance everything by `dt` seconds. `player` is moved if a mover
    /// carries or pushes it. Call before the player's own move.
    pub fn step(&mut self, dt: f32, fixed: &dyn World, player: &mut Pawn) -> Vec<MoverEvent> {
        let mut events = Vec::new();
        self.time += dt;
        let now = self.time;
        let extent = player.extent();
        let min_floor = player.params.min_floor_z;

        // Triggers the player is inside.
        let mut fire = Vec::new();
        let mut release = Vec::new();
        for (ti, t) in self.triggers.iter_mut().enumerate() {
            t.cooldown = (t.cooldown - dt).max(0.0);
            let i = &t.info;
            let d = player.location - v3(i.location);
            let inside = d.length_2d() < i.radius + extent.x && d.z.abs() < i.height + extent.z;
            let player_ok = matches!(i.trigger_type, 0 | 1 | 3 | 5);
            if inside && !t.touching && player_ok && !(i.once_only && t.used) && t.cooldown <= 0.0 {
                t.used = true;
                t.cooldown = i.retrigger_delay;
                fire.push((ti, i.event.clone()));
            }
            if !inside && t.touching {
                release.push(i.event.clone());
            }
            t.touching = inside;
        }
        for (ti, e) in fire {
            let n = self.send(&e, &mut events);
            events.push(MoverEvent::Triggered { trigger: ti, event: e, movers: n });
        }
        for e in release {
            self.unsend(&e, &mut events);
        }

        // Which mover the player stands on, and which they're pressed
        // against.
        let reach = self.settings.stand_reach;
        let down = Vec3::new(0.0, 0.0, -reach);
        let mut standing_on = None;
        let mut best = f32::MAX;
        for (i, m) in self.movers.iter().enumerate() {
            if let Some(h) = m.sweep(extent, player.location, player.location + down) {
                if !h.start_solid && h.normal.z >= min_floor && h.time < best {
                    best = h.time;
                    standing_on = Some(i);
                }
            }
        }
        if standing_on != self.riding {
            if let Some(i) = self.riding {
                events.push(MoverEvent::LeftBoard { mover: i });
            }
            if let Some(i) = standing_on {
                events.push(MoverEvent::Boarded { mover: i });
            }
            self.riding = standing_on;
        }
        let grow = Vec3::new(self.settings.bump_reach, self.settings.bump_reach, self.settings.bump_reach);
        for i in 0..self.movers.len() {
            let standing = standing_on == Some(i);
            let bumped = standing || {
                let m = &self.movers[i];
                m.overlaps_at(&m.motion(), extent + grow, player.location)
            };
            let m = &mut self.movers[i];
            if bumped && !m.was_touched {
                m.touched(i, now, standing, &mut events);
            }
            m.was_touched = bumped;
        }

        // Move each mover, carrying or pushing the player.
        for i in 0..self.movers.len() {
            let (pose, phase, key) = self.movers[i].advance(dt);
            let old = self.movers[i].pose;
            if pose == old {
                // Nothing moves (a wait, or keys in the same place): the
                // timer still runs.
                let m = &mut self.movers[i];
                m.phase = phase;
                m.key = key;
            } else {
                let step = Motion::new(old, pose);
                let built = self.movers[i].built;
                let new_motion = Motion::new(built, pose);
                let mut allowed = true;
                let others = WithMovers {
                    fixed,
                    movers: &self.movers,
                    skip: Some(i),
                };
                // Where the bit of mover under (or against) the player goes.
                let delta = step.apply(player.location) - player.location;
                let carried = standing_on == Some(i);
                if carried {
                    shove(player, delta, &others);
                }
                if self.movers[i].overlaps_at(&new_motion, extent, player.location) {
                    // In the way: try pushing them along with the mover.
                    let save = player.location;
                    shove(player, delta, &others);
                    if self.movers[i].overlaps_at(&new_motion, extent, player.location) {
                        player.location = save;
                        let what = self.movers[i].info.encroach;
                        match what {
                            Encroach::Crush | Encroach::Ignore => {}
                            Encroach::Stop | Encroach::Return => allowed = false,
                        }
                        let m = &mut self.movers[i];
                        if !allowed {
                            m.stats.blocked += 1;
                            events.push(MoverEvent::Blocked { mover: i, what });
                            if what == Encroach::Return {
                                if m.opening {
                                    m.start_closing(i, now, &mut events);
                                } else {
                                    m.start_opening(i, now, &mut events);
                                }
                            }
                        }
                    }
                }
                if allowed {
                    let m = &mut self.movers[i];
                    m.stats.carried_frames += carried as u32;
                    m.pose = pose;
                    m.phase = phase;
                    m.key = key;
                }
            }
            self.movers[i].settle(i, dt, now, &mut events);
        }
        self.send_queued(&mut events);
        // Don't log the same block every frame.
        events.dedup();
        events
    }
}

#[cfg(test)]
mod tests;
