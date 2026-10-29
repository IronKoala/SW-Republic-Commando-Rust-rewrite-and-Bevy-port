//! Reads a map's movers (doors, lifts, hatches: anything that slides or
//! turns between fixed key positions) and the triggers that set them off.
//!
//! What a mover stores, as Unreal Engine 2 describes it publicly (the
//! `Mover` and `Trigger` classes in the engine's script packages, read for
//! their property names; nothing copied):
//! - up to 8 key positions and turns (`KeyPos`, `KeyRot`), counted from a
//!   base (`BasePos`, `BaseRot`); `NumKeys` of them are used;
//! - which key it starts at (`KeyNum`), how long each move takes, how
//!   long it waits before going back (`StayOpenTime`), and how it eases in
//!   and out (`MoverGlideType`). Republic Commando keeps one time per key
//!   (`MoveTimes[k]`, the move from key k to key k+1) where UE2 has one
//!   `MoveTime`, found by listing what the game's movers set;
//! - what sets it off (`InitialState`, such as `BumpOpenTimed` or
//!   `TriggerOpenTimed`) and its `Tag`, which triggers name in their
//!   `Event`;
//! - what it does when it runs into someone (`MoverEncroachType`);
//! - Republic Commando's own events, sent when it starts opening, is open,
//!   starts closing and is closed (`OpeningEvent`, `OpenedEvent`,
//!   `ClosingEvent`, `ClosedEvent`), and whether it's an automatic door
//!   (`bAutoDoor`, read and logged, not acted on yet);
//! - sounds for opening, opened, closing, closed and while moving.
//!
//! A map only stores what a designer changed, so every value falls back
//! to the class's defaults (`crate::class_look`), then to the engine's own
//! defaults below. Like the rest of this crate it opens no files.

use crate::actors::{self, Placement};
use crate::bsp::Vec3;
use crate::class_look::{object_path, ClassLooks, Loaded};
use rc_package::properties::{Property, Value};
use rc_package::Package;

/// Most keys a mover can have. UE2 has 24; the game's maps use up to 12.
pub const MAX_KEYS: usize = 24;

/// Engine defaults, used when neither the map nor the class sets a value.
/// From the public UE2 `Mover` class defaults.
/// Republic Commando's `Mover` class sets no `MoveTimes`, so a mover whose
/// map and class set none would move instantly. Our own fallback: 1 s, the
/// UE2 `MoveTime` default.
pub const DEFAULT_MOVE_TIME: f32 = 1.0;
pub const DEFAULT_STAY_OPEN_TIME: f32 = 4.0;
pub const DEFAULT_NUM_KEYS: u8 = 2;

/// How a mover eases between keys (`MoverGlideType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glide {
    /// Constant speed (`MV_MoveByTime`).
    Linear,
    /// Starts and stops gently (`MV_GlideByTime`, the default).
    Smooth,
}

/// What a mover does when it runs into a player or character
/// (`MoverEncroachType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encroach {
    /// Stop where it is.
    Stop,
    /// Go back the way it came (the default).
    Return,
    /// Keep going and hurt them.
    Crush,
    /// Pass through.
    Ignore,
}

