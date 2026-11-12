//! The lines and sounds a level's scripts play (`ACTION_PlaySound`,
//! `ACTION_PlayLocalSound`): squad radio chatter, the briefing voice,
//! explosions on cue. All of them come from the game's own sound packages,
//! read while the level loads.
//!
//! A script names a sound only by its name (`CA1G1701`); the map's import
//! table says which package holds it (`GEO_Voice`, see
//! `rc_level::scripts::sound_refs`). While loading we read every sound the
//! map's scripts can play, measure each one's length (so a script that
//! waits for a line waits exactly that long), and keep them ready.
//!
//! **Our choices** (the rules live in compiled game code we don't read):
//! - each speaker (the action's `SoundSourceTag`, or the script itself when
//!   there is none) says one thing at a time: a new line of speech (a
//!   sound from a `*_voice` package) stops the line it was saying, unless
//!   the action sets `bDontOverrideOtherSounds`. Other sounds (explosions,
//!   rumbles) never stop each other;
//! - a group (`SoundMultiple`) waits for its longest variant.
//!
//! **Where a sound comes from.** Each of the game's sounds names its
//! settings (`AudioParams`, in `Params_VOX.uax` / `Params_SFX.uax`, read at
//! load). Those say whether it is heard from nowhere in particular (radio:
//! the Clone Advisor, most squad lines, briefings) or from a place, and how
//! far it carries (`Radius`). A placed sound comes from:
//! - the character or actor its `SoundSourceTag` names (a character is
//!   followed as it moves; another actor stays where the map put it);
//! - with no tag: the character running the sequence (a
//!   `ScriptedSequence`'s pawn, as in UE2), else the script's own spot in
//!   the map.
//!
//! Heard everywhere instead: `ACTION_PlayLocalSound` ("local" means the
//! player's own ears), sounds the game marks `NonPositional`, sounds aimed
//! at the player (`SoundSourceTag=PlayerCommando`), and a tag nothing in
//! the level has (logged).
//!
//! **Loudness with distance (the game's own rule, read from its sound
//! code):** full loudness out to the sound's `Radius`, then
//! `Radius / distance`. The PC game plays through OpenAL with the
//! "inverse distance, clamped" model, a roll-off of 1, no far limit, and
//! each sound's reference distance set to its `AudioParams.Radius`
//! (0 counts as 10), with distances and radius both in Unreal units
//! (checked in the game's `Engine.dll` and `ALAudio.dll`, 2026-11-12;
//! the decompiled code stays on the PC and is never committed). Two parts
//! are still **our reading**: an action `Volume` above 1 (a few maps use 2
//! to 10000) makes it carry that many times further instead of louder, and
//! the quietest a sound gets is 1/1000 (so it can come back up).
//! `bDisable3DFalloff` sounds come from their side but don't fade. Which
//! side a sound is on is the 3D SOUND option's job (`rc_game_core::SoundAt`).

use bevy::audio::{
    AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink, Volume,
};
use bevy::prelude::*;
use rc_combat::{AudioParams, GameSound};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, GamePaths, Play, PlayerCamera, ScriptTags, SoundAt};
use rc_level::scripts::{MapScripts, ScriptObject};
use rc_package::sound::read_streamed;
use rc_package::Package;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use crate::{LevelScript, PART};

/// Every sound the level's scripts can play, read while it loaded.
#[derive(Resource, Debug, Default)]
pub struct ScriptSounds {
    /// By lower-case name.
    pub sounds: HashMap<String, GameSound>,
    /// Seconds, by lower-case name (the longest variant of a group).
    pub lengths: HashMap<String, f32>,
    /// Lower-case names of the sounds that are speech (from a `*_voice`
    /// package).
    pub speech: std::collections::HashSet<String>,
    /// How many sounds came from each package.
    pub packages: BTreeMap<String, usize>,
    /// Sounds the scripts name that couldn't be read, with why.
    pub missing: Vec<String>,
    /// The game's own settings for each sound (`AudioParams`), by
    /// lower-case sound name. A sound missing here is placed with the
    /// settings' `default_radius`.
    pub params: HashMap<String, AudioParams>,
}

