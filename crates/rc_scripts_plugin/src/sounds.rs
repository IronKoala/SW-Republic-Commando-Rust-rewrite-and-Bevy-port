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
//! - sounds are not placed in the world yet: every one plays at full
//!   loudness wherever the speaker is (the speaker is logged);
//! - a group (`SoundMultiple`) waits for its longest variant.

use bevy::audio::{AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_combat::GameSound;
use rc_game_core::{GameLog, GamePaths, Play};
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
            out.sounds.insert(name.to_ascii_lowercase(), sound);
        }
    }
    for (package, names) in &streamed {
        let n = read_streamed_sounds(paths, package, names, audio, &mut out);
        *out.packages.entry(package.clone()).or_default() += n;
    }
    let total: f32 = out.lengths.values().sum();
    log.line(format!(
        "{PART}: {} script sounds read ({:.0} s of sound) from {:?}, {} missing, {:.0} ms",
        out.sounds.len(),
        total,
        out.packages,
        out.missing.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    for m in &out.missing {
        log.line(format!("{PART}:   sound {m}"));
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
                out.sounds
                    .insert(name.to_ascii_lowercase(), GameSound { variants: vec![h] });
                n += 1;
            }
            Err(e) => out
                .missing
                .push(format!("{name}: can't read {} ({e})", ogg.display())),
        }
    }
    n
}

/// What a sound action asks for.
pub(crate) fn request(who: &str, index: usize, a: &ScriptObject, volume: f32) -> SoundRequest {
    let source = a
        .text("SoundSourceTag")
        .filter(|s| !s.eq_ignore_ascii_case("None"))
        .unwrap_or(who)
        .to_string();
    SoundRequest {
        name: a.text("Sound").unwrap_or("").to_string(),
        source,
        volume: (volume * a.float("Volume").unwrap_or(1.0)).clamp(0.0, 1.0),
        cut_off: !a.flag("bDontOverrideOtherSounds", false),
        by: format!("{who} #{index}"),
    }
}

/// Play what the scripts asked for this frame.
pub(crate) fn play(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    sounds: Option<Res<ScriptSounds>>,
    playing: Query<(Entity, &ScriptSound)>,
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
        log.line(format!(
            "{PART}: t={now:.2} {}: {} {} {} ({secs}, volume {:.2}){}",
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
        let tag = ScriptSound {
            name: r.name,
            source: r.source,
            speech,
        };
        let e = commands
            .spawn((
                tag.clone(),
                AudioPlayer(h),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(r.volume)),
            ))
            .id();
        on.push((e, tag));
    }
}

/// Log when a script sound really starts on the sound card.
pub(crate) fn started(
    new: Query<&ScriptSound, Added<AudioSink>>,
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

/// Lines stop while the game is paused, and go on after.
pub(crate) fn pause(
    play: Option<Res<State<Play>>>,
    sinks: Query<&AudioSink, With<ScriptSound>>,
    mut was_paused: Local<bool>,
) {
    let paused = play.is_some_and(|p| *p.get() == Play::Paused);
    if paused == *was_paused {
        return;
    }
    *was_paused = paused;
    for s in &sinks {
        if paused {
            s.pause();
        } else {
            s.play();
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
    use rc_level::scripts::{Val, Vals};
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
            sounds
                .sounds
                .insert(name.into(), GameSound { variants: vec![h] });
        }
        sounds.speech = HashSet::from(["line1".to_string(), "line2".to_string()]);
        w.insert_resource(audio);
        w.insert_resource(sounds);
        w.insert_resource(LevelScript {
            director: Director::new(&MapScripts::default(), &[], Settings::default()),
            dispensers: vec![],
            hidden: Default::default(),
            held: vec![],
            sounds: vec![],
            started: true,
            stats: Default::default(),
        });
        w
    }

    fn ask(w: &mut World, who: &str, vals: &[(&str, Val)]) {
        let r = request(who, 0, &action(vals), 1.0);
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
        );
        assert_eq!(r.name, "CA1G1701");
        assert_eq!(r.source, "ScriptedTrigger52");
        assert!((r.volume - 0.4).abs() < 1e-6);
        assert!(r.cut_off);
        assert_eq!(r.by, "ScriptedTrigger52 #13");
    }
}
