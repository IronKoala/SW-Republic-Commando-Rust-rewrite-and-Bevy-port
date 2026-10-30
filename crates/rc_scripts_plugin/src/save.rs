//! The `[scripts]` section of a save file (see `rc_game_core::save`): the
//! director's state (`rc_scripts::save`: where each script is, what it
//! waits for, triggers, counters, conditions, broken props), plus what the
//! scripts changed in the game: parts no longer solid or hidden, bacta
//! dispensers, objectives, and checkpoints waiting for the player to get
//! up.
//!
//! Loading it marks the level as started, so the level-start events
//! (`levelstart`) aren't sent a second time. A save without this section
//! (made before it existed) starts the scripts from the beginning, as
//! before.

use crate::{LevelScript, Objectives, PART};
use bevy::prelude::*;
use rc_game_core::save::SavePart;
use rc_game_core::GameLog;
use rc_level_plugin::LevelParts;
use rc_scripts::save::DirectorSave;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptsSave {
    /// Level parts (by map export number) a script made not solid.
    #[serde(default)]
    pub not_solid: Vec<usize>,
    /// Level parts (by map export number) a script hid.
    #[serde(default)]
    pub hidden: Vec<usize>,
    /// Checkpoints reached while the player was down, still to be saved.
    #[serde(default)]
    pub held_checkpoints: Vec<String>,
    #[serde(default)]
    pub dispenser: Vec<DispenserSave>,
    #[serde(default)]
    pub objective: Vec<ObjectiveSave>,
    pub director: DirectorSave,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispenserSave {
    pub name: String,
    pub enabled: bool,
    /// Seconds before it can be used again.
    pub recharge_left: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveSave {
    /// When the script changed it, on the scripts' clock.
    pub time: f32,
    pub objective: i32,
    pub change_to: i32,
}

impl SavePart for ScriptsSave {
    const KEY: &'static str = "scripts";

    fn save(world: &mut World) -> Option<Self> {
        let not_solid = world.get_resource::<LevelParts>().map_or(Vec::new(), |p| {
            let mut v: Vec<usize> = p
                .list()
                .into_iter()
                .filter(|(_, _, _, solid, _)| !solid)
                .map(|(e, ..)| e)
                .collect();
            v.sort_unstable();
            v
        });
        let objective = world.get_resource::<Objectives>().map_or(Vec::new(), |o| {
            o.updates
                .iter()
                .map(|&(time, objective, change_to)| ObjectiveSave {
                    time,
                    objective,
                    change_to,
                })
                .collect()
        });
        let script = world.get_resource::<LevelScript>()?;
        let mut hidden: Vec<usize> = script.hidden.iter().copied().collect();
        hidden.sort_unstable();
        Some(ScriptsSave {
            not_solid,
            hidden,
            held_checkpoints: script.held.clone(),
            dispenser: script
                .dispensers
                .iter()
                .map(|d| DispenserSave {
                    name: d.name.clone(),
                    enabled: d.enabled,
                    recharge_left: d.recharge_left,
                })
                .collect(),
            objective,
            director: script.director.snapshot(),
        })
    }

    fn load(world: &mut World, d: Self) -> Result<(), String> {
        let parts = world.get_resource::<LevelParts>().cloned();
        let mut unknown = Vec::new();
        if let Some(p) = &parts {
            let known: Vec<usize> = p.list().into_iter().map(|(e, ..)| e).collect();
            for &e in &d.not_solid {
                if known.contains(&e) {
                    p.set_solid_export(e, false);
                } else {
                    unknown.push(format!("part {e}"));
                }
            }
        }
        if let Some(mut o) = world.get_resource_mut::<Objectives>() {
            o.updates = d
                .objective
                .iter()
                .map(|o| (o.time, o.objective, o.change_to))
                .collect();
        }
        let mut script = world
            .get_resource_mut::<LevelScript>()
            .ok_or("no level scripts on this map")?;
        let report = script.director.restore(&d.director);
        unknown.extend(report.unknown.iter().cloned());
        // Started already: no level-start events this time.
        script.started = true;
        script.hidden = d.hidden.iter().copied().collect();
        script.held = d.held_checkpoints.clone();
        for s in &d.dispenser {
            match script
                .dispensers
                .iter_mut()
                .find(|x| x.name.eq_ignore_ascii_case(&s.name))
            {
                Some(x) => {
                    x.enabled = s.enabled;
                    x.recharge_left = s.recharge_left.max(0.0);
                    x.in_use = false;
                }
                None => unknown.push(s.name.clone()),
            }
        }
        // Keep the counts the self-tests and logging read in step.
        script.stats.props_destroyed = script
            .director
            .destroyed_props()
            .into_iter()
            .map(|(_, tag)| tag)
            .collect();
        let states = script.director.script_states();
        let finished = states.iter().filter(|s| s.2 == "finished").count();
        let waiting = states.iter().filter(|s| s.2.starts_with("waits")).count();
        let log = world.resource::<GameLog>();
        log.line(format!(
            "{PART}: put back t={:.1}s: {} scripts ({finished} finished, {waiting} waiting), {} parts not solid, {} hidden, {} objectives; level start not sent again",
            d.director.time,
            states.len(),
            d.not_solid.len(),
            d.hidden.len(),
            d.objective.len()
        ));
        for (name, next, what) in &states {
            log.line(format!("{PART}:   {name}: next {next}, {what}"));
        }
        for n in &report.past_end {
            log.line(format!(
                "{PART}:   {n}: saved past its last action; finished"
            ));
        }
        for n in &unknown {
            log.line(format!(
                "{PART}:   {n} is in the save but not on this map; skipped"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayStats;
    use rc_level::scripts::{MapScripts, ScriptActor, ScriptObject, Val, Vals};
    use rc_movement::TriangleWorld;
    use rc_scripts::{Director, Settings};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    fn obj(export: usize, class: &str, name: &str, vals: &[(&str, Val)]) -> ScriptObject {
        let mut values = Vals::new();
        for (k, v) in vals {
            values.insert(k.to_string(), v.clone());
        }
        ScriptObject {
            export,
            class: class.into(),
            name: name.into(),
            values,
        }
    }

    fn actor(o: ScriptObject, actions: Vec<ScriptObject>) -> ScriptActor {
        ScriptActor {
            obj: o,
            location: [0.0; 3],
            actions,
        }
    }

    /// geo_01a in small: the level start runs a script that blows up a
    /// barricade prop (part 12), switches a blocking volume (part 13) off,
    /// then waits for `BactaUsed`.
    fn map() -> MapScripts {
        let t = |s: &str| Val::Text(s.into());
        let act =
            |class: &str, vals: &[(&str, Val)]| obj(0, &format!("ACTION_{class}"), class, vals);
        MapScripts {
            actors: vec![
                actor(
                    obj(
                        2,
                        "PlayerStart",
                        "PlayerStart0",
                        &[("Event", t("levelstart"))],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        12,
                        "Prop",
                        "Blockade01",
                        &[("Tag", t("Barricade")), ("Health", Val::Float(100.0))],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        1,
                        "ScriptedTrigger",
                        "ScriptedTrigger15",
                        &[("Tag", t("ScriptedTrigger"))],
                    ),
                    vec![
                        act("WaitForEvent", &[("ExternalEvent", t("levelstart"))]),
                        act("DestroyActor", &[("DestroyTag", t("Barricade"))]),
                        act("WaitForEvent", &[("ExternalEvent", t("BactaUsed"))]),
                        act("TriggerEvent", &[("Event", t("Done"))]),
                    ],
                ),
            ],
            failures: vec![],
            sounds: Default::default(),
        }
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        let tagged: HashMap<usize, (String, String)> = [
            (12, ("Prop".to_string(), "Barricade".to_string())),
            (
                13,
                ("BlockingVolume".to_string(), "BarricadeVolume".to_string()),
            ),
        ]
        .into();
        w.insert_resource(LevelParts::new(
            Arc::new(TriangleWorld::new(vec![])),
            &tagged,
            HashMap::new(),
        ));
        w.insert_resource(LevelScript {
            director: Director::new(&map(), &[], Settings::default()),
            dispensers: vec![],
            hidden: HashSet::new(),
            held: vec![],
            sounds: vec![],
            started: false,
            stats: PlayStats::default(),
        });
        w.insert_resource(Objectives::default());
        w
    }

    /// Play the start in one game (as `run` does), save, and load into a
    /// freshly loaded map: the scripts carry on from where they were, the
    /// level start isn't sent again, and the barricade stays gone.
    #[test]
    fn scripts_carry_on_after_a_load() {
        let mut a = world();
        {
            let parts = a.resource::<LevelParts>().clone();
            let mut s = a.resource_mut::<LevelScript>();
            s.started = true;
            s.director.start();
            for e in s.director.step(0.1, None) {
                if let rc_scripts::Effect::PropDestroyed { export, .. } = e {
                    parts.set_solid_export(export, false);
                    s.hidden.insert(export);
                }
            }
            parts.set_solid("BarricadeVolume", false);
            s.held.push("checkpoint0".into());
        }
        a.resource_mut::<Objectives>().updates.push((0.1, 1, 2));
        let saved = ScriptsSave::save(&mut a).expect("a section");
        assert_eq!(saved.not_solid, [12, 13]);
        assert_eq!(saved.hidden, [12]);
        let text = toml::to_string(&saved).unwrap();
        let back: ScriptsSave = toml::from_str(&text).unwrap();
        // Destroyed props have no health left at all (minus infinity).
        assert_eq!(
            toml::to_string(&back).unwrap(),
            text,
            "the file reads back the same"
        );

        let mut b = world();
        ScriptsSave::load(&mut b, back).unwrap();
        let parts = b.resource::<LevelParts>().list();
        assert!(parts.iter().all(|p| !p.3), "{parts:?}");
        assert_eq!(b.resource::<Objectives>().updates, [(0.1, 1, 2)]);
        let s = b.resource::<LevelScript>();
        assert!(s.started, "a loaded level counts as started");
        assert_eq!(s.held, ["checkpoint0"]);
        assert!(s.hidden.contains(&12));
        assert_eq!(s.stats.props_destroyed, ["Barricade"]);
        assert_eq!(
            s.director.snapshot(),
            a.resource::<LevelScript>().director.snapshot()
        );

        // It carries on: BactaUsed finishes the script; no level start.
        let mut s = b.resource_mut::<LevelScript>();
        s.director.fire("BactaUsed", "test");
        s.director.step(0.1, None);
        let sent: Vec<&str> = s.director.history.iter().map(|h| h.1.as_str()).collect();
        assert_eq!(sent, ["BactaUsed", "Done"]);
    }

    #[test]
    fn section_goes_through_the_file_and_back() {
        let save = ScriptsSave {
            not_solid: vec![12, 40],
            hidden: vec![40],
            held_checkpoints: vec!["Checkpoint2".into()],
            dispenser: vec![DispenserSave {
                name: "BactaDispenser0".into(),
                enabled: false,
                recharge_left: 3.5,
            }],
            objective: vec![ObjectiveSave {
                time: 20.0,
                objective: 1,
                change_to: 2,
            }],
            director: DirectorSave {
                time: 31.25,
                random_state: -5,
                ..Default::default()
            },
        };
        let text = toml::to_string(&save).unwrap();
        let back: ScriptsSave = toml::from_str(&text).unwrap();
        assert_eq!(back, save, "{text}");
    }
}