/// A sound a script asked for, waiting to be played this frame.
#[derive(Debug, Clone)]
pub(crate) struct SoundRequest {
    pub name: String,
    /// Who says it: `SoundSourceTag`, or the script's name.
    pub source: String,
    pub volume: f32,
    /// Stop what the same speaker was saying.
    pub cut_off: bool,
    /// For the log: `ScriptedTrigger52 #13`.
    pub by: String,
    /// `ACTION_PlayLocalSound`: for the player's ears only, never placed.
    pub local: bool,
    /// `source` is the action's `SoundSourceTag` (not the script's name).
    pub tagged: bool,
    /// The sequence's tag, for a `ScriptedSequence`'s own sounds: its
    /// character says them.
    pub owner: Option<String>,
    /// How many times further than its radius it carries: the action's
    /// `Volume` when above 1.
    pub reach: f32,
}

/// A script sound with a place in the level: how loud it is with distance.
#[derive(Component, Debug, Clone)]
pub struct Placed {
    /// Unreal units: full loudness out to here (radius times reach).
    pub radius: f32,
    /// Quieter past `radius` (false for `bDisable3DFalloff`).
    pub falloff: bool,
    /// The character it comes from, followed as it moves.
    pub follow: Option<Entity>,
    /// The distance loudness last given to the sound card.
    pub gain: f32,
    /// The sound card's volume divided by `gain` (the SOUND options'
    /// volumes included), once it has started.
    base: Option<f32>,
}

impl Placed {
    /// A placed sound `gain` loud now (see [`loudness`]).
    pub fn new(radius: f32, falloff: bool, follow: Option<Entity>, gain: f32) -> Self {
        Placed {
            radius,
            falloff,
            follow,
            gain,
            base: None,
        }
    }
}

/// How loud a placed sound is at `distance` (Unreal units), 0 to 1. See
/// the module notes.
pub fn loudness(distance: f32, radius: f32, falloff: bool) -> f32 {
    if !falloff || distance <= radius {
        1.0
    } else {
        (radius / distance).max(MIN_GAIN)
    }
}

/// Quietest a placed sound is made (so its loudness can still be scaled
/// back up as the listener comes nearer).
const MIN_GAIN: f32 = 1e-3;

/// A sound a level script is playing.
#[derive(Component, Debug, Clone)]
pub struct ScriptSound {
    pub name: String,
    pub source: String,
    /// A line of speech (see the module notes).
    pub speech: bool,
}