#[derive(Debug, Clone)]
pub struct Sounds {
    pub opening: Option<String>,
    pub opened: Option<String>,
    pub closing: Option<String>,
    pub closed: Option<String>,
    pub moving: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MoverInfo {
    /// Where the mover is placed, and its drawing values.
    pub place: Placement,
    /// Class first, then its parents.
    pub chain: Vec<String>,
    pub base_pos: Vec3,
    /// Unreal angle units (65536 = one turn).
    pub base_rot: [i32; 3],
    pub key_pos: [Vec3; MAX_KEYS],
    pub key_rot: [[i32; 3]; MAX_KEYS],
    pub num_keys: u8,
    /// The key it starts at.
    pub key_num: u8,
    /// Seconds for the move between key k and key k+1 (either way).
    pub move_times: [f32; MAX_KEYS],
    pub stay_open_time: f32,
    pub delay_time: f32,
    pub glide: Glide,
    pub encroach: Encroach,
    /// `InitialState`, e.g. `BumpOpenTimed`; empty if none was set.
    pub state: String,
    pub tag: String,
    /// Events it sends when it starts opening, is open, starts closing and
    /// is closed (empty if none). `opened` falls back to UE2's `Event`.
    pub opening_event: String,
    pub opened_event: String,
    pub closing_event: String,
    pub closed_event: String,
    /// `bAutoDoor`: Republic Commando's automatic doors.
    pub auto_door: bool,
    pub trigger_once_only: bool,
    pub sounds: Sounds,
    /// Values that were missing from both the map and the class, so the
    /// engine default was used. For the log.
    pub defaulted: Vec<&'static str>,
}

impl MoverInfo {
    /// Where key `k` puts the mover: position and turn.
    pub fn key(&self, k: usize) -> (Vec3, [i32; 3]) {
        let p = self.key_pos[k.min(MAX_KEYS - 1)];
        let r = self.key_rot[k.min(MAX_KEYS - 1)];
        (
            [
                self.base_pos[0] + p[0],
                self.base_pos[1] + p[1],
                self.base_pos[2] + p[2],
            ],
            [
                self.base_rot[0] + r[0],
                self.base_rot[1] + r[1],
                self.base_rot[2] + r[2],
            ],
        )
    }

    /// Does it ever turn, or only slide?
    pub fn turns(&self) -> bool {
        (0..self.num_keys as usize).any(|k| self.key_rot[k] != self.key_rot[0])
    }

    /// Seconds for one move between keys `a` and `b` (next to each other).
    pub fn segment_time(&self, a: usize, b: usize) -> f32 {
        self.move_times[a.min(b).min(MAX_KEYS - 1)]
    }

    /// Seconds to go from key 0 to the last key.
    pub fn open_time(&self) -> f32 {
        (0..self.num_keys.max(1) as usize - 1).map(|k| self.segment_time(k, k + 1)).sum()
    }

    /// Longest distance between its keys, Unreal units.
    pub fn travel(&self) -> f32 {
        let mut best = 0.0f32;
        for k in 1..self.num_keys as usize {
            let a = self.key_pos[0];
            let b = self.key_pos[k];
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            best = best.max(d);
        }
        best
    }
}

/// A trigger: touching it (or something else) sets off `event`.
#[derive(Debug, Clone)]
pub struct TriggerInfo {
    pub export: usize,
    pub class: String,
    pub name: String,
    pub location: Vec3,
    pub radius: f32,
    pub height: f32,
    pub event: String,
    pub tag: String,
    /// `TriggerType`: 0 player, 1 pawn, 2 class, 3 any actor, 4 shot,
    /// 5 human player (UE2 numbering).
    pub trigger_type: u8,
    pub once_only: bool,
    /// `InitialState`, e.g. `NormalTrigger`.
    pub state: String,
    pub retrigger_delay: f32,
}

#[derive(Debug, Default, Clone)]
pub struct MapMovers {
    pub movers: Vec<MoverInfo>,
    pub triggers: Vec<TriggerInfo>,
    /// Actors that looked like movers but couldn't be read, and why.
    pub failures: Vec<String>,
}

impl MapMovers {
    /// Just mover `i`, with no triggers. For testing one mover at a time.
    pub fn only(&self, i: usize) -> MapMovers {
        MapMovers {
            movers: self.movers.get(i).cloned().into_iter().collect(),
            triggers: Vec::new(),
            failures: Vec::new(),
        }
    }
}

/// Values from the map first, then the class. Names and object references
/// are resolved in the package they came from.
struct Lookup<'a> {
    map: &'a Package,
    props: &'a [Property],
    look: Option<&'a crate::class_look::ClassLook>,
}

