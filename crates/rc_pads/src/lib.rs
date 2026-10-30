//! rc_pads: controllers. Which kind each one is, the names of its buttons
//! for on-screen prompts, and gyro aiming.
//!
//! Controllers are read one of two ways:
//!
//! - **SDL3** (the `sdl` feature, on by default on Windows and Linux): reads
//!   Xbox, PlayStation, Switch Pro, Joy-Con, GameCube adapters, the Steam
//!   Deck and most others, says exactly which kind each one is, and reads
//!   their gyros and accelerometers. It also reads a handheld PC's own
//!   built-in gyro, where the system offers one. SDL is built from source
//!   with the game, so nothing extra needs installing to play.
//! - **gilrs** (Bevy's own gamepad reader), used when the game is built
//!   without SDL or SDL can't start, and on Android. No gyro; the kind is
//!   guessed from the controller's USB ids and name.
//!
//! Either way, controllers appear as Bevy `Gamepad` entities, so the game's
//! input code reads them the same way. This plugin adds a
//! [`ControllerKind`] to each one, keeps [`ActivePad`] (the controller used
//! last) and turns gyro readings into [`GyroLook`].

pub mod buttons;
pub mod gyro;
pub mod kind;
pub mod names;
#[cfg(all(feature = "sdl", not(target_os = "android")))]
pub mod sdl;

pub use gyro::{GyroFilter, GyroMode, GyroSettings, GyroSpace};
pub use kind::{ControllerKind, Family};
pub use names::ButtonNames;

use bevy::input::gamepad::{Gamepad, GamepadConnection, GamepadConnectionEvent};
use bevy::input::InputSystems;
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use rc_game_core::GameLog;
use std::collections::HashMap;

/// Where a gyro reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GyroSource {
    /// A controller's gyro.
    Pad(Entity),
    /// The device's own built-in gyro (a handheld PC, later a phone).
    BuiltIn,
}

/// One gyro reading. The SDL reader sends these; tests and a future phone
/// reader can send them too.
#[derive(Message, Debug, Clone, Copy)]
pub struct GyroSample {
    pub source: GyroSource,
    /// Radians per second around the device's X (right), Y (up) and Z
    /// (towards the player) axes.
    pub rate: Vec3,
    /// The accelerometer, m/s², if the device has one.
    pub accel: Option<Vec3>,
    /// Seconds since the previous reading from this source.
    pub dt: f32,
}

/// The controller used most recently, for button prompts and the gyro.
#[derive(Resource, Debug, Clone, Default)]
pub struct ActivePad {
    pub entity: Option<Entity>,
    pub kind: ControllerKind,
    pub name: String,
}

/// This frame's gyro turn, radians (`x` right, `y` up), already through
/// calibration, smoothing and sensitivity. The game's input code adds it to
/// the look when gyro aiming is on and the ratchet isn't held.
#[derive(Resource, Debug, Clone, Default)]
pub struct GyroLook {
    pub turn: Vec2,
    /// Where `turn` came from, or `None` if no gyro sent anything.
    pub source: Option<GyroSource>,
    /// Every source that has sent a reading since it connected.
    pub sources: Vec<GyroSource>,
}

/// Which reader is in use, for the log and the options screen.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadReader {
    Sdl,
    Gilrs,
}

/// The plugin. `rc_game` adds it; with `sdl` on it first tries
/// [`sdl::SdlPads::open`] and only adds gilrs if that fails.
pub struct PadsPlugin;

impl Plugin for PadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActivePad>()
            .init_resource::<GyroLook>()
            .init_resource::<GyroSettings>()
            .init_resource::<ButtonNames>()
            .init_resource::<GyroFilters>()
            .add_message::<GyroSample>()
            .add_systems(
                PreUpdate,
                (name_new_pads, track_active_pad, gyro_to_look)
                    .chain()
                    .after(InputSystems),
            );
    }
}

#[derive(Resource, Default)]
struct GyroFilters(HashMap<GyroSource, GyroFilter>);

