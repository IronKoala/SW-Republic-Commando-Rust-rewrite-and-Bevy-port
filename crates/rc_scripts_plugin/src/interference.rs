//! Interference generators: static on the helmet's view near a jamming
//! device (geo_03a places 4 `InterferenceGenerator`s round its
//! `JammingDevice`).
//!
//! All of this is the original's own rules, read from its files and its
//! compiled code (decompiled for reference only, nothing copied):
//!
//! - **The generator** (ctgame.u `InterferenceGenerator`, an invisible
//!   actor): `bIsOn` (class default true, the map may change it); a level
//!   event naming its tag (`Trigger`) switches it off when on and on when
//!   off. geo_03a's script sends `InterferenceGenerator` once the jamming
//!   device is destroyed (`ScriptedTrigger76`, after `Target01Destroyed`),
//!   so the static stops then.
//! - **How much static** (ctgame.u `CTHUD.Tick`, every frame): the
//!   nearest switched-on generator within `ScavDroidMaxRange` (CTHUD's
//!   default, 1000 units) of the view; `StaticAlpha =
//!   int((1 - distance / range) ^ ScavStaticGamma * MAX_SCAV_STATIC)` with
//!   `ScavStaticGamma` 1.5 (CTHUD's default) and `MAX_SCAV_STATIC` 128 (a
//!   constant in the script's source); 0 when none is in range. The HUD
//!   is where the view is (`CTHUD.PreRender` moves it there each frame).
//!   The result goes to [`HudStatic`], which the visor draws.
//! - **What the static looks like** (engine.dll
//!   `UFrameFX::AccumulateWorkingTarget`): `StaticAlpha` (held to 0..255)
//!   plus the vision mode's own `Noise`, held to 255, divided by 255, is
//!   the film grain strength of the vision mode's shader (pixel constant
//!   c1). Drawn by `rc_visor`.
//! - **The hiss** (engine.dll `UHelmet::DrawHUD`, the end): while
//!   `StaticAlpha` is above 0 the HUD loops its `StaticScavenger` sound
//!   (`GEN_Sound.Interface.int_GEN_squadHeal_lp_01`, engine.u's `HUD`
//!   default), heard from the helmet; each frame its volume moves to
//!   `StaticAlpha / 255` over 0.1 seconds, in a straight line. When it
//!   drops to 0 the sound stops, fading out over 0.5 seconds. (While the
//!   player is incapacitated the original plays `StaticIncapacitated` at
//!   180 instead: not done, the port has no incapacitated state yet.)
//!
//! Not done: a Trandoshan scavenger droid carries its own generator in the
//! original (ctcharacters.u `TrandoshanScav`), and one on the helmet
//! (`bScavangerHead`) gives full static; the port has no scavengers yet.
//! Which generators are off is not kept in save files. Logged with the
//! `scripts:` prefix.

use crate::PART;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::units::to_unreal;
use rc_game_core::{GameLog, GamePaths, GameState, HudStatic, PlayerCamera, ScriptEvent};
use rc_level::scripts::MapScripts;
use rc_package::class_library::ClassLibrary;

/// The original's class.
pub const CLASS: &str = "InterferenceGenerator";
/// ctgame.u `CTHUD`: `const MAX_SCAV_STATIC = 128;`.
pub const MAX_STATIC: f32 = 128.0;
/// engine.dll `UHelmet::DrawHUD`: the hiss's volume follows over this
/// long, and fades out over `STOP_SECONDS`.
pub const FOLLOW_SECONDS: f32 = 0.1;
pub const STOP_SECONDS: f32 = 0.5;

/// One generator placed in the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Generator {
    pub name: String,
    pub tag: String,
    pub location: [f32; 3],
    pub on: bool,
}

/// The HUD's numbers for the static, from CTHUD's class defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticRule {
    /// `ScavDroidMaxRange`, Unreal units.
    pub range: f32,
    /// `ScavStaticGamma`.
    pub gamma: f32,
}