impl<'a> Lookup<'a> {
    fn get(&self, name: &str, index: u32) -> Option<(&'a Package, &'a Property)> {
        if let Some(p) = self
            .props
            .iter()
            .find(|p| p.array_index == index && self.map.name(p.name).eq_ignore_ascii_case(name))
        {
            return Some((self.map, p));
        }
        self.look
            .and_then(|l| l.find(name, index))
            .map(|(l, p)| (&l.0, p))
    }

    fn float(&self, name: &str) -> Option<f32> {
        match self.get(name, 0)?.1.value {
            Value::Float(f) => Some(f),
            _ => None,
        }
    }

    fn byte(&self, name: &str) -> Option<u8> {
        match self.get(name, 0)?.1.value {
            Value::Byte(b) => Some(b),
            _ => None,
        }
    }

    fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name, 0)?.1.value {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// A name value. A map that sets it to `None` on purpose gets "None",
    /// not the class's value.
    fn name(&self, name: &str) -> Option<String> {
        let (pkg, p) = self.get(name, 0)?;
        match p.value {
            Value::Name(n) => Some(pkg.name(n).to_string()),
            _ => None,
        }
    }

    /// An event name: "None" means no event.
    fn event(&self, name: &str) -> Option<String> {
        self.name(name).filter(|s| !s.eq_ignore_ascii_case("None"))
    }

    fn object(&self, name: &str) -> Option<String> {
        let (pkg, p) = self.get(name, 0)?;
        match p.value {
            Value::Object(o) if o != 0 => Some(object_path(pkg, o).join(".")),
            _ => None,
        }
    }

    fn vec3(&self, name: &str, index: u32) -> Option<Vec3> {
        match &self.get(name, index)?.1.value {
            Value::Raw(b) if b.len() == 12 => {
                let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
                Some([f(0), f(1), f(2)])
            }
            _ => None,
        }
    }

    fn ints3(&self, name: &str, index: u32) -> Option<[i32; 3]> {
        match &self.get(name, index)?.1.value {
            Value::Raw(b) if b.len() == 12 => {
                let f = |i: usize| i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
                Some([f(0), f(1), f(2)])
            }
            _ => None,
        }
    }
}

