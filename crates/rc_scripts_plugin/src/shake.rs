//! The level's screen shakes: the scripts' `ACTION_ShakeView` steps and the
//! map's `ViewShaker` actors. Both end in a [`ShakeView`] message, which
//! the player's part carries out with the original's rules
//! (`rc_game_core::shake`).
//!
//! What each does, from the game's own `Gameplay.u` script source:
//!
//! - `ACTION_ShakeView` shakes the player wherever they are, with its
//!   `ShakeTranslationMag`, `ShakeRotationMag`, `InTime`, `SustainTime`,
//!   `OutTime` and `Frequency`, and the script carries on at once.
//! - `ViewShaker`, when its tag's event is sent (`Trigger`), shakes the
//!   player if the player is closer than its `ShakeRadius`
//!   (`VDistSq(Location, ViewTarget.Location) < Square(ShakeRadius)`), with
//!   `ShakeMag`, `ShakeRotationMag`, `InTime`, `SustainTime`, `OutTime`,
//!   `Frequency`. No fading with distance.
//!
//! Any value the map leaves out takes the class's default, read from the
//! game's `Gameplay.u` when the level loads (`ACTION_ShakeView`: move 3 3 3,
//! rotation 200 200, in 0.3 s, hold 1 s, out 0.7 s, frequency 1;
//! `ViewShaker`: move 0 5 7, rotation 500 500, in 0.1, hold 0.5, out 0.4,
//! frequency 1, and no `ShakeRadius` default, so 0). If those can't be
//! read, the shakes aren't done and the log says so; no numbers are made
//! up.
//!
//! The four `ViewShaker`s in the campaign (ras_02b, ras_02d x2, yyy_04f)
//! set no `ShakeRadius` in their maps, so with the class's 0 they never
//! reach the player, in the original as here. The log says so each time.

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_game_core::units::to_unreal;
use rc_game_core::{GameLog, GamePaths, Player, ScriptEvent, ShakeView};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_package::class_library::{ClassLibrary, DefaultValue, Defaults};

/// The values of one kind of shake, all filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct ShakeValues {
    /// Unreal units along world X, Y, Z.
    pub offset: [f32; 3],
    /// Unreal rotation units (65536 a full turn).
    pub pitch: f32,
    pub yaw: f32,
    pub in_time: f32,
    pub sustain_time: f32,
    pub out_time: f32,
    pub frequency: f32,
    /// `ViewShaker` only.
    pub radius: f32,
}

/// The class defaults, from the game's files.
#[derive(Debug, Clone, Default)]
pub struct ShakeDefaults {
    pub action: Option<ShakeValues>,
    pub shaker: Option<ShakeValues>,
    /// Why they couldn't be read.
    pub problem: Option<String>,
}

/// A `ViewShaker` placed in the map.
#[derive(Debug, Clone)]
pub struct Shaker {
    pub name: String,
    pub tag: String,
    pub location: [f32; 3],
    pub obj: ScriptObject,
}

/// The level's shakes: defaults, shakers, and the shakes the scripts asked
/// for this frame.
#[derive(Debug, Clone, Default)]
pub struct LevelShakes {
    pub defaults: ShakeDefaults,
    pub shakers: Vec<Shaker>,
    pending: Vec<ShakeView>,
    /// For the self-tests: shakes sent, and `ViewShaker`s that fired but
    /// didn't reach the player.
    pub sent: Vec<String>,
    pub out_of_reach: Vec<String>,
}

fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
}

/// A `Vector` value from the map: three floats, or named X, Y, Z.
fn map_vector(v: Option<&Val>) -> Option<[f32; 3]> {
    match v? {
        Val::Vector(v) => Some(*v),
        Val::Raw(b) if b.len() == 12 => Some([f32_at(b, 0), f32_at(b, 1), f32_at(b, 2)]),
        Val::Struct(s) => {
            let f = |k: &str| match s.get(k) {
                Some(Val::Float(f)) => *f,
                _ => 0.0,
            };
            Some([f("X"), f("Y"), f("Z")])
        }
        _ => None,
    }
}

/// A `Rotator` value from the map, as (pitch, yaw): three whole numbers
/// (Pitch, Yaw, Roll), or named.
fn map_rotator(v: Option<&Val>) -> Option<(f32, f32)> {
    match v? {
        Val::Raw(b) if b.len() == 12 => Some((i32_at(b, 0) as f32, i32_at(b, 1) as f32)),
        Val::Struct(s) => {
            let f = |k: &str| match s.get(k) {
                Some(Val::Int(i)) => *i as f32,
                _ => 0.0,
            };
            Some((f("Pitch"), f("Yaw")))
        }
        _ => None,
    }
}

