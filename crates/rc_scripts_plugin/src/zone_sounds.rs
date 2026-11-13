//! The sound of the room the player is in: each zone's audio class
//! (`ZoneInfo.ZoneAudioClass`, read by `rc_level::zone_audio`) gives the
//! room its echo, a looping room sound (`PreAmbient`) and a sound that
//! plays now and then (`RandomOneShot`). Scripts swap a zone's class with
//! `ACTION_ChangeZoneAudio` (geo_03d, after the explosion).
//!
//! **The game's rules** (read in its `Engine.dll`, `FListener::Update`,
//! `UAudioSubsystem::UpdateAudioEnvData` and `PlayEnvironmentalAudio`,
//! 2026-11-12; the decompiled code stays on the PC and is never
//! committed):
//! - the room is the zone the *camera* is in (a zone with no `ZoneInfo`
//!   uses the map's `LevelInfo`);
//! - when the room's class changes, the echo switches to the new class's
//!   `Effect` (no `Effect`: no echo);
//! - the old room sound fades out over 2 seconds and stops, unless the new
//!   room has the same one; the new one starts heard everywhere (not
//!   placed), fading in over 2 seconds, and loops if the sound itself is
//!   marked `Looping`;
//! - the one-shot plays when its time has come and it isn't still playing.
//!   The next time is now + `ROSintervalMin` + a random fraction of
//!   (`ROSintervalMax` - `ROSintervalMin`). The very first one plays at
//!   once, and the time is not reset when the room changes. With
//!   `ROSradiusMax` 0 or less it is heard everywhere; otherwise it comes
//!   from a random direction around the camera (level, not above or
//!   below) at a random distance between `ROSradiusMin` and
//!   `ROSradiusMax`, fading with distance like every placed sound
//!   (`crate::sounds`). It does not loop and starts at full loudness.
//! - `ACTION_ChangeZoneAudio` sets `ZoneAudioClass` on every `ZoneInfo`
//!   whose `Tag` is the action's `ZoneInfoTag`.
//!
//! **Ours:** the shape of the 2-second fades (straight lines in loudness;
//! the curve is in `ALAudio.dll` and was not read), the random numbers
//! (a fixed sequence, so logs repeat), and the loudness of the room sounds
//! (the scripts' `[sound] volume`, as the map's other looping sounds).
//!
//! **The visor's echo** (`FListener::Update`): while the view is in a
//! vision mode (`rc_game_core::VisionModeNow`) whose `EAXEffect` is set,
//! that echo replaces the room's. That is done here, but none of the 17
//! vision modes in the game's `FrameFX` package sets one (read at load:
//! `ZoneAudio::vision_echoes`), so in the game as shipped the visor never
//! changes the echo. A mod that gives a mode an echo gets it.

use bevy::audio::{AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_audio::room::EaxReverb;
use rc_combat::{AudioParams, GameSound};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, GamePaths, PlayerCamera, RoomEcho, SoundAt, VisionModeNow};
use rc_level::scripts::ScriptObject;
use rc_level::zone_audio::{AudioEnv, Reverb};
use rc_level_plugin::LevelZoneAudio;
use std::collections::{BTreeMap, HashMap};

use crate::sounds::{loudness, Placed};
use crate::{LevelScript, PART};

/// The game's fade for room sounds, seconds.
const FADE: f32 = 2.0;

/// A script's `ACTION_ChangeZoneAudio`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ZoneAudioChange {
    /// `ZoneInfoTag`.
    pub tag: String,
    /// The new class's name, `None` for no class.
    pub class: Option<String>,
}

impl ZoneAudioChange {
    pub fn from_action(a: &ScriptObject) -> Self {
        ZoneAudioChange {
            tag: a.text("ZoneInfoTag").unwrap_or("").to_string(),
            class: a.text("ZoneAudioClass").map(str::to_string),
        }
    }
}

/// A room sound playing (the room loop or a one-shot).
#[derive(Component, Debug, Clone)]
pub struct ZoneSound {
    /// The sound's name.
    pub sound: String,
    /// The room loop (`PreAmbient`), not a one-shot.
    pub room_loop: bool,
}

/// Loudness going up or down over time (the room loop's fades).
#[derive(Component, Debug, Clone)]
pub struct Fade {
    from: f32,
    to: f32,
    t: f32,
    /// The sound card's volume at full loudness, once it has started.
    base: Option<f32>,
    /// Stop the sound when it gets there.
    stop: bool,
}

/// The room sounds of the level being played.
#[derive(Resource, Default)]
pub struct ZoneSounds {
    sounds: HashMap<String, GameSound>,
    params: HashMap<String, AudioParams>,
    /// Each zone's audio class now (scripts change them).
    zone_env: Vec<Option<usize>>,
    /// The class of the room the camera is in, once known.
    current: Option<Option<usize>>,
    zone: Option<usize>,
    room_loop: Option<(Entity, String)>,
    one_shot: Option<Entity>,
    /// When the next one-shot may play (seconds of play).
    next_one_shot: f32,
    random: u32,
    clock: f32,
}