/// Read every mover and trigger in the map. `actors` is the level's actor
/// list; `open` opens a script package by name, for class defaults.
pub fn read_movers(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    actor_list: &[Option<usize>],
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> MapMovers {
    let mut out = MapMovers::default();
    for &a in actor_list.iter().flatten() {
        let look = looks.for_actor(map, a, open);
        let class = map.export_class_name(&map.exports[a]).to_string();
        let is_mover = look.as_ref().map_or(class == "Mover", |l| l.is_a("Mover"));
        let is_trigger = look.as_ref().map_or(class == "Trigger", |l| l.is_a("Trigger"));
        if !is_mover && !is_trigger {
            continue;
        }
        let props = match actors::read_actor_properties(map, map_data, a) {
            Ok(p) => p,
            Err(e) => {
                out.failures.push(format!("{class} export {a}: {e}"));
                continue;
            }
        };
        let place = match actors::read_placement(map, map_data, a) {
            Ok(p) => p,
            Err(e) => {
                out.failures.push(format!("{class} export {a}: {e}"));
                continue;
            }
        };
        let l = Lookup {
            map,
            props: &props,
            look: look.as_ref(),
        };
        if is_trigger {
            out.triggers.push(TriggerInfo {
                export: a,
                class,
                name: place.name.clone(),
                location: place.location,
                radius: l.float("CollisionRadius").unwrap_or(22.0),
                height: l.float("CollisionHeight").unwrap_or(22.0),
                event: l.event("Event").unwrap_or_default(),
                tag: l.event("Tag").unwrap_or_default(),
                trigger_type: l.byte("TriggerType").unwrap_or(0),
                once_only: l.bool("bTriggerOnceOnly").unwrap_or(false),
                state: l.name("InitialState").unwrap_or_default(),
                retrigger_delay: l.float("ReTriggerDelay").unwrap_or(0.0),
            });
            continue;
        }
        let mut defaulted = Vec::new();
        let mut key_pos = [[0.0f32; 3]; MAX_KEYS];
        let mut key_rot = [[0i32; 3]; MAX_KEYS];
        for k in 0..MAX_KEYS {
            if let Some(v) = l.vec3("KeyPos", k as u32) {
                key_pos[k] = v;
            }
            if let Some(v) = l.ints3("KeyRot", k as u32) {
                key_rot[k] = v;
            }
        }
        let num_keys = l.byte("NumKeys").unwrap_or_else(|| {
            defaulted.push("NumKeys");
            DEFAULT_NUM_KEYS
        });
        // Each move's time; a key with none set takes the one before's
        // (our guess), and the first falls back to UE2's `MoveTime`.
        let mut move_times = [0.0f32; MAX_KEYS];
        let mut last = l.float("MoveTime");
        for (k, t) in move_times.iter_mut().enumerate() {
            match l.get("MoveTimes", k as u32).map(|(_, p)| &p.value) {
                Some(Value::Float(f)) if *f > 0.0 => last = Some(*f),
                _ => {}
            }
            *t = last.unwrap_or(DEFAULT_MOVE_TIME);
        }
        if last.is_none() {
            defaulted.push("MoveTimes");
        }
        let stay_open_time = l.float("StayOpenTime").unwrap_or_else(|| {
            defaulted.push("StayOpenTime");
            DEFAULT_STAY_OPEN_TIME
        });
        let key_num = l.byte("KeyNum").unwrap_or(0).min(MAX_KEYS as u8 - 1);
        // A mover with no base saved: it was placed at its starting key.
        let k = key_num as usize;
        let base_pos = l.vec3("BasePos", 0).unwrap_or_else(|| {
            defaulted.push("BasePos");
            let (p, kp) = (place.location, key_pos[k]);
            [p[0] - kp[0], p[1] - kp[1], p[2] - kp[2]]
        });
        let base_rot = l.ints3("BaseRot", 0).unwrap_or_else(|| {
            defaulted.push("BaseRot");
            let (r, kr) = (place.rotation, key_rot[k]);
            [r[0] - kr[0], r[1] - kr[1], r[2] - kr[2]]
        });
        let glide = match l.byte("MoverGlideType") {
            Some(0) => Glide::Linear,
            _ => Glide::Smooth,
        };
        let encroach = match l.byte("MoverEncroachType") {
            Some(0) => Encroach::Stop,
            Some(2) => Encroach::Crush,
            Some(3) => Encroach::Ignore,
            _ => Encroach::Return,
        };
        out.movers.push(MoverInfo {
            chain: look.as_ref().map(|l| l.chain.clone()).unwrap_or_else(|| vec![class.clone()]),
            base_pos,
            base_rot,
            key_pos,
            key_rot,
            num_keys: num_keys.clamp(1, MAX_KEYS as u8),
            key_num,
            move_times,
            stay_open_time,
            delay_time: l.float("DelayTime").unwrap_or(0.0),
            glide,
            encroach,
            state: l.name("InitialState").unwrap_or_default(),
            tag: l.event("Tag").unwrap_or_default(),
            opening_event: l.event("OpeningEvent").unwrap_or_default(),
            opened_event: l.event("OpenedEvent").or_else(|| l.event("Event")).unwrap_or_default(),
            closing_event: l.event("ClosingEvent").unwrap_or_default(),
            closed_event: l.event("ClosedEvent").unwrap_or_default(),
            auto_door: l.bool("bAutoDoor").unwrap_or(false),
            trigger_once_only: l.bool("bTriggerOnceOnly").unwrap_or(false),
            sounds: Sounds {
                opening: l.object("OpeningSound"),
                opened: l.object("OpenedSound"),
                closing: l.object("ClosingSound"),
                closed: l.object("ClosedSound"),
                moving: l.object("MoveAmbientSound"),
            },
            defaulted,
            place,
        });
    }
    out
}
