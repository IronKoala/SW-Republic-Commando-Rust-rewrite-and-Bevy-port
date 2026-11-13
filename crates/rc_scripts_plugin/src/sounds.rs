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
//! the decompiled code stays on the PC and is never committed). When a
//! wall of the level stands between the sound and the listener, the reach
//! shrinks to `Radius × 0.85^(rooms apart²) × 0.35`, eased frame by frame
//! (see [`Reach`] and `rc_level::hearing`, read from the same code,
//! 2026-11-13). One part is still **our reading**: an action `Volume`
//! above 1 (a few maps use 2 to 10000) makes it carry that many times
//! further instead of louder.
//! `bDisable3DFalloff` sounds come from their side but don't fade. Which
//! side a sound is on is the 3D SOUND option's job (`rc_game_core::SoundAt`).

use bevy::audio::{
    AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink, Volume,
};
use bevy::prelude::*;
use rc_combat::{AudioParams, GameSound};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, GamePaths, Play, PlayerCamera, ScriptTags, SoundAt, SoundBy};
use rc_level::scripts::{MapScripts, ScriptObject};
use rc_package::sound::read_streamed;
use rc_package::Package;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use crate::{LevelScript, PART};
use rc_level::hearing::Hearing;
use rc_level_plugin::LevelHearing;
use rc_movement::Vec3 as UVec3;

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
    /// `PlayOwnedSound` without `bDontOverrideOtherSounds`: the line may
    /// be cut for a newer one from the same speaker (see `crate::engine`).
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
}

/// A sound with a place in the level: how loud it is with distance, and
/// what the level's walls and rooms do to it (see [`Reach`]).
#[derive(Component, Debug, Clone)]
pub struct Placed {
    /// Unreal units: full loudness out to here (radius times reach). A
    /// `Radius` of 0 counts as 10, as in the game's `FAbstractSource::Init`.
    pub radius: f32,
    /// Quieter past `radius` (false for `bDisable3DFalloff`).
    pub falloff: bool,
    /// The character it comes from, followed as it moves.
    pub follow: Option<Entity>,
    /// The share of its loudness left at this distance (0 to 1), which the
    /// sound engine (`crate::engine`) multiplies in.
    pub gain: f32,
    /// Whether the level's walls muffle it: the game's `ULevel::IsAudibleAt`
    /// says "always heard" only for a mover's sound or an actor with
    /// `SoundOcclusion=OCCLUSION_None`.
    pub walls: bool,
    /// How far it carries now, after walls and rooms.
    pub reach: Reach,
}

impl Placed {
    /// A placed sound with reach `radius` (0 counts as 10).
    pub fn new(radius: f32, falloff: bool, follow: Option<Entity>) -> Self {
        let radius = if radius == 0.0 { 10.0 } else { radius };
        Placed {
            radius,
            falloff,
            follow,
            gain: 1.0,
            walls: true,
            reach: Reach::new(radius),
        }
    }

    /// The same, never muffled by walls.
    pub fn through_walls(mut self) -> Self {
        self.walls = false;
        self
    }

    /// Work out how loud it is now: the sound at `at`, the listener at
    /// `ear` (both Unreal units), `now` the game's clock in seconds. Keeps
    /// the result in `gain` and returns it.
    pub fn update(&mut self, hearing: Option<&Hearing>, at: UVec3, ear: UVec3, now: f64) -> f32 {
        let a = [at.x, at.y, at.z];
        let e = [ear.x, ear.y, ear.z];
        let target = match hearing {
            Some(h) if self.walls && !h.line_clear(a, e) => {
                self.radius * h.radius_multiplier(h.zone_at(a), h.zone_at(e)) * OCCLUDED
            }
            _ => self.radius,
        };
        self.reach.step(target, now);
        let gain = loudness((at - ear).length(), self.reach.applied, self.falloff);
        self.gain = gain;
        gain
    }
}

/// A sound's reach shrinks to this share when a wall is between
/// (`FAbstractSource::UpdateRadii`, Engine.dll).
pub const OCCLUDED: f32 = 0.35;

/// How far a placed sound carries right now, eased the way the game's
/// `FAbstractSource::UpdateRadii` (Engine.dll) eases it.
///
/// Each frame the game works out a target: the sound's `Radius` when no
/// wall is between, else `Radius` times `0.85 ^ (rooms apart squared)`
/// times 0.35. The reach then moves toward the target by
/// `(now - when the target last changed)` of the way, with the target's
/// change noted after the move. Until the target first changes, the reach
/// jumps straight to it. The game hands the reach to OpenAL as the
/// reference distance; OpenAL refuses a value below 0 and keeps the last
/// one, so `applied` does the same. These steps are copied as they are,
/// odd parts included: on a later change the first step can overshoot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// The reach the game is moving (`+0x7c`).
    pub current: f32,
    /// The last target (`+0x80`).
    pub target: f32,
    /// When the target last changed, in seconds (`+0x84`); -1 at first.
    pub changed_at: f64,
    /// What OpenAL was last given (never below 0).
    pub applied: f32,
}

