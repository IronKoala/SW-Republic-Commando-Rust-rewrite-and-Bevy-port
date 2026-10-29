//! Devices to actions. Reads the bindings file and, every frame, turns the
//! keyboard, mouse, gamepads and touch screen into `ActionState`.
//!
//! Nothing else in the game reads devices. A scripted test run (or a VR mod)
//! replaces this plugin's system and writes `ActionState` itself.

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use rc_game_core::{Action, ActionState, GameSet, Play};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

/// Pixels of finger drag for full-speed movement on a touch screen.
const TOUCH_MOVE_RADIUS: f32 = 80.0;
/// Turn per pixel of finger drag, radians.
const TOUCH_LOOK_PER_PIXEL: f32 = 0.006;
/// A touch shorter than this, that hardly moved, is a tap (seconds, pixels).
const TAP_SECONDS: f32 = 0.25;
const TAP_PIXELS: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    Button(MouseButton),
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stick {
    Left,
    Right,
    None,
}

/// The bindings file, checked and turned into Bevy's own key types.
#[derive(Resource, Debug, Clone)]
pub struct Bindings {
    pub keys: Vec<(Action, KeyCode)>,
    pub mouse: Vec<(Action, MouseInput)>,
    pub pad: Vec<(Action, GamepadButton)>,
    pub move_stick: Stick,
    pub look_stick: Stick,
    pub touch: bool,
}

