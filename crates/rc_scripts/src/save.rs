//! What the director keeps for a saved game: where each script is, what it
//! waits for, which triggers were used, what the counters and conditions
//! say, which props are broken, and events not handed out yet. Matched by
//! the map's actor names when loading, so a save stays readable and a
//! changed map loads what still fits.
//!
//! Counts for the log (`stats`, `history`) aren't kept: they restart.

use crate::{Director, Wait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DirectorSave {
    /// Seconds since the level started, on the scripts' clock. Script
    /// timers (`waits_until`) count on it.
    pub time: f32,
    /// Where the random numbers (`ACTION_WaitForRandomTimer`,
    /// `ACTION_IfRandomPct`) carry on from.
    pub random_state: i64,
    #[serde(default)]
    pub script: Vec<ScriptSave>,
    #[serde(default)]
    pub trigger: Vec<TriggerSave>,
    #[serde(default)]
    pub counter: Vec<CounterSave>,
    #[serde(default)]
    pub condition: Vec<ConditionSave>,
    #[serde(default)]
    pub prop: Vec<PropSave>,
    /// Events sent but not handed out yet (normally none).
    #[serde(default)]
    pub pending: Vec<EventSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptSave {
    pub name: String,
    /// The action it runs next (counted from 0).
    pub next: usize,
    pub running: bool,
    pub finished: bool,
    /// The event it waits for (lower case), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_for: Option<String>,
    /// The scripts' clock time it waits until, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_until: Option<f32>,
    /// The action it waits for the game to finish (a character walking
    /// somewhere), if any; `waits_until` is then its time-out. After a
    /// load the game starts that action again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_for_action: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerSave {
    pub name: String,
    pub active: bool,
    pub touching: bool,
    pub used: bool,
    pub cooldown: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterSave {
    pub name: String,
    pub left: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionSave {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropSave {
    pub name: String,
    pub health: f32,
    pub destroyed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSave {
    pub event: String,
    pub by: String,
}

/// What [`Director::restore`] couldn't match up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RestoreReport {
    /// How many things were put back.
    pub restored: usize,
    /// Names in the save this map doesn't have (skipped).
    pub unknown: Vec<String>,
    /// Scripts whose saved next action is past their end (finished).
    pub past_end: Vec<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Director {
    /// Everything that changes while the level is played, for a saved
    /// game.
    pub fn snapshot(&self) -> DirectorSave {
        let mut counters: Vec<CounterSave> = self
            .counters
            .values()
            .flatten()
            .map(|c| CounterSave {
                name: c.name.clone(),
                left: c.left,
            })
            .collect();
        counters.sort_by(|a, b| a.name.cmp(&b.name));
        let mut conditions: Vec<ConditionSave> = self
            .conditions
            .values()
            .flatten()
            .map(|c| ConditionSave {
                name: c.name.clone(),
                enabled: c.enabled,
            })
            .collect();
        conditions.sort_by(|a, b| a.name.cmp(&b.name));
        DirectorSave {
            time: self.time,
            random_state: self.rng as i64,
            script: self
                .scripts
                .iter()
                .map(|s| ScriptSave {
                    name: s.name.clone(),
                    next: s.pc,
                    running: s.running,
                    finished: s.finished,
                    waits_for: match &s.wait {
                        Wait::Event(e) => Some(e.clone()),
                        _ => None,
                    },
                    waits_until: match s.wait {
                        Wait::Until(t) | Wait::Game { until: t, .. } => Some(t),
                        _ => None,
                    },
                    waits_for_action: match s.wait {
                        Wait::Game { index, .. } => Some(index),
                        _ => None,
                    },
                })
                .collect(),
            trigger: self
                .triggers
                .iter()
                .map(|t| TriggerSave {
                    name: t.info.name.clone(),
                    active: t.active,
                    touching: t.touching,
                    used: t.used,
                    cooldown: t.cooldown,
                })
                .collect(),
            counter: counters,
            condition: conditions,
            prop: self
                .props
                .iter()
                .map(|p| PropSave {
                    name: p.name.clone(),
                    health: p.health,
                    destroyed: p.destroyed,
                })
                .collect(),
            pending: self
                .queue
                .iter()
                .map(|(event, by)| EventSave {
                    event: event.clone(),
                    by: by.clone(),
                })
                .collect(),
        }
    }

    /// Put back a [`Self::snapshot`] taken on the same map, in place of
    /// starting the level. Anything the save doesn't name stays as the map
    /// starts it.
    pub fn restore(&mut self, save: &DirectorSave) -> RestoreReport {
        let mut r = RestoreReport::default();
        self.time = save.time;
        self.rng = (save.random_state as u64).max(1);
        for s in &save.script {
            let Some(sc) = self.scripts.iter_mut().find(|x| same(&x.name, &s.name)) else {
                r.unknown.push(s.name.clone());
                continue;
            };
            sc.pc = s.next.min(sc.actions.len());
            sc.running = s.running;
            sc.finished = s.finished;
            if s.next > sc.actions.len() {
                r.past_end.push(s.name.clone());
                sc.finished = true;
            }
            sc.wait = match (&s.waits_for, s.waits_until, s.waits_for_action) {
                (Some(e), _, _) => Wait::Event(e.to_ascii_lowercase()),
                // Run the unfinished character action again, so the game
                // hears it (its characters were put back from the save,
                // not mid-walk).
                (None, _, Some(i)) if i < sc.actions.len() => {
                    sc.pc = i;
                    Wait::Nothing
                }
                (None, Some(t), _) => Wait::Until(t),
                _ => Wait::Nothing,
            };
            r.restored += 1;
        }
        for s in &save.trigger {
            let Some(t) = self
                .triggers
                .iter_mut()
                .find(|x| same(&x.info.name, &s.name))
            else {
                r.unknown.push(s.name.clone());
                continue;
            };
            t.active = s.active;
            t.touching = s.touching;
            t.used = s.used;
            t.cooldown = s.cooldown.max(0.0);
            r.restored += 1;
        }
        for s in &save.counter {
            let Some(c) = self
                .counters
                .values_mut()
                .flatten()
                .find(|x| same(&x.name, &s.name))
            else {
                r.unknown.push(s.name.clone());
                continue;
            };
            c.left = s.left;
            r.restored += 1;
        }
        for s in &save.condition {
            let Some(c) = self
                .conditions
                .values_mut()
                .flatten()
                .find(|x| same(&x.name, &s.name))
            else {
                r.unknown.push(s.name.clone());
                continue;
            };
            c.enabled = s.enabled;
            r.restored += 1;
        }
        for s in &save.prop {
            let Some(p) = self.props.iter_mut().find(|x| same(&x.name, &s.name)) else {
                r.unknown.push(s.name.clone());
                continue;
            };
            p.health = s.health;
            p.destroyed = s.destroyed;
            r.restored += 1;
        }
        self.queue = save
            .pending
            .iter()
            .map(|e| (e.event.clone(), e.by.clone()))
            .collect();
        self.out.clear();
        r
    }

    /// Props destroyed so far: (export, tag).
    pub fn destroyed_props(&self) -> Vec<(usize, String)> {
        self.props
            .iter()
            .filter(|p| p.destroyed)
            .map(|p| (p.export, p.tag.clone()))
            .collect()
    }
}
