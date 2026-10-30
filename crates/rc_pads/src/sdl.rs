//! Reading controllers and gyros with SDL3, in place of gilrs.
//!
//! SDL's events become the same Bevy gamepad events gilrs sends, so Bevy
//! keeps each controller's `Gamepad` component up to date as usual. Gyro
//! and accelerometer readings become [`GyroSample`]s.
//!
//! SDL is started without its window, sound or graphics parts: only
//! controllers, HID devices and sensors. It must stay on the main thread,
//! so it lives in a non-send resource.

use crate::buttons::{LEFT_PADDLE1, LEFT_PADDLE2, MISC1, RIGHT_PADDLE1, RIGHT_PADDLE2, TOUCHPAD};
use crate::kind::ControllerKind;
use crate::{GyroSample, GyroSource};
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::InputSystems;
use bevy::math::Vec3;
use bevy::prelude::*;
use rc_game_core::GameLog;
use sdl3_sys::everything::*;
use std::collections::HashMap;
use std::ffi::CStr;

/// One open controller.
struct Pad {
    handle: *mut SDL_Gamepad,
    entity: Entity,
    /// The last gyro reading's time, nanoseconds (SDL's sensor clock).
    gyro_time: Option<u64>,
    accel: Option<Vec3>,
}

/// SDL, started. Dropping it closes every controller and stops SDL.
pub struct SdlPads {
    pads: HashMap<u32, Pad>,
    /// The device's own sensors (a handheld PC's gyro), by SDL id.
    sensors: Vec<(u32, *mut SDL_Sensor)>,
    builtin_gyro_time: Option<u64>,
    builtin_accel: Option<Vec3>,
    /// What SDL said at start-up, for the log.
    pub notes: Vec<String>,
}

fn error() -> String {
    // SAFETY: SDL_GetError always returns a valid string.
    unsafe {
        CStr::from_ptr(SDL_GetError())
            .to_string_lossy()
            .into_owned()
    }
}