fn raw12(d: &Defaults, name: &str) -> Option<[u8; 12]> {
    match d.get(name).map(|e| &e.value) {
        Some(DefaultValue::Raw(b)) if b.len() == 12 => b.as_slice().try_into().ok(),
        _ => None,
    }
}

/// One class's shake defaults. A value the class (and its parents) don't
/// set is the engine's zero.
fn class_values(lib: &ClassLibrary, class: &str, offset_key: &str) -> Result<ShakeValues, String> {
    if !lib.has_class(class) {
        return Err(format!("no class {class}"));
    }
    let d = lib.defaults(class);
    let num = |k: &str| d.number(k).unwrap_or(0.0);
    let offset = raw12(&d, offset_key)
        .map(|b| [f32_at(&b, 0), f32_at(&b, 1), f32_at(&b, 2)])
        .unwrap_or([0.0; 3]);
    let (pitch, yaw) = raw12(&d, "ShakeRotationMag")
        .map(|b| (i32_at(&b, 0) as f32, i32_at(&b, 1) as f32))
        .unwrap_or((0.0, 0.0));
    Ok(ShakeValues {
        offset,
        pitch,
        yaw,
        in_time: num("InTime"),
        sustain_time: num("SustainTime"),
        out_time: num("OutTime"),
        frequency: num("Frequency"),
        radius: num("ShakeRadius"),
    })
}

/// Read `ACTION_ShakeView`'s and `ViewShaker`'s defaults from the game.
pub fn read_defaults(paths: &GamePaths) -> ShakeDefaults {
    let mut lib = ClassLibrary::new();
    for name in ["Core", "Engine", "Gameplay"] {
        let path = paths.game_file(&format!("GameData/System/{name}.u"));
        let added = std::fs::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|data| {
                lib.add_package(name, data)
                    .map_err(|e| format!("{name}.u: {e}"))
            });
        if let Err(e) = added {
            return ShakeDefaults {
                problem: Some(e),
                ..ShakeDefaults::default()
            };
        }
    }
    let action = class_values(&lib, "ACTION_ShakeView", "ShakeTranslationMag");
    let shaker = class_values(&lib, "ViewShaker", "ShakeMag");
    let problem = [&action, &shaker]
        .iter()
        .filter_map(|r| r.as_ref().err().cloned())
        .reduce(|a, b| format!("{a}; {b}"));
    ShakeDefaults {
        action: action.ok(),
        shaker: shaker.ok(),
        problem,
    }
}

/// An object's shake: the map's values over the class's defaults.
pub fn values_of(o: &ScriptObject, defaults: &ShakeValues, offset_key: &str) -> ShakeValues {
    let (pitch, yaw) =
        map_rotator(o.get("ShakeRotationMag")).unwrap_or((defaults.pitch, defaults.yaw));
    ShakeValues {
        offset: map_vector(o.get(offset_key)).unwrap_or(defaults.offset),
        pitch,
        yaw,
        in_time: o.float("InTime").unwrap_or(defaults.in_time),
        sustain_time: o.float("SustainTime").unwrap_or(defaults.sustain_time),
        out_time: o.float("OutTime").unwrap_or(defaults.out_time),
        frequency: o.float("Frequency").unwrap_or(defaults.frequency),
        radius: o.float("ShakeRadius").unwrap_or(defaults.radius),
    }
}

fn message(v: &ShakeValues, by: String) -> ShakeView {
    // `ShakeView(InTime, SustainTime, OutTime, X, Y, Z,
    // ShakeRotationMag.Yaw, ShakeRotationMag.Pitch, Frequency)`.
    ShakeView {
        in_time: v.in_time,
        sustain_time: v.sustain_time,
        out_time: v.out_time,
        offset: v.offset,
        yaw_mag: v.yaw,
        pitch_mag: v.pitch,
        frequency: v.frequency,
        by,
    }
}

impl LevelShakes {
    /// The map's `ViewShaker`s and the game's defaults.
    pub fn read(map: &MapScripts, defaults: ShakeDefaults) -> Self {
        let shakers = map
            .of_class("ViewShaker")
            .map(|a| Shaker {
                name: a.obj.name.clone(),
                tag: a.obj.tag().to_string(),
                location: a.location,
                obj: a.obj.clone(),
            })
            .collect();
        LevelShakes {
            defaults,
            shakers,
            ..LevelShakes::default()
        }
    }