/// The names of every sound the map's scripts can play.
pub fn sounds_used(map: &MapScripts) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in map.scripts() {
        for act in &a.actions {
            if rc_scripts::sound_action(act) {
                if let Some(n) = act.text("Sound") {
                    if !n.eq_ignore_ascii_case("None")
                        && !out.iter().any(|m| m.eq_ignore_ascii_case(n))
                    {
                        out.push(n.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Read every sound the map's scripts can play.
pub(crate) fn load(
    map: &MapScripts,
    paths: &GamePaths,
    audio: &mut Assets<AudioSource>,
    log: &GameLog,
) -> ScriptSounds {
    let t = Instant::now();
    let mut out = ScriptSounds::default();
    let mut wanted_params: Vec<(String, String)> = Vec::new();
    // Group the names by package, and pick out the streamed ones (music
    // stingers, kept as .ogg files in GameData/Music).
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut streamed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in sounds_used(map) {
        match map.sounds.get(&name.to_ascii_lowercase()) {
            Some(r) => match &r.package {
                Some(p) if r.class.eq_ignore_ascii_case("SoundStreamed") => {
                    streamed.entry(p.clone()).or_default().push(name)
                }
                Some(p) => wanted.entry(p.clone()).or_default().push(name),
                None => out
                    .missing
                    .push(format!("{name}: stored in the map itself (not read yet)")),
            },
            None => out
                .missing
                .push(format!("{name}: the map doesn't say which package has it")),
        }
    }
    for (package, names) in &wanted {
        let (found, notes) = rc_combat::read_sounds(paths, package, names, audio);
        out.missing.extend(notes);
        *out.packages.entry(package.clone()).or_default() += found.len();
        let speech = package.to_ascii_lowercase().ends_with("_voice");
        for (name, sound) in found {
            // The longest variant: a script waiting for a group never cuts
            // it short.
            let secs = sound
                .variants
                .iter()
                .filter_map(|h| audio.get(h))
                .filter_map(|s| rc_audio::Wav::parse(&s.bytes).ok())
                .map(|w| w.seconds() as f32)
                .fold(0.0f32, f32::max);
            if secs > 0.0 {
                out.lengths.insert(name.to_ascii_lowercase(), secs);
            }
            if speech {
                out.speech.insert(name.to_ascii_lowercase());
            }
            if let Some(p) = &sound.params {
                wanted_params.push((name.to_ascii_lowercase(), p.clone()));
            }
            out.sounds.insert(name.to_ascii_lowercase(), sound);
        }
    }
    // The game's own settings for each sound: where it's heard from.
    let mut param_packages: Vec<String> = wanted_params
        .iter()
        .filter_map(|(_, p)| p.split_once('.').map(|(pk, _)| pk.to_string()))
        .collect();
    param_packages.sort();
    param_packages.dedup();
    // Settings that can't be found don't stop a sound playing: it is
    // placed with the default distance (logged below).
    let (all_params, mut unplaced) = rc_combat::read_audio_params(paths, &param_packages);
    for (name, p) in wanted_params {
        match all_params.get(&p) {
            Some(a) => {
                out.params.insert(name, a.clone());
            }
            None => unplaced.push(format!("{name}: its settings {p} weren't found")),
        }
    }
    for (package, names) in &streamed {
        let n = read_streamed_sounds(paths, package, names, audio, &mut out);
        *out.packages.entry(package.clone()).or_default() += n;
    }
    let total: f32 = out.lengths.values().sum();
    let everywhere = out.params.values().filter(|p| p.non_positional).count();
    log.line(format!(
        "{PART}: {} script sounds read ({:.0} s of sound) from {:?}, {} missing, {:.0} ms; the game's settings: {} heard everywhere, {} from a place, {} without settings",
        out.sounds.len(),
        total,
        out.packages,
        out.missing.len(),
        t.elapsed().as_secs_f64() * 1000.0,
        everywhere,
        out.params.len() - everywhere,
        out.sounds.len() - out.params.len(),
    ));
    for m in &out.missing {
        log.line(format!("{PART}:   sound {m}"));
    }
    for m in &unplaced {
        log.line(format!(
            "{PART}:   sound settings: {m} (placed with the default distance)"
        ));
    }
    out
}

/// `SoundStreamed` sounds: the package only names an .ogg file in
/// `GameData/Music`. Their length isn't measured (a script waiting for
/// one waits the settings' `sound_seconds`). Returns how many were read.
fn read_streamed_sounds(
    paths: &GamePaths,
    package: &str,
    names: &[String],
    audio: &mut Assets<AudioSource>,
    out: &mut ScriptSounds,
) -> usize {
    let path = paths.game_file(&format!("GameData/Sounds/{package}.uax"));
    let read = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            Package::parse(&d)
                .map(|p| (p, d))
                .map_err(|e| e.to_string())
        });
    let (pkg, data) = match read {
        Ok(v) => v,
        Err(e) => {
            out.missing
                .push(format!("{names:?}: can't read {}: {e}", path.display()));
            return 0;
        }
    };
    let mut n = 0;
    for name in names {
        let export = pkg.exports.iter().find(|e| {
            pkg.export_class_name(e) == "SoundStreamed"
                && pkg.name(e.object_name).eq_ignore_ascii_case(name)
        });
        let Some(export) = export else {
            out.missing.push(format!("{name}: not in {package}"));
            continue;
        };
        let file = match read_streamed(&pkg, &data, export) {
            Ok(s) => s.file_name,
            Err(e) => {
                out.missing.push(format!("{name}: didn't read: {e}"));
                continue;
            }
        };
        let stem = file.trim_end_matches(".ogg").trim_end_matches(".OGG");
        let ogg = paths.game_file(&format!("GameData/Music/{stem}.ogg"));
        match std::fs::read(&ogg) {
            Ok(bytes) => {
                let h = audio.add(AudioSource {
                    bytes: bytes.into(),
                });
                out.sounds.insert(
                    name.to_ascii_lowercase(),
                    GameSound {
                        variants: vec![h],
                        params: None,
                    },
                );
                n += 1;
            }
            Err(e) => out
                .missing
                .push(format!("{name}: can't read {} ({e})", ogg.display())),
        }
    }
    n
}

/// What a sound action asks for. `owner`: the sequence's tag, for a
/// `ScriptedSequence`'s actions.
pub(crate) fn request(
    who: &str,
    index: usize,
    a: &ScriptObject,
    volume: f32,
    owner: Option<String>,
) -> SoundRequest {
    let tag = a
        .text("SoundSourceTag")
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("None"));
    let own = a.float("Volume").unwrap_or(1.0);
    SoundRequest {
        name: a.text("Sound").unwrap_or("").to_string(),
        source: tag.unwrap_or(who).to_string(),
        volume: (volume * own).clamp(0.0, 1.0),
        cut_off: !a.flag("bDontOverrideOtherSounds", false),
        by: format!("{who} #{index}"),
        local: a.class.to_ascii_lowercase().ends_with("playlocalsound"),
        tagged: tag.is_some(),
        owner: owner.filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case("None")),
        reach: own.max(1.0),
    }
}

/// Where a sound comes from.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    /// Heard the same everywhere, and why.
    Everywhere(String),
    /// A character, followed (Bevy units, its place now).
    Follow(Entity, Vec3),
    /// A fixed spot, Bevy units.
    At(Vec3),
}

/// Work out where a requested sound comes from (see the module notes).
fn source_of(
    r: &SoundRequest,
    params: Option<&AudioParams>,
    script: &LevelScript,
    chars: &Query<(Entity, &ScriptTags, &GlobalTransform)>,
) -> Source {
    if r.local {
        return Source::Everywhere("a local sound".into());
    }
    if params.is_some_and(|p| p.non_positional) {
        return Source::Everywhere("the game plays it as radio".into());
    }
    if r.tagged && script.director.settings.is_player(&r.source) {
        return Source::Everywhere("aimed at the player".into());
    }
    let character = |want: &dyn Fn(&ScriptTags) -> bool| {
        chars
            .iter()
            .find(|(_, t, _)| want(t))
            .map(|(e, _, g)| Source::Follow(e, g.translation()))
    };
    let same =
        |a: &Option<String>, b: &str| a.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(b));
    if r.tagged {
        if let Some(f) = character(&|t| same(&t.tag, &r.source)) {
            return f;
        }
    } else if let Some(owner) = &r.owner {
        if let Some(f) = character(&|t| same(&t.script, owner)) {
            return f;
        }
    }
    match script.places.find(&r.source) {
        Some(p) => Source::At(to_bevy(rc_movement::Vec3::new(
            p.location[0],
            p.location[1],
            p.location[2],
        ))),
        None => Source::Everywhere(format!("nothing in the level is called {}", r.source)),
    }
}