impl StaticRule {
    /// ctgame.u `CTHUD.Tick`: the static for the nearest switched-on
    /// generator, 0 when none is in range.
    pub fn alpha(&self, eye: [f32; 3], generators: &[Generator]) -> i32 {
        // The script starts from 10000 squared and keeps the nearest.
        let mut nearest = 10000.0f32 * 10000.0;
        for g in generators.iter().filter(|g| g.on) {
            let d = (0..3)
                .map(|i| (g.location[i] - eye[i]).powi(2))
                .sum::<f32>();
            // `ForEach CollidingActors(..., ScavDroidMaxRange, Location)`.
            if d <= self.range * self.range && d < nearest {
                nearest = d;
            }
        }
        if nearest < self.range * self.range {
            let t = (1.0 - nearest.sqrt() / self.range).powf(self.gamma);
            // UnrealScript's int() cuts off the fraction.
            (t * MAX_STATIC) as i32
        } else {
            0
        }
    }
}

/// The level's generators while it is played.
#[derive(Resource, Debug, Clone)]
pub struct LevelInterference {
    pub list: Vec<Generator>,
    pub rule: StaticRule,
    /// The hiss (`StaticScavenger`), when it was found.
    pub sound: Option<rc_combat::GameSound>,
    /// For the self-tests and the log: (seconds, generator, now on).
    pub switched: Vec<(f32, String, bool)>,
    /// Highest static seen so far.
    pub most: i32,
}

/// Marks the hiss while it plays.
#[derive(Component, Debug)]
pub struct StaticHiss {
    /// Volume it is moving from and to, and seconds into the move.
    from: f32,
    to: f32,
    t: f32,
    seconds: f32,
    /// Fading out: removed when the fade ends.
    stopping: bool,
}

impl StaticHiss {
    /// The volume now: a straight line from `from` to `to`.
    pub fn volume(&self) -> f32 {
        if self.seconds <= 0.0 {
            return self.to;
        }
        let k = (self.t / self.seconds).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * k
    }

    fn head_for(&mut self, to: f32, seconds: f32) {
        self.from = self.volume();
        self.to = to;
        self.t = 0.0;
        self.seconds = seconds;
    }
}

/// The script packages the classes below live in.
pub fn class_library(paths: &GamePaths) -> Result<ClassLibrary, String> {
    let mut lib = ClassLibrary::new();
    for name in ["Core", "Engine", "CTGame"] {
        let path = paths.game_file(&format!("GameData/System/{name}.u"));
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        lib.add_package(name, data)
            .map_err(|e| format!("{name}.u: {e}"))?;
    }
    Ok(lib)
}

/// Whether a placed class is an `InterferenceGenerator` (or made from one).
fn is_generator(lib: Option<&ClassLibrary>, class: &str) -> bool {
    class.eq_ignore_ascii_case(CLASS)
        || lib.is_some_and(|l| l.chain(class).iter().any(|c| c.eq_ignore_ascii_case(CLASS)))
}

/// Every generator in the map, switched on or off as it starts.
pub fn read(map: &MapScripts, lib: Option<&ClassLibrary>) -> Vec<Generator> {
    map.actors
        .iter()
        .filter(|a| is_generator(lib, &a.obj.class))
        .map(|a| {
            let class_on = lib
                .and_then(|l| l.defaults(&a.obj.class).bool("bIsOn"))
                .unwrap_or(true);
            Generator {
                name: a.obj.name.clone(),
                tag: a.obj.tag().to_string(),
                location: a.location,
                on: a.obj.flag("bIsOn", class_on),
            }
        })
        .collect()
}

/// CTHUD's numbers, and the path of its `StaticScavenger` sound.
pub fn rule(lib: Option<&ClassLibrary>) -> (StaticRule, Option<String>, Vec<String>) {
    let mut problems = Vec::new();
    let d = lib.map(|l| l.defaults("CTHUD"));
    let mut num = |n: &str, fallback: f32| match d.as_ref().and_then(|d| d.number(n)) {
        Some(v) => v,
        None => {
            problems.push(format!("CTHUD {n} not read, using {fallback}"));
            fallback
        }
    };
    let rule = StaticRule {
        range: num("ScavDroidMaxRange", 1000.0),
        gamma: num("ScavStaticGamma", 1.5),
    };
    // Its full path (package.group.name), as the class stores it.
    let sound = lib
        .and_then(|l| l.object_path("CTHUD", "StaticScavenger"))
        .flatten()
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("None"));
    if sound.is_none() {
        problems.push("CTHUD StaticScavenger not read: no hiss".into());
    }
    (rule, sound, problems)
}

