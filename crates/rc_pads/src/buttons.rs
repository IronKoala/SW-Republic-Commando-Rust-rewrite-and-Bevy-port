//! Gamepad button names, as written in `bindings.toml` and
//! `button_names.toml`.
//!
//! Bevy names the face buttons by where they sit (South is the bottom one).
//! Buttons Bevy has no name for (back paddles, the PlayStation touchpad) are
//! `GamepadButton::Other(n)` with the numbers below; only the SDL reader
//! sends them.

use bevy::input::gamepad::GamepadButton;

pub const RIGHT_PADDLE1: u32 = 1;
pub const LEFT_PADDLE1: u32 = 2;
pub const RIGHT_PADDLE2: u32 = 3;
pub const LEFT_PADDLE2: u32 = 4;
pub const TOUCHPAD: u32 = 5;
pub const MISC1: u32 = 6;

const NAMES: [(&str, GamepadButton); 23] = [
    ("South", GamepadButton::South),
    ("East", GamepadButton::East),
    ("North", GamepadButton::North),
    ("West", GamepadButton::West),
    ("LeftTrigger", GamepadButton::LeftTrigger),
    ("LeftTrigger2", GamepadButton::LeftTrigger2),
    ("RightTrigger", GamepadButton::RightTrigger),
    ("RightTrigger2", GamepadButton::RightTrigger2),
    ("Select", GamepadButton::Select),
    ("Start", GamepadButton::Start),
    ("Mode", GamepadButton::Mode),
    ("LeftThumb", GamepadButton::LeftThumb),
    ("RightThumb", GamepadButton::RightThumb),
    ("DPadUp", GamepadButton::DPadUp),
    ("DPadDown", GamepadButton::DPadDown),
    ("DPadLeft", GamepadButton::DPadLeft),
    ("DPadRight", GamepadButton::DPadRight),
    ("RightPaddle1", GamepadButton::Other(RIGHT_PADDLE1)),
    ("LeftPaddle1", GamepadButton::Other(LEFT_PADDLE1)),
    ("RightPaddle2", GamepadButton::Other(RIGHT_PADDLE2)),
    ("LeftPaddle2", GamepadButton::Other(LEFT_PADDLE2)),
    ("Touchpad", GamepadButton::Other(TOUCHPAD)),
    ("Misc1", GamepadButton::Other(MISC1)),
];

pub fn button_from_name(name: &str) -> Option<GamepadButton> {
    NAMES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

pub fn button_name(button: GamepadButton) -> Option<&'static str> {
    NAMES.iter().find(|(_, b)| *b == button).map(|(n, _)| *n)
}

/// Triggers are pressed by how far they are pulled, not by a click.
pub fn is_trigger(button: GamepadButton) -> bool {
    matches!(
        button,
        GamepadButton::LeftTrigger2 | GamepadButton::RightTrigger2
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_both_ways() {
        for (n, b) in NAMES {
            assert_eq!(button_from_name(n), Some(b));
            assert_eq!(button_name(b), Some(n));
        }
        assert_eq!(button_from_name("Souht"), None);
    }
}
