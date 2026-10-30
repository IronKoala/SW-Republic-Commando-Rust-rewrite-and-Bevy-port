//! The `[movers]` section of a save file: where each door and lift is, what
//! it is doing, and what the map's touch triggers remember (see
//! `rc_game_core::save`). A door that was open stays open after a load, a
//! lift halfway up carries on from there, and a once-only door stays used.

use crate::{LevelMovers, PART};
use bevy::prelude::*;
use rc_game_core::save::SavePart;
use rc_game_core::GameLog;
use rc_movers::{MoverSnapshot, Phase, SetSnapshot, TriggerSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoversSave {
    /// Seconds since the level started, on the movers' clock.
    pub time: f32,
    /// The mover the player stood on, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub riding: Option<String>,
    #[serde(default)]
    pub movers: Vec<MoverSave>,
    #[serde(default)]
    pub triggers: Vec<TriggerSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoverSave {
    /// The mover's name in the map (`Mover37`).
    pub name: String,
    /// The key it is at, or last left (0 is shut, the last key open).
    pub key: usize,
    /// `rest`, `delay` (waiting to start opening), `moving` or `wait`
    /// (open, waiting to close).
    pub phase: String,
    /// For `moving`: from key, to key, and how far (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f32>,
    /// For `delay` and `wait`: seconds left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<f32>,
    pub opening: bool,
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub used: bool,
    #[serde(default)]
    pub touched: bool,
    #[serde(default)]
    pub run_started: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerSave {
    pub name: String,
    pub touching: bool,
    pub used: bool,
    pub cooldown: f32,
}

impl From<&MoverSnapshot> for MoverSave {
    fn from(m: &MoverSnapshot) -> Self {
        let (phase, from, to, t, left) = match m.phase {
            Phase::Rest => ("rest", None, None, None, None),
            Phase::Delay { left } => ("delay", None, None, None, Some(left)),
            Phase::Moving { from, to, t } => ("moving", Some(from), Some(to), Some(t), None),
            Phase::Wait { left } => ("wait", None, None, None, Some(left)),
        };
        MoverSave {
            name: m.name.clone(),
            key: m.key,
            phase: phase.into(),
            from,
            to,
            t,
            left,
            opening: m.opening,
            held: m.held,
            used: m.used,
            touched: m.touched,
            run_started: m.run_started,
        }
    }
}

impl MoverSave {
    fn snapshot(&self) -> Result<MoverSnapshot, String> {
        let left = self.left.unwrap_or(0.0).max(0.0);
        let phase = match self.phase.as_str() {
            "rest" => Phase::Rest,
            "delay" => Phase::Delay { left },
            "wait" => Phase::Wait { left },
            "moving" => Phase::Moving {
                from: self
                    .from
                    .ok_or_else(|| format!("{}: moving, but from no key", self.name))?,
                to: self
                    .to
                    .ok_or_else(|| format!("{}: moving, but to no key", self.name))?,
                t: self.t.unwrap_or(0.0),
            },
            other => return Err(format!("{}: unknown phase {other:?}", self.name)),
        };
        Ok(MoverSnapshot {
            name: self.name.clone(),
            key: self.key,
            phase,
            opening: self.opening,
            held: self.held,
            used: self.used,
            touched: self.touched,
            run_started: self.run_started,
        })
    }
}

impl SavePart for MoversSave {
    const KEY: &'static str = "movers";

    fn save(world: &mut World) -> Option<Self> {
        let movers = world.get_resource::<LevelMovers>()?;
        let snap = movers.read().snapshot();
        Some(MoversSave {
            time: snap.time,
            riding: snap.riding,
            movers: snap.movers.iter().map(MoverSave::from).collect(),
            triggers: snap
                .triggers
                .into_iter()
                .map(|t| TriggerSave {
                    name: t.name,
                    touching: t.touching,
                    used: t.used,
                    cooldown: t.cooldown,
                })
                .collect(),
        })
    }

    fn load(world: &mut World, d: Self) -> Result<(), String> {
        let movers = world
            .get_resource::<LevelMovers>()
            .ok_or("no doors or lifts on this map")?
            .clone();
        let mut problems = Vec::new();
        let snap = SetSnapshot {
            time: d.time,
            riding: d.riding.clone(),
            movers: d
                .movers
                .iter()
                .filter_map(|m| m.snapshot().map_err(|e| problems.push(e)).ok())
                .collect(),
            triggers: d
                .triggers
                .iter()
                .map(|t| TriggerSnapshot {
                    name: t.name.clone(),
                    touching: t.touching,
                    used: t.used,
                    cooldown: t.cooldown,
                })
                .collect(),
        };
        let report = movers.restore(&snap);
        let open = {
            let set = movers.read();
            set.movers.iter().filter(|m| !m.is_closed()).count()
        };
        let log = world.resource::<GameLog>();
        log.line(format!(
            "{PART}: put back t={:.1}s: {} movers ({open} not shut), {} triggers",
            d.time, report.movers, report.triggers
        ));
        for n in &report.unknown {
            log.line(format!(
                "{PART}:   {n} is in the save but not on this map; skipped"
            ));
        }
        for n in &report.bad_keys {
            log.line(format!(
                "{PART}:   {n}: saved key is past its last key; clamped"
            ));
        }
        for p in &problems {
            log.line(format!("{PART}:   ERROR {p}; left as the level starts"));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::actors::Placement;
    use rc_level::movers::{Encroach, Glide, MapMovers, MoverInfo, Sounds, MAX_KEYS};
    use rc_movement::{MoveParams, Pawn, TriangleWorld, Vec3 as UVec3};
    use rc_movers::{MoverSet, Settings};
    use std::sync::Arc;

    /// A door named `Mover5`, tag `Door1`, that rises 190 uu in 1.5 s and
    /// stays open 2 s.
    fn door_set() -> MoverSet {
        let mut key_pos = [[0.0; 3]; MAX_KEYS];
        key_pos[1] = [0.0, 0.0, 190.0];
        let info = MoverInfo {
            place: Placement {
                export: 7,
                class: "Mover".into(),
                name: "Mover5".into(),
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
            num_keys: 2,
            key_num: 0,
            move_times: [1.5; MAX_KEYS],
            stay_open_time: 2.0,
            delay_time: 0.0,
            glide: Glide::Smooth,
            encroach: Encroach::Return,
            state: "TriggerOpenTimed".into(),
            tag: "Door1".into(),
            opening_event: String::new(),
            opened_event: String::new(),
            closing_event: String::new(),
            closed_event: String::new(),
            auto_door: false,
            trigger_once_only: false,
            sounds: Sounds {
                opening: None,
                opened: None,
                closing: None,
                closed: None,
                moving: None,
            },
            defaulted: vec![],
        };
        let map = MapMovers {
            movers: vec![info],
            triggers: vec![],
            failures: vec![],
        };
        MoverSet::new(map, Settings::default(), |_| Vec::new())
    }

    fn world_with_door() -> (World, LevelMovers) {
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        let movers = LevelMovers::new(door_set(), Arc::new(TriangleWorld::new(vec![])));
        w.insert_resource(movers.clone());
        (w, movers)
    }

    /// Save a door halfway open from one game, load it into a freshly
    /// loaded map, and it is at the same place doing the same thing.
    #[test]
    fn a_door_half_open_loads_half_open() {
        let (mut a, door_a) = world_with_door();
        let mut far = Pawn::new(MoveParams::player_commando(), UVec3::new(5000.0, 0.0, 0.0));
        assert_eq!(door_a.send("Door1"), 1);
        for _ in 0..45 {
            door_a.step(1.0 / 60.0, &mut far);
        }
        let saved = MoversSave::save(&mut a).expect("a section");
        let text = toml::to_string(&saved).unwrap();
        assert!(text.contains("phase = \"moving\""), "{text}");

        let (mut b, door_b) = world_with_door();
        assert!(door_b.read().movers[0].is_closed());
        MoversSave::load(&mut b, toml::from_str(&text).unwrap()).unwrap();
        assert_eq!(door_b.read().snapshot(), door_a.read().snapshot());
        let (za, zb) = (
            door_a.read().movers[0].pose.pos.z,
            door_b.read().movers[0].pose.pos.z,
        );
        assert!(za > 10.0 && (za - zb).abs() < 1e-3, "{za} {zb}");
    }

    /// A section naming a mover this map hasn't got still loads the rest.
    #[test]
    fn unknown_movers_are_skipped() {
        let (mut b, door_b) = world_with_door();
        let mut saved = MoversSave::save(&mut b).unwrap();
        saved.movers[0].key = 1;
        saved.movers.push(MoverSave {
            name: "Mover99".into(),
            ..saved.movers[0].clone()
        });
        MoversSave::load(&mut b, saved).unwrap();
        assert_eq!(door_b.read().movers[0].key, 1);
        // A phase this version doesn't know is an error, not a crash.
        let mut bad = MoversSave::save(&mut b).unwrap();
        bad.movers[0].phase = "spinning".into();
        assert!(MoversSave::load(&mut b, bad).is_err());
    }

    #[test]
    fn every_phase_goes_through_the_file_and_back() {
        for phase in [
            Phase::Rest,
            Phase::Delay { left: 0.5 },
            Phase::Moving {
                from: 1,
                to: 2,
                t: 0.25,
            },
            Phase::Wait { left: 1.75 },
        ] {
            let m = MoverSnapshot {
                name: "Mover3".into(),
                key: 1,
                phase,
                opening: true,
                held: true,
                used: true,
                touched: false,
                run_started: 4.5,
            };
            let save = MoversSave {
                time: 9.0,
                riding: Some("Mover3".into()),
                movers: vec![MoverSave::from(&m)],
                triggers: vec![],
            };
            let text = toml::to_string(&save).unwrap();
            let back: MoversSave = toml::from_str(&text).unwrap();
            assert_eq!(back, save, "{text}");
            assert_eq!(back.movers[0].snapshot().unwrap(), m);
        }
    }
}
