//! Devices to actions. Reads the bindings file and, every frame, turns the
//! keyboard, mouse, gamepads, gyros and touch screen into `ActionState`,
//! and names each action's button in `InputPrompts`.
//!
//! Nothing else in the game reads devices. A scripted test run (or a VR mod)
//! replaces this plugin's system and writes `ActionState` itself.
//! Controllers themselves (which kind, their button names, the gyro) are in
//! `rc_pads`; changing a binding in the game is in `rebind.rs`.

use crate::settings::Settings;
use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use rc_game_core::{Action, ActionState, GameLog, GameSet, InputDevice, InputPrompts, Play};
use rc_pads::buttons::{button_from_name, button_name, is_trigger};
use rc_pads::{ActivePad, ButtonNames, ControllerKind, GyroLook, GyroMode, GyroSettings};
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
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Bindings {
    pub keys: Vec<(Action, KeyCode)>,
    pub mouse: Vec<(Action, MouseInput)>,
    pub pad: Vec<(Action, GamepadButton)>,
    pub move_stick: Stick,
    pub look_stick: Stick,
    pub touch: bool,
}

/// Sensitivity, dead zone and curve numbers from the settings file.
#[derive(Resource, Debug, Clone)]
pub struct LookSettings {
    pub mouse_sensitivity: f32,
    pub stick_look_speed: f32,
    pub invert_y: bool,
    pub move_dead_zone: f32,
    pub look_dead_zone: f32,
    /// Stick response curves: 1 is straight, higher is gentler near the
    /// middle.
    pub move_curve: f32,
    pub look_curve: f32,
    /// Look stick up and down speed compared with across.
    pub look_vertical: f32,
    pub trigger_threshold: f32,
}

impl LookSettings {
    pub fn from_settings(s: &Settings) -> LookSettings {
        let c = &s.controls;
        LookSettings {
            mouse_sensitivity: c.mouse_sensitivity,
            stick_look_speed: c.stick_look_speed,
            invert_y: c.invert_y,
            move_dead_zone: c.stick_dead_zone,
            look_dead_zone: c.look_stick_dead_zone,
            move_curve: c.stick_move_curve,
            look_curve: c.stick_look_curve,
            look_vertical: c.stick_look_vertical,
            trigger_threshold: c.trigger_threshold,
        }
    }
}

/// Whether gyro aiming is switched on (the gyro_toggle button flips it).
#[derive(Resource, Debug, Clone, Copy)]
pub struct GyroAim {
    pub on: bool,
    /// Whether the gyro turned the view this frame.
    pub used: bool,
}

impl Default for GyroAim {
    fn default() -> Self {
        GyroAim {
            on: true,
            used: false,
        }
    }
}

/// `--log-input`: write every change of pressed actions, and the movement
/// and look numbers, to the log.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct LogInput(pub bool);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    keyboard: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    mouse: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    gamepad: BTreeMap<String, Vec<String>>,
    gamepad_sticks: Option<Sticks>,
    touch: Option<Touch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sticks {
    #[serde(rename = "move")]
    movement: String,
    look: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    enabled: bool,
}

macro_rules! key_names {
    ($($k:ident),* $(,)?) => {
        fn key_from_name(name: &str) -> Option<KeyCode> {
            match name {
                $(stringify!($k) => Some(KeyCode::$k),)*
                _ => None,
            }
        }
        pub fn key_name(key: KeyCode) -> Option<&'static str> {
            match key {
                $(KeyCode::$k => Some(stringify!($k)),)*
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
const MOUSE_NAMES: [(&str, MouseInput); 7] = [
    ("Left", MouseInput::Button(MouseButton::Left)),
    ("Right", MouseInput::Button(MouseButton::Right)),
    ("Middle", MouseInput::Button(MouseButton::Middle)),
    ("Back", MouseInput::Button(MouseButton::Back)),
    ("Forward", MouseInput::Button(MouseButton::Forward)),
    ("WheelUp", MouseInput::WheelUp),
    ("WheelDown", MouseInput::WheelDown),
];

fn mouse_from_name(name: &str) -> Option<MouseInput> {
    MOUSE_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, m)| *m)
}

pub fn mouse_name(m: MouseInput) -> Option<&'static str> {
    MOUSE_NAMES.iter().find(|(_, x)| *x == m).map(|(n, _)| *n)
}