/// Play what the scripts asked for this frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    sounds: Option<Res<ScriptSounds>>,
    playing: Query<(Entity, &ScriptSound)>,
    chars: Query<(Entity, &ScriptTags, &GlobalTransform)>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    if script.sounds.is_empty() {
        return;
    }
    let now = script.director.time;
    // What is playing, with what this frame starts.
    let mut on: Vec<(Entity, ScriptSound)> = playing.iter().map(|(e, s)| (e, s.clone())).collect();
    for r in std::mem::take(&mut script.sounds) {
        let key = r.name.to_ascii_lowercase();
        let Some(h) = sounds
            .as_ref()
            .and_then(|s| s.sounds.get(&key))
            .and_then(|s| s.pick())
        else {
            script.stats.sounds_missing.push(r.name.clone());
            log.line(format!(
                "{PART}: t={now:.2} {}: sound {} not loaded; nothing plays",
                r.by, r.name
            ));
            continue;
        };
        let speech = sounds.as_ref().is_some_and(|s| s.speech.contains(&key));
        let mut stopped = Vec::new();
        if r.cut_off && speech {
            on.retain(|(e, s)| {
                let same = s.speech && s.source.eq_ignore_ascii_case(&r.source);
                if same {
                    stopped.push(s.name.clone());
                    commands.entity(*e).try_despawn();
                }
                !same
            });
        }
        let secs = sounds
            .as_ref()
            .and_then(|s| s.lengths.get(&key))
            .map(|l| format!("{l:.1} s"))
            .unwrap_or_else(|| "length unknown".into());
        // Where it comes from, and how loud it is from here.
        let params = sounds.as_ref().and_then(|s| s.params.get(&key));
        let from = source_of(&r, params, &script, &chars);
        let ear = ears.iter().next().map(|g| g.translation());
        let radius = params
            .and_then(|p| p.radius)
            .unwrap_or(script.director.settings.sound_radius)
            * r.reach;
        let falloff = !params.is_some_and(|p| p.no_falloff);
        let (place, follow) = match &from {
            Source::Everywhere(_) => (None, None),
            Source::Follow(e, at) => (Some(*at), Some(*e)),
            Source::At(at) => (Some(*at), None),
        };
        let distance = match (place, ear) {
            (Some(p), Some(e)) => Some(to_unreal(p - e).length()),
            _ => None,
        };
        let gain = distance.map_or(1.0, |d| loudness(d, radius, falloff));
        let heard = match (&from, distance) {
            (Source::Everywhere(why), _) => format!("heard everywhere: {why}"),
            (Source::Follow(..), d) => format!(
                "from {} (followed), {} uu away, reach {radius:.0} uu, loudness {gain:.2}",
                r.source,
                d.map_or("?".into(), |d| format!("{d:.0}"))
            ),
            (Source::At(p), d) => {
                let u = to_unreal(*p);
                format!(
                    "from {} at ({:.0}, {:.0}, {:.0}), {} uu away, reach {radius:.0} uu, loudness {gain:.2}",
                    r.source,
                    u.x,
                    u.y,
                    u.z,
                    d.map_or("?".into(), |d| format!("{d:.0}"))
                )
            }
        };
        log.line(format!(
            "{PART}: t={now:.2} {}: {} {} {} ({secs}, volume {:.2}; {heard}){}",
            r.by,
            r.source,
            if speech { "says" } else { "plays" },
            r.name,
            r.volume,
            if stopped.is_empty() {
                String::new()
            } else {
                format!("; stops {}", stopped.join(", "))
            }
        ));
        script.stats.sounds_played.push(r.name.clone());
        if place.is_some() {
            script.stats.sounds_placed.push(r.name.clone());
        }
        let tag = ScriptSound {
            name: r.name,
            source: r.source,
            speech,
        };
        let mut spawned = commands.spawn((
            tag.clone(),
            AudioPlayer(h),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(r.volume * gain)),
        ));
        if let Some(at) = place {
            spawned.insert((
                SoundAt::place(at),
                Placed {
                    radius,
                    falloff,
                    follow,
                    gain,
                    base: None,
                },
            ));
        }
        // Speech follows the VOICES volume (rc_game_core::Voice).
        if speech {
            spawned.insert(rc_game_core::Voice);
        }
        let e = spawned.id();
        on.push((e, tag));
    }
}

