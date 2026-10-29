//! Changing a control in the game: "press the new key for Fire".
//!
//! A menu (the options screen's controls page) sends [`StartRebind`] with
//! the action and the device. The next key, mouse button or wheel turn (or,
//! for a controller, the next button or trigger) becomes that action's only
//! button on that device. Escape, or five seconds with nothing pressed,
//! cancels. The change is saved straight away to `my_bindings.toml` next to
//! the settings file, which holds only the player's changes, and
//! [`RebindDone`] tells the menu what happened.
//!
//! Nothing else happens while waiting, so the key chosen doesn't also fire
//! or jump; and after a choice, nothing happens until it is let go.

use crate::input::{key_name, mouse_name, BaseBindings, Bindings, LookSettings, MouseInput};
use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use rc_game_core::{Action, GameLog, GameSet, InputDevice};
use rc_pads::buttons::{button_name, is_trigger};
use std::path::PathBuf;

/// How long to wait for a button before giving up, seconds.
const REBIND_SECONDS: f32 = 5.0;

/// Ask for a new button for `action` on `device` (keyboard and mouse, or a
/// controller).
#[derive(Message, Debug, Clone, Copy)]
pub struct StartRebind {
    pub action: Action,
    pub device: InputDevice,
}

/// The result of a [`StartRebind`].
#[derive(Message, Debug, Clone)]
// Read by the options screen's controls page, which the menus part makes.
#[allow(dead_code)]
pub struct RebindDone {
    pub action: Action,
    pub device: InputDevice,
    /// The new button's name as in the bindings file ("KeyG", "South"), or
    /// `None` if cancelled.
    pub button: Option<String>,
    /// Other gameplay actions on the same button, so the menu can warn.
    pub also_used_by: Vec<Action>,
}

/// Where the player's changes are saved.
#[derive(Resource, Debug, Clone)]
pub struct UserBindingsPath(pub PathBuf);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Wait {
    Idle,
    /// Waiting for a button, since this time.
    For(Action, InputDevice, f32),
    /// Chosen (or cancelled); waiting for everything to be let go.
    LetGo,
}

/// Whether a new button is being chosen.
#[derive(Resource, Debug, Clone)]
pub struct RebindCapture(Wait);

impl Default for RebindCapture {
    fn default() -> Self {
        RebindCapture(Wait::Idle)
    }
}

impl RebindCapture {
    /// True while choosing, and until the chosen button is let go.
    pub fn busy(&self) -> bool {
        self.0 != Wait::Idle
    }
}

/// Runs before the device reading, in `GameSet::Input`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RebindSet;

pub struct RebindPlugin;

impl Plugin for RebindPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RebindCapture>()
            .add_message::<StartRebind>()
            .add_message::<RebindDone>()
            .add_systems(Update, capture.in_set(RebindSet).in_set(GameSet::Input));
    }
}

/// What was chosen.
enum Chosen {
    Key(KeyCode),
    Mouse(MouseInput),
    Pad(GamepadButton),
}