/// Sensitivity numbers from the settings file.
#[derive(Resource, Debug, Clone)]
pub struct LookSettings {
    pub mouse_sensitivity: f32,
    pub stick_look_speed: f32,
    pub invert_y: bool,
    pub dead_zone: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    keyboard: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    mouse: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    gamepad: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    gamepad_sticks: Sticks,
    #[serde(default)]
    touch: Touch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sticks {
    #[serde(rename = "move")]
    movement: String,
    look: String,
}

impl Default for Sticks {
    fn default() -> Self {
        Sticks {
            movement: "left".into(),
            look: "right".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    enabled: bool,
}

impl Default for Touch {
    fn default() -> Self {
        Touch { enabled: true }
    }
}

macro_rules! key_names {
    ($($k:ident),* $(,)?) => {
        fn key_from_name(name: &str) -> Option<KeyCode> {
            match name {
                $(stringify!($k) => Some(KeyCode::$k),)*
                _ => None,
            }
        }
    };
}

key_names!(
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Slash,
    Backslash,
    BracketLeft,
    BracketRight,
    Minus,
    Equal,
    Comma,
    Period,
    Semicolon,
    Quote,
    Backquote,
    CapsLock,
    Pause,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadEnter,
);

fn mouse_from_name(name: &str) -> Option<MouseInput> {
    Some(match name {
        "Left" => MouseInput::Button(MouseButton::Left),
        "Right" => MouseInput::Button(MouseButton::Right),
        "Middle" => MouseInput::Button(MouseButton::Middle),
        "Back" => MouseInput::Button(MouseButton::Back),
        "Forward" => MouseInput::Button(MouseButton::Forward),
        "WheelUp" => MouseInput::WheelUp,
        "WheelDown" => MouseInput::WheelDown,
        _ => return None,
    })
}

fn pad_from_name(name: &str) -> Option<GamepadButton> {
    use GamepadButton as B;
    Some(match name {
        "South" => B::South,
        "East" => B::East,
        "North" => B::North,
        "West" => B::West,
        "LeftTrigger" => B::LeftTrigger,
        "LeftTrigger2" => B::LeftTrigger2,
        "RightTrigger" => B::RightTrigger,
        "RightTrigger2" => B::RightTrigger2,
        "Select" => B::Select,
        "Start" => B::Start,
        "Mode" => B::Mode,
        "LeftThumb" => B::LeftThumb,
        "RightThumb" => B::RightThumb,
        "DPadUp" => B::DPadUp,
        "DPadDown" => B::DPadDown,
        "DPadLeft" => B::DPadLeft,
        "DPadRight" => B::DPadRight,
        _ => return None,
    })
}

fn stick_from_name(name: &str) -> Option<Stick> {
    match name {
        "left" => Some(Stick::Left),
        "right" => Some(Stick::Right),
        "none" => Some(Stick::None),
        _ => None,
    }
}

fn section<T>(
    table: &BTreeMap<String, Vec<String>>,
    section: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Vec<(Action, T)>, String> {
    let mut out = Vec::new();
    for (action, names) in table {
        let a = Action::from_name(action)
            .ok_or_else(|| format!("[{section}]: unknown action `{action}`"))?;
        for n in names {
            let b = parse(n).ok_or_else(|| format!("[{section}] {action}: unknown name `{n}`"))?;
            out.push((a, b));
        }
    }
    Ok(out)
}

impl Bindings {
    pub fn parse(text: &str) -> Result<Bindings, String> {
        let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(Bindings {
            keys: section(&f.keyboard, "keyboard", key_from_name)?,
            mouse: section(&f.mouse, "mouse", mouse_from_name)?,
            pad: section(&f.gamepad, "gamepad", pad_from_name)?,
            move_stick: stick_from_name(&f.gamepad_sticks.movement)
                .ok_or("[gamepad_sticks] move: use left, right or none")?,
            look_stick: stick_from_name(&f.gamepad_sticks.look)
                .ok_or("[gamepad_sticks] look: use left, right or none")?,
            touch: f.touch.enabled,
        })
    }
}

/// Reads devices into `ActionState`. Left out by scripted runs.
pub struct DeviceInputPlugin;

impl Plugin for DeviceInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchTaps>()
            .add_systems(Update, read_devices.in_set(GameSet::Input))
            .add_systems(OnEnter(Play::Running), grab_cursor)
            .add_systems(OnExit(Play::Running), release_cursor);
    }
}

/// Fingers on the screen: where each started and when.
#[derive(Resource, Default)]
struct TouchTaps {
    started: Vec<(u64, f32)>,
}

fn stick(pad: &Gamepad, which: Stick, dead_zone: f32) -> Vec2 {
    let v = match which {
        Stick::Left => pad.left_stick(),
        Stick::Right => pad.right_stick(),
        Stick::None => Vec2::ZERO,
    };
    if v.length() < dead_zone {
        Vec2::ZERO
    } else {
        v
    }
}

#[allow(clippy::too_many_arguments)]
fn read_devices(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    touches: Option<Res<Touches>>,
    pads: Query<&Gamepad>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    window: Query<&Window, With<PrimaryWindow>>,
    bindings: Res<Bindings>,
    look: Res<LookSettings>,
    time: Res<Time>,
    mut taps: ResMut<TouchTaps>,
    mut state: ResMut<ActionState>,
) {
    let mut now = HashSet::new();
    for (a, k) in &bindings.keys {
        if keys.pressed(*k) {
            now.insert(*a);
        }
    }
    for (a, m) in &bindings.mouse {
        let on = match m {
            MouseInput::Button(b) => buttons.pressed(*b),
            MouseInput::WheelUp => scroll.delta.y > 0.0,
            MouseInput::WheelDown => scroll.delta.y < 0.0,
        };
        if on {
            now.insert(*a);
        }
    }
    let dt = time.delta_secs();
    let y_sign = if look.invert_y { -1.0 } else { 1.0 };
    let mut movement = Vec2::ZERO;
    let mut turn = Vec2::ZERO;
    for pad in &pads {
        for (a, b) in &bindings.pad {
            if pad.pressed(*b) {
                now.insert(*a);
            }
        }
        movement += stick(pad, bindings.move_stick, look.dead_zone);
        turn += stick(pad, bindings.look_stick, look.dead_zone) * look.stick_look_speed * dt;
    }
    // Keys add full steps to the stick's movement.
    let axis = |plus: Action, minus: Action| {
        (now.contains(&plus) as i32 - now.contains(&minus) as i32) as f32
    };
    movement += Vec2::new(
        axis(Action::StrafeRight, Action::StrafeLeft),
        axis(Action::MoveForward, Action::MoveBack),
    );
    // The mouse only turns the view while it is captured (during play).
    let grabbed = cursor
        .single()
        .map(|c| c.grab_mode != CursorGrabMode::None)
        .unwrap_or(false);
    if grabbed {
        turn += Vec2::new(motion.delta.x, -motion.delta.y) * look.mouse_sensitivity;
    }
    if bindings.touch {
        if let Some(touches) = touches {
            // Left half of the screen moves, right half looks.
            let half = window.single().map(|w| w.width() / 2.0).unwrap_or(400.0);
            let t = time.elapsed_secs();
            for f in touches.iter_just_pressed() {
                taps.started.push((f.id(), t));
            }
            for f in touches.iter() {
                if f.start_position().x < half {
                    movement += ((f.position() - f.start_position()) / TOUCH_MOVE_RADIUS)
                        .clamp_length_max(1.0)
                        * Vec2::new(1.0, -1.0);
                } else {
                    turn += f.delta() * Vec2::new(1.0, -1.0) * TOUCH_LOOK_PER_PIXEL;
                }
            }
            for f in touches.iter_just_released() {
                let start = taps
                    .started
                    .iter()
                    .find(|(id, _)| *id == f.id())
                    .map(|s| s.1);
                let quick = start.is_some_and(|s| t - s < TAP_SECONDS);
                if quick && f.start_position().x >= half && f.distance().length() < TAP_PIXELS {
                    now.insert(Action::Fire);
                }
            }
            taps.started
                .retain(|(id, _)| touches.get_pressed(*id).is_some());
        }
    }
    turn.y *= y_sign;
    state.update(now, movement, turn);
}

fn grab_cursor(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
}

fn release_cursor(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_bindings_file_reads() {
        let text = include_str!("../../../data/input/bindings.toml");
        let b = Bindings::parse(text).unwrap();
        assert!(b.keys.contains(&(Action::MoveForward, KeyCode::KeyW)));
        assert!(b
            .mouse
            .contains(&(Action::Fire, MouseInput::Button(MouseButton::Left))));
        assert!(b
            .pad
            .contains(&(Action::Fire, GamepadButton::RightTrigger2)));
        assert_eq!(b.move_stick, Stick::Left);
    }

    #[test]
    fn typos_name_the_problem() {
        let e = Bindings::parse("[keyboard]\nfire = ['KeyQQ']\n").unwrap_err();
        assert!(e.contains("KeyQQ"), "{e}");
        let e = Bindings::parse("[keyboard]\nfrie = ['KeyQ']\n").unwrap_err();
        assert!(e.contains("frie"), "{e}");
    }
}