fn text(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: SDL returns valid strings or null.
        unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

impl SdlPads {
    /// Start SDL's controller and sensor parts.
    pub fn open() -> Result<SdlPads, String> {
        // SAFETY: plain calls into SDL, on the main thread, before any
        // other SDL use.
        unsafe {
            // SDL has no window of its own here, so it would otherwise not
            // know whether the game is in front; the game decides that.
            SDL_SetHint(SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, c"1".as_ptr());
            if !SDL_Init(SDL_INIT_GAMEPAD | SDL_INIT_SENSOR) {
                return Err(format!("SDL could not start: {}", error()));
            }
            let v = SDL_GetVersion();
            let mut notes = vec![format!(
                "pads: SDL {}.{}.{} reads the controllers and gyros",
                v / 1_000_000,
                (v / 1000) % 1000,
                v % 1000
            )];
            let mut sensors = Vec::new();
            let mut n = 0;
            let ids = SDL_GetSensors(&mut n);
            if !ids.is_null() {
                for i in 0..n.max(0) as usize {
                    let id = *ids.add(i);
                    let ty = SDL_GetSensorTypeForID(id);
                    if ty == SDL_SENSOR_GYRO || ty == SDL_SENSOR_ACCEL {
                        let s = SDL_OpenSensor(id);
                        if !s.is_null() {
                            notes.push(format!(
                                "pads: built-in {} \"{}\"",
                                if ty == SDL_SENSOR_GYRO {
                                    "gyro"
                                } else {
                                    "accelerometer"
                                },
                                text(SDL_GetSensorNameForID(id))
                            ));
                            sensors.push((id.0, s));
                        }
                    }
                }
                SDL_free(ids as *mut _);
            }
            Ok(SdlPads {
                pads: HashMap::new(),
                sensors,
                builtin_gyro_time: None,
                builtin_accel: None,
                notes,
            })
        }
    }
}

impl Drop for SdlPads {
    fn drop(&mut self) {
        // SAFETY: every handle was opened by us and is closed once.
        unsafe {
            for p in self.pads.values() {
                SDL_CloseGamepad(p.handle);
            }
            for (_, s) in &self.sensors {
                SDL_CloseSensor(*s);
            }
            SDL_Quit();
        }
    }
}

fn kind_of(handle: *mut SDL_Gamepad, vendor: u16, product: u16, name: &str) -> ControllerKind {
    // SAFETY: a valid open gamepad.
    let t = unsafe { SDL_GetGamepadType(handle) };
    match t {
        SDL_GAMEPAD_TYPE_XBOX360 | SDL_GAMEPAD_TYPE_XBOXONE => ControllerKind::Xbox,
        SDL_GAMEPAD_TYPE_PS3 | SDL_GAMEPAD_TYPE_PS4 => ControllerKind::PlayStation4,
        SDL_GAMEPAD_TYPE_PS5 => ControllerKind::PlayStation5,
        SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_PRO => ControllerKind::SwitchPro,
        SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_LEFT => ControllerKind::JoyConLeft,
        SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_RIGHT => ControllerKind::JoyConRight,
        SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_PAIR => ControllerKind::JoyConPair,
        SDL_GAMEPAD_TYPE_GAMECUBE => ControllerKind::GameCube,
        // SDL calls both the Steam Deck and the Steam Controller "Steam".
        SDL_GAMEPAD_TYPE_STEAM => match ControllerKind::detect(Some(vendor), Some(product), name) {
            ControllerKind::SteamDeck => ControllerKind::SteamDeck,
            _ => ControllerKind::Steam,
        },
        _ => ControllerKind::detect(Some(vendor), Some(product), name),
    }
}

fn button(b: u8) -> Option<GamepadButton> {
    let b = SDL_GamepadButton(b as i32);
    Some(match b {
        SDL_GAMEPAD_BUTTON_SOUTH => GamepadButton::South,
        SDL_GAMEPAD_BUTTON_EAST => GamepadButton::East,
        SDL_GAMEPAD_BUTTON_WEST => GamepadButton::West,
        SDL_GAMEPAD_BUTTON_NORTH => GamepadButton::North,
        SDL_GAMEPAD_BUTTON_BACK => GamepadButton::Select,
        SDL_GAMEPAD_BUTTON_GUIDE => GamepadButton::Mode,
        SDL_GAMEPAD_BUTTON_START => GamepadButton::Start,
        SDL_GAMEPAD_BUTTON_LEFT_STICK => GamepadButton::LeftThumb,
        SDL_GAMEPAD_BUTTON_RIGHT_STICK => GamepadButton::RightThumb,
        SDL_GAMEPAD_BUTTON_LEFT_SHOULDER => GamepadButton::LeftTrigger,
        SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER => GamepadButton::RightTrigger,
        SDL_GAMEPAD_BUTTON_DPAD_UP => GamepadButton::DPadUp,
        SDL_GAMEPAD_BUTTON_DPAD_DOWN => GamepadButton::DPadDown,
        SDL_GAMEPAD_BUTTON_DPAD_LEFT => GamepadButton::DPadLeft,
        SDL_GAMEPAD_BUTTON_DPAD_RIGHT => GamepadButton::DPadRight,
        SDL_GAMEPAD_BUTTON_RIGHT_PADDLE1 => GamepadButton::Other(RIGHT_PADDLE1),
        SDL_GAMEPAD_BUTTON_LEFT_PADDLE1 => GamepadButton::Other(LEFT_PADDLE1),
        SDL_GAMEPAD_BUTTON_RIGHT_PADDLE2 => GamepadButton::Other(RIGHT_PADDLE2),
        SDL_GAMEPAD_BUTTON_LEFT_PADDLE2 => GamepadButton::Other(LEFT_PADDLE2),
        SDL_GAMEPAD_BUTTON_TOUCHPAD => GamepadButton::Other(TOUCHPAD),
        SDL_GAMEPAD_BUTTON_MISC1 => GamepadButton::Other(MISC1),
        _ => return None,
    })
}

/// An SDL axis reading to Bevy's: sticks -1..1 with up positive (SDL's is
/// down positive), triggers 0..1 as analogue buttons.
enum Moved {
    Axis(GamepadAxis, f32),
    Trigger(GamepadButton, f32),
}

fn axis(a: u8, value: i16) -> Option<Moved> {
    let v = (value as f32 / 32767.0).clamp(-1.0, 1.0);
    let a = SDL_GamepadAxis(a as i32);
    Some(match a {
        SDL_GAMEPAD_AXIS_LEFTX => Moved::Axis(GamepadAxis::LeftStickX, v),
        SDL_GAMEPAD_AXIS_LEFTY => Moved::Axis(GamepadAxis::LeftStickY, -v),
        SDL_GAMEPAD_AXIS_RIGHTX => Moved::Axis(GamepadAxis::RightStickX, v),
        SDL_GAMEPAD_AXIS_RIGHTY => Moved::Axis(GamepadAxis::RightStickY, -v),
        SDL_GAMEPAD_AXIS_LEFT_TRIGGER => Moved::Trigger(GamepadButton::LeftTrigger2, v.max(0.0)),
        SDL_GAMEPAD_AXIS_RIGHT_TRIGGER => Moved::Trigger(GamepadButton::RightTrigger2, v.max(0.0)),
        _ => return None,
    })
}

/// Seconds between two sensor readings, from SDL's nanosecond clock.
fn step(last: &mut Option<u64>, now: u64) -> f32 {
    let dt = match *last {
        Some(t) if now > t => (now - t) as f32 * 1e-9,
        _ => 0.0,
    };
    *last = Some(now);
    dt
}

/// Adds the SDL reader. `rc_game` calls this only when [`SdlPads::open`]
/// worked, and leaves gilrs out.
pub fn add_to(app: &mut App, pads: SdlPads) {
    app.insert_non_send(pads)
        .insert_resource(crate::PadReader::Sdl)
        .add_systems(PreUpdate, read_sdl.before(InputSystems));
}

#[allow(clippy::too_many_arguments)]
fn read_sdl(
    mut sdl: NonSendMut<SdlPads>,
    mut commands: Commands,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
    mut gyro: MessageWriter<GyroSample>,
    log: Option<Res<GameLog>>,
) {
    let sdl = &mut *sdl;
    let mut e = SDL_Event::default();
    // SAFETY: SDL_PollEvent fills `e`; each union field is read only for
    // the event type that uses it; handles are SDL's own.
    unsafe {
        while SDL_PollEvent(&mut e) {
            let ty = SDL_EventType(e.r#type);
            match ty {
                SDL_EVENT_GAMEPAD_ADDED => {
                    let id = e.gdevice.which;
                    if sdl.pads.contains_key(&id.0) {
                        continue;
                    }
                    let handle = SDL_OpenGamepad(id);
                    if handle.is_null() {
                        if let Some(log) = &log {
                            log.line(format!(
                                "pads: SDL could not open a controller: {}",
                                error()
                            ));
                        }
                        continue;
                    }
                    let name = text(SDL_GetGamepadName(handle));
                    let vendor = SDL_GetGamepadVendor(handle);
                    let product = SDL_GetGamepadProduct(handle);
                    let kind = kind_of(handle, vendor, product, &name);
                    let mut gyro_note = "no gyro";
                    if SDL_GamepadHasSensor(handle, SDL_SENSOR_GYRO) {
                        SDL_SetGamepadSensorEnabled(handle, SDL_SENSOR_GYRO, true);
                        gyro_note = "gyro";
                        if SDL_GamepadHasSensor(handle, SDL_SENSOR_ACCEL) {
                            SDL_SetGamepadSensorEnabled(handle, SDL_SENSOR_ACCEL, true);
                            gyro_note = "gyro and accelerometer";
                        }
                    }
                    let entity = commands.spawn((kind, Name::new(name.clone()))).id();
                    if let Some(log) = &log {
                        log.line(format!(
                            "pads: SDL opened \"{name}\" as {} ({gyro_note}, {:.0} readings a second)",
                            kind.name(),
                            SDL_GetGamepadSensorDataRate(handle, SDL_SENSOR_GYRO)
                        ));
                    }
                    sdl.pads.insert(
                        id.0,
                        Pad {
                            handle,
                            entity,
                            gyro_time: None,
                            accel: None,
                        },
                    );
                    let c = GamepadConnectionEvent::new(
                        entity,
                        GamepadConnection::Connected {
                            name,
                            vendor_id: Some(vendor),
                            product_id: Some(product),
                        },
                    );
                    events.write(c.clone().into());
                    connections.write(c);
                }
                SDL_EVENT_GAMEPAD_REMOVED => {
                    if let Some(p) = sdl.pads.remove(&e.gdevice.which.0) {
                        SDL_CloseGamepad(p.handle);
                        let c =
                            GamepadConnectionEvent::new(p.entity, GamepadConnection::Disconnected);
                        events.write(c.clone().into());
                        connections.write(c);
                    }
                }
                SDL_EVENT_GAMEPAD_BUTTON_DOWN | SDL_EVENT_GAMEPAD_BUTTON_UP => {
                    let b = e.gbutton;
                    if let (Some(p), Some(button)) = (sdl.pads.get(&b.which.0), button(b.button)) {
                        let v = if b.down { 1.0 } else { 0.0 };
                        let ev = RawGamepadButtonChangedEvent::new(p.entity, button, v);
                        events.write(ev.into());
                        buttons.write(ev);
                    }
                }
                SDL_EVENT_GAMEPAD_AXIS_MOTION => {
                    let a = e.gaxis;
                    let Some(p) = sdl.pads.get(&a.which.0) else {
                        continue;
                    };
                    match axis(a.axis, a.value) {
                        Some(Moved::Axis(ax, v)) => {
                            let ev = RawGamepadAxisChangedEvent::new(p.entity, ax, v);
                            events.write(ev.into());
                            axes.write(ev);
                        }
                        Some(Moved::Trigger(b, v)) => {
                            let ev = RawGamepadButtonChangedEvent::new(p.entity, b, v);
                            events.write(ev.into());
                            buttons.write(ev);
                        }
                        None => {}
                    }
                }
                SDL_EVENT_GAMEPAD_SENSOR_UPDATE => {
                    let s = e.gsensor;
                    let Some(p) = sdl.pads.get_mut(&s.which.0) else {
                        continue;
                    };
                    let v = Vec3::from_array(s.data);
                    let st = SDL_SensorType(s.sensor);
                    if st == SDL_SENSOR_ACCEL {
                        p.accel = Some(v);
                    } else if st == SDL_SENSOR_GYRO {
                        let dt = step(&mut p.gyro_time, s.sensor_timestamp);
                        gyro.write(GyroSample {
                            source: GyroSource::Pad(p.entity),
                            rate: v,
                            accel: p.accel,
                            dt,
                        });
                    }
                }
                SDL_EVENT_SENSOR_UPDATE => {
                    let s = e.sensor;
                    let Some((_, handle)) = sdl.sensors.iter().find(|(id, _)| *id == s.which.0)
                    else {
                        continue;
                    };
                    let v = Vec3::new(s.data[0], s.data[1], s.data[2]);
                    let st = SDL_GetSensorType(*handle);
                    if st == SDL_SENSOR_ACCEL {
                        sdl.builtin_accel = Some(v);
                    } else if st == SDL_SENSOR_GYRO {
                        let dt = step(&mut sdl.builtin_gyro_time, s.sensor_timestamp);
                        gyro.write(GyroSample {
                            source: GyroSource::BuiltIn,
                            rate: v,
                            accel: sdl.builtin_accel,
                            dt,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDL's own pretend controller (a "virtual joystick"), with PS5 ids,
    /// buttons, sticks, a gyro and an accelerometer, read through the real
    /// SDL reader into Bevy. SDL is one per program, so this is one test.
    #[test]
    fn a_virtual_ps5_controller_through_sdl() {
        use crate::{ActivePad, GyroLook, GyroSettings, PadsPlugin};
        use bevy::input::gamepad::Gamepad;
        use bevy::input::InputPlugin;

        let pads = SdlPads::open().expect("SDL starts without a window");
        assert!(pads.notes[0].contains("SDL 3."), "{:?}", pads.notes);
        let sensors = [
            SDL_VirtualJoystickSensorDesc {
                r#type: SDL_SENSOR_GYRO,
                rate: 250.0,
            },
            SDL_VirtualJoystickSensorDesc {
                r#type: SDL_SENSOR_ACCEL,
                rate: 250.0,
            },
        ];
        let desc = SDL_VirtualJoystickDesc {
            r#type: SDL_JOYSTICK_TYPE_GAMEPAD.0 as u16,
            vendor_id: 0x054c,
            product_id: 0x0ce6,
            naxes: SDL_GAMEPAD_AXIS_COUNT.0 as u16,
            nbuttons: SDL_GAMEPAD_BUTTON_COUNT.0 as u16,
            nsensors: 2,
            sensors: sensors.as_ptr(),
            name: c"Pretend DualSense".as_ptr(),
            ..Default::default()
        };
        // SAFETY: SDL is started; `desc` and `sensors` outlive the call.
        let js = unsafe {
            let id = SDL_AttachVirtualJoystick(&desc);
            assert_ne!(id.0, 0, "{}", error());
            SDL_OpenJoystick(id)
        };
        assert!(!js.is_null(), "{}", error());

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, PadsPlugin))
            .insert_resource(GyroSettings {
                smoothing_threshold: 0.0,
                auto_calibrate: false,
                ..GyroSettings::default()
            });
        add_to(&mut app, pads);
        app.update();
        app.update();
        let (e, kind) = {
            let w = app.world_mut();
            let mut q = w.query::<(Entity, &ControllerKind, &Gamepad)>();
            let found: Vec<_> = q.iter(w).map(|(e, k, _)| (e, *k)).collect();
            assert_eq!(found.len(), 1, "one controller: {found:?}");
            found[0]
        };
        println!("SDL controller {e} is {kind:?}");
        assert_eq!(kind, ControllerKind::PlayStation5);

        // Cross down, left stick fully up, right trigger half pulled.
        // SAFETY: `js` is open.
        unsafe {
            SDL_SetJoystickVirtualButton(js, SDL_GAMEPAD_BUTTON_SOUTH.0, true);
            SDL_SetJoystickVirtualAxis(js, SDL_GAMEPAD_AXIS_LEFTY.0, -32768);
            // A raw trigger axis runs -32768 (let go) to 32767 (pulled), so
            // 0 is half way.
            SDL_SetJoystickVirtualAxis(js, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER.0, 0);
        }
        app.update();
        app.update();
        let pad = app.world().get::<Gamepad>(e).unwrap();
        println!(
            "SDL: south {} left stick {} right trigger {:?}",
            pad.pressed(GamepadButton::South),
            pad.left_stick(),
            pad.get(GamepadButton::RightTrigger2)
        );
        assert!(pad.pressed(GamepadButton::South));
        assert!((pad.left_stick().y - 1.0).abs() < 0.01);
        assert!((pad.get(GamepadButton::RightTrigger2).unwrap() - 0.5).abs() < 0.01);
        assert_eq!(app.world().resource::<ActivePad>().entity, Some(e));

        // Lying flat, turning left at 1 radian a second: four readings
        // 4 ms apart (the first only starts the clock).
        // SAFETY: `js` is open; the data arrays outlive each call.
        unsafe {
            for i in 0..4u64 {
                let t = 1_000_000_000 + i * 4_000_000;
                let a = [0.0f32, 9.81, 0.0];
                SDL_SendJoystickVirtualSensorData(js, SDL_SENSOR_ACCEL, t, a.as_ptr(), 3);
                let g = [0.0f32, 1.0, 0.0];
                SDL_SendJoystickVirtualSensorData(js, SDL_SENSOR_GYRO, t, g.as_ptr(), 3);
            }
        }
        app.update();
        let look = app.world().resource::<GyroLook>().clone();
        println!("SDL gyro: {look:?} (expected 3 x 4 ms x 1 rad/s x sensitivity 2 = 0.024 left)");
        assert_eq!(look.source, Some(GyroSource::Pad(e)));
        assert!((look.turn.x + 0.024).abs() < 1e-3, "{look:?}");

        // Unplugged: the controller goes away.
        // SAFETY: `js` is open; closed once.
        unsafe {
            let id = SDL_GetJoystickID(js);
            SDL_CloseJoystick(js);
            SDL_DetachVirtualJoystick(id);
        }
        app.update();
        app.update();
        assert!(app.world().get::<Gamepad>(e).is_none(), "unplugged");
    }

    #[test]
    fn axes_and_buttons() {
        assert!(matches!(
            axis(SDL_GAMEPAD_AXIS_LEFTY.0 as u8, -32768),
            Some(Moved::Axis(GamepadAxis::LeftStickY, v)) if (v - 1.0).abs() < 1e-6
        ));
        assert!(matches!(
            axis(SDL_GAMEPAD_AXIS_RIGHT_TRIGGER.0 as u8, 16384),
            Some(Moved::Trigger(GamepadButton::RightTrigger2, v)) if (v - 0.5).abs() < 0.01
        ));
        assert_eq!(
            button(SDL_GAMEPAD_BUTTON_SOUTH.0 as u8),
            Some(GamepadButton::South)
        );
        assert_eq!(
            button(SDL_GAMEPAD_BUTTON_LEFT_PADDLE1.0 as u8),
            Some(GamepadButton::Other(LEFT_PADDLE1))
        );
        let mut t = None;
        assert_eq!(step(&mut t, 1_000_000_000), 0.0);
        assert!((step(&mut t, 1_004_000_000) - 0.004).abs() < 1e-6);
    }
}
