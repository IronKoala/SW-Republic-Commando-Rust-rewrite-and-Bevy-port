//! What the player asked for, whatever device they used.
//!
//! The shell's input plugin turns keys, mouse, gamepad and touch into an
//! `ActionState` once per frame, using the bindings file
//! (`data/input/bindings.toml`). Gameplay reads only `ActionState`, so a
//! new device (or a VR controller) needs no gameplay changes.

use bevy::math::Vec2;
use bevy::prelude::Resource;
use std::collections::HashSet;

/// Everything the player can ask for. The name in the bindings file is the
/// snake_case name from [`Action::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    // Moving. These also feed `ActionState::movement`.
    MoveForward,
    MoveBack,
    StrafeLeft,
    StrafeRight,
    Jump,
    Crouch,
    Walk,
    // Fighting.
    Fire,
    /// The weapon's second mode (the DC-17m's sniper or grenade attachment).
    AltFire,
    Reload,
    Melee,
    ThrowGrenade,
    NextGrenade,
    NextWeapon,
    PrevWeapon,
    Zoom,
    /// Use, pick up, or interact (doors, consoles, revive a squad mate).
    Use,
    Flashlight,
    // Squad orders, as in the original: aim at a spot and order, or the
    // three general orders.
    SquadCommand,
    SquadSearchAndDestroy,
    SquadFormUp,
    SquadSecurePosition,
    SquadCancel,
    // Menus.
    Pause,
    MenuUp,
    MenuDown,
    MenuLeft,
    MenuRight,
    MenuAccept,
    MenuBack,
}

impl Action {
    pub const ALL: [Action; 30] = [
        Action::MoveForward,
        Action::MoveBack,
        Action::StrafeLeft,
        Action::StrafeRight,
        Action::Jump,
        Action::Crouch,
        Action::Walk,
        Action::Fire,
        Action::AltFire,
        Action::Reload,
        Action::Melee,
        Action::ThrowGrenade,
        Action::NextGrenade,
        Action::NextWeapon,
        Action::PrevWeapon,
        Action::Zoom,
        Action::Use,
        Action::Flashlight,
        Action::SquadCommand,
        Action::SquadSearchAndDestroy,
        Action::SquadFormUp,
        Action::SquadSecurePosition,
        Action::SquadCancel,
        Action::Pause,
        Action::MenuUp,
        Action::MenuDown,
        Action::MenuLeft,
        Action::MenuRight,
        Action::MenuAccept,
        Action::MenuBack,
    ];

    /// The name used in the bindings file.
    pub fn name(self) -> &'static str {
        match self {
            Action::MoveForward => "move_forward",
            Action::MoveBack => "move_back",
            Action::StrafeLeft => "strafe_left",
            Action::StrafeRight => "strafe_right",
            Action::Jump => "jump",
            Action::Crouch => "crouch",
            Action::Walk => "walk",
            Action::Fire => "fire",
            Action::AltFire => "alt_fire",
            Action::Reload => "reload",
            Action::Melee => "melee",
            Action::ThrowGrenade => "throw_grenade",
            Action::NextGrenade => "next_grenade",
            Action::NextWeapon => "next_weapon",
            Action::PrevWeapon => "prev_weapon",
            Action::Zoom => "zoom",
            Action::Use => "use",
            Action::Flashlight => "flashlight",
            Action::SquadCommand => "squad_command",
            Action::SquadSearchAndDestroy => "squad_search_and_destroy",
            Action::SquadFormUp => "squad_form_up",
            Action::SquadSecurePosition => "squad_secure_position",
            Action::SquadCancel => "squad_cancel",
            Action::Pause => "pause",
            Action::MenuUp => "menu_up",
            Action::MenuDown => "menu_down",
            Action::MenuLeft => "menu_left",
            Action::MenuRight => "menu_right",
            Action::MenuAccept => "menu_accept",
            Action::MenuBack => "menu_back",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// This frame's actions. Written in `GameSet::Input`, read by everyone else.
#[derive(Resource, Debug, Default, Clone)]
pub struct ActionState {
    pressed: HashSet<Action>,
    just_pressed: HashSet<Action>,
    just_released: HashSet<Action>,
    /// Movement wanted, each part -1 to 1: `x` is right, `y` is forward.
    /// Sticks give partial values; keys give -1, 0 or 1.
    pub movement: Vec2,
    /// How far to turn this frame, radians: `x` turns right, `y` looks up.
    /// Already scaled by the sensitivity settings.
    pub look: Vec2,
    /// The device the player touched most recently, so on-screen prompts
    /// can show keys or pad buttons to match. Every device works at once;
    /// this only says which one to show.
    pub last_device: InputDevice,
}

/// A kind of input device, for on-screen prompts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputDevice {
    #[default]
    KeyboardMouse,
    Gamepad,
    Touch,
}

impl ActionState {
    pub fn pressed(&self, a: Action) -> bool {
        self.pressed.contains(&a)
    }
    pub fn just_pressed(&self, a: Action) -> bool {
        self.just_pressed.contains(&a)
    }
    pub fn just_released(&self, a: Action) -> bool {
        self.just_released.contains(&a)
    }

    /// Start a new frame: everything held this frame is in `now`. Works
    /// out what was just pressed and just released. Used by the input
    /// plugin, and by tests and scripted runs to fake input.
    pub fn update(&mut self, now: HashSet<Action>, movement: Vec2, look: Vec2) {
        self.just_pressed = now.difference(&self.pressed).copied().collect();
        self.just_released = self.pressed.difference(&now).copied().collect();
        self.pressed = now;
        self.movement = movement.clamp_length_max(1.0);
        self.look = look;
    }
}