/// Give each new controller a kind (the SDL reader already did) and log it.
fn name_new_pads(
    mut commands: Commands,
    mut connections: MessageReader<GamepadConnectionEvent>,
    kinds: Query<&ControllerKind>,
    log: Option<Res<GameLog>>,
) {
    for c in connections.read() {
        match &c.connection {
            GamepadConnection::Connected {
                name,
                vendor_id,
                product_id,
            } => {
                let kind = kinds.get(c.gamepad).copied().unwrap_or_else(|_| {
                    let k = ControllerKind::detect(*vendor_id, *product_id, name);
                    commands.entity(c.gamepad).insert(k);
                    k
                });
                commands.entity(c.gamepad).insert(Name::new(name.clone()));
                if let Some(log) = &log {
                    log.line(format!(
                        "pads: connected \"{name}\" ({:04x}:{:04x}) as {}",
                        vendor_id.unwrap_or(0),
                        product_id.unwrap_or(0),
                        kind.name()
                    ));
                }
            }
            GamepadConnection::Disconnected => {
                if let Some(log) = &log {
                    log.line(format!("pads: disconnected {}", c.gamepad));
                }
            }
        }
    }
}

/// A controller counts as used when a button is down or a stick is pushed
/// past this.
const ACTIVE_STICK: f32 = 0.3;

fn track_active_pad(
    pads: Query<(Entity, &Gamepad, Option<&ControllerKind>, Option<&Name>)>,
    mut active: ResMut<ActivePad>,
    log: Option<Res<GameLog>>,
) {
    for (e, pad, kind, name) in &pads {
        let used = pad.get_pressed().next().is_some()
            || pad.left_stick().length() > ACTIVE_STICK
            || pad.right_stick().length() > ACTIVE_STICK;
        if used && active.entity != Some(e) {
            active.entity = Some(e);
            active.kind = kind.copied().unwrap_or_default();
            active.name = name.map(|n| n.to_string()).unwrap_or_default();
            if let Some(log) = &log {
                log.line(format!(
                    "pads: now using {e} ({}) for prompts and gyro",
                    active.kind.name()
                ));
            }
        }
    }
    if let Some(e) = active.entity {
        if pads.get(e).is_err() {
            *active = ActivePad::default();
        }
    }
}

/// Use the active controller's gyro if it has one, else the device's own.
fn gyro_to_look(
    mut samples: MessageReader<GyroSample>,
    settings: Res<GyroSettings>,
    active: Res<ActivePad>,
    pads: Query<(), With<Gamepad>>,
    mut filters: ResMut<GyroFilters>,
    mut look: ResMut<GyroLook>,
    log: Option<Res<GameLog>>,
) {
    let mut turns: HashMap<GyroSource, Vec2> = HashMap::new();
    for s in samples.read() {
        let f = filters.0.entry(s.source).or_default();
        let before = f.calibrations;
        *turns.entry(s.source).or_default() += f.sample(s.rate, s.accel, s.dt, &settings);
        if f.calibrations != before {
            if let Some(log) = &log {
                let b = f.bias * 180.0 / std::f32::consts::PI;
                log.line(format!(
                    "pads: gyro {:?} lay still; drift measured as ({:.2}, {:.2}, {:.2}) deg/s",
                    s.source, b.x, b.y, b.z
                ));
            }
        }
        if !look.sources.contains(&s.source) {
            look.sources.push(s.source);
            if let Some(log) = &log {
                log.line(format!("pads: gyro found on {:?}", s.source));
            }
        }
    }
    // Forget controllers that were unplugged.
    look.sources.retain(|s| match s {
        GyroSource::Pad(e) => pads.contains(*e),
        GyroSource::BuiltIn => true,
    });
    filters.0.retain(|s, _| match s {
        GyroSource::Pad(e) => pads.contains(*e),
        GyroSource::BuiltIn => true,
    });
    let pick = active
        .entity
        .map(GyroSource::Pad)
        .filter(|p| look.sources.contains(p))
        .or_else(|| {
            look.sources
                .contains(&GyroSource::BuiltIn)
                .then_some(GyroSource::BuiltIn)
        })
        .or_else(|| look.sources.first().copied());
    look.source = pick;
    look.turn = pick
        .and_then(|p| turns.get(&p).copied())
        .unwrap_or_default();
}