impl ZoneSounds {
    /// 0 to 1, from a fixed sequence.
    fn random(&mut self) -> f32 {
        self.random = self
            .random
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.random >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The EAX values the game sends for an `Effect` (field for field).
pub fn eax(r: &Reverb) -> EaxReverb {
    EaxReverb {
        environment_size: r.environment_size,
        diffusion: r.diffusion,
        room: r.room,
        room_hf: r.room_hf,
        room_lf: r.room_lf,
        decay_time: r.decay_time,
        decay_hf_ratio: r.decay_hf_ratio,
        decay_lf_ratio: r.decay_lf_ratio,
        reflections: r.reflections,
        reflections_delay: r.reflections_delay,
        reverb: r.reverb,
        reverb_delay: r.reverb_delay,
        air_absorption_hf: r.air_absorption_hf,
        hf_reference: r.hf_reference,
        lf_reference: r.lf_reference,
        room_rolloff_factor: r.room_rolloff_factor,
        echo_time: r.echo_time,
        echo_depth: r.echo_depth,
        flags: r.flags,
    }
}

/// Read the room sounds every audio class of the level can play, once
/// per level.
pub(crate) fn prepare(
    mut commands: Commands,
    zones: Option<Res<LevelZoneAudio>>,
    ready: Option<Res<ZoneSounds>>,
    paths: Option<Res<GamePaths>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    log: Res<GameLog>,
) {
    let (Some(zones), Some(paths), Some(mut audio)) = (zones, paths, audio) else {
        return;
    };
    if ready.is_some() {
        return;
    }
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &zones.0.envs {
        for path in [&e.pre_ambient, &e.random_one_shot].into_iter().flatten() {
            let (Some(package), Some(name)) = (path.first(), path.last()) else {
                continue;
            };
            let list = wanted.entry(package.to_ascii_lowercase()).or_default();
            if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                list.push(name.clone());
            }
        }
    }
    let mut sounds = HashMap::new();
    let mut notes = Vec::new();
    for (package, names) in &wanted {
        let (found, n) = rc_combat::read_sounds(&paths, package, names, &mut audio);
        notes.extend(n);
        for (name, s) in found {
            sounds.insert(name.to_ascii_lowercase(), s);
        }
    }
    let mut param_packages: Vec<String> = sounds
        .values()
        .filter_map(|s: &GameSound| s.params.as_ref())
        .filter_map(|p| p.split_once('.').map(|(pk, _)| pk.to_string()))
        .collect();
    param_packages.sort();
    param_packages.dedup();
    let (params, more) = rc_combat::read_audio_params(&paths, &param_packages);
    notes.extend(more);
    log.line(format!(
        "{PART}: room sounds: {} audio classes, {} sounds read",
        zones.0.envs.len(),
        sounds.len()
    ));
    for e in &zones.0.envs {
        log.line(format!("{PART}:   room sound class {}", describe(e)));
    }
    for n in notes {
        log.line(format!("{PART}:   room sound: {n}"));
    }
    commands.insert_resource(ZoneSounds {
        sounds,
        params,
        zone_env: zones.0.zones.iter().map(|z| z.env).collect(),
        random: 0x5eed,
        ..Default::default()
    });
}

fn describe(e: &AudioEnv) -> String {
    let last = |p: &Option<Vec<String>>| {
        p.as_ref()
            .and_then(|p| p.last().cloned())
            .unwrap_or_else(|| "none".into())
    };
    format!(
        "{}: echo {}, room loop {}, one-shot {} (every {}-{} s, {}-{} uu away)",
        e.class,
        e.reverb.as_ref().map_or("none".into(), |r| {
            if r.is_heard() {
                format!("{} (room {} mB, {:.2} s)", r.name, r.room, r.decay_time)
            } else {
                format!("{} (silent: room {} mB)", r.name, r.room)
            }
        }),
        last(&e.pre_ambient),
        last(&e.random_one_shot),
        e.ros_interval_min,
        e.ros_interval_max,
        e.ros_radius_min,
        e.ros_radius_max,
    )
}

/// Scripts swapping zones' audio classes.
pub(crate) fn swap(
    mut changes: MessageReader<ZoneAudioChange>,
    zones: Option<Res<LevelZoneAudio>>,
    state: Option<ResMut<ZoneSounds>>,
    log: Res<GameLog>,
) {
    let (Some(zones), Some(mut state)) = (zones, state) else {
        changes.clear();
        return;
    };
    for c in changes.read() {
        if c.tag.is_empty() {
            continue;
        }
        let env = match &c.class {
            Some(class) => match zones.0.env_named(class) {
                Some(e) => Some(e),
                None => {
                    log.line(format!(
                        "{PART}: room sound: audio class {class} wasn't read; zones {} keep theirs",
                        c.tag
                    ));
                    continue;
                }
            },
            None => None,
        };
        let mut n = 0;
        for (i, z) in zones.0.zones.iter().enumerate() {
            if z.tag.eq_ignore_ascii_case(&c.tag) {
                if let Some(slot) = state.zone_env.get_mut(i) {
                    *slot = env;
                    n += 1;
                }
            }
        }
        log.line(format!(
            "{PART}: room sound: {n} zones tagged {} now use {}",
            c.tag,
            c.class.as_deref().unwrap_or("no audio class")
        ));
    }
}

/// Follow the camera from room to room: the echo, the room loop and the
/// one-shots.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut commands: Commands,
    time: Res<Time>,
    zones: Option<Res<LevelZoneAudio>>,
    state: Option<ResMut<ZoneSounds>>,
    script: Option<Res<LevelScript>>,
    mut echo: ResMut<RoomEcho>,
    vision: Res<VisionModeNow>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    playing: Query<(), With<ZoneSound>>,
    log: Res<GameLog>,
) {
    let (Some(zones), Some(mut state), Some(script)) = (zones, state, script) else {
        return;
    };
    let Some(ear) = ears.iter().next().map(|g| g.translation()) else {
        return;
    };
    state.clock += time.delta_secs();
    let now = state.clock;
    let p = to_unreal(ear);
    let Some(zone) = zones.0.zone_at([p.x, p.y, p.z]) else {
        return;
    };
    let env = state.zone_env.get(zone).copied().flatten();
    let volume = script.director.settings.sound_volume;
    if state.current != Some(env) {
        let new = env.map(|e| &zones.0.envs[e]);
        let zone_name = zones.0.zones.get(zone).map_or("?", |z| z.actor.as_str());
        log.line(format!(
            "{PART}: t={now:.2} room sound: camera in zone {zone} ({zone_name}): {}",
            new.map_or("no audio class (no echo, no room sounds)".into(), describe)
        ));
        // The room loop.
        let new_loop = new
            .and_then(|e| e.pre_ambient.as_ref())
            .and_then(|p| p.last());
        let same = matches!((&state.room_loop, new_loop),
            (Some((_, a)), Some(b)) if a.eq_ignore_ascii_case(b));
        if !same {
            if let Some((e, name)) = state.room_loop.take() {
                if playing.contains(e) {
                    commands.entity(e).insert(Fade {
                        from: 1.0,
                        to: 0.0,
                        t: 0.0,
                        base: None,
                        stop: true,
                    });
                    log.line(format!("{PART}: room sound: {name} fades out"));
                }
            }
            if let Some(name) = new_loop {
                match state.sounds.get(&name.to_ascii_lowercase()).cloned() {
                    Some(sound) => {
                        if let Some(h) = sound.pick() {
                            let mut settings = if sound.looping {
                                PlaybackSettings::LOOP
                            } else {
                                PlaybackSettings::DESPAWN
                            }
                            .with_volume(Volume::Linear(volume));
                            settings.muted = true;
                            let e = commands
                                .spawn((
                                    ZoneSound {
                                        sound: name.clone(),
                                        room_loop: true,
                                    },
                                    AudioPlayer(h),
                                    settings,
                                    Fade {
                                        from: 0.0,
                                        to: 1.0,
                                        t: 0.0,
                                        base: None,
                                        stop: false,
                                    },
                                ))
                                .id();
                            state.room_loop = Some((e, name.clone()));
                            log.line(format!(
                                "{PART}: room sound: {name} fades in{}",
                                if sound.looping {
                                    ", looping"
                                } else {
                                    " (the sound isn't marked Looping: plays once)"
                                }
                            ));
                        }
                    }
                    None => log.line(format!("{PART}: room sound: {name} wasn't read")),
                }
            }
        }
        state.current = Some(env);
        state.zone = Some(zone);
    }
    // The echo: the vision mode's own when it sets one, else the room's.
    let mode_echo = vision
        .0
        .as_ref()
        .and_then(|m| {
            zones
                .0
                .vision_echoes
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(m))
        })
        .and_then(|(_, r)| r.as_ref());
    let reverb = mode_echo.or(env.and_then(|e| zones.0.envs[e].reverb.as_ref()));
    let values = reverb.filter(|r| r.is_heard()).map(eax);
    let name = reverb.map_or("none".to_string(), |r| r.name.clone());
    if echo.values != values || echo.name != name {
        if let Some(r) = mode_echo {
            log.line(format!(
                "{PART}: t={now:.2} room sound: vision mode {} sets its own echo {}",
                vision.0.as_deref().unwrap_or(""),
                r.name
            ));
        }
        *echo = RoomEcho { values, name };
    }
    // The one-shot.
    let Some(e) = env.map(|e| &zones.0.envs[e]) else {
        return;
    };
    let Some(name) = e.random_one_shot.as_ref().and_then(|p| p.last()).cloned() else {
        return;
    };
    if now <= state.next_one_shot || state.one_shot.is_some_and(|s| playing.contains(s)) {
        return;
    }
    let (min, max) = (e.ros_interval_min, e.ros_interval_max);
    let r = state.random();
    state.next_one_shot = now + min + r * (max - min);
    let Some(sound) = state.sounds.get(&name.to_ascii_lowercase()).cloned() else {
        return;
    };
    let Some(h) = sound.pick() else { return };
    let params = sound
        .params
        .as_ref()
        .and_then(|p| state.params.get(p))
        .cloned();
    let params = params.as_ref();
    let tag = ZoneSound {
        sound: name.clone(),
        room_loop: false,
    };
    let settings = PlaybackSettings::DESPAWN;
    let placed = e.ros_radius_max > 0.0 && !params.is_some_and(|p| p.non_positional);
    let id = if placed {
        let a = state.random() * std::f32::consts::TAU;
        let r = e.ros_radius_min + state.random() * (e.ros_radius_max - e.ros_radius_min);
        let at = to_bevy(rc_movement::Vec3::new(
            p.x + a.cos() * r,
            p.y + a.sin() * r,
            p.z,
        ));
        // A Radius of 0 counts as 10 (the game's sound code).
        let radius = match params.and_then(|p| p.radius) {
            Some(x) if x > 0.0 => x,
            Some(_) => 10.0,
            None => script.director.settings.sound_radius,
        };
        let falloff = !params.is_some_and(|p| p.no_falloff);
        let gain = loudness(r, radius, falloff);
        log.line(format!(
            "{PART}: t={now:.2} room sound: one-shot {name}, {r:.0} uu away, loudness {gain:.2}; next in {:.1} s",
            state.next_one_shot - now
        ));
        commands
            .spawn((
                tag,
                AudioPlayer(h),
                settings.with_volume(Volume::Linear(volume * gain)),
                SoundAt::place(at),
                Placed::new(radius, falloff, None, gain),
            ))
            .id()
    } else {
        log.line(format!(
            "{PART}: t={now:.2} room sound: one-shot {name}, heard everywhere; next in {:.1} s",
            state.next_one_shot - now
        ));
        commands
            .spawn((
                tag,
                AudioPlayer(h),
                settings.with_volume(Volume::Linear(volume)),
            ))
            .id()
    };
    state.one_shot = Some(id);
}