/// Read the level's generators and the HUD's numbers (while the level
/// loads).
pub fn prepare(
    map: &MapScripts,
    paths: &GamePaths,
    audio: &mut Assets<bevy::audio::AudioSource>,
    log: &GameLog,
) -> LevelInterference {
    let lib = match class_library(paths) {
        Ok(l) => Some(l),
        Err(e) => {
            log.line(format!("{PART}: interference: classes not read ({e})"));
            None
        }
    };
    let list = read(map, lib.as_ref());
    let (rule, sound_path, problems) = if list.is_empty() {
        (
            StaticRule {
                range: 1000.0,
                gamma: 1.5,
            },
            None,
            Vec::new(),
        )
    } else {
        rule(lib.as_ref())
    };
    for p in problems {
        log.line(format!("{PART}: interference: {p}"));
    }
    let mut sound = None;
    if let Some(path) = &sound_path {
        let package = path.split('.').next().unwrap_or("");
        let name = path.rsplit('.').next().unwrap_or("").to_string();
        let (found, notes) =
            rc_combat::read_sounds(paths, package, std::slice::from_ref(&name), audio);
        for n in notes {
            log.line(format!("{PART}: interference: {n}"));
        }
        sound = found
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
            .map(|(_, s)| s.clone());
        if sound.is_none() {
            log.line(format!("{PART}: interference: hiss {path} not found"));
        }
    }
    if !list.is_empty() {
        log.line(format!(
            "{PART}: {} interference generators (static within {:.0} units, gamma {}, hiss {})",
            list.len(),
            rule.range,
            rule.gamma,
            sound_path.as_deref().unwrap_or("none")
        ));
        for g in &list {
            log.line(format!(
                "{PART}:   interference generator {} (tag {}) at ({:.0}, {:.0}, {:.0}), {}",
                g.name,
                g.tag,
                g.location[0],
                g.location[1],
                g.location[2],
                if g.on { "on" } else { "off" }
            ));
        }
    }
    LevelInterference {
        list,
        rule,
        sound,
        switched: Vec::new(),
        most: 0,
    }
}

/// Switch generators on events, work out the static, and play the hiss.
#[allow(clippy::too_many_arguments)]
pub fn run(
    mut commands: Commands,
    level: Option<ResMut<LevelInterference>>,
    mut events: MessageReader<ScriptEvent>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut hiss: Query<(Entity, &mut StaticHiss, Option<&mut AudioSink>)>,
    mut out: ResMut<HudStatic>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let Some(mut level) = level else {
        events.clear();
        return;
    };
    let now = time.elapsed_secs();
    for ev in events.read() {
        for i in 0..level.list.len() {
            if !level.list[i].tag.eq_ignore_ascii_case(&ev.name) {
                continue;
            }
            // InterferenceGenerator.Trigger: on -> off, off -> on.
            let g = &mut level.list[i];
            g.on = !g.on;
            let (name, on) = (g.name.clone(), g.on);
            log.line(format!(
                "{PART}: t={now:.2} interference generator {name} switched {} by {}",
                if on { "on" } else { "off" },
                ev.name
            ));
            level.switched.push((now, name, on));
        }
    }
    let alpha = match camera.iter().next() {
        Some(cam) => {
            let e = to_unreal(cam.translation());
            level.rule.alpha([e.x, e.y, e.z], &level.list)
        }
        None => 0,
    };
    if alpha != out.alpha {
        if (alpha > 0) != (out.alpha > 0) {
            log.line(format!(
                "{PART}: t={now:.2} helmet static {} (StaticAlpha {alpha})",
                if alpha > 0 { "starts" } else { "stops" }
            ));
        }
        out.alpha = alpha;
    }
    level.most = level.most.max(alpha);
    // The hiss (UHelmet::DrawHUD).
    let dt = time.delta_secs();
    let mut playing = false;
    for (e, mut h, sink) in &mut hiss {
        h.t += dt;
        if h.stopping {
            if h.t >= h.seconds {
                commands.entity(e).despawn();
                continue;
            }
        } else if alpha > 0 {
            playing = true;
            let to = alpha.min(255) as f32 / 255.0;
            if to != h.to {
                h.head_for(to, FOLLOW_SECONDS);
            }
        } else {
            h.head_for(0.0, STOP_SECONDS);
            h.stopping = true;
        }
        if let Some(mut s) = sink {
            s.set_volume(Volume::Linear(h.volume()));
        }
    }
    if alpha > 0 && !playing {
        if let Some(handle) = level.sound.as_ref().and_then(|s| s.pick()) {
            // Started at the sound's own volume, then moved each frame.
            commands.spawn((
                AudioPlayer(handle),
                PlaybackSettings::LOOP,
                StaticHiss {
                    from: 1.0,
                    to: 1.0,
                    t: 0.0,
                    seconds: 0.0,
                    stopping: false,
                },
                DespawnOnExit(GameState::InGame),
            ));
        }
    }
}

