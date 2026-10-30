//! Which controller is this? Used for the button names on screen.
//!
//! SDL tells us directly. gilrs (the fallback reader) only gives the USB
//! vendor and product numbers and a name, so those are matched here. The
//! numbers are the public USB ids of each maker's controllers.

use bevy::prelude::Component;

/// A kind of controller, as precisely as we can tell.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ControllerKind {
    Xbox,
    PlayStation4,
    PlayStation5,
    SwitchPro,
    JoyConLeft,
    JoyConRight,
    /// Two Joy-Cons held together, used as one controller.
    JoyConPair,
    GameCube,
    SteamDeck,
    /// The Steam Controller, or Steam Input's own virtual controller.
    Steam,
    #[default]
    Generic,
}

/// Button-name sets in `button_names.toml`. Controllers of one family share
/// their names (a Joy-Con pair and a Switch Pro both say A, B, X, Y in the
/// Nintendo places).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Xbox,
    PlayStation,
    Nintendo,
    SteamDeck,
    Generic,
}

impl ControllerKind {
    pub fn family(self) -> Family {
        match self {
            ControllerKind::Xbox | ControllerKind::Steam => Family::Xbox,
            ControllerKind::PlayStation4 | ControllerKind::PlayStation5 => Family::PlayStation,
            ControllerKind::SwitchPro
            | ControllerKind::JoyConLeft
            | ControllerKind::JoyConRight
            | ControllerKind::JoyConPair
            | ControllerKind::GameCube => Family::Nintendo,
            ControllerKind::SteamDeck => Family::SteamDeck,
            ControllerKind::Generic => Family::Generic,
        }
    }

    /// The name used in the log and in `button_names.toml` sections.
    pub fn name(self) -> &'static str {
        match self {
            ControllerKind::Xbox => "xbox",
            ControllerKind::PlayStation4 => "ps4",
            ControllerKind::PlayStation5 => "ps5",
            ControllerKind::SwitchPro => "switch_pro",
            ControllerKind::JoyConLeft => "joycon_left",
            ControllerKind::JoyConRight => "joycon_right",
            ControllerKind::JoyConPair => "joycon_pair",
            ControllerKind::GameCube => "gamecube",
            ControllerKind::SteamDeck => "steam_deck",
            ControllerKind::Steam => "steam",
            ControllerKind::Generic => "generic",
        }
    }

    /// Guess from USB ids and the name the system gives. Used when SDL is
    /// not there to say.
    pub fn detect(vendor: Option<u16>, product: Option<u16>, name: &str) -> ControllerKind {
        const MICROSOFT: u16 = 0x045e;
        const SONY: u16 = 0x054c;
        const NINTENDO: u16 = 0x057e;
        const VALVE: u16 = 0x28de;
        match (vendor, product) {
            (Some(SONY), Some(0x0ce6 | 0x0df2)) => return ControllerKind::PlayStation5,
            (Some(SONY), Some(_)) => return ControllerKind::PlayStation4,
            (Some(NINTENDO), Some(0x2006)) => return ControllerKind::JoyConLeft,
            (Some(NINTENDO), Some(0x2007)) => return ControllerKind::JoyConRight,
            // The charging grip, which joins two Joy-Cons into one controller.
            (Some(NINTENDO), Some(0x200e)) => return ControllerKind::JoyConPair,
            (Some(NINTENDO), Some(0x0337)) => return ControllerKind::GameCube,
            (Some(NINTENDO), Some(_)) => return ControllerKind::SwitchPro,
            (Some(VALVE), Some(0x1205)) => return ControllerKind::SteamDeck,
            (Some(VALVE), Some(_)) => return ControllerKind::Steam,
            (Some(MICROSOFT), Some(_)) => return ControllerKind::Xbox,
            _ => {}
        }
        // Many third-party pads copy one of the big three; their names say
        // which.
        let n = name.to_ascii_lowercase();
        if n.contains("dualsense") || n.contains("ps5") {
            ControllerKind::PlayStation5
        } else if n.contains("playstation") || n.contains("dualshock") || n.contains("ps4") {
            ControllerKind::PlayStation4
        } else if n.contains("joy-con (l)") {
            ControllerKind::JoyConLeft
        } else if n.contains("joy-con (r)") {
            ControllerKind::JoyConRight
        } else if n.contains("joy-con") {
            ControllerKind::JoyConPair
        } else if n.contains("switch") || n.contains("nintendo") {
            ControllerKind::SwitchPro
        } else if n.contains("steam deck") {
            ControllerKind::SteamDeck
        } else if n.contains("steam") {
            ControllerKind::Steam
        } else if n.contains("xbox") || n.contains("xinput") || n.contains("x-box") {
            ControllerKind::Xbox
        } else {
            ControllerKind::Generic
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids() {
        let d = |v, p| ControllerKind::detect(Some(v), Some(p), "");
        assert_eq!(d(0x045e, 0x0b13), ControllerKind::Xbox);
        assert_eq!(d(0x054c, 0x09cc), ControllerKind::PlayStation4);
        assert_eq!(d(0x054c, 0x0ce6), ControllerKind::PlayStation5);
        assert_eq!(d(0x057e, 0x2009), ControllerKind::SwitchPro);
        assert_eq!(d(0x057e, 0x2006), ControllerKind::JoyConLeft);
        assert_eq!(d(0x28de, 0x1205), ControllerKind::SteamDeck);
        assert_eq!(d(0x28de, 0x11ff), ControllerKind::Steam);
        assert_eq!(ControllerKind::Steam.family(), Family::Xbox);
    }

    #[test]
    fn names_when_ids_are_unknown() {
        let n = |s| ControllerKind::detect(Some(0x1234), Some(1), s);
        assert_eq!(
            n("Wireless Controller DualSense"),
            ControllerKind::PlayStation5
        );
        assert_eq!(n("PowerA Xbox Series X Controller"), ControllerKind::Xbox);
        assert_eq!(n("8BitDo Pro 2"), ControllerKind::Generic);
        assert_eq!(
            ControllerKind::detect(None, None, "Nintendo Switch Pro Controller"),
            ControllerKind::SwitchPro
        );
    }
}