fn stick_from_name(name: &str) -> Option<Stick> {
    match name {
        "left" => Some(Stick::Left),
        "right" => Some(Stick::Right),
        "none" => Some(Stick::None),
        _ => None,
    }
}

fn stick_name(s: Stick) -> &'static str {
    match s {
        Stick::Left => "left",
        Stick::Right => "right",
        Stick::None => "none",
    }
}

/// Read one section. Actions listed replace whatever `out` had for them,
/// so a player's file only needs the actions they changed.
fn section<T>(
    out: &mut Vec<(Action, T)>,
    table: &BTreeMap<String, Vec<String>>,
    section: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<(), String> {
    for (action, names) in table {
        let a = Action::from_name(action)
            .ok_or_else(|| format!("[{section}]: unknown action `{action}`"))?;
        out.retain(|(x, _)| *x != a);
        for n in names {
            let b = parse(n).ok_or_else(|| format!("[{section}] {action}: unknown name `{n}`"))?;
            out.push((a, b));
        }
    }
    Ok(())
}

impl Bindings {
    pub fn parse(text: &str) -> Result<Bindings, String> {
        let empty = Bindings {
            keys: vec![],
            mouse: vec![],
            pad: vec![],
            move_stick: Stick::Left,
            look_stick: Stick::Right,
            touch: true,
        };
        empty.with_changes(text)
    }

    /// These bindings with a player's changes on top (`my_bindings.toml`):
    /// each action the file lists replaces that action's buttons on that
    /// device; everything else stays.
    pub fn with_changes(&self, text: &str) -> Result<Bindings, String> {
        let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut b = self.clone();
        section(&mut b.keys, &f.keyboard, "keyboard", key_from_name)?;
        section(&mut b.mouse, &f.mouse, "mouse", mouse_from_name)?;
        section(&mut b.pad, &f.gamepad, "gamepad", button_from_name)?;
        if let Some(s) = f.gamepad_sticks {
            b.move_stick = stick_from_name(&s.movement)
                .ok_or("[gamepad_sticks] move: use left, right or none")?;
            b.look_stick =
                stick_from_name(&s.look).ok_or("[gamepad_sticks] look: use left, right or none")?;
        }
        if let Some(t) = f.touch {
            b.touch = t.enabled;
        }
        Ok(b)
    }

    /// The player's changes from `base`, as a file `with_changes` reads
    /// back: only the actions whose buttons differ.
    pub fn changes_from(&self, base: &Bindings) -> String {
        fn names<T: Copy + PartialEq>(
            list: &[(Action, T)],
            a: Action,
            name: fn(T) -> Option<&'static str>,
        ) -> Vec<&'static str> {
            list.iter()
                .filter(|(x, _)| *x == a)
                .filter_map(|(_, b)| name(*b))
                .collect()
        }
        fn write<T: Copy + PartialEq>(
            out: &mut String,
            title: &str,
            mine: &[(Action, T)],
            base: &[(Action, T)],
            name: fn(T) -> Option<&'static str>,
        ) {
            let mut lines = Vec::new();
            for a in Action::ALL {
                let m = names(mine, a, name);
                if m != names(base, a, name) {
                    let list: Vec<String> = m.iter().map(|n| format!("\"{n}\"")).collect();
                    lines.push(format!("{} = [{}]", a.name(), list.join(", ")));
                }
            }
            if !lines.is_empty() {
                out.push_str(&format!("\n[{title}]\n{}\n", lines.join("\n")));
            }
        }
        let mut out = String::from(
            "# Your own controls, changed in the game. Each line replaces that\n\
             # action's buttons in the game's bindings.toml; everything else uses\n\
             # the defaults. Delete this file to go back to the defaults.\n",
        );
        write(&mut out, "keyboard", &self.keys, &base.keys, key_name);
        write(&mut out, "mouse", &self.mouse, &base.mouse, mouse_name);
        write(&mut out, "gamepad", &self.pad, &base.pad, button_name);
        if (self.move_stick, self.look_stick) != (base.move_stick, base.look_stick) {
            out.push_str(&format!(
                "\n[gamepad_sticks]\nmove = \"{}\"\nlook = \"{}\"\n",
                stick_name(self.move_stick),
                stick_name(self.look_stick)
            ));
        }
        if self.touch != base.touch {
            out.push_str(&format!("\n[touch]\nenabled = {}\n", self.touch));
        }
        out
    }
}