/// Log when a script sound really starts on the sound card (a placed
/// sound gets a `SpatialAudioSink` when 3D SOUND is on).
#[allow(clippy::type_complexity)]
pub(crate) fn started(
    new: Query<&ScriptSound, Or<(Added<AudioSink>, Added<SpatialAudioSink>)>>,
    script: Option<ResMut<LevelScript>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    for s in &new {
        script.stats.sounds_started += 1;
        log.line(format!(
            "{PART}: sound {} ({}) started on the sound card",
            s.name, s.source
        ));
    }
}

/// Lines (and the map's looping sounds) stop while the game is paused,
/// and go on after.
pub(crate) fn pause(
    play: Option<Res<State<Play>>>,
    sinks: Query<&AudioSink, Or<(With<ScriptSound>, With<crate::ambient::AmbientLoop>)>>,
    spatial: Query<&SpatialAudioSink, Or<(With<ScriptSound>, With<crate::ambient::AmbientLoop>)>>,
    mut was_paused: Local<bool>,
) {
    let paused = play.is_some_and(|p| *p.get() == Play::Paused);
    if paused == *was_paused {
        return;
    }
    *was_paused = paused;
    let all = sinks
        .iter()
        .map(|s| s as &dyn AudioSinkPlayback)
        .chain(spatial.iter().map(|s| s as &dyn AudioSinkPlayback));
    for s in all {
        if paused {
            s.pause();
        } else {
            s.play();
        }
    }
}