#[allow(clippy::too_many_arguments)]
fn capture(
    mut starts: MessageReader<StartRebind>,
    mut done: MessageWriter<RebindDone>,
    mut wait: ResMut<RebindCapture>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    pads: Query<&Gamepad>,
    look: Res<LookSettings>,
    time: Res<Time>,
    mut bindings: ResMut<Bindings>,
    base: Option<Res<BaseBindings>>,
    path: Option<Res<UserBindingsPath>>,
    log: Res<GameLog>,
) {
    let t = time.elapsed_secs();
    for s in starts.read() {
        log.line(format!(
            "rebind: press a {} button for {}",
            if s.device == InputDevice::Gamepad {
                "controller"
            } else {
                "key or mouse"
            },
            s.action.name()
        ));
        wait.0 = Wait::For(s.action, s.device, t);
    }
    let anything_down = keys.get_pressed().next().is_some()
        || mouse.get_pressed().next().is_some()
        || pads.iter().any(|p| {
            p.get_pressed().next().is_some()
                || [GamepadButton::LeftTrigger2, GamepadButton::RightTrigger2]
                    .into_iter()
                    .any(|b| p.get(b).is_some_and(|v| v >= look.trigger_threshold))
        });
    let (action, device, since) = match wait.0 {
        Wait::Idle => return,
        Wait::LetGo => {
            if !anything_down {
                wait.0 = Wait::Idle;
            }
            return;
        }
        Wait::For(a, d, s) => (a, d, s),
    };
    let cancel = |wait: &mut RebindCapture, done: &mut MessageWriter<RebindDone>, why: &str| {
        log.line(format!("rebind: {} not changed ({why})", action.name()));
        done.write(RebindDone {
            action,
            device,
            button: None,
            also_used_by: vec![],
        });
        wait.0 = Wait::LetGo;
    };
    if keys.just_pressed(KeyCode::Escape) {
        cancel(&mut wait, &mut done, "Escape");
        return;
    }
    if t - since > REBIND_SECONDS {
        cancel(&mut wait, &mut done, "nothing pressed");
        return;
    }
    let chosen = match device {
        InputDevice::Gamepad => pads.iter().find_map(|p| {
            p.get_just_pressed()
                .copied()
                .find(|b| !is_trigger(*b))
                .or_else(|| {
                    [GamepadButton::LeftTrigger2, GamepadButton::RightTrigger2]
                        .into_iter()
                        .find(|b| p.get(*b).is_some_and(|v| v >= look.trigger_threshold))
                })
                .map(Chosen::Pad)
        }),
        _ => keys
            .get_just_pressed()
            .next()
            .map(|k| Chosen::Key(*k))
            .or_else(|| {
                mouse
                    .get_just_pressed()
                    .next()
                    .map(|b| Chosen::Mouse(MouseInput::Button(*b)))
            })
            .or_else(|| {
                if scroll.delta.y > 0.0 {
                    Some(Chosen::Mouse(MouseInput::WheelUp))
                } else if scroll.delta.y < 0.0 {
                    Some(Chosen::Mouse(MouseInput::WheelDown))
                } else {
                    None
                }
            }),
    };
    let Some(chosen) = chosen else {
        return;
    };
    let name = match &chosen {
        Chosen::Key(k) => key_name(*k),
        Chosen::Mouse(m) => mouse_name(*m),
        Chosen::Pad(b) => button_name(*b),
    };
    let Some(name) = name else {
        // A key the bindings file has no name for: keep waiting.
        return;
    };
    let also = set_binding(&mut bindings, action, &chosen);
    log.line(format!(
        "rebind: {} is now {name}{}",
        action.name(),
        if also.is_empty() {
            String::new()
        } else {
            format!(
                " (also used by {})",
                also.iter().map(|a| a.name()).collect::<Vec<_>>().join(", ")
            )
        }
    ));
    if let (Some(base), Some(path)) = (base, path) {
        let text = bindings.changes_from(&base.0);
        match std::fs::write(&path.0, text) {
            Ok(()) => log.line(format!("rebind: saved to {}", path.0.display())),
            Err(e) => log.line(format!("rebind: ERROR saving {}: {e}", path.0.display())),
        }
    }
    done.write(RebindDone {
        action,
        device,
        button: Some(name.to_string()),
        also_used_by: also,
    });
    wait.0 = Wait::LetGo;
}

/// Make `chosen` the only button for `action` on its device. Returns the
/// other gameplay actions on the same button (menu actions share buttons
/// with gameplay on purpose, so they don't count).
fn set_binding(b: &mut Bindings, action: Action, chosen: &Chosen) -> Vec<Action> {
    fn put<T: Copy + PartialEq>(list: &mut Vec<(Action, T)>, a: Action, v: T) -> Vec<Action> {
        list.retain(|(x, _)| *x != a);
        list.push((a, v));
        list.iter()
            .filter(|(x, y)| *y == v && *x != a && !is_menu(*x))
            .map(|(x, _)| *x)
            .collect()
    }
    match chosen {
        Chosen::Key(k) => {
            // A key replaces the keyboard and mouse buttons both, so the
            // action ends up on exactly the one button chosen.
            b.mouse.retain(|(x, _)| *x != action);
            put(&mut b.keys, action, *k)
        }
        Chosen::Mouse(m) => {
            b.keys.retain(|(x, _)| *x != action);
            put(&mut b.mouse, action, *m)
        }
        Chosen::Pad(p) => put(&mut b.pad, action, *p),
    }
}

fn is_menu(a: Action) -> bool {
    matches!(
        a,
        Action::MenuUp
            | Action::MenuDown
            | Action::MenuLeft
            | Action::MenuRight
            | Action::MenuAccept
            | Action::MenuBack
            | Action::Pause
    )
}
