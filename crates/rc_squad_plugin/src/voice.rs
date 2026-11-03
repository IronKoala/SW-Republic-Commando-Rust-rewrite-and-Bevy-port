//! The squad's voices: Sev, Fixer, Scorch and the player say the game's
//! own lines when things happen to the squad.
//!
//! - The rules for *when* (our own) are `data/squad/voice.toml`, run by
//!   `rc_squad::Chatter`.
//! - The lines are the game's: each member's `audio_table` (squad.toml)
//!   and the player's `leader_table` (voice.toml) are read from
//!   `System/CTAudio.u`, event names from `System/Engine.u`, and the
//!   sounds from `Sounds/character_voice.uax`, all at run time. Nothing is
//!   copied into the project.
//! - One squad line at a time (our choice); a line is a `Voice` sound
//!   (VOICES VOLUME), at full volume like a helmet radio, or placed at
//!   the speaker (`SoundAt`) with `[voice] positional = "yes"`.
//! - Everything said or skipped goes to the log (`squad: voice ...`).

use crate::{log_line, SquadMate, SquadState};
use bevy::prelude::*;
use rc_combat::GameSound;
use rc_game_core::units::to_unreal;
use rc_game_core::{GameLog, GamePaths, Player, PlayerCamera};
use rc_squad::{Chatter, Heard, SquadEvent, VoiceSettings};
use std::collections::HashMap;
use std::path::PathBuf;

/// The voice rules file, under `data/` (or a mod's folder).
pub const VOICE_FILE: &str = "squad/voice.toml";

/// This frame's squad events, gathered from every squad system for the
/// voices to hear.
#[derive(Resource, Default, Debug)]
pub struct SquadHeard(pub Vec<SquadEvent>);

/// One squad line playing.
#[derive(Component, Debug)]
pub struct SquadLine;

/// A line's length in seconds, from its WAV header (0 if unknown).
fn seconds(audio: &Assets<AudioSource>, h: &Handle<AudioSource>) -> f32 {
    audio
        .get(h)
        .and_then(|a| rc_audio::Wav::parse(&a.bytes).ok())
        .map_or(0.0, |w| w.seconds() as f32)
}

/// What the squad has said, for the log and tests.
#[derive(Resource, Default, Debug, Clone)]
pub struct SquadVoiceStats {
    /// (speaker, event, sound group), in the order said.
    pub said: Vec<(String, String, String)>,
    /// Lines a rule wanted but no table had a sound for.
    pub missing: u32,
}

/// The voice rules and every table's sounds, kept between levels.
#[derive(Resource, Default)]
pub struct SquadVoices {
    pub chatter: Option<Chatter>,
    /// By table class (lower case): event name (lower case) -> (group,
    /// sounds).
    tables: HashMap<String, HashMap<String, (String, GameSound)>>,
    tried: bool,
}

/// Read the rules, the tables and their sounds, once.
pub fn load_voices(
    mut voices: ResMut<SquadVoices>,
    state: Res<SquadState>,
    paths: Option<Res<GamePaths>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    log: Option<Res<GameLog>>,
) {
    let Some(squad) = &state.squad else {
        return;
    };
    if voices.tried {
        return;
    }
    voices.tried = true;
    let settings = read_settings(paths.as_deref(), &log);
    for l in settings.summary() {
        log_line(&log, l);
    }
    let (Some(paths), Some(mut audio)) = (paths, audio) else {
        log_line(
            &log,
            "voice: no game folder or no sound; the squad stays quiet",
        );
        voices.chatter = Some(Chatter::new(settings));
        return;
    };
    let events = match rc_combat::audio_table::read_audio_events(&paths) {
        Ok(e) => e,
        Err(e) => {
            log_line(
                &log,
                format!("voice: event names not read ({e}); the squad stays quiet"),
            );
            voices.chatter = Some(Chatter::new(settings));
            return;
        }
    };
    let mut classes: Vec<String> = squad
        .members
        .iter()
        .map(|m| m.settings.audio_table.clone())
        .filter(|t| !t.is_empty())
        .collect();
    classes.push(settings.leader_table.clone());
    for class in classes {
        let key = class.to_ascii_lowercase();
        if voices.tables.contains_key(&key) {
            continue;
        }
        let table = match rc_combat::audio_table::read_audio_table(&paths, &class, &events) {
            Ok(t) => t,
            Err(e) => {
                log_line(&log, format!("voice: table {class} not read: {e}"));
                voices.tables.insert(key, HashMap::new());
                continue;
            }
        };
        let groups: Vec<String> = table.iter().map(|(_, g)| g.clone()).collect();
        let (found, notes) = rc_combat::read_sounds(&paths, &settings.package, &groups, &mut audio);
        for n in notes {
            log_line(&log, format!("voice: {class}: {n}"));
        }
        let mut by_event = HashMap::new();
        for (event, group) in table {
            if let Some(s) = found.get(&group) {
                by_event.insert(event.to_ascii_lowercase(), (group, s.clone()));
            }
        }
        log_line(
            &log,
            format!(
                "voice: {class}: {} events with lines, {} sounds found in {}",
                groups.len(),
                by_event.len(),
                settings.package
            ),
        );
        voices.tables.insert(key, by_event);
    }
    // Rules naming an event no table has: most likely a typo.
    for l in &settings.lines {
        for e in &l.events {
            if e.contains('{') {
                continue;
            }
            let k = e.to_ascii_lowercase();
            if !events.iter().any(|n| n.eq_ignore_ascii_case(e)) {
                log_line(
                    &log,
                    format!("voice: WARNING rule {}: `{e}` is not a game event", l.key),
                );
            } else if !voices.tables.values().any(|t| t.contains_key(&k)) {
                log_line(
                    &log,
                    format!("voice: rule {}: nobody has a line for {e}", l.key),
                );
            }
        }
    }
    voices.chatter = Some(Chatter::new(settings));
}

