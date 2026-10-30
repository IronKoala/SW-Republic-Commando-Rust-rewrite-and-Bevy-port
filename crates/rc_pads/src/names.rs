//! Button names for on-screen prompts, from `data/input/button_names.toml`.
//!
//! Everything works on the names used in `bindings.toml` ("South",
//! "KeyW", "Left"), so a prompt can be made from the bindings file alone.

use crate::kind::{ControllerKind, Family};
use bevy::prelude::Resource;
use std::collections::BTreeMap;

/// The names file, read and checked.
#[derive(Resource, Debug, Clone, Default)]
pub struct ButtonNames {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl ButtonNames {
    pub fn parse(text: &str) -> Result<ButtonNames, String> {
        let sections: BTreeMap<String, BTreeMap<String, String>> =
            toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(ButtonNames { sections })
    }

    fn find(&self, section: &str, name: &str) -> Option<&str> {
        self.sections.get(section)?.get(name).map(String::as_str)
    }

    /// The name to show for a gamepad button (a bindings name like
    /// "South") on this kind of controller.
    pub fn pad<'a>(&'a self, kind: ControllerKind, button: &'a str) -> &'a str {
        let family = match kind.family() {
            Family::Xbox => "xbox",
            Family::PlayStation => "playstation",
            Family::Nintendo => "nintendo",
            Family::SteamDeck => "steam_deck",
            Family::Generic => "generic",
        };
        self.find(kind.name(), button)
            .or_else(|| self.find(family, button))
            .or_else(|| self.find("generic", button))
            .unwrap_or(button)
    }

    /// The name to show for a key (a bindings name like "KeyW").
    pub fn key(&self, key: &str) -> String {
        if let Some(n) = self.find("keyboard", key) {
            return n.to_string();
        }
        for prefix in ["Key", "Digit"] {
            if let Some(rest) = key.strip_prefix(prefix) {
                if !rest.is_empty() {
                    return rest.to_string();
                }
            }
        }
        if let Some(rest) = key.strip_prefix("Numpad") {
            return format!("Num {rest}");
        }
        key.to_string()
    }

    /// The name to show for a mouse button or wheel ("Left", "WheelUp").
    pub fn mouse<'a>(&'a self, button: &'a str) -> &'a str {
        self.find("mouse", button).unwrap_or(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> ButtonNames {
        ButtonNames::parse(include_str!("../../../data/input/button_names.toml")).unwrap()
    }

    #[test]
    fn each_family_names_the_bottom_button() {
        let n = shipped();
        assert_eq!(n.pad(ControllerKind::Xbox, "South"), "A");
        assert_eq!(n.pad(ControllerKind::PlayStation4, "South"), "Cross");
        assert_eq!(n.pad(ControllerKind::SwitchPro, "South"), "B");
        assert_eq!(n.pad(ControllerKind::JoyConPair, "East"), "A");
        assert_eq!(n.pad(ControllerKind::SteamDeck, "LeftTrigger2"), "L2");
        assert_eq!(n.pad(ControllerKind::Generic, "Start"), "Start");
    }

    #[test]
    fn kind_section_wins_over_family() {
        let n = shipped();
        assert_eq!(n.pad(ControllerKind::PlayStation5, "Select"), "Create");
        assert_eq!(n.pad(ControllerKind::PlayStation4, "Select"), "Share");
        // Not in ps5's own section: falls back to the family's.
        assert_eq!(n.pad(ControllerKind::PlayStation5, "North"), "Triangle");
    }

    #[test]
    fn keys_and_mouse() {
        let n = shipped();
        assert_eq!(n.key("KeyW"), "W");
        assert_eq!(n.key("Digit4"), "4");
        assert_eq!(n.key("ShiftLeft"), "Shift");
        assert_eq!(n.key("F1"), "F1");
        assert_eq!(n.key("Numpad5"), "Num 5");
        assert_eq!(n.mouse("Left"), "Left Mouse");
        // An empty file still gives readable names.
        let e = ButtonNames::default();
        assert_eq!(e.pad(ControllerKind::Xbox, "South"), "South");
        assert_eq!(e.key("KeyQ"), "Q");
    }
}
