//! Music: the game's own Ogg Vorbis tracks from `GameData/Music`, picked by
//! `data/music/music.toml`. The menu has its track; each level plays its
//! campaign's calm list, or its battle list while `MusicMood` is `Battle`.

use crate::settings::Settings;
use bevy::audio::{AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{CurrentLevel, GameLog, GamePaths, GameState, MusicMood};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Resource, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MusicPlan {
    #[serde(default)]
    pub menu: Vec<String>,
    #[serde(default)]
    pub level: Vec<LevelMusic>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LevelMusic {
    pub prefix: String,
    #[serde(default)]
    pub calm: Vec<String>,
    #[serde(default)]
    pub battle: Vec<String>,
}

impl MusicPlan {
    pub fn parse(text: &str) -> Result<MusicPlan, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The list to play, and a name for it (to notice when it changes).
    pub fn pick(
        &self,
        state: GameState,
        map: Option<&str>,
        mood: MusicMood,
    ) -> (String, Vec<String>) {
        match (state, map) {
            (GameState::InGame, Some(map)) => {
                let m = map.to_ascii_lowercase();
                match self
                    .level
                    .iter()
                    .find(|l| m.starts_with(&l.prefix.to_ascii_lowercase()))
                {
                    Some(l) => {
                        let battle = mood == MusicMood::Battle && !l.battle.is_empty();
                        let (kind, list) = if battle {
                            ("battle", &l.battle)
                        } else {
                            ("calm", &l.calm)
                        };
                        (format!("{} {kind}", l.prefix), list.clone())
                    }
                    None => ("none".into(), vec![]),
                }
            }
            // The menu track keeps playing through the loading screen.
            _ => ("menu".into(), self.menu.clone()),
        }
    }
}

#[derive(Component)]
struct Track;

#[derive(Resource, Default)]
struct NowPlaying {
    key: String,
    list: Vec<String>,
    next: usize,
    entity: Option<Entity>,
    cache: HashMap<String, Option<Handle<AudioSource>>>,
}

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NowPlaying>()
            .add_systems(Update, (choose_music, set_volume, watch_started))
            .add_systems(
                PostUpdate,
                effects_volume.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// Every other sound (weapons, droids, menu clicks) is turned down by the
/// master and SOUND FX volumes as it starts. The sound parts play at their
/// own loudness; this scales it before Bevy starts the sound (Bevy's audio
/// runs after the transform update in `PostUpdate`).
fn effects_volume(
    settings: Res<Settings>,
    mut new: Query<&mut PlaybackSettings, (Added<PlaybackSettings>, Without<Track>)>,
) {
    let scale = (settings.audio.master_volume * settings.audio.effects_volume).clamp(0.0, 1.0);
    if scale >= 1.0 {
        return;
    }
    for mut p in &mut new {
        p.volume = Volume::Linear(p.volume.to_linear() * scale);
    }
}

fn volume(settings: &Settings) -> Volume {
    Volume::Linear((settings.audio.master_volume * settings.audio.music_volume).clamp(0.0, 1.0))
}

#[allow(clippy::too_many_arguments)]
fn choose_music(
    mut commands: Commands,
    plan: Res<MusicPlan>,
    state: Res<State<GameState>>,
    level: Res<CurrentLevel>,
    mood: Res<MusicMood>,
    paths: Res<GamePaths>,
    settings: Res<Settings>,
    mut now: ResMut<NowPlaying>,
    mut sources: ResMut<Assets<AudioSource>>,
    tracks: Query<(), With<Track>>,
    log: Res<GameLog>,
) {
    let (key, list) = plan.pick(*state.get(), level.map.as_deref(), *mood);
    if key != now.key {
        if let Some(e) = now.entity.take() {
            commands.entity(e).try_despawn();
        }
        log.line(format!(
            "music: switching to '{key}' ({} tracks)",
            list.len()
        ));
        now.key = key;
        now.list = list;
        now.next = 0;
    }
    // Start the next track when nothing is playing (the last one ended and
    // despawned itself, or the list just changed).
    if now.entity.is_some_and(|e| tracks.get(e).is_ok()) || now.list.is_empty() {
        return;
    }
    // Try each track once, so a missing file can't stall the list.
    for _ in 0..now.list.len() {
        let i = now.next % now.list.len();
        now.next = i + 1;
        let name = now.list[i].clone();
        let handle = now
            .cache
            .entry(name.clone())
            .or_insert_with(|| {
                let path = paths.game_file(&format!("GameData/Music/{name}.ogg"));
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        log.line(format!(
                            "music: read {} ({} KB)",
                            path.display(),
                            bytes.len() / 1024
                        ));
                        Some(sources.add(AudioSource {
                            bytes: bytes.into(),
                        }))
                    }
                    Err(e) => {
                        log.line(format!("music: can't read {} ({e})", path.display()));
                        None
                    }
                }
            })
            .clone();
        if let Some(h) = handle {
            let e = commands
                .spawn((
                    Track,
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN.with_volume(volume(&settings)),
                ))
                .id();
            now.entity = Some(e);
            log.line(format!("music: playing {name}"));
            return;
        }
    }
    // Nothing in the list could be read: stop trying until it changes.
    now.list.clear();
}

fn set_volume(settings: Res<Settings>, mut sinks: Query<&mut AudioSink, With<Track>>) {
    if !settings.is_changed() {
        return;
    }
    for mut s in &mut sinks {
        s.set_volume(volume(&settings));
    }
}

/// Log when a track really starts on the sound card.
fn watch_started(
    started: Query<(), (With<Track>, Added<AudioSink>)>,
    now: Res<NowPlaying>,
    log: Res<GameLog>,
) {
    if !started.is_empty() {
        let name = now
            .list
            .get(now.next.wrapping_sub(1) % now.list.len().max(1))
            .cloned()
            .unwrap_or_default();
        log.line(format!("music: {name} started on the sound card"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_plan_picks_by_map() {
        let p = MusicPlan::parse(include_str!("../../../data/music/music.toml")).unwrap();
        let (k, l) = p.pick(GameState::MainMenu, None, MusicMood::Calm);
        assert_eq!((k.as_str(), l[0].as_str()), ("menu", "musui_mainmenu_lp"));
        let (_, l) = p.pick(GameState::InGame, Some("RAS_02A"), MusicMood::Calm);
        assert_eq!(l[0], "musras_suspensestate01_lp");
        let (k, _) = p.pick(GameState::InGame, Some("geo_01a"), MusicMood::Battle);
        assert_eq!(k, "geo_ battle");
        let (_, l) = p.pick(GameState::InGame, Some("dm_canyon"), MusicMood::Calm);
        assert_eq!(l[0], "musgeo_suspensestate01_lp");
    }
}
