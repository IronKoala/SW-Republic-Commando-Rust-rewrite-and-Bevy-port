//! Effect spawners (`FXFactory` in the map): actors that put a particle
//! effect into the level when a level event switches them on, once or
//! again and again. geo_01a's gunships and Geonosian fighters flying
//! overhead, its random battle explosions, steam jets and sparks in other
//! levels all come from them. Plain Rust, Unreal units; the level scripts
//! part (`rc_scripts_plugin`) reads them from the map and runs them.
//!
//! How the original works, from the script source in `CTGame.u`
//! (`Factory` and `FXFactory`; read only, described in our own words):
//!
//! - It starts switched off. An event named like its `Tag` switches it on
//!   (only if it puts effects at itself, `SpawnMode` 2, or has
//!   `SpawnPoint`s sharing its tag); the same event again switches it off.
//! - Switching on makes one straight away, then a timer runs: every
//!   `SpawnInterval` seconds, or a random time between `SpawnInterval`
//!   and `SpawnIntervalMax` when that is larger, picked afresh each time.
//!   Each tick tries to make one more.
//! - It makes one only while fewer than `MaxAlive` of its effects are
//!   still there (an effect is there until its life is over) and it has
//!   some of its `Capacity` left or `bGenerateForever` is set. Each one it
//!   makes uses one of its capacity (not when it generates forever) and
//!   sends its `SpawnEvent`.
//! - A tick that can't make one, with the capacity used up and nothing of
//!   its own left, sends `ExhaustedEvent` and switches it off.
//! - Where: `SpawnMode` 2 at the spawner itself, turned as it is; 0 at its
//!   spawn points in turn; 1 at a random one (never the same twice in a
//!   row); 3 at the one nearest the player. Spawn points carry their own
//!   turn.
//! - `bCovert`: only where the player can't see it: not where it is inside
//!   the player's field of view with nothing in the way.
//! - `MinPlayerRange`/`MaxPlayerRange`: only while the player is that far
//!   from the place (when min < max).
//! - `FXFactory` adds: the effect's draw scale is picked between
//!   `EmitterDrawScale` Min and Max (both 1 by default), and the place is
//!   moved by a random `SpawnOffset` (the level's axes).
//! - `ACTION_ResetFactory` gives back the whole capacity (it doesn't
//!   switch the spawner on or off).
//!
//! Not done (no campaign map's effect spawner uses them; checked with all
//! 55 campaign maps on 2026-11-11): `bStartTimerAfterDeath`,
//! `bWaitForIllegalSpawns`, `MaxInWorld`, `EmitterDrawScale3D` (one map
//! sets it, to its default), and the physics and speed an effect is given
//! (`InitialPhysicsFromFactory`/`InitialSpeedFromFactory`: 8 spawners set a
//! speed, all with `PHYS_Trailer`, which follows an owner they don't
//! have).

use crate::def::Range;
use crate::sim::Rng;

/// Where a spawner may put an effect (a `SpawnPoint` sharing its tag).
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub location: [f32; 3],
    /// Unreal pitch, yaw, roll.
    pub rotation: [i32; 3],
}

/// One `FXFactory` as the map sets it, with the class defaults for what it
/// doesn't.
#[derive(Debug, Clone, PartialEq)]
pub struct FxFactory {
    /// Its name in the map (`FXFactory19`).
    pub name: String,
    /// The event that switches it on and off.
    pub tag: String,
    /// The effect class (`cFX_Gunships`).
    pub prototype: String,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub spots: Vec<Spot>,
    pub capacity: u32,
    pub max_alive: u32,
    pub generate_forever: bool,
    pub spawn_mode: u8,
    pub spawn_interval: f32,
    pub spawn_interval_max: f32,
    pub min_player_range: f32,
    pub max_player_range: f32,
    pub covert: bool,
    pub spawn_event: Option<String>,
    pub exhausted_event: Option<String>,
    pub draw_scale: Range,
    pub spawn_offset: [Range; 3],
}

impl Default for FxFactory {
    /// The class defaults (`Factory` and `FXFactory` in `CTGame.u`).
    fn default() -> Self {
        FxFactory {
            name: String::new(),
            tag: String::new(),
            prototype: String::new(),
            location: [0.0; 3],
            rotation: [0; 3],
            spots: Vec::new(),
            capacity: 2,
            max_alive: 1,
            generate_forever: false,
            spawn_mode: 2,
            spawn_interval: 1.0,
            spawn_interval_max: 0.0,
            min_player_range: 0.0,
            max_player_range: 0.0,
            covert: false,
            spawn_event: None,
            exhausted_event: None,
            draw_scale: Range::fixed(1.0),
            spawn_offset: [Range::fixed(0.0); 3],
        }
    }
}