    /// Lines for the level-load log.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(p) = &self.defaults.problem {
            out.push(format!(
                "{PART}: shake defaults not read ({p}); shakes that rely on them are not done"
            ));
        }
        for s in &self.shakers {
            let v = self
                .defaults
                .shaker
                .as_ref()
                .map(|d| values_of(&s.obj, d, "ShakeMag"));
            out.push(format!(
                "{PART}:   view shaker {} (tag {}) at ({:.0}, {:.0}, {:.0}), reach {}",
                s.name,
                s.tag,
                s.location[0],
                s.location[1],
                s.location[2],
                v.map_or("unknown".to_string(), |v| format!("{} uu", v.radius))
            ));
        }
        out
    }

    /// A script's `ACTION_ShakeView`: shake the player. `None` (with the
    /// reason) when the defaults couldn't be read.
    pub fn action(&mut self, a: &ScriptObject, by: String) -> Result<ShakeView, String> {
        let d = self
            .defaults
            .action
            .as_ref()
            .ok_or("ACTION_ShakeView's defaults weren't read")?;
        let m = message(&values_of(a, d, "ShakeTranslationMag"), by);
        self.sent.push(m.by.clone());
        self.pending.push(m.clone());
        Ok(m)
    }

    /// An event was sent: `ViewShaker`s with that tag shake the player if
    /// within reach. One log line each.
    pub fn event(&mut self, name: &str, player_centre: Option<[f32; 3]>, now: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let Some(d) = self.defaults.shaker.clone() else {
            return lines;
        };
        let mut hits = Vec::new();
        for s in self
            .shakers
            .iter()
            .filter(|s| s.tag.eq_ignore_ascii_case(name))
        {
            let v = values_of(&s.obj, &d, "ShakeMag");
            let dist_sq =
                player_centre.map(|p| (0..3).map(|i| (p[i] - s.location[i]).powi(2)).sum::<f32>());
            match dist_sq {
                Some(d2) if d2 < v.radius * v.radius => {
                    lines.push(format!(
                        "{PART}: t={now:.2} view shaker {} heard {name}: the player is {:.0} uu away, within {} uu",
                        s.name,
                        d2.sqrt(),
                        v.radius
                    ));
                    hits.push(message(&v, format!("view shaker {}", s.name)));
                }
                _ => {
                    lines.push(format!(
                        "{PART}: t={now:.2} view shaker {} heard {name}: the player is {} away, not within its reach of {} uu{}, so no shake, as in the original",
                        s.name,
                        dist_sq.map_or("nowhere".to_string(), |d2| format!("{:.0} uu", d2.sqrt())),
                        v.radius,
                        if s.obj.get("ShakeRadius").is_none() {
                            " (the map sets none; the class default is used)"
                        } else {
                            ""
                        }
                    ));
                    self.out_of_reach.push(s.name.clone());
                }
            }
        }
        for h in hits {
            self.sent.push(h.by.clone());
            self.pending.push(h);
        }
        lines
    }
}