/// The bindings the game shipped with (and mods), before the player's
/// changes. Kept so only the changes are saved.
#[derive(Resource, Debug, Clone)]
pub struct BaseBindings(pub Bindings);

/// Reads devices into `ActionState`. Left out by scripted runs.
pub struct DeviceInputPlugin;

impl Plugin for DeviceInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchTaps>()
            .init_resource::<GyroAim>()
            .init_resource::<LogInput>()
            .add_plugins(crate::rebind::RebindPlugin)
            .add_systems(
                Update,
                (read_devices, update_prompts, log_input)
                    .chain()
                    .in_set(GameSet::Input)
                    .after(crate::rebind::RebindSet),
            )
            .add_systems(OnEnter(Play::Running), grab_cursor)
            .add_systems(OnExit(Play::Running), release_cursor);
    }
}

/// Fingers on the screen: where each started and when.
#[derive(Resource, Default)]
struct TouchTaps {
    started: Vec<(u64, f32)>,
}

fn stick(pad: &Gamepad, which: Stick, dead_zone: f32, curve: f32) -> Vec2 {
    let v = match which {
        Stick::Left => pad.left_stick(),
        Stick::Right => pad.right_stick(),
        Stick::None => Vec2::ZERO,
    };
    shape_stick(v, dead_zone, curve)
}

/// Ignore the dead zone in the middle, stretch the rest back to 0..1 so
/// the stick doesn't jump from nothing to the dead zone's size, then apply
/// the response curve.
pub fn shape_stick(v: Vec2, dead_zone: f32, curve: f32) -> Vec2 {
    let len = v.length();
    if len <= dead_zone || len == 0.0 {
        return Vec2::ZERO;
    }
    let t = ((len - dead_zone) / (1.0 - dead_zone).max(1e-3)).min(1.0);
    v / len * t.powf(curve.max(0.1))
}