/// "Put this effect here now."
#[derive(Debug, Clone, PartialEq)]
pub struct Spawn {
    /// Which spawner (its index in [`Spawners::list`]).
    pub factory: usize,
    /// Its number, for [`Spawners::ended`].
    pub id: u64,
    pub effect: String,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub draw_scale: f32,
}

/// What happened in one step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub spawns: Vec<Spawn>,
    /// Events to send (`SpawnEvent`, `ExhaustedEvent`), with the spawner's
    /// name.
    pub events: Vec<(String, String)>,
}

/// What a spawner needs to know about the player.
pub trait PlayerView {
    /// Where the player stands (the original measures to the pawn).
    fn location(&self) -> Option<[f32; 3]>;
    /// Whether the player could see `p`: inside their field of view and
    /// nothing in the way.
    fn sees(&self, p: [f32; 3]) -> bool;
}

/// A spawner's state.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub online: bool,
    pub remaining: u32,
    /// Effects of its own still there (by number, see [`Spawners::ended`]).
    pub alive: Vec<u64>,
    /// Seconds to the next tick, while online.
    pub timer: f32,
    pub next_spot: usize,
    pub was_once_online: bool,
}

/// Every effect spawner in a level.
#[derive(Debug, Clone)]
pub struct Spawners {
    pub list: Vec<FxFactory>,
    pub states: Vec<State>,
    rng: Rng,
    next_id: u64,
}

impl Spawners {
    pub fn new(list: Vec<FxFactory>, seed: u64) -> Self {
        let states = list
            .iter()
            .map(|f| State {
                online: false,
                remaining: f.capacity,
                alive: Vec::new(),
                timer: 0.0,
                next_spot: 0,
                was_once_online: false,
            })
            .collect();
        Spawners {
            list,
            states,
            rng: Rng::new(seed),
            next_id: 1,
        }
    }

    /// The number the next effect will get; effects are numbered so their
    /// end can be reported with [`Self::ended`].
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// An event was sent: every spawner tagged with it switches on, or off
    /// if it was on. Returns what switching on made at once.
    pub fn trigger(&mut self, event: &str, player: &dyn PlayerView) -> Step {
        let mut out = Step::default();
        for i in 0..self.list.len() {
            if !self.list[i].tag.eq_ignore_ascii_case(event) {
                continue;
            }
            if self.states[i].online {
                self.states[i].online = false;
                continue;
            }
            let f = &self.list[i];
            if f.spawn_mode != 2 && f.spots.is_empty() {
                // The original logs "Factory has no Spawn Points".
                continue;
            }
            self.states[i].online = true;
            self.states[i].was_once_online = true;
            if f.spawn_mode == 1 {
                self.states[i].next_spot = self.rng_index(f.spots.len());
            }
            self.generate(i, player, &mut out);
            self.set_timer(i);
        }
        out
    }

    /// `ACTION_ResetFactory`: the spawners tagged `tag` get their whole
    /// capacity back. Returns how many there were.
    pub fn reset(&mut self, tag: &str) -> usize {
        let mut n = 0;
        for (f, s) in self.list.iter().zip(&mut self.states) {
            if f.tag.eq_ignore_ascii_case(tag) {
                s.remaining = f.capacity;
                n += 1;
            }
        }
        n
    }

    /// `ACTION_StopFactorySpawning`: switch the spawners tagged `tag` off.
    pub fn stop(&mut self, tag: &str) -> usize {
        let mut n = 0;
        for (f, s) in self.list.iter().zip(&mut self.states) {
            if f.tag.eq_ignore_ascii_case(tag) {
                s.online = false;
                n += 1;
            }
        }
        n
    }

    /// The effect numbered `id` is over (its life ended).
    pub fn ended(&mut self, id: u64) {
        for s in &mut self.states {
            s.alive.retain(|&a| a != id);
        }
    }

    /// Move the timers on by `dt` seconds.
    pub fn step(&mut self, dt: f32, player: &dyn PlayerView) -> Step {
        let mut out = Step::default();
        for i in 0..self.list.len() {
            if !self.states[i].online {
                continue;
            }
            self.states[i].timer -= dt;
            if self.states[i].timer > 0.0 {
                continue;
            }
            // Online.Timer: set the next tick, then try to make one.
            self.set_timer(i);
            let made = self.generate(i, player, &mut out);
            let f = &self.list[i];
            let s = &mut self.states[i];
            if !made && s.remaining == 0 && !f.generate_forever && s.alive.is_empty() {
                if let Some(e) = &f.exhausted_event {
                    out.events.push((e.clone(), f.name.clone()));
                }
                s.online = false;
            }
        }
        out
    }

