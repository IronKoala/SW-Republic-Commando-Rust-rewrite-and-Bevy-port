//! The mission objectives, as the original keeps them.
//!
//! Each map has one `MissionObjectives` actor (engine.u) with a list,
//! `MissionObj`, of objectives: words (`Objective`, "Use bacta to heal")
//! and a state (`Stat`: hidden, incomplete, completed, failed, removed).
//! Its script, read from engine.u and gameplay.u:
//!
//! - `PostBeginPlay`: `CurrentObjective` (default -1) becomes the last
//!   objective the map starts as incomplete.
//! - `Trigger` (any event with the actor's `Tag`, usually
//!   `MissionObjectives`, sent by `ACTION_TriggerEvent`): move on one,
//!   skip any that aren't hidden, and make that one incomplete (shown).
//! - `ACTION_ObjectiveUpdate`: `UpdateAction` 0 (the default) completes
//!   `ObjectiveNum`, 1 fails it, 2 removes it and makes `ChangeToObjNum`
//!   incomplete.
//!
//! The helmet (Engine.dll `UHelmet::DrawCurrentObjectiveText`) shows the
//! first incomplete objective in the list, in capitals.

use rc_level::scripts::{MapScripts, Val};

pub const HIDDEN: u8 = 0;
pub const INCOMPLETE: u8 = 1;
pub const COMPLETED: u8 = 2;
pub const FAILED: u8 = 3;
pub const REMOVED: u8 = 4;

/// The map's objectives and where they stand.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MissionList {
    /// The event that reveals the next one (the actor's `Tag`), lower case.
    pub tag: String,
    /// (words, state) in the map's order.
    pub list: Vec<(String, u8)>,
    pub current: i32,
}

impl MissionList {
    /// The map's `MissionObjectives` actor, if it has one.
    pub fn from_map(map: &MapScripts) -> Option<MissionList> {
        let a = map.of_class("MissionObjectives").next()?;
        let list = match a.obj.get("MissionObj") {
            Some(Val::Structs(s)) => s
                .iter()
                .map(|v| {
                    let words = match v.get("Objective") {
                        Some(Val::Text(t)) => t.clone(),
                        _ => String::new(),
                    };
                    let stat = match v.get("Stat") {
                        Some(Val::Byte(b)) => *b,
                        _ => HIDDEN,
                    };
                    (words, stat)
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(MissionList::new(a.obj.tag(), list))
    }

    pub fn new(tag: &str, list: Vec<(String, u8)>) -> MissionList {
        // PostBeginPlay.
        let current = list
            .iter()
            .rposition(|(_, s)| *s == INCOMPLETE)
            .map_or(-1, |i| i as i32);
        MissionList {
            tag: tag.to_ascii_lowercase(),
            list,
            current,
        }
    }

    /// An event was sent: if it is ours, reveal the next objective.
    /// Returns the one revealed.
    pub fn event(&mut self, name: &str) -> Option<usize> {
        if !name.eq_ignore_ascii_case(&self.tag) || self.list.is_empty() {
            return None;
        }
        let len = self.list.len() as i32;
        self.current = (self.current + 1).min(len - 1);
        while self.list[self.current as usize].1 != HIDDEN {
            self.current += 1;
            if self.current >= len || self.current < 0 {
                return None;
            }
        }
        self.list[self.current as usize].1 = INCOMPLETE;
        Some(self.current as usize)
    }

    /// `ACTION_ObjectiveUpdate`.
    pub fn update(&mut self, action: u8, num: i32, change_to: i32) {
        let set = |l: &mut Vec<(String, u8)>, i: i32, s: u8| {
            if let Some(o) = usize::try_from(i).ok().and_then(|i| l.get_mut(i)) {
                o.1 = s;
            }
        };
        match action {
            1 => set(&mut self.list, num, FAILED),
            2 => {
                set(&mut self.list, num, REMOVED);
                set(&mut self.list, change_to, INCOMPLETE);
            }
            _ => set(&mut self.list, num, COMPLETED),
        }
    }

    /// What the helmet shows: the first incomplete objective, in capitals.
    pub fn shown(&self) -> Option<String> {
        self.list
            .iter()
            .find(|(w, s)| *s == INCOMPLETE && !w.is_empty())
            .map(|(w, _)| w.to_uppercase())
    }

    pub fn states(&self) -> Vec<u8> {
        self.list.iter().map(|o| o.1).collect()
    }

    /// Put saved states back (ignored if the list doesn't match).
    pub fn restore(&mut self, states: &[u8], current: i32) {
        if states.len() == self.list.len() {
            for (o, &s) in self.list.iter_mut().zip(states) {
                o.1 = s;
            }
            self.current = current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo_01a() -> MissionList {
        MissionList::new(
            "MissionObjectives",
            [
                "Use bacta to heal",
                "Secure area around crash",
                "Pick up grenades",
            ]
            .iter()
            .map(|w| (w.to_string(), HIDDEN))
            .collect(),
        )
    }

    #[test]
    fn events_reveal_objectives_in_order() {
        let mut m = geo_01a();
        assert_eq!(m.shown(), None);
        assert_eq!(m.event("Other"), None);
        assert_eq!(m.event("missionobjectives"), Some(0));
        assert_eq!(m.shown().as_deref(), Some("USE BACTA TO HEAL"));
        assert_eq!(m.event("MissionObjectives"), Some(1));
        // The first incomplete one is still shown.
        assert_eq!(m.shown().as_deref(), Some("USE BACTA TO HEAL"));
        m.update(0, 0, 0);
        assert_eq!(m.shown().as_deref(), Some("SECURE AREA AROUND CRASH"));
        m.update(2, 1, 2);
        assert_eq!(m.shown().as_deref(), Some("PICK UP GRENADES"));
        // Nothing hidden is left: the event does nothing.
        assert_eq!(m.event("MissionObjectives"), None);
        // Out-of-range numbers are ignored.
        m.update(0, 9, 0);
        m.update(1, -1, 0);
    }

    #[test]
    fn starts_at_the_last_incomplete_one() {
        let m = MissionList::new(
            "t",
            vec![
                ("a".into(), INCOMPLETE),
                ("b".into(), INCOMPLETE),
                ("c".into(), HIDDEN),
            ],
        );
        assert_eq!(m.current, 1);
        assert_eq!(m.shown().as_deref(), Some("A"));
    }
}
