//! What the player asked for, whatever device they used.
//!
//! The shell's input plugin turns keys, mouse, gamepad and touch into an
//! `ActionState` once per frame, using the bindings file
//! (`data/input/bindings.toml`). Gameplay reads only `ActionState`, so a
//! new device (or a VR controller) needs no gameplay changes.

use bevy::math::Vec2;
use bevy::prelude::Resource;
use std::collections::{HashMap, HashSet};

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
    /// Turning with keys (at the controller's look speed), for players who
    /// don't use the mouse to turn.
    TurnLeft,
    TurnRight,
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
    /// Straight to one weapon, as the original's number keys.
    WeaponPistol,
    WeaponBlaster,
    WeaponSniper,
    WeaponAntiArmor,
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
    // Saving.
    QuickSave,
    QuickLoad,
    // Menus.
    Pause,
    MenuUp,
    MenuDown,
    MenuLeft,
    MenuRight,
    MenuAccept,
    MenuBack,
    /// Save a picture of the screen (to the `screenshots` folder next to the
    /// settings file).
    Screenshot,
    // Gyro aiming.
    /// Held: the gyro stops turning the view, so the controller can be
    /// re-centred (like lifting a mouse off the mat).
    GyroRatchet,
    /// Pressed: gyro aiming on or off.
    GyroToggle,
}

impl Action {
    pub const ALL: [Action; 41] = [
        Action::MoveForward,
        Action::MoveBack,
        Action::StrafeLeft,
        Action::StrafeRight,
        Action::Jump,
        Action::Crouch,
        Action::Walk,
        Action::TurnLeft,
        Action::TurnRight,
        Action::Fire,
        Action::AltFire,
        Action::Reload,
        Action::Melee,
        Action::ThrowGrenade,
        Action::NextGrenade,
        Action::NextWeapon,
        Action::PrevWeapon,
        Action::WeaponPistol,
        Action::WeaponBlaster,
        Action::WeaponSniper,
        Action::WeaponAntiArmor,
        Action::Zoom,
        Action::Use,
        Action::Flashlight,
        Action::SquadCommand,
        Action::SquadSearchAndDestroy,
        Action::SquadFormUp,
        Action::SquadSecurePosition,
        Action::SquadCancel,
        Action::QuickSave,
        Action::QuickLoad,
        Action::Pause,
        Action::MenuUp,
        Action::MenuDown,
        Action::MenuLeft,
        Action::MenuRight,
        Action::MenuAccept,
        Action::MenuBack,
        Action::Screenshot,
        Action::GyroRatchet,
        Action::GyroToggle,
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
            Action::TurnLeft => "turn_left",
            Action::TurnRight => "turn_right",
            Action::Fire => "fire",
            Action::AltFire => "alt_fire",
            Action::Reload => "reload",
            Action::Melee => "melee",
            Action::ThrowGrenade => "throw_grenade",
            Action::NextGrenade => "next_grenade",
            Action::NextWeapon => "next_weapon",
            Action::PrevWeapon => "prev_weapon",
            Action::WeaponPistol => "weapon_pistol",
            Action::WeaponBlaster => "weapon_blaster",
            Action::WeaponSniper => "weapon_sniper",
            Action::WeaponAntiArmor => "weapon_anti_armor",
            Action::Zoom => "zoom",
            Action::Use => "use",
            Action::Flashlight => "flashlight",
            Action::SquadCommand => "squad_command",
            Action::SquadSearchAndDestroy => "squad_search_and_destroy",
            Action::SquadFormUp => "squad_form_up",
            Action::SquadSecurePosition => "squad_secure_position",
            Action::SquadCancel => "squad_cancel",
            Action::QuickSave => "quick_save",
            Action::QuickLoad => "quick_load",
            Action::Pause => "pause",
            Action::MenuUp => "menu_up",
            Action::MenuDown => "menu_down",
            Action::MenuLeft => "menu_left",
            Action::MenuRight => "menu_right",
            Action::MenuAccept => "menu_accept",
            Action::MenuBack => "menu_back",
            Action::Screenshot => "screenshot",
            Action::GyroRatchet => "gyro_ratchet",
            Action::GyroToggle => "gyro_toggle",
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
    /// Everything held this frame, in a fixed order (for logs and tests).
    pub fn all_pressed(&self) -> Vec<Action> {
        let mut v: Vec<Action> = self.pressed.iter().copied().collect();
        v.sort();
        v
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

/// What to show on screen for each action's button right now: "F" or
/// "Left Mouse" after keyboard and mouse, "X" or "Square" after a
/// controller (its own names: Xbox, PlayStation, Switch, Steam Deck).
/// Follows `ActionState::last_device`. Written by the input plugin; menus,
/// the HUD and level scripts read it for "Press F to use" messages.
#[derive(Resource, Debug, Clone, Default)]
pub struct InputPrompts {
    /// The device the names are for.
    pub device: InputDevice,
    /// The controller's kind when `device` is a gamepad, as in
    /// `data/input/button_names.toml`: "xbox", "ps5", "switch_pro",
    /// "steam_deck", ...
    pub controller: String,
    labels: HashMap<Action, Vec<String>>,
}

impl InputPrompts {
    pub fn new(device: InputDevice, controller: String) -> InputPrompts {
        InputPrompts {
            device,
            controller,
            labels: HashMap::new(),
        }
    }

    /// The first button bound to `a` on the current device.
    pub fn label(&self, a: Action) -> Option<&str> {
        self.labels.get(&a)?.first().map(String::as_str)
    }

    /// Every button bound to `a` on the current device.
    pub fn labels(&self, a: Action) -> &[String] {
        self.labels.get(&a).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn add(&mut self, a: Action, label: String) {
        let l = self.labels.entry(a).or_default();
        if !l.contains(&label) {
            l.push(label);
        }
    }
}