/// The room loop's 2-second fades.
pub(crate) fn fade(
    mut commands: Commands,
    time: Res<Time>,
    mut fading: Query<(Entity, &mut Fade, &mut AudioSink)>,
) {
    for (e, mut f, mut sink) in &mut fading {
        let base = *f.base.get_or_insert_with(|| sink.volume().to_linear());
        f.t += time.delta_secs();
        let k = (f.t / FADE).min(1.0);
        let level = f.from + (f.to - f.from) * k;
        sink.set_volume(Volume::Linear(base * level));
        if sink.is_muted() {
            sink.unmute();
        }
        if k >= 1.0 {
            if f.stop {
                commands.entity(e).try_despawn();
            } else {
                commands.entity(e).remove::<Fade>();
            }
        }
    }
}

/// Leaving the level: no room sounds, no echo.
pub(crate) fn forget(
    mut commands: Commands,
    playing: Query<Entity, With<ZoneSound>>,
    mut echo: ResMut<RoomEcho>,
) {
    for e in &playing {
        commands.entity(e).try_despawn();
    }
    commands.remove_resource::<ZoneSounds>();
    *echo = RoomEcho::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_numbers_stay_between_0_and_1() {
        let mut s = ZoneSounds {
            random: 1,
            ..Default::default()
        };
        for _ in 0..1000 {
            let r = s.random();
            assert!((0.0..1.0).contains(&r), "{r}");
        }
    }

    #[test]
    fn change_reads_the_action() {
        use rc_level::scripts::Val;
        let a = ScriptObject {
            export: 0,
            class: "ACTION_ChangeZoneAudio".into(),
            name: "ACTION_ChangeZoneAudio0".into(),
            values: [
                ("ZoneInfoTag".to_string(), Val::Text("Broken1".into())),
                (
                    "ZoneAudioClass".to_string(),
                    Val::Text("GEO_03D_Broken1_sfx01".into()),
                ),
            ]
            .into_iter()
            .collect(),
        };
        let c = ZoneAudioChange::from_action(&a);
        assert_eq!(c.tag, "Broken1");
        assert_eq!(c.class.as_deref(), Some("GEO_03D_Broken1_sfx01"));
    }
}
