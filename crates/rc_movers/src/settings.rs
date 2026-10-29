//! `data/movers/movers.toml`: the values that are our own choices, not
//! the map's, so they can be changed without rebuilding.
//!
//! The format is a small part of TOML: `[section]` lines, then
//! `name = value` lines, `#` starts a comment. Numbers are plain; text is
//! in double quotes. An unknown name or a bad value is an error with its
//! line number, so a typo in a mod is reported, not ignored.

use crate::Behaviour;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// How close (uu) the player's box must come to a mover to count as
    /// bumping it.
    pub bump_reach: f32,
    /// How far below the player's feet (uu) a mover may be and still count
    /// as stood on.
    pub stand_reach: f32,
    /// What each `InitialState` name does, lower-case name to behaviour.
    /// The game's own state names can be added here (or a mod's).
    pub states: HashMap<String, Behaviour>,
    /// What a mover with an unknown state does.
    pub unknown_state: Behaviour,
}

impl Default for Settings {
    fn default() -> Self {
        let states = [
            ("triggeropentimed", Behaviour::TriggerOpenTimed),
            ("triggertoggle", Behaviour::TriggerToggle),
            ("triggercontrol", Behaviour::TriggerControl),
            ("triggerpound", Behaviour::TriggerPound),
            ("bumpopentimed", Behaviour::BumpOpenTimed),
            ("standopentimed", Behaviour::StandOpenTimed),
            ("bumpbutton", Behaviour::BumpButton),
            ("constantloop", Behaviour::Loop),
            ("loopmove", Behaviour::Loop),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Settings {
            bump_reach: 4.0,
            stand_reach: 4.0,
            states,
            unknown_state: Behaviour::TriggerOpenTimed,
        }
    }
}

impl Settings {
    /// What a mover with this `InitialState` does. Empty means the engine
    /// default: UE2 movers start in `BumpOpenTimed`.
    pub fn behaviour(&self, state: &str) -> (Behaviour, bool) {
        let key = if state.is_empty() {
            "bumpopentimed".to_string()
        } else {
            state.to_ascii_lowercase()
        };
        match self.states.get(&key) {
            Some(b) => (*b, true),
            None => (self.unknown_state, false),
        }
    }

    /// Start from `self` and apply the file's values.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        let mut section = String::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = s.trim().to_ascii_lowercase();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected `name = value`, got `{line}`"))?;
            let (key, value) = (key.trim(), value.trim());
            let number = || {
                value
                    .parse::<f32>()
                    .map_err(|_| format!("line {n}: `{value}` is not a number"))
            };
            let text = || {
                value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .ok_or_else(|| format!("line {n}: `{value}` should be in double quotes"))
            };
            let behaviour = || -> Result<Behaviour, String> {
                let t = text()?;
                Behaviour::from_name(t).ok_or_else(|| {
                    format!(
                        "line {n}: unknown behaviour `{t}` (one of: {})",
                        Behaviour::ALL.iter().map(|b| b.name()).collect::<Vec<_>>().join(", ")
                    )
                })
            };
            match (section.as_str(), key) {
                ("touch", "bump_reach") => self.bump_reach = number()?,
                ("touch", "stand_reach") => self.stand_reach = number()?,
                ("states", "unknown") => self.unknown_state = behaviour()?,
                ("states", state) => {
                    self.states.insert(state.to_ascii_lowercase(), behaviour()?);
                }
                _ => return Err(format!("line {n}: unknown setting `{key}` in [{section}]")),
            }
        }
        Ok(self)
    }
}

fn strip_comment(line: &str) -> &str {
    // A `#` inside quotes is text, not a comment.
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_values_and_states() {
        let s = Settings::default()
            .with_overrides(
                "# test\n[touch]\nbump_reach = 6 # uu\n[states]\nMyDoorState = \"trigger_toggle\"\nunknown = \"static\"\n",
            )
            .unwrap();
        assert_eq!(s.bump_reach, 6.0);
        assert_eq!(s.behaviour("MYDOORSTATE"), (Behaviour::TriggerToggle, true));
        assert_eq!(s.behaviour("Whatever"), (Behaviour::Static, false));
        assert_eq!(s.behaviour(""), (Behaviour::BumpOpenTimed, true));
    }

    #[test]
    fn typos_are_errors() {
        let e = Settings::default().with_overrides("[touch]\nbump_raech = 1\n").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
        let e = Settings::default().with_overrides("[states]\nX = \"fly\"\n").unwrap_err();
        assert!(e.contains("unknown behaviour"), "{e}");
    }
}
