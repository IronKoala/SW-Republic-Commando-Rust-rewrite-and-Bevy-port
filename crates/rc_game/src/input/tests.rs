//! Input tests. The last group runs the real input systems in a small Bevy
//! app with pretend controllers (Bevy's own gamepad messages, the same ones
//! SDL and gilrs send) and pretend gyro readings, and checks the actions
//! that come out. Each prints its numbers, so `cargo test -- --nocapture`
//! shows them.

use super::*;
use crate::rebind::{RebindDone, StartRebind, UserBindingsPath};
use bevy::input::gamepad::{
    GamepadAxis, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_game_core::CorePlugin;
use rc_pads::{GyroSample, GyroSource, PadsPlugin};
use std::time::Duration;

fn shipped() -> Bindings {
    Bindings::parse(include_str!("../../../../data/input/bindings.toml")).unwrap()
}

fn names() -> ButtonNames {
    ButtonNames::parse(include_str!("../../../../data/input/button_names.toml")).unwrap()
}

#[test]
fn stick_dead_zone_and_curve() {
    // Inside the dead zone: nothing. Just outside: a small value, not a
    // jump to the dead zone's size. Full tilt stays full.
    assert_eq!(shape_stick(Vec2::new(0.1, 0.0), 0.15, 1.0), Vec2::ZERO);
    let small = shape_stick(Vec2::new(0.2, 0.0), 0.15, 1.0).x;
    assert!((small - 0.0588).abs() < 0.001, "{small}");
    assert!((shape_stick(Vec2::new(0.0, 1.0), 0.15, 2.0).y - 1.0).abs() < 1e-6);
    // A curve of 2 halves a half-way push (after the dead zone).
    let half = shape_stick(Vec2::new(0.575, 0.0), 0.15, 2.0).x;
    assert!((half - 0.25).abs() < 0.001, "{half}");
}

#[test]
fn shipped_bindings_file_reads() {
    let b = shipped();
    assert!(b.keys.contains(&(Action::MoveForward, KeyCode::KeyW)));
    assert!(b
        .mouse
        .contains(&(Action::Fire, MouseInput::Button(MouseButton::Left))));
    assert!(b
        .pad
        .contains(&(Action::Fire, GamepadButton::RightTrigger2)));
    assert!(b
        .pad
        .contains(&(Action::GyroToggle, button_from_name("Touchpad").unwrap())));
    assert_eq!(b.move_stick, Stick::Left);
}

#[test]
fn typos_name_the_problem() {
    let e = Bindings::parse("[keyboard]\nfire = ['KeyQQ']\n").unwrap_err();
    assert!(e.contains("KeyQQ"), "{e}");
    let e = Bindings::parse("[keyboard]\nfrie = ['KeyQ']\n").unwrap_err();
    assert!(e.contains("frie"), "{e}");
    let e = Bindings::parse("[gamepad]\nfire = ['Soutth']\n").unwrap_err();
    assert!(e.contains("Soutth"), "{e}");
}

#[test]
fn players_changes_save_and_read_back() {
    let base = shipped();
    let mut mine = base.clone();
    mine.keys.retain(|(a, _)| *a != Action::Reload);
    mine.keys.push((Action::Reload, KeyCode::KeyG));
    mine.pad.retain(|(a, _)| *a != Action::Jump);
    mine.pad.push((Action::Jump, GamepadButton::North));
    mine.look_stick = Stick::Left;
    mine.move_stick = Stick::Right;
    let text = mine.changes_from(&base);
    println!("{text}");
    // Only what changed is written.
    assert!(text.contains("reload = [\"KeyG\"]"), "{text}");
    assert!(text.contains("jump = [\"North\"]"), "{text}");
    assert!(!text.contains("fire"), "{text}");
    let back = base.with_changes(&text).unwrap();
    assert!(back.keys.contains(&(Action::Reload, KeyCode::KeyG)));
    assert!(!back.keys.contains(&(Action::Reload, KeyCode::KeyR)));
    assert_eq!(back.look_stick, Stick::Left);
    // Same buttons, maybe in another order.
    let sorted = |b: &Bindings| {
        let mut k: Vec<String> = b.keys.iter().map(|x| format!("{x:?}")).collect();
        k.extend(b.pad.iter().map(|x| format!("{x:?}")));
        k.extend(b.mouse.iter().map(|x| format!("{x:?}")));
        k.sort();
        k
    };
    assert_eq!(sorted(&back), sorted(&mine));
    // No changes: nothing but the header.
    assert!(!base.changes_from(&base).contains('['));
}

#[test]
fn prompts_name_each_controllers_buttons() {
    let (b, n) = (shipped(), names());
    let km = make_prompts(InputDevice::KeyboardMouse, ControllerKind::Generic, &b, &n);
    assert_eq!(km.label(Action::Fire), Some("Left Mouse"));
    assert_eq!(km.label(Action::Use), Some("F"));
    assert_eq!(km.label(Action::Jump), Some("Space"));
    let table = [
        (ControllerKind::Xbox, "RT", "X", "A"),
        (ControllerKind::PlayStation5, "R2", "Square", "Cross"),
        (ControllerKind::SwitchPro, "ZR", "Y", "B"),
        (ControllerKind::JoyConPair, "ZR", "Y", "B"),
        (ControllerKind::SteamDeck, "R2", "X", "A"),
    ];
    for (kind, fire, use_, jump) in table {
        let p = make_prompts(InputDevice::Gamepad, kind, &b, &n);
        println!(
            "{:>11}: fire {:?}, use {:?}, jump {:?}, gyro ratchet {:?}",
            kind.name(),
            p.label(Action::Fire),
            p.label(Action::Use),
            p.label(Action::Jump),
            p.labels(Action::GyroRatchet)
        );
        assert_eq!(p.label(Action::Fire), Some(fire), "{kind:?}");
        assert_eq!(p.label(Action::Use), Some(use_), "{kind:?}");
        assert_eq!(p.label(Action::Jump), Some(jump), "{kind:?}");
    }
    let deck = make_prompts(InputDevice::Gamepad, ControllerKind::SteamDeck, &b, &n);
    assert_eq!(deck.labels(Action::GyroRatchet), ["L4", "R4"]);
}

// ---- Pretend devices in a running app ----

/// A small app with the real input systems, 60 frames a second, no window.
fn app(log_name: &str) -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rc_game_input_{}_{log_name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("input.log");
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, InputPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_plugins(CorePlugin)
        .insert_resource(GameLog::create(&log).unwrap())
        .insert_resource(LookSettings::from_settings(&Settings::default()))
        .insert_resource(GyroSettings {
            smoothing_threshold: 0.0,
            auto_calibrate: false,
            ..GyroSettings::default()
        })
        .insert_resource(names())
        .insert_resource(shipped())
        .insert_resource(BaseBindings(shipped()))
        .insert_resource(UserBindingsPath(dir.join("my_bindings.toml")))
        .insert_resource(LogInput(true))
        .add_plugins((PadsPlugin, DeviceInputPlugin));
    app.update();
    (app, dir)
}

/// Plug in a pretend controller, as SDL or gilrs would.
fn plug_in(app: &mut App, name: &str, vendor: u16, product: u16) -> Entity {
    let e = app.world_mut().spawn_empty().id();
    let c = GamepadConnectionEvent::new(
        e,
        GamepadConnection::Connected {
            name: name.into(),
            vendor_id: Some(vendor),
            product_id: Some(product),
        },
    );
    app.world_mut()
        .write_message(RawGamepadEvent::from(c.clone()));
    app.world_mut().write_message(c);
    app.update();
    e
}

fn button(app: &mut App, pad: Entity, b: GamepadButton, v: f32) {
    let ev = RawGamepadButtonChangedEvent::new(pad, b, v);
    app.world_mut().write_message(RawGamepadEvent::from(ev));
    app.world_mut().write_message(ev);
}

fn axis(app: &mut App, pad: Entity, a: GamepadAxis, v: f32) {
    let ev = RawGamepadAxisChangedEvent::new(pad, a, v);
    app.world_mut().write_message(RawGamepadEvent::from(ev));
    app.world_mut().write_message(ev);
}

/// Press or let go of a key, as the window would.
fn key(app: &mut App, k: KeyCode, down: bool) {
    app.world_mut().write_message(KeyboardInput {
        key_code: k,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: if down {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

fn actions(app: &App) -> ActionState {
    app.world().resource::<ActionState>().clone()
}

fn log_text(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("input.log")).unwrap_or_default()
}

#[test]
fn controller_buttons_sticks_and_triggers_become_actions() {
    let (mut app, dir) = app("pad");
    let pad = plug_in(&mut app, "DualSense Wireless Controller", 0x054c, 0x0ce6);
    // Cross, a half-pulled right trigger (over the 0.3 threshold), the left
    // stick pushed forward and the right stick right.
    button(&mut app, pad, GamepadButton::South, 1.0);
    button(&mut app, pad, GamepadButton::RightTrigger2, 0.5);
    axis(&mut app, pad, GamepadAxis::LeftStickY, 1.0);
    axis(&mut app, pad, GamepadAxis::RightStickX, 1.0);
    app.update();
    let s = actions(&app);
    println!(
        "pad: pressed {:?} move {} look {}",
        s.all_pressed(),
        s.movement,
        s.look
    );
    assert!(s.pressed(Action::Jump) && s.pressed(Action::Fire), "{s:?}");
    assert!((s.movement.y - 1.0).abs() < 1e-3, "{}", s.movement);
    // 2.5 radians a second at full stick, one 60th of a second.
    assert!((s.look.x - 2.5 / 60.0).abs() < 2e-3, "{}", s.look);
    assert_eq!(s.last_device, InputDevice::Gamepad);
    // A trigger only lightly touched doesn't fire.
    button(&mut app, pad, GamepadButton::RightTrigger2, 0.1);
    app.update();
    assert!(!actions(&app).pressed(Action::Fire));
    // Prompts follow: PlayStation names.
    let p = app.world().resource::<InputPrompts>().clone();
    assert_eq!(p.controller, "ps5");
    assert_eq!(p.label(Action::Jump), Some("Cross"));
    let log = log_text(&dir);
    assert!(log.contains("as ps5"), "{log}");
    assert!(
        log.contains("prompts now for Gamepad (ps5): fire is R2"),
        "{log}"
    );
    assert!(
        log.contains("pressed [\"jump\", \"fire\", \"menu_accept\"] move (0.000, 1.000) look (0.0417, 0.0000)"),
        "{log}"
    );
}

#[test]
fn keyboard_and_controller_together_and_prompts_follow_the_last_used() {
    let (mut app, dir) = app("dual");
    let pad = plug_in(&mut app, "Xbox Wireless Controller", 0x045e, 0x0b13);
    // Walk with the stick and press the reload key at the same time.
    axis(&mut app, pad, GamepadAxis::LeftStickX, 1.0);
    key(&mut app, KeyCode::KeyR, true);
    app.update();
    let s = actions(&app);
    assert!(s.pressed(Action::Reload));
    assert!((s.movement.x - 1.0).abs() < 1e-3);
    // Let go of everything, then only the keyboard.
    axis(&mut app, pad, GamepadAxis::LeftStickX, 0.0);
    app.update();
    assert_eq!(
        app.world().resource::<InputPrompts>().label(Action::Fire),
        Some("Left Mouse"),
        "keyboard was used last"
    );
    key(&mut app, KeyCode::KeyR, false);
    button(&mut app, pad, GamepadButton::West, 1.0);
    app.update();
    let p = app.world().resource::<InputPrompts>().clone();
    assert_eq!(p.device, InputDevice::Gamepad);
    assert_eq!(p.label(Action::Fire), Some("RT"));
    let log = log_text(&dir);
    println!("{log}");
    assert!(log.contains("prompts now for KeyboardMouse"), "{log}");
    assert!(log.contains("prompts now for Gamepad (xbox)"), "{log}");
}

#[test]
fn gyro_aims_with_ratchet_and_toggle() {
    let (mut app, dir) = app("gyro");
    let pad = plug_in(&mut app, "Nintendo Switch Pro Controller", 0x057e, 0x2009);
    // Use it once so it is the active controller.
    button(&mut app, pad, GamepadButton::South, 1.0);
    app.update();
    button(&mut app, pad, GamepadButton::South, 0.0);
    app.update();
    // 4 readings this frame, each 1/240 s, turning left at 1 radian a
    // second around the controller's up axis, lying flat.
    let send = |app: &mut App| {
        for _ in 0..4 {
            app.world_mut().write_message(GyroSample {
                source: GyroSource::Pad(pad),
                rate: Vec3::new(0.0, 1.0, 0.0),
                accel: Some(Vec3::Y * 9.81),
                dt: 1.0 / 240.0,
            });
        }
        app.update();
        actions(app).look
    };
    let turn = send(&mut app);
    println!("gyro: look {turn} (sensitivity 2: 2 x 1 rad/s x 1/60 s = 0.0333 left)");
    assert!((turn.x + 2.0 / 60.0).abs() < 1e-3, "{turn}");
    // Holding the ratchet (Switch Pro has no paddles; bind one to test).
    app.world_mut()
        .resource_mut::<Bindings>()
        .pad
        .push((Action::GyroRatchet, GamepadButton::LeftTrigger));
    button(&mut app, pad, GamepadButton::LeftTrigger, 1.0);
    let held = send(&mut app);
    assert_eq!(held, Vec2::ZERO, "ratchet held: no gyro turn");
    button(&mut app, pad, GamepadButton::LeftTrigger, 0.0);
    assert!(send(&mut app).x < 0.0, "ratchet let go");
    // The toggle switches it off and on.
    app.world_mut()
        .resource_mut::<Bindings>()
        .pad
        .push((Action::GyroToggle, GamepadButton::Select));
    button(&mut app, pad, GamepadButton::Select, 1.0);
    assert_eq!(send(&mut app), Vec2::ZERO, "toggled off");
    button(&mut app, pad, GamepadButton::Select, 0.0);
    assert_eq!(send(&mut app), Vec2::ZERO, "still off after letting go");
    button(&mut app, pad, GamepadButton::Select, 1.0);
    assert!(send(&mut app).x < 0.0, "toggled on again");
    // "aiming" mode: only while zoom is held (East on the shipped layout).
    app.world_mut().resource_mut::<GyroSettings>().mode = GyroMode::Aiming;
    button(&mut app, pad, GamepadButton::Select, 0.0);
    assert_eq!(send(&mut app), Vec2::ZERO, "not aiming");
    button(&mut app, pad, GamepadButton::East, 1.0);
    assert!(send(&mut app).x < 0.0, "aiming");
    let log = log_text(&dir);
    assert!(log.contains("gyro found on Pad"), "{log}");
    assert!(log.contains("gyro on"), "{log}");
}

#[test]
fn rebinding_a_key_and_a_button_is_saved() {
    let (mut app, dir) = app("rebind");
    #[derive(Resource, Default)]
    struct Got(Vec<RebindDone>);
    app.init_resource::<Got>().add_systems(
        Update,
        |mut m: MessageReader<RebindDone>, mut got: ResMut<Got>| got.0.extend(m.read().cloned()),
    );
    let pad = plug_in(&mut app, "Steam Deck", 0x28de, 0x1205);
    app.world_mut().write_message(StartRebind {
        action: Action::Reload,
        device: InputDevice::KeyboardMouse,
    });
    app.update();
    key(&mut app, KeyCode::KeyG, true);
    app.update();
    // While G is still held nothing happens, so the new key doesn't
    // reload straight away.
    assert!(!actions(&app).pressed(Action::Reload));
    key(&mut app, KeyCode::KeyG, false);
    app.update();
    app.update();
    let b = app.world().resource::<Bindings>().clone();
    assert!(b.keys.contains(&(Action::Reload, KeyCode::KeyG)));
    assert!(!b.keys.contains(&(Action::Reload, KeyCode::KeyR)));
    // Now a controller button: the Deck's L4 for crouch.
    app.world_mut().write_message(StartRebind {
        action: Action::Crouch,
        device: InputDevice::Gamepad,
    });
    app.update();
    button(&mut app, pad, button_from_name("LeftPaddle1").unwrap(), 1.0);
    app.update();
    button(&mut app, pad, button_from_name("LeftPaddle1").unwrap(), 0.0);
    app.update();
    app.update();
    let done = app.world().resource::<Got>().0.clone();
    println!("{done:?}");
    assert_eq!(done.len(), 2);
    assert_eq!(done[1].button.as_deref(), Some("LeftPaddle1"));
    // LeftPaddle1 was also the gyro ratchet: the menu is told.
    assert_eq!(done[1].also_used_by, [Action::GyroRatchet]);
    let saved = std::fs::read_to_string(dir.join("my_bindings.toml")).unwrap();
    println!("{saved}");
    assert!(saved.contains("reload = [\"KeyG\"]"), "{saved}");
    assert!(saved.contains("crouch = [\"LeftPaddle1\"]"), "{saved}");
    // The saved file gives the same controls on the next start.
    let again = shipped().with_changes(&saved).unwrap();
    assert!(again.keys.contains(&(Action::Reload, KeyCode::KeyG)));
    // Prompts name the new key.
    key(&mut app, KeyCode::KeyW, true);
    app.update();
    let p = app.world().resource::<InputPrompts>().clone();
    assert_eq!(p.label(Action::Reload), Some("G"));
    // Escape cancels.
    app.world_mut().write_message(StartRebind {
        action: Action::Jump,
        device: InputDevice::KeyboardMouse,
    });
    app.update();
    key(&mut app, KeyCode::Escape, true);
    app.update();
    let log = log_text(&dir);
    assert!(log.contains("rebind: jump not changed (Escape)"), "{log}");
}