/// Placed sounds move with the character saying them, and get louder or
/// quieter as the player comes nearer or goes away.
#[allow(clippy::type_complexity)]
pub(crate) fn follow(
    mut placed: Query<(
        &mut Placed,
        &mut Transform,
        &mut SoundAt,
        Option<&mut AudioSink>,
        Option<&mut SpatialAudioSink>,
    )>,
    bodies: Query<&GlobalTransform, Without<Placed>>,
    ears: Query<&GlobalTransform, (With<PlayerCamera>, Without<Placed>)>,
) {
    let ear = ears.iter().next().map(|g| g.translation());
    for (mut p, mut t, mut at, sink, spatial) in &mut placed {
        if let Some(g) = p.follow.and_then(|e| bodies.get(e).ok()) {
            t.translation = g.translation();
            at.0 = g.translation();
        }
        let Some(ear) = ear else { continue };
        let sink: &mut dyn AudioSinkPlayback = match (sink, spatial) {
            (Some(s), _) => s.into_inner(),
            (_, Some(s)) => s.into_inner(),
            _ => continue,
        };
        let old = p.gain.max(MIN_GAIN);
        let base = *p
            .base
            .get_or_insert_with(|| sink.volume().to_linear() / old);
        let gain = loudness(to_unreal(t.translation - ear).length(), p.radius, p.falloff);
        if (gain - p.gain).abs() > 0.005 {
            p.gain = gain;
            sink.set_volume(Volume::Linear(base * gain));
        }
    }
}