/// Leaving the level: no static.
pub fn forget(mut commands: Commands, mut out: ResMut<HudStatic>) {
    commands.remove_resource::<LevelInterference>();
    out.alpha = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(at: [f32; 3], on: bool) -> Generator {
        Generator {
            name: "g".into(),
            tag: CLASS.into(),
            location: at,
            on,
        }
    }

    const RULE: StaticRule = StaticRule {
        range: 1000.0,
        gamma: 1.5,
    };

    #[test]
    fn static_grows_towards_the_nearest_generator_that_is_on() {
        let list = [gen([0.0, 0.0, 0.0], true)];
        // On top of it: the most, 128.
        assert_eq!(RULE.alpha([0.0; 3], &list), 128);
        // Half way: 0.5^1.5 * 128 = 45.25, cut to 45.
        assert_eq!(RULE.alpha([500.0, 0.0, 0.0], &list), 45);
        // At the edge and beyond: none.
        assert_eq!(RULE.alpha([1000.0, 0.0, 0.0], &list), 0);
        assert_eq!(RULE.alpha([3000.0, 0.0, 0.0], &list), 0);
        // Only the nearest counts, and one that is off doesn't.
        let two = [gen([0.0, 0.0, 0.0], false), gen([800.0, 0.0, 0.0], true)];
        assert_eq!(
            RULE.alpha([0.0; 3], &two),
            (0.2f32.powf(1.5) * 128.0) as i32
        );
    }

    #[test]
    fn the_hiss_moves_in_a_straight_line() {
        let mut h = StaticHiss {
            from: 1.0,
            to: 1.0,
            t: 0.0,
            seconds: 0.0,
            stopping: false,
        };
        h.head_for(0.5, 0.1);
        assert_eq!(h.volume(), 1.0);
        h.t = 0.05;
        assert!((h.volume() - 0.75).abs() < 1e-6);
        h.t = 0.2;
        assert_eq!(h.volume(), 0.5);
    }

    /// geo_03a as the install has it (skipped without RC_GAME_DIR): its 4
    /// generators start on, CTHUD's numbers and hiss are read, and the
    /// script that runs once the jamming device is destroyed sends their
    /// tag.
    #[test]
    fn geo_03a_from_the_install() {
        let Some(dir) = std::env::var_os("RC_GAME_DIR") else {
            return;
        };
        let paths = GamePaths::new(dir, vec![]);
        let file = paths.game_file("GameData/Maps/geo_03a.ctm");
        let Ok(data) = std::fs::read(&file) else {
            return;
        };
        let pkg = rc_package::Package::parse(&data).unwrap();
        let level = rc_level::actors::read_level(&pkg, &data).unwrap();
        let map = rc_level::scripts::read(&pkg, &data, &level);
        let lib = class_library(&paths).unwrap();
        let list = read(&map, Some(&lib));
        assert_eq!(list.len(), 4, "{list:?}");
        assert!(list.iter().all(|g| g.on && g.tag == CLASS));
        let (rule, sound, problems) = rule(Some(&lib));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rule, RULE);
        assert_eq!(
            sound.as_deref(),
            Some("GEN_Sound.Interface.int_GEN_squadHeal_lp_01")
        );
        let sender = map
            .actors
            .iter()
            .find(|a| a.obj.name == "ScriptedTrigger76")
            .unwrap();
        assert!(sender
            .actions
            .iter()
            .any(|a| a.class.ends_with("TriggerEvent") && a.text("Event") == Some(CLASS)));
        // Standing at the jamming device (JammingDevice0): static.
        let a = rule.alpha([-2544.0, -15766.0, 5024.0 + 60.0], &list);
        println!("static at the jamming device: {a}");
        assert!(a > 0 && a < 128);
    }
}