    fn set_timer(&mut self, i: usize) {
        let f = &self.list[i];
        let wait = if f.spawn_interval < f.spawn_interval_max {
            let k = self.rng.unit();
            f.spawn_interval + k * (f.spawn_interval_max - f.spawn_interval)
        } else {
            f.spawn_interval
        };
        // A timer of 0 never fires in Unreal.
        let s = &mut self.states[i];
        if wait <= 0.0 {
            s.timer = f32::INFINITY;
        } else {
            s.timer = wait;
        }
    }

    fn rng_index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((self.rng.unit() * n as f32) as usize).min(n - 1)
    }

    fn in_range(f: &FxFactory, p: [f32; 3], player: &dyn PlayerView) -> bool {
        if f.min_player_range >= f.max_player_range {
            return true;
        }
        let Some(at) = player.location() else {
            return false;
        };
        let d2: f32 = (0..3).map(|k| (at[k] - p[k]).powi(2)).sum();
        d2 >= f.min_player_range.powi(2) && d2 <= f.max_player_range.powi(2)
    }

    /// `GenerateActor`: true if it tried (room and capacity left), as the
    /// original's return value.
    fn generate(&mut self, i: usize, player: &dyn PlayerView, out: &mut Step) -> bool {
        let f = self.list[i].clone();
        {
            let s = &self.states[i];
            if s.alive.len() as u32 >= f.max_alive || (s.remaining == 0 && !f.generate_forever) {
                return false;
            }
        }
        let place = match f.spawn_mode {
            2 => Some(Spot {
                location: f.location,
                rotation: f.rotation,
            }),
            3 => player.location().and_then(|at| {
                f.spots
                    .iter()
                    .filter(|s| !f.covert || !player.sees(s.location))
                    .min_by(|a, b| {
                        let da: f32 = (0..3).map(|k| (a.location[k] - at[k]).powi(2)).sum();
                        let db: f32 = (0..3).map(|k| (b.location[k] - at[k]).powi(2)).sum();
                        da.total_cmp(&db)
                    })
                    .cloned()
            }),
            _ => f.spots.get(self.states[i].next_spot).cloned(),
        };
        let ok_place = place.filter(|p| {
            Self::in_range(&f, p.location, player)
                && (f.spawn_mode == 3 || !f.covert || !player.sees(p.location))
        });
        let mut spawned = false;
        if let Some(p) = ok_place {
            let offset = [0, 1, 2].map(|k| self.rng.range(f.spawn_offset[k]));
            let draw_scale = self.rng.range(f.draw_scale);
            let id = self.next_id;
            self.next_id += 1;
            self.states[i].alive.push(id);
            out.spawns.push(Spawn {
                factory: i,
                id,
                effect: f.prototype.clone(),
                location: [0, 1, 2].map(|k| p.location[k] + offset[k]),
                rotation: p.rotation,
                draw_scale,
            });
            spawned = true;
        }
        if spawned && !f.generate_forever {
            self.states[i].remaining -= 1;
        }
        // The next spawn point.
        match f.spawn_mode {
            1 => {
                let n = f.spots.len();
                if n > 1 {
                    loop {
                        let t = self.rng_index(n);
                        if t != self.states[i].next_spot {
                            self.states[i].next_spot = t;
                            break;
                        }
                    }
                }
            }
            0 => {
                let n = f.spots.len().max(1);
                self.states[i].next_spot = (self.states[i].next_spot + 1) % n;
            }
            _ => {}
        }
        if let Some(e) = &f.spawn_event {
            out.events.push((e.clone(), f.name.clone()));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct At(Option<[f32; 3]>, bool);
    impl PlayerView for At {
        fn location(&self) -> Option<[f32; 3]> {
            self.0
        }
        fn sees(&self, _: [f32; 3]) -> bool {
            self.1
        }
    }
    const NOWHERE: At = At(Some([0.0; 3]), false);

    fn overhead() -> FxFactory {
        // geo_01a's FXFactory19, as the map sets it.
        FxFactory {
            name: "FXFactory19".into(),
            tag: "OverheadVehicles".into(),
            prototype: "cFX_Gunships".into(),
            location: [-5728.0, 28149.0, 14770.0],
            rotation: [3376, -12847, -35],
            generate_forever: true,
            spawn_interval: 3.0,
            spawn_interval_max: 7.0,
            draw_scale: Range::fixed(2.0),
            spawn_offset: [
                Range::fixed(0.0),
                Range::new(-300.0, 300.0),
                Range::fixed(0.0),
            ],
            ..FxFactory::default()
        }
    }

    #[test]
    fn switches_on_makes_one_and_waits_for_it_to_end() {
        let mut s = Spawners::new(vec![overhead()], 7);
        assert!(
            s.step(10.0, &NOWHERE).spawns.is_empty(),
            "off until its event"
        );
        let first = s.trigger("overheadvehicles", &NOWHERE);
        assert_eq!(first.spawns.len(), 1);
        let sp = &first.spawns[0];
        assert_eq!(sp.effect, "cFX_Gunships");
        assert_eq!(sp.draw_scale, 2.0);
        assert_eq!(sp.rotation, [3376, -12847, -35]);
        assert!((sp.location[1] - 28149.0).abs() <= 300.0);
        // MaxAlive 1: nothing more while the first is there.
        for _ in 0..30 {
            assert!(s.step(1.0, &NOWHERE).spawns.is_empty());
        }
        s.ended(1);
        // The next tick (3-7 s apart) makes another.
        let mut n = 0;
        for _ in 0..8 {
            n += s.step(1.0, &NOWHERE).spawns.len();
        }
        assert_eq!(n, 1);
        // Forever: capacity isn't used up.
        assert_eq!(s.states[0].remaining, 2);
    }

    #[test]
    fn same_event_again_switches_off() {
        let mut s = Spawners::new(vec![overhead()], 7);
        s.trigger("OverheadVehicles", &NOWHERE);
        s.trigger("OverheadVehicles", &NOWHERE);
        s.ended(1);
        for _ in 0..20 {
            assert!(s.step(1.0, &NOWHERE).spawns.is_empty());
        }
    }

    #[test]
    fn capacity_runs_out_then_exhausted_and_reset_gives_it_back() {
        let f = FxFactory {
            name: "FXFactory3".into(),
            tag: "BattleExplosions".into(),
            prototype: "Dirt_Explo_Lrg".into(),
            capacity: 1,
            exhausted_event: Some("AllBlown".into()),
            ..FxFactory::default()
        };
        let mut s = Spawners::new(vec![f], 3);
        assert_eq!(s.trigger("BattleExplosions", &NOWHERE).spawns.len(), 1);
        assert_eq!(s.states[0].remaining, 0);
        // Still there: no "exhausted" yet.
        assert!(s.step(1.0, &NOWHERE).events.is_empty());
        s.ended(1);
        let st = s.step(1.0, &NOWHERE);
        assert_eq!(
            st.events,
            vec![("AllBlown".to_string(), "FXFactory3".to_string())]
        );
        assert!(!s.states[0].online);
        assert_eq!(s.reset("battleexplosions"), 1);
        assert_eq!(s.states[0].remaining, 1);
    }

    #[test]
    fn nearest_spawn_point_to_the_player_and_covert() {
        let f = FxFactory {
            name: "FXFactory3".into(),
            tag: "BattleExplosions".into(),
            prototype: "Dirt_Explo_Lrg".into(),
            spawn_mode: 3,
            spots: vec![
                Spot {
                    location: [1000.0, 0.0, 0.0],
                    rotation: [0; 3],
                },
                Spot {
                    location: [100.0, 0.0, 0.0],
                    rotation: [0, 16384, 0],
                },
            ],
            ..FxFactory::default()
        };
        let mut s = Spawners::new(vec![f.clone()], 3);
        let st = s.trigger("BattleExplosions", &NOWHERE);
        assert_eq!(st.spawns[0].location, [100.0, 0.0, 0.0]);
        assert_eq!(st.spawns[0].rotation, [0, 16384, 0]);
        // Covert while the player sees every spot: none.
        let mut s = Spawners::new(vec![FxFactory { covert: true, ..f }], 3);
        assert!(
            s.trigger("BattleExplosions", &At(Some([0.0; 3]), true))
                .spawns
                .is_empty()
        );
    }

    #[test]
    fn no_spawn_points_no_switching_on() {
        let f = FxFactory {
            tag: "T".into(),
            spawn_mode: 0,
            ..FxFactory::default()
        };
        let mut s = Spawners::new(vec![f], 1);
        assert!(s.trigger("T", &NOWHERE).spawns.is_empty());
        assert!(!s.states[0].online);
    }
}