/// Is this pad button down? Triggers count from `threshold` of a pull.
fn pad_down(pad: &Gamepad, b: GamepadButton, threshold: f32) -> bool {
    if is_trigger(b) {
        pad.get(b).is_some_and(|v| v >= threshold.max(0.01))
    } else {
        pad.pressed(b)
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
    (gyro, gyro_settings, mut gyro_aim): (Res<GyroLook>, Res<GyroSettings>, ResMut<GyroAim>),
    capture: Res<crate::rebind::RebindCapture>,
    time: Res<Time>,
    mut taps: ResMut<TouchTaps>,
    mut state: ResMut<ActionState>,
) {
    // While a new button is being chosen, nothing else happens.
    if capture.busy() {
        state.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
        return;
    }
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
    let mut device = None;
    if keys.get_pressed().next().is_some()
        || buttons.get_pressed().next().is_some()
        || motion.delta != Vec2::ZERO
    {
        device = Some(InputDevice::KeyboardMouse);
    }
    for pad in &pads {
        let mut any = false;
        for (a, b) in &bindings.pad {
            if pad_down(pad, *b, look.trigger_threshold) {
                now.insert(*a);
                any = true;
            }
        }
        let m = stick(
            pad,
            bindings.move_stick,
            look.move_dead_zone,
            look.move_curve,
        );
        let l = stick(
            pad,
            bindings.look_stick,
            look.look_dead_zone,
            look.look_curve,
        );
        if any || m != Vec2::ZERO || l != Vec2::ZERO || pad.get_pressed().next().is_some() {
            device = Some(InputDevice::Gamepad);
        }
        movement += m;
        turn += l * Vec2::new(1.0, look.look_vertical) * look.stick_look_speed * dt;
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
                device = Some(InputDevice::Touch);
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
    // The gyro: its own sensitivity and invert are already in `gyro.turn`.
    if now.contains(&Action::GyroToggle) && !state.pressed(Action::GyroToggle) {
        gyro_aim.on = !gyro_aim.on;
    }
    gyro_aim.used = gyro_aim.on
        && !now.contains(&Action::GyroRatchet)
        && match gyro_settings.mode {
            GyroMode::Off => false,
            GyroMode::Always => true,
            GyroMode::Aiming => now.contains(&Action::Zoom),
        };
    if gyro_aim.used {
        turn += gyro.turn;
    }
    state.update(now, movement, turn);
    if let Some(d) = device {
        state.last_device = d;
    }
}

/// The name to show for each action on the device used last.
fn update_prompts(
    state: Res<ActionState>,
    active: Res<ActivePad>,
    bindings: Res<Bindings>,
    names: Res<ButtonNames>,
    mut prompts: ResMut<InputPrompts>,
    log: Res<GameLog>,
) {
    let kind = active.kind;
    let controller = kind.name();
    let same = prompts.device == state.last_device && prompts.controller == controller;
    if same && !bindings.is_changed() && !names.is_changed() && !prompts.is_added() {
        return;
    }
    *prompts = make_prompts(state.last_device, kind, &bindings, &names);
    log.line(format!(
        "input: prompts now for {:?}{}: fire is {}, use is {}",
        state.last_device,
        if state.last_device == InputDevice::Gamepad {
            format!(" ({controller})")
        } else {
            String::new()
        },
        prompts.label(Action::Fire).unwrap_or("not bound"),
        prompts.label(Action::Use).unwrap_or("not bound"),
    ));
}

/// Every action's button names for one device.
pub fn make_prompts(
    device: InputDevice,
    kind: ControllerKind,
    bindings: &Bindings,
    names: &ButtonNames,
) -> InputPrompts {
    let mut p = InputPrompts::new(device, kind.name().to_string());
    match device {
        InputDevice::KeyboardMouse => {
            for (a, k) in &bindings.keys {
                if let Some(n) = key_name(*k) {
                    p.add(*a, names.key(n));
                }
            }
            for (a, m) in &bindings.mouse {
                if let Some(n) = mouse_name(*m) {
                    p.add(*a, names.mouse(n).to_string());
                }
            }
        }
        InputDevice::Gamepad => {
            for (a, b) in &bindings.pad {
                if let Some(n) = button_name(*b) {
                    p.add(*a, names.pad(kind, n).to_string());
                }
            }
        }
        InputDevice::Touch => {
            p.add(Action::Fire, "Tap".to_string());
        }
    }
    p
}

/// `--log-input`: what was pressed, and how far it moved and turned.
fn log_input(
    on: Res<LogInput>,
    state: Res<ActionState>,
    aim: Res<GyroAim>,
    gyro: Res<GyroLook>,
    time: Res<Time>,
    log: Res<GameLog>,
    mut last: Local<(Vec<Action>, f32)>,
) {
    if !on.0 {
        return;
    }
    let pressed = state.all_pressed();
    let t = time.elapsed_secs();
    let moving = state.movement != Vec2::ZERO || state.look != Vec2::ZERO;
    if pressed != last.0 || (moving && t - last.1 >= 0.25) {
        log.line(format!(
            "input: t={t:.2} {:?} pressed {:?} move ({:.3}, {:.3}) look ({:.4}, {:.4}) gyro {} ({:.4}, {:.4})",
            state.last_device,
            pressed.iter().map(|a| a.name()).collect::<Vec<_>>(),
            state.movement.x,
            state.movement.y,
            state.look.x,
            state.look.y,
            if aim.used { "on" } else { "off" },
            gyro.turn.x,
            gyro.turn.y,
        ));
        *last = (pressed, t);
    }
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
mod tests;
