//! rc_padtest: check controllers and gyros without the game.
//!
//! Usage: rc_padtest [SECONDS]   (default 20)
//!
//! Lists every controller SDL finds, which kind it is taken for, and then,
//! four times a second, its pressed buttons, sticks, triggers and gyro
//! turn. Everything also goes to `logs/rc_padtest.log`.

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::InputPlugin;
use bevy::prelude::*;
use rc_game_core::GameLog;
use rc_pads::buttons::button_name;
use rc_pads::sdl::SdlPads;
use rc_pads::{ControllerKind, GyroLook, PadsPlugin};
use std::time::Duration;

#[derive(Resource)]
struct Until(f32);

fn main() -> AppExit {
    let seconds: f32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    let log = match GameLog::create(std::path::Path::new("logs/rc_padtest.log")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't create logs/rc_padtest.log: {e}");
            return AppExit::from_code(1);
        }
    };
    let pads = match SdlPads::open() {
        Ok(p) => p,
        Err(e) => {
            log.line(format!("ERROR {e}"));
            return AppExit::from_code(1);
        }
    };
    for n in &pads.notes {
        log.line(n);
    }
    log.line(format!(
        "rc_padtest: watching for {seconds} s; press buttons, move sticks, turn the controller"
    ));
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 250.0),
        )),
        InputPlugin,
        PadsPlugin,
    ))
    .insert_resource(log)
    .insert_resource(Until(seconds))
    .add_systems(Update, report);
    rc_pads::sdl::add_to(&mut app, pads);
    app.run()
}

fn report(
    time: Res<Time>,
    until: Res<Until>,
    pads: Query<(Entity, &Gamepad, Option<&ControllerKind>, Option<&Name>)>,
    gyro: Res<GyroLook>,
    log: Res<GameLog>,
    mut next: Local<f32>,
    mut turned: Local<Vec2>,
    mut exit: MessageWriter<AppExit>,
) {
    *turned += gyro.turn;
    let t = time.elapsed_secs();
    if t < *next {
        return;
    }
    *next = t + 0.25;
    for (e, pad, kind, name) in &pads {
        let pressed: Vec<&str> = pad
            .get_pressed()
            .filter_map(|b| button_name(*b))
            .collect();
        log.line(format!(
            "t={t:5.2} {e} {} \"{}\": pressed {pressed:?} left ({:.2}, {:.2}) right ({:.2}, {:.2}) triggers ({:.2}, {:.2})",
            kind.map(|k| k.name()).unwrap_or("?"),
            name.map(|n| n.as_str()).unwrap_or(""),
            pad.left_stick().x,
            pad.left_stick().y,
            pad.right_stick().x,
            pad.right_stick().y,
            pad.get(GamepadButton::LeftTrigger2).unwrap_or(0.0),
            pad.get(GamepadButton::RightTrigger2).unwrap_or(0.0),
        ));
    }
    if gyro.source.is_some() {
        let d = *turned * (180.0 / std::f32::consts::PI);
        log.line(format!(
            "t={t:5.2} gyro {:?}: turned ({:.1}, {:.1}) degrees in the last 0.25 s at sensitivity 2",
            gyro.source, d.x, d.y
        ));
    }
    *turned = Vec2::ZERO;
    if t >= until.0 {
        log.line("rc_padtest: done");
        exit.write(AppExit::Success);
    }
}