fn read_settings(paths: Option<&GamePaths>, log: &Option<Res<GameLog>>) -> VoiceSettings {
    let file = match paths {
        Some(p) => p.data_file(VOICE_FILE),
        None => Some(PathBuf::from("data").join(VOICE_FILE)).filter(|p| p.exists()),
    };
    let Some(file) = file else {
        log_line(
            log,
            format!("voice: no {VOICE_FILE}; the squad stays quiet"),
        );
        return VoiceSettings::default();
    };
    match std::fs::read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|t| VoiceSettings::default().with_overrides(&t))
    {
        Ok(s) => {
            log_line(log, format!("voice: rules from {}", file.display()));
            s
        }
        Err(e) => {
            log_line(
                log,
                format!(
                    "voice: ERROR in {}: {e}; the squad stays quiet",
                    file.display()
                ),
            );
            VoiceSettings {
                enabled: false,
                ..VoiceSettings::default()
            }
        }
    }
}

/// Hear this frame's squad events and say what the rules pick.
#[allow(clippy::too_many_arguments)]
pub fn speak(
    mut voices: ResMut<SquadVoices>,
    mut heard: ResMut<SquadHeard>,
    state: Res<SquadState>,
    audio: Option<Res<Assets<AudioSource>>>,
    mates: Query<(&SquadMate, &Transform)>,
    player: Query<&Transform, With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut stats: ResMut<SquadVoiceStats>,
    mut commands: Commands,
    log: Option<Res<GameLog>>,
) {
    let events = std::mem::take(&mut heard.0);
    let Some(squad) = &state.squad else {
        return;
    };
    let voices = &mut *voices;
    let Some(chatter) = voices.chatter.as_mut() else {
        return;
    };
    // Who is talking is kept by `Chatter` from each line's length, so it
    // works the same with no sound device (tests).
    let busy = false;
    let listener = camera.single().ok().map(|c| to_unreal(c.translation()));
    let full = chatter.settings.full_volume_within.max(1.0);
    for h in chatter.hear(squad, &events, busy) {
        let s = match h {
            Heard::Quiet { rule, why } => {
                log_line(
                    &log,
                    format!("t={:.2} voice: {rule} quiet ({why})", squad.time),
                );
                continue;
            }
            Heard::Say(s) => s,
        };
        let (who, table, at) = match s.speaker {
            Some(i) => {
                let m = &squad.members[i];
                let at = mates
                    .iter()
                    .find(|(mm, _)| mm.index == i)
                    .map(|(_, t)| t.translation);
                (m.settings.name.clone(), m.settings.audio_table.clone(), at)
            }
            None => (
                "the player".to_string(),
                chatter.settings.leader_table.clone(),
                player.single().ok().map(|t| t.translation),
            ),
        };
        let lines = voices.tables.get(&table.to_ascii_lowercase());
        let found = s.events.iter().find_map(|e| {
            lines
                .and_then(|t| t.get(&e.to_ascii_lowercase()))
                .map(|(g, snd)| (e.clone(), g.clone(), snd.pick()))
        });
        let Some((event, group, Some(sound))) = found else {
            stats.missing += 1;
            log_line(
                &log,
                format!(
                    "t={:.2} voice: {who} has no line for {} (rule {})",
                    squad.time,
                    s.events.join(" / "),
                    s.rule
                ),
            );
            continue;
        };
        // Full volume like a helmet radio (our choice), unless the file
        // asks for positional voices: then quieter with distance, like the
        // enemies'. The player's own lines are always full volume.
        let positional = chatter.settings.positional;
        let volume = match (s.speaker, at, listener) {
            (Some(_), Some(at), Some(l)) if positional => {
                let d = (to_unreal(at) - l).length();
                (full / d.max(full)).max(0.25)
            }
            _ => 1.0,
        };
        let secs = audio.as_deref().map_or(0.0, |a| seconds(a, &sound));
        chatter.speaking(squad.time, secs);
        let mut e = commands.spawn((
            AudioPlayer(sound),
            bevy::audio::PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(volume)),
            rc_game_core::Voice,
            SquadLine,
        ));
        if let (Some(_), Some(at), true) = (s.speaker, at, positional) {
            e.insert(rc_game_core::SoundAt::place(at));
        }
        log_line(
            &log,
            format!(
                "t={:.2} voice: {who} says {event} ({group}, {secs:.1} s) [rule {}]",
                squad.time, s.rule
            ),
        );
        stats.said.push((who, event, group));
    }
}