/// Send the shakes the scripts asked for, and let `ViewShaker`s hear the
/// level's events.
pub fn send(
    script: Option<ResMut<LevelScript>>,
    mut events: MessageReader<ScriptEvent>,
    player: Query<&Transform, With<Player>>,
    mut out: MessageWriter<ShakeView>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else {
        events.clear();
        return;
    };
    let now = script.director.time;
    let centre = player.single().ok().map(|t| {
        let feet = to_unreal(t.translation);
        [feet.x, feet.y, feet.z + 84.0]
    });
    let shakes = &mut script.shakes;
    if !shakes.shakers.is_empty() {
        for e in events.read() {
            for line in shakes.event(&e.name, centre, now) {
                log.line(line);
            }
        }
    } else {
        events.clear();
    }
    for m in shakes.pending.drain(..) {
        out.write(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn action_defaults() -> ShakeValues {
        // ACTION_ShakeView's defaults as Gameplay.u stores them (read with
        // `rc_probe --classes Gameplay.u ACTION_ShakeView`).
        ShakeValues {
            offset: [3.0; 3],
            pitch: 200.0,
            yaw: 200.0,
            in_time: 0.3,
            sustain_time: 1.0,
            out_time: 0.7,
            frequency: 1.0,
            radius: 0.0,
        }
    }

    fn obj(class: &str, values: Vec<(&str, Val)>) -> ScriptObject {
        ScriptObject {
            export: 0,
            class: class.into(),
            name: format!("{class}0"),
            values: values
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn map_values_override_defaults_and_rotation_is_pitch_yaw_roll() {
        // geo_05c's kind: rotation Pitch 300, Yaw 250, Roll 10.
        let mut rot = Vec::new();
        for v in [300i32, 250, 10] {
            rot.extend_from_slice(&v.to_le_bytes());
        }
        let a = obj(
            "ACTION_ShakeView",
            vec![
                ("ShakeRotationMag", Val::Raw(rot)),
                ("ShakeTranslationMag", Val::Vector([4.0, 6.0, 1.0])),
                ("OutTime", Val::Float(0.09)),
                ("SustainTime", Val::Float(0.7)),
            ],
        );
        let v = values_of(&a, &action_defaults(), "ShakeTranslationMag");
        assert_eq!(v.offset, [4.0, 6.0, 1.0]);
        assert_eq!((v.pitch, v.yaw), (300.0, 250.0));
        assert_eq!((v.in_time, v.sustain_time, v.out_time), (0.3, 0.7, 0.09));
        assert_eq!(v.frequency, 1.0);
        // The message carries the Yaw value first, as the original's call.
        let m = message(&v, "t".into());
        assert_eq!((m.yaw_mag, m.pitch_mag), (250.0, 300.0));
    }

    #[test]
    fn an_empty_action_is_all_defaults() {
        let v = values_of(
            &obj("ACTION_ShakeView", vec![]),
            &action_defaults(),
            "ShakeTranslationMag",
        );
        assert_eq!(v, action_defaults());
    }

    #[test]
    fn the_system_sends_script_shakes_and_shakers_that_reach() {
        use rc_scripts::{Director, Settings};
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        w.init_resource::<Messages<ScriptEvent>>();
        w.init_resource::<Messages<ShakeView>>();
        w.spawn((Player, Transform::default()));
        let mut script = LevelScript::bare(Director::new(
            &MapScripts::default(),
            &[],
            Settings::default(),
        ));
        script.shakes = LevelShakes {
            defaults: ShakeDefaults {
                action: Some(action_defaults()),
                shaker: Some(ShakeValues {
                    radius: 0.0,
                    ..action_defaults()
                }),
                problem: None,
            },
            shakers: vec![
                Shaker {
                    name: "Near".into(),
                    tag: "Boom".into(),
                    location: [0.0, 0.0, 84.0],
                    obj: obj("ViewShaker", vec![("ShakeRadius", Val::Float(200.0))]),
                },
                Shaker {
                    name: "NoReach".into(),
                    tag: "Boom".into(),
                    location: [0.0, 0.0, 84.0],
                    obj: obj("ViewShaker", vec![]),
                },
            ],
            ..LevelShakes::default()
        };
        // A script's ACTION_ShakeView with nothing set: the defaults.
        script
            .shakes
            .action(
                &obj("ACTION_ShakeView", vec![]),
                "ScriptedTrigger62 #1".into(),
            )
            .unwrap();
        w.insert_resource(script);
        w.write_message(ScriptEvent {
            name: "Boom".into(),
            by: "test".into(),
        });
        w.run_system_cached(send).unwrap();
        let sent: Vec<ShakeView> = w.resource_mut::<Messages<ShakeView>>().drain().collect();
        let by: Vec<&str> = sent.iter().map(|m| m.by.as_str()).collect();
        assert_eq!(by, vec!["ScriptedTrigger62 #1", "view shaker Near"]);
        assert_eq!(sent[0].offset, [3.0; 3]);
        assert_eq!(
            (sent[0].in_time, sent[0].sustain_time, sent[0].out_time),
            (0.3, 1.0, 0.7)
        );
        let s = w.resource::<LevelScript>();
        assert_eq!(s.shakes.out_of_reach, vec!["NoReach".to_string()]);
    }

    #[test]
    fn a_shaker_with_no_reach_never_shakes() {
        let mut l = LevelShakes {
            defaults: ShakeDefaults {
                shaker: Some(ShakeValues {
                    offset: [0.0, 5.0, 7.0],
                    pitch: 500.0,
                    yaw: 500.0,
                    in_time: 0.1,
                    sustain_time: 0.5,
                    out_time: 0.4,
                    frequency: 1.0,
                    radius: 0.0,
                }),
                ..ShakeDefaults::default()
            },
            shakers: vec![Shaker {
                name: "ViewShaker1".into(),
                tag: "ViewShaker".into(),
                location: [0.0; 3],
                obj: obj("ViewShaker", vec![]),
            }],
            ..LevelShakes::default()
        };
        l.event("ViewShaker", Some([0.0; 3]), 1.0);
        assert!(l.pending.is_empty());
        assert_eq!(l.out_of_reach, vec!["ViewShaker1".to_string()]);
        // With a reach set in the map, it shakes when close.
        l.shakers[0].obj = obj("ViewShaker", vec![("ShakeRadius", Val::Float(100.0))]);
        l.event("viewshaker", Some([50.0, 0.0, 0.0]), 1.0);
        assert_eq!(l.pending.len(), 1);
        assert_eq!(l.pending[0].offset, [0.0, 5.0, 7.0]);
        l.event("ViewShaker", Some([150.0, 0.0, 0.0]), 1.0);
        assert_eq!(l.pending.len(), 1);
    }
}