impl Reach {
    /// A sound starting with reach `radius`: the game starts its reach at
    /// `radius`, or 1 when smaller.
    pub fn new(radius: f32) -> Self {
        let current = if radius >= 1.0 { radius } else { 1.0 };
        Reach {
            current,
            target: radius,
            changed_at: -1.0,
            applied: current,
        }
    }

    /// One frame toward `target` at time `now` (seconds).
    pub fn step(&mut self, target: f32, now: f64) {
        let since = now - self.changed_at;
        if self.changed_at > 0.0 || since < 1.0 {
            self.current = (since * (target - self.current) as f64 + self.current as f64) as f32;
        } else {
            self.current = target;
        }
        if self.target != target {
            self.target = target;
            self.changed_at = now;
        }
        if self.current >= 0.0 {
            self.applied = self.current;
        }
    }
}

/// How loud a placed sound is at `distance` (Unreal units), 0 to 1, with
/// reference distance `reach`: OpenAL's "inverse distance, clamped" with
/// a roll-off of 1, as the game sets it up. See the module notes.
pub fn loudness(distance: f32, reach: f32, falloff: bool) -> f32 {
    if !falloff || distance <= reach {
        1.0
    } else {
        reach / distance
    }
}

/// The clock the sound code eases reach by: the game uses its level time
/// (`GEngineTime`, seconds since the level started, stopped while paused);
/// here that is the level scripts' clock, or the game's own clock before
/// a level runs.
pub fn sound_clock(script: Option<&LevelScript>, time: &Time) -> f64 {
    script.map_or(time.elapsed_secs_f64(), |s| s.director.time as f64)
}


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
                register_streamed(paths, &pkg, &data, package, export, name, &h);
                out.sounds.insert(
                    name.to_ascii_lowercase(),
                    GameSound {
                        variants: vec![h],
                        ..Default::default()
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

/// Tell the sound engine about a streamed sound: its own `Params` (the
/// music stingers name `Params_SFX` objects or none), and that it streams.
fn register_streamed(
    paths: &GamePaths,
    pkg: &Package,
    data: &[u8],
    package: &str,
    export: &rc_package::Export,
    name: &str,
    h: &Handle<AudioSource>,
) {
    use rc_combat::sound_meta;
    let Some(i) = pkg.exports.iter().position(|e| std::ptr::eq(e, export)) else {
        return;
    };
    let props = sound_meta::export_properties(pkg, data, i).unwrap_or_default();
    let params = rc_package::sound::params_of(pkg, &props, package);
    let facts = sound_meta::resolve_params(paths, params.as_deref());
    let mut meta = sound_meta::meta_of(pkg, &props, name, params, &facts, 0.0);
    meta.streaming = true;
    sound_meta::register(h.id(), meta);
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
    // The action's own `Volume` is passed to `PlayOwnedSound`, which
    // doesn't use it (`AActor::execPlayOwnedSound`, Engine.dll).
    SoundRequest {
        name: a.text("Sound").unwrap_or("").to_string(),
        source: tag.unwrap_or(who).to_string(),
        volume: volume.clamp(0.0, 1.0),
        cut_off: !a.flag("bDontOverrideOtherSounds", false),
        by: format!("{who} #{index}"),
        local: a.class.to_ascii_lowercase().ends_with("playlocalsound"),
        tagged: tag.is_some(),
        owner: owner.filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case("None")),
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
    /// The `SoundSourceTag` names nothing: `ACTION_PlaySound` logs
    /// "non-existant SoundSourceTag, not playing" and plays nothing.
    Nowhere,
}

/// Work out where a requested sound comes from (see the module notes).
fn source_of(
    r: &SoundRequest,
    non_positional: bool,
    script: &LevelScript,
    chars: &Query<(Entity, &ScriptTags, &GlobalTransform)>,
) -> Source {
    if r.local {
        return Source::Everywhere("a local sound".into());
    }
    // `ACTION_PlaySound` looks the tag up first, and plays nothing when
    // nothing has it, whatever the sound's own `AudioParams` say.
    let tag_found = !r.tagged
        || script.director.settings.is_player(&r.source)
        || script.places.find(&r.source).is_some()
        || chars.iter().any(|(_, t, _)| {
            t.tag
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(&r.source))
        });
    if !tag_found {
        return Source::Nowhere;
    }
    if non_positional {
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
        None if r.tagged => Source::Nowhere,
        None => Source::Everywhere(format!("nothing in the level is called {}", r.source)),
    }
}

/// Play what the scripts asked for this frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    sounds: Option<Res<ScriptSounds>>,
    chars: Query<(Entity, &ScriptTags, &GlobalTransform)>,
    dead: Query<(), With<rc_combat::Dead>>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    hearing: Option<Res<LevelHearing>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    if script.sounds.is_empty() {
        return;
    }
    let now = script.director.time;
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
        let secs = sounds
            .as_ref()
            .and_then(|s| s.lengths.get(&key))
            .map(|l| format!("{l:.1} s"))
            .unwrap_or_else(|| "length unknown".into());
        // Where it comes from: the picked sound's own `AudioParams` (the
        // group's when it wasn't read through `rc_combat::read_sounds`).
        let params = sounds.as_ref().and_then(|s| s.params.get(&key));
        let leaf = rc_combat::sound_meta::meta(h.id());
        let non_positional = leaf.as_ref().map_or_else(
            || params.is_some_and(|p| p.non_positional),
            |m| m.non_positional,
        );
        let from = source_of(&r, non_positional, &script, &chars);
        let ear = ears.iter().next().map(|g| g.translation());
        let radius = leaf.as_ref().map_or_else(
            || {
                params
                    .and_then(|p| p.radius)
                    .unwrap_or(rc_combat::sound_meta::SoundMeta::default().radius)
            },
            |m| m.radius,
        );
        let falloff = !leaf
            .as_ref()
            .map_or_else(|| params.is_some_and(|p| p.no_falloff), |m| m.no_falloff);
        let (place, follow) = match &from {
            Source::Everywhere(_) | Source::Nowhere => (None, None),
            Source::Follow(e, at) => (Some(*at), Some(*e)),
            Source::At(at) => (Some(*at), None),
        };
        if matches!(from, Source::Nowhere) {
            log.line(format!(
                "{PART}: t={now:.2} {}: {} {}: non-existant SoundSourceTag, not playing (as the game's ACTION_PlaySound)",
                r.by, r.source, r.name
            ));
            continue;
        }
        // `ACTION_PlaySound` plays nothing from a dead character.
        if follow.is_some_and(|e| dead.contains(e)) {
            log.line(format!(
                "{PART}: t={now:.2} {}: {} is dead; {} not played (as the game's ACTION_PlaySound)",
                r.by, r.source, r.name
            ));
            continue;
        }
        let distance = match (place, ear) {
            (Some(p), Some(e)) => Some(to_unreal(p - e).length()),
            _ => None,
        };
        let mut placed = place.map(|_| Placed::new(radius, falloff, follow));
        let radius = placed.as_ref().map_or(radius, |p| p.radius);
        let gain = match (&mut placed, place, ear) {
            (Some(p), Some(at), Some(e)) => p.update(
                hearing.as_ref().map(|h| &*h.0),
                to_unreal(at),
                to_unreal(e),
                now as f64,
            ),
            _ => 1.0,
        };
        let heard = match (&from, distance) {
            (Source::Everywhere(why), _) => format!("heard everywhere: {why}"),
            (Source::Nowhere, _) => String::new(),
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
            "{PART}: t={now:.2} {}: {} {} {} ({secs}, volume {:.2}; {heard})",
            r.by,
            r.source,
            if speech { "says" } else { "plays" },
            r.name,
            r.volume,
        ));
        script.stats.sounds_played.push(r.name.clone());
        if place.is_some() {
            script.stats.sounds_placed.push(r.name.clone());
        }
        // Who plays it, for the sound engine: `PlayOwnedSound` from the
        // speaker (an actor's number; a script's own spot has no entity,
        // so its name stands in for it).
        let by = SoundBy {
            actor: follow,
            actor_index: follow.is_none().then(|| speaker_number(&r.source)),
            owned: r.cut_off,
            everywhere: matches!(from, Source::Everywhere(_)),
            ..default()
        };
        let tag = ScriptSound {
            name: r.name,
            source: r.source,
            speech,
        };
        let mut spawned = commands.spawn((
            tag,
            by,
            AudioPlayer(h),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(r.volume)),
        ));
        if let (Some(at), Some(placed)) = (place, placed) {
            spawned.insert((SoundAt::place(at), placed));
        }
        // Speech follows the VOICES volume (rc_game_core::Voice).
        if speech {
            spawned.insert(rc_game_core::Voice);
        }
    }
}