/// Leaving the level silences its lines.
pub(crate) fn forget(mut commands: Commands, playing: Query<Entity, With<ScriptSound>>) {
    for e in &playing {
        commands.entity(e).try_despawn();
    }
    commands.remove_resource::<ScriptSounds>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::{Place, Val, Vals};
    use rc_scripts::{Director, Settings};
    use std::collections::HashSet;

    fn action(vals: &[(&str, Val)]) -> ScriptObject {
        let mut values = Vals::new();
        for (k, v) in vals {
            values.insert(k.to_string(), v.clone());
        }
        ScriptObject {
            export: 0,
            class: "ACTION_PlaySound".into(),
            name: "PlaySound".into(),
            values,
        }
    }

    fn t(s: &str) -> Val {
        Val::Text(s.into())
    }

    /// A world with three loaded sounds: two lines of speech and a bang.
    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        let mut audio = Assets::<AudioSource>::default();
        let mut sounds = ScriptSounds::default();
        for name in ["line1", "line2", "bang"] {
            let h = audio.add(AudioSource {
                bytes: vec![0u8; 4].into(),
            });
            sounds.sounds.insert(
                name.into(),
                GameSound {
                    variants: vec![h],
                    params: None,
                },
            );
        }
        sounds.speech = HashSet::from(["line1".to_string(), "line2".to_string()]);
        // line1 is radio, line2 carries 600 uu, bang doesn't fade.
        let params = |radius, non_positional, no_falloff| AudioParams {
            radius,
            non_positional,
            no_falloff,
        };
        sounds
            .params
            .insert("line1".into(), params(None, true, false));
        sounds
            .params
            .insert("line2".into(), params(Some(600.0), false, false));
        sounds
            .params
            .insert("bang".into(), params(Some(500.0), false, true));
        w.insert_resource(audio);
        w.insert_resource(sounds);
        w.insert_resource(LevelScript {
            director: Director::new(&MapScripts::default(), &[], Settings::default()),
            dispensers: vec![],
            hidden: Default::default(),
            held: vec![],
            calls: vec![],
            places: crate::characters::Places::new(vec![Place {
                name: "Script1".into(),
                class: "ScriptedTrigger".into(),
                tag: "ScriptedTrigger".into(),
                location: [1000.0, 0.0, 0.0],
                yaw: 0.0,
            }]),
            sounds: vec![],
            ammo: vec![],
            hud_blink: Default::default(),
            anchors: vec![],
            started: true,
            scene: crate::cutscene::Scene::default(),
            stats: Default::default(),
            breaking: Default::default(),
            props: Default::default(),
        });
        w
    }

    fn ask(w: &mut World, who: &str, vals: &[(&str, Val)]) {
        let r = request(who, 0, &action(vals), 1.0, None);
        w.resource_mut::<LevelScript>().sounds.push(r);
        w.run_system_cached(play).unwrap();
    }

    fn playing(w: &mut World) -> Vec<String> {
        let mut q = w.query::<&ScriptSound>();
        let mut v: Vec<String> = q.iter(w).map(|s| s.name.clone()).collect();
        v.sort();
        v
    }

    /// A speaker's new line stops its last one; another speaker, a bang,
    /// or `bDontOverrideOtherSounds` doesn't; an unknown sound is counted.
    #[test]
    fn one_line_at_a_time_per_speaker() {
        let mut w = world();
        ask(&mut w, "Script1", &[("Sound", t("line1"))]);
        ask(&mut w, "Script1", &[("Sound", t("bang"))]);
        assert_eq!(playing(&mut w), ["bang", "line1"]);
        ask(&mut w, "Script1", &[("Sound", t("line2"))]);
        assert_eq!(playing(&mut w), ["bang", "line2"], "line1 is cut off");
        ask(
            &mut w,
            "Script2",
            &[("Sound", t("line1")), ("SoundSourceTag", t("Sev"))],
        );
        ask(
            &mut w,
            "Script1",
            &[
                ("Sound", t("line1")),
                ("bDontOverrideOtherSounds", Val::Bool(true)),
            ],
        );
        assert_eq!(playing(&mut w), ["bang", "line1", "line1", "line2"]);
        ask(&mut w, "Script1", &[("Sound", t("nothing"))]);
        let st = &w.resource::<LevelScript>().stats;
        assert_eq!(st.sounds_played.len(), 5);
        assert_eq!(st.sounds_missing, ["nothing"]);
    }

    /// The action's own `Volume` scales the setting, and the speaker is
    /// the `SoundSourceTag`, or the script.
    #[test]
    fn request_reads_the_action() {
        let r = request(
            "ScriptedTrigger52",
            13,
            &action(&[
                ("Sound", t("CA1G1701")),
                ("Volume", Val::Float(0.5)),
                ("SoundSourceTag", t("None")),
            ]),
            0.8,
            None,
        );
        assert_eq!(r.name, "CA1G1701");
        assert_eq!(r.source, "ScriptedTrigger52");
        assert!((r.volume - 0.4).abs() < 1e-6);
        assert!(r.cut_off);
        assert_eq!(r.by, "ScriptedTrigger52 #13");
        assert!(!r.tagged && !r.local);
        assert_eq!(r.reach, 1.0);
        let r = request(
            "S",
            0,
            &action(&[("Sound", t("x")), ("Volume", Val::Float(8.0))]),
            1.0,
            None,
        );
        assert_eq!(
            (r.volume, r.reach),
            (1.0, 8.0),
            "louder than 1 carries further"
        );
    }

    /// Full loudness out to the radius, then radius over distance; no
    /// fading for sounds that don't fall off.
    #[test]
    fn loudness_with_distance() {
        assert_eq!(loudness(300.0, 600.0, true), 1.0);
        assert_eq!(loudness(1200.0, 600.0, true), 0.5);
        assert_eq!(loudness(6000.0, 600.0, false), 1.0);
        assert!(loudness(1e9, 600.0, true) > 0.0);
    }

    fn placed(w: &mut World, name: &str) -> Option<(Placed, Vec3)> {
        let mut q = w.query::<(&ScriptSound, Option<&Placed>, Option<&SoundAt>)>();
        q.iter(w)
            .filter(|(s, _, _)| s.name == name)
            .last()
            .map(|(_, p, a)| (p.cloned(), a.map(|a| a.0)))
            .and_then(|(p, a)| Some((p?, a?)))
    }

    /// Where each sound is heard from: radio everywhere, a placed line at
    /// the script's spot, a tagged one at its character, a local sound
    /// everywhere, a tag nothing has everywhere.
    #[test]
    fn sounds_come_from_their_place() {
        let mut w = world();
        // The player's eyes 2000 uu from the script (Bevy units: /50).
        w.spawn((
            PlayerCamera,
            GlobalTransform::from_translation(to_bevy(rc_movement::Vec3::new(3000.0, 0.0, 0.0))),
        ));
        let sev = w
            .spawn((
                ScriptTags {
                    tag: Some("Sev".into()),
                    ..default()
                },
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 4.0)),
            ))
            .id();
        ask(&mut w, "Script1", &[("Sound", t("line1"))]);
        assert!(
            placed(&mut w, "line1").is_none(),
            "radio is heard everywhere"
        );
        ask(&mut w, "Script1", &[("Sound", t("line2"))]);
        let (p, at) = placed(&mut w, "line2").expect("line2 is placed");
        assert_eq!(to_unreal(at), rc_movement::Vec3::new(1000.0, 0.0, 0.0));
        assert_eq!(p.follow, None);
        assert!((p.gain - 0.3).abs() < 1e-3, "600 / 2000: {}", p.gain);
        ask(
            &mut w,
            "Script1",
            &[("Sound", t("bang")), ("SoundSourceTag", t("Sev"))],
        );
        let (p, at) = placed(&mut w, "bang").expect("bang is placed");
        assert_eq!((p.follow, at), (Some(sev), Vec3::new(0.0, 0.0, 4.0)));
        assert_eq!(p.gain, 1.0, "bang doesn't fade");
        let mut local = action(&[("Sound", t("line2"))]);
        local.class = "ACTION_PlayLocalSound".into();
        let r = request("Script1", 0, &local, 1.0, None);
        w.resource_mut::<LevelScript>().sounds.push(r);
        w.run_system_cached(play).unwrap();
        ask(
            &mut w,
            "Script1",
            &[("Sound", t("line2")), ("SoundSourceTag", t("Nobody"))],
        );
        ask(
            &mut w,
            "Script1",
            &[
                ("Sound", t("line2")),
                ("SoundSourceTag", t("PlayerCommando")),
            ],
        );
        let st = &w.resource::<LevelScript>().stats;
        assert_eq!(st.sounds_played.len(), 6);
        assert_eq!(st.sounds_placed, ["line2", "bang"]);
    }
}