/// A number standing for a speaker with no entity (a script's spot), the
/// same for the same name.
fn speaker_number(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.to_ascii_lowercase().bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    h | 0x8000_0000
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
    sinks: Query<
        &AudioSink,
        Or<(
            With<ScriptSound>,
            With<crate::ambient::AmbientLoop>,
            With<crate::anim_sounds::AnimSoundPlaying>,
        )>,
    >,
    spatial: Query<
        &SpatialAudioSink,
        Or<(
            With<ScriptSound>,
            With<crate::ambient::AmbientLoop>,
            With<crate::anim_sounds::AnimSoundPlaying>,
        )>,
    >,
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
/// quieter as the player comes nearer or goes away, or a wall comes
/// between (see [`Placed::update`]).
#[allow(clippy::type_complexity)]
pub(crate) fn follow(
    mut placed: Query<(&mut Placed, &mut Transform, &mut SoundAt)>,
    bodies: Query<&GlobalTransform, Without<Placed>>,
    ears: Query<&GlobalTransform, (With<PlayerCamera>, Without<Placed>)>,
    hearing: Option<Res<LevelHearing>>,
    script: Option<Res<LevelScript>>,
    time: Res<Time>,
) {
    let ear = ears.iter().next().map(|g| to_unreal(g.translation()));
    let hearing = hearing.as_ref().map(|h| &*h.0);
    let now = sound_clock(script.as_deref(), &time);
    for (mut p, mut t, mut at) in &mut placed {
        if let Some(g) = p.follow.and_then(|e| bodies.get(e).ok()) {
            t.translation = g.translation();
            at.0 = g.translation();
        }
        let Some(ear) = ear else { continue };
        p.update(hearing, to_unreal(t.translation), ear, now);
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
                    ..Default::default()
                },
            );
        }
        sounds.speech = HashSet::from(["line1".to_string(), "line2".to_string()]);
        // line1 is radio, line2 carries 600 uu, bang doesn't fade.
        let params = |radius, non_positional, no_falloff| AudioParams {
            radius,
            non_positional,
            no_falloff,
            ..Default::default()
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
            shakes: Default::default(),
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

    /// Every line is handed to the sound engine, which decides what a
    /// speaker's new line does to its last one (`FindSource`): a speaker's
    /// lines share a number, and `bDontOverrideOtherSounds` makes a line
    /// not "owned". A tag nothing has plays nothing; an unknown sound is
    /// counted.
    #[test]
    fn lines_go_to_the_engine() {
        let mut w = world();
        ask(&mut w, "Script1", &[("Sound", t("line1"))]);
        ask(&mut w, "Script1", &[("Sound", t("bang"))]);
        ask(&mut w, "Script1", &[("Sound", t("line2"))]);
        assert_eq!(playing(&mut w), ["bang", "line1", "line2"]);
        ask(
            &mut w,
            "Script2",
            &[("Sound", t("line1")), ("SoundSourceTag", t("Sev"))],
        );
        assert_eq!(
            playing(&mut w),
            ["bang", "line1", "line2"],
            "no Sev in the level"
        );
        ask(
            &mut w,
            "Script1",
            &[
                ("Sound", t("line1")),
                ("bDontOverrideOtherSounds", Val::Bool(true)),
            ],
        );
        let mut q = w.query::<(&ScriptSound, &SoundBy)>();
        let by: Vec<SoundBy> = q.iter(&w).map(|(_, b)| *b).collect();
        assert_eq!(by.len(), 4);
        let number = Some(speaker_number("Script1"));
        assert!(by.iter().all(|b| b.actor_index == number));
        assert_eq!(by.iter().filter(|b| b.owned).count(), 3);
        ask(&mut w, "Script1", &[("Sound", t("nothing"))]);
        let st = &w.resource::<LevelScript>().stats;
        assert_eq!(st.sounds_played.len(), 4);
        assert_eq!(st.sounds_missing, ["nothing"]);
    }

    /// The action's own `Volume` is ignored (`PlayOwnedSound` doesn't use
    /// it), and the speaker is the `SoundSourceTag`, or the script.
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
        assert!((r.volume - 0.8).abs() < 1e-6);
        assert!(r.cut_off);
        assert_eq!(r.by, "ScriptedTrigger52 #13");
        assert!(!r.tagged && !r.local);
        let r = request(
            "S",
            0,
            &action(&[("Sound", t("x")), ("Volume", Val::Float(8.0))]),
            1.0,
            None,
        );
        assert_eq!(r.volume, 1.0);
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
    /// everywhere, a tag nothing has not at all.
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
        assert_eq!(st.sounds_played.len(), 5, "Nobody's line isn't played");
        assert_eq!(st.sounds_placed, ["line2", "bang"]);
    }
}
