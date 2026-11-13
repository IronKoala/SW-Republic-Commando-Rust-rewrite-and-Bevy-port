//! Music: the game's own tracks, played by the game's own rules.
//!
//! Everything about the music comes from the install at run time:
//! - which pieces exist, their files, fades, loops and loudness: the
//!   `GameData/Sounds/*_music.uax` packages (`rc_music::MusicLibrary`);
//! - room music: the map's volumes and their `AudioEnvironment` classes in
//!   `GameData/Properties/audioenv.u`;
//! - battle music and when fights start: the map's `LevelInfo`, changed by
//!   its scripts;
//! - scripted music: the map's `ACTION_SetImuseState`, `ACTION_PlayMusic`
//!   and battle actions, passed on by the level scripts as `MusicCue`s;
//! - menu music: each menu class's `BackgroundMusic` in
//!   `XInterfaceCTMenus.u` (the main menu plays the sung "Vode An",
//!   `musUI_mainmenuVOX_lp`; the loading screen and pause menu have none).
//!
//! `rc_music::Imuse` decides what plays; this file plays it with Bevy's
//! audio, does the fades, pauses the level's music while a menu is open,
//! and logs every change to `logs/rc_game.log` ("music: ...").
//!
//! `data/music/music.toml` holds only our own choices where the game's
//! data says nothing (see there).

use crate::settings::Settings;
use bevy::audio::{AudioSinkPlayback, AudioSource, PlaybackMode, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::units::to_unreal;
use rc_game_core::{
    CurrentLevel, GameLog, GamePaths, GameState, MusicCue, MusicFades, MusicThreats, Play, Player,
    PlayerVitals,
};
use rc_level::music::AudioVolume;
use rc_music::{
    BattleSettings, Command, Criteria, Fade, FadeOverride, FadeShape, Imuse, Layer, MusicLibrary,
    Piece, RoomMusic, Threat, Tuning,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// `data/music/music.toml`: our own choices (the game's data has none).
#[derive(Resource, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, default)]
pub struct MusicPlan {
    /// Seconds between checks for a fight starting or ending.
    pub battle_check_seconds: f32,
    /// A fight's music ends this many seconds after the last enemy left
    /// the outer radius.
    pub battle_end_seconds: f32,
    /// Fade (seconds) for room overlays cut short by a fight or a script.
    pub overlay_stop_fade: f32,
    /// The "player down" loop when the level's battle music names none.
    pub player_down: String,
    /// Replace the main menu's music with another piece from the music
    /// packages (a mod's choice). Empty: the original's.
    pub menu: String,
}

impl Default for MusicPlan {
    fn default() -> Self {
        let t = Tuning::default();
        MusicPlan {
            battle_check_seconds: t.battle_check_seconds,
            battle_end_seconds: t.battle_end_seconds,
            overlay_stop_fade: t.overlay_stop_fade,
            player_down: t.player_down,
            menu: String::new(),
        }
    }
}

impl MusicPlan {
    pub fn parse(text: &str) -> Result<MusicPlan, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    fn tuning(&self) -> Tuning {
        Tuning {
            battle_check_seconds: self.battle_check_seconds.max(0.05),
            battle_end_seconds: self.battle_end_seconds.max(0.0),
            overlay_stop_fade: self.overlay_stop_fade.max(0.0),
            player_down: self.player_down.clone(),
        }
    }
}

/// A piece of music playing (never cut for other sounds).
#[derive(Component)]
pub(crate) struct Track {
    layer: Layer,
    name: String,
    /// Loudness from the piece's own settings.
    base: f32,
    fade: Fade,
    fading_in: bool,
    fading_out: bool,
    /// Seconds into the current fade.
    t: f32,
}

impl Track {
    fn gain(&self) -> f32 {
        let f = if self.fade.seconds <= 0.0 {
            1.0
        } else {
            (self.t / self.fade.seconds).clamp(0.0, 1.0)
        };
        let g = if self.fading_in {
            self.fade.shape.gain(f)
        } else if self.fading_out {
            self.fade.shape.gain(1.0 - f)
        } else {
            1.0
        };
        self.base * g
    }
}

/// The music library, read once.
#[derive(Resource, Default)]
struct Library(Option<Arc<MusicLibrary>>);

/// What the menus play, from their classes' `BackgroundMusic`.
#[derive(Resource, Default)]
struct MenuTracks {
    read: bool,
    main_menu: Option<String>,
    loading: Option<String>,
    pause: Option<String>,
    /// The credits page's own piece. The original's console swaps the
    /// music whenever a menu with other `BackgroundMusic` opens
    /// (`ManageMenuBackgroundMusic` in `Engine.u`), and starts the main
    /// menu's piece again when the credits close.
    credits: Option<String>,
}

/// One music layer's track entity, and whether it has been seen playing
/// (so its disappearing means the piece ended).
#[derive(Default, Clone, Copy)]
struct Current {
    entity: Option<Entity>,
    seen: bool,
}

#[derive(Resource, Default)]
struct Layers {
    current: HashMap<Layer, Current>,
    /// What the shell itself put on the main layer outside levels
    /// ("menu", "loading"), so it starts only once.
    shell: String,
    cache: HashMap<String, Option<Handle<AudioSource>>>,
}

/// The level's music director and room volumes. Exists from loading a map
/// until leaving it.
#[derive(Resource)]
struct LevelMusic {
    director: Imuse,
    rooms: Vec<(AudioVolume, RoomMusic)>,
    inside: Vec<bool>,
}

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Library>()
            .init_resource::<MenuTracks>()
            .init_resource::<Layers>()
            .init_resource::<MusicThreats>()
            .add_message::<MusicCue>()
            .add_systems(OnEnter(GameState::Loading), load_level_music)
            .add_systems(OnExit(GameState::InGame), leave_level)
            .add_systems(OnEnter(Play::Paused), pause_level_music)
            .add_systems(OnExit(Play::Paused), resume_level_music)
            .add_systems(
                Update,
                (
                    shell_music,
                    level_music.run_if(rc_game_core::playing),
                    fades,
                    watch_ends,
                    watch_started,
                )
                    .chain(),
            );
    }
}

fn library(lib: &mut Library, paths: &GamePaths, log: &GameLog) -> Arc<MusicLibrary> {
    lib.0
        .get_or_insert_with(|| {
            let l = MusicLibrary::load(&paths.game_data());
            let (pieces, groups) = l.counts();
            log.line(format!(
                "music: read {pieces} pieces, {groups} groups and {} rooms with music from the game's packages",
                l.rooms.len()
            ));
            for n in &l.notes {
                log.line(format!("music: {n}"));
            }
            Arc::new(l)
        })
        .clone()
}

/// A menu class's `BackgroundMusic`, by the piece's name (`None` for none).
fn menu_music(lib: &mut rc_menus::Library, class: &str, log: &GameLog) -> Option<String> {
    match lib.class(class) {
        Ok(c) => match c.get("BackgroundMusic") {
            Some(rc_menus::value::V::Object { path, .. }) if !path.is_empty() => {
                path.rsplit('.').next().map(str::to_string)
            }
            _ => None,
        },
        Err(e) => {
            log.line(format!("music: can't read {class} ({e}); no music there"));
            None
        }
    }
}

fn fade_of(seconds: f32, shape: u8) -> Fade {
    Fade {
        seconds,
        shape: FadeShape::from_byte(shape),
    }
}

fn fades_of(f: Option<MusicFades>) -> Option<FadeOverride> {
    f.map(|f| FadeOverride {
        fade_in: fade_of(f.fade_in, f.fade_in_shape),
        fade_out: fade_of(f.fade_out, f.fade_out_shape),
    })
}

/// Start a piece on a layer; what played there fades out.
#[allow(clippy::too_many_arguments)]
fn play(
    commands: &mut Commands,
    layers: &mut Layers,
    tracks: &mut Query<&mut Track>,
    sources: &mut Assets<AudioSource>,
    paths: &GamePaths,
    log: &GameLog,
    layer: Layer,
    piece: &Piece,
    volume: f32,
    fade_in: Fade,
    fade_out_old: Fade,
) -> bool {
    stop(commands, layers, tracks, layer, fade_out_old);
    let handle = layers
        .cache
        .entry(piece.file.to_ascii_lowercase())
        .or_insert_with(|| {
            let path = paths.game_file(&format!("GameData/Music/{}.ogg", piece.file));
            match std::fs::read(&path) {
                Ok(bytes) => Some(sources.add(AudioSource {
                    bytes: bytes.into(),
                })),
                Err(e) => {
                    log.line(format!("music: can't read {} ({e})", path.display()));
                    None
                }
            }
        })
        .clone();
    let Some(handle) = handle else {
        return false;
    };
    let track = Track {
        layer,
        name: piece.name.clone(),
        base: volume,
        fade: fade_in,
        fading_in: fade_in.seconds > 0.0,
        fading_out: false,
        t: 0.0,
    };
    let start = track.gain();
    let mode = if piece.looping {
        PlaybackMode::Loop
    } else {
        PlaybackMode::Despawn
    };
    let e = commands
        .spawn((
            track,
            rc_game_core::MusicTrack,
            AudioPlayer(handle),
            PlaybackSettings {
                mode,
                volume: Volume::Linear(start),
                ..PlaybackSettings::DESPAWN
            },
        ))
        .id();
    layers.current.insert(
        layer,
        Current {
            entity: Some(e),
            seen: false,
        },
    );
    true
}

/// Fade out what plays on a layer (at once for a zero fade).
fn stop(
    commands: &mut Commands,
    layers: &mut Layers,
    tracks: &mut Query<&mut Track>,
    layer: Layer,
    fade: Fade,
) {
    let Some(e) = layers.current.remove(&layer).and_then(|c| c.entity) else {
        return;
    };
    match tracks.get_mut(e) {
        Ok(mut t) if fade.seconds > 0.0 => {
            // Fade out from wherever a fade in had got to.
            let now = t.gain() / t.base.max(1e-4);
            t.fading_in = false;
            t.fading_out = true;
            t.fade = fade;
            // Start the fade-out curve at the current loudness.
            let mut k = 0.0;
            while k < 1.0 && fade.shape.gain(1.0 - k) > now {
                k += 0.02;
            }
            t.t = k * fade.seconds;
        }
        _ => {
            commands.entity(e).try_despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply(
    cmds: Vec<Command>,
    director: Option<&mut Imuse>,
    commands: &mut Commands,
    layers: &mut Layers,
    tracks: &mut Query<&mut Track>,
    sources: &mut Assets<AudioSource>,
    paths: &GamePaths,
    log: &GameLog,
) {
    let mut failed = Vec::new();
    for c in cmds {
        match c {
            Command::Play {
                layer,
                piece,
                volume,
                fade_in,
                fade_out_old,
            } => {
                if !play(
                    commands,
                    layers,
                    tracks,
                    sources,
                    paths,
                    log,
                    layer,
                    &piece,
                    volume,
                    fade_in,
                    fade_out_old,
                ) {
                    failed.push(layer);
                }
            }
            Command::Stop { layer, fade } => stop(commands, layers, tracks, layer, fade),
        }
    }
    if let Some(d) = director {
        for l in failed {
            d.finished(l);
        }
    }
}

/// Menu and loading-screen music, outside levels.
#[allow(clippy::too_many_arguments)]
fn shell_music(
    mut commands: Commands,
    state: Res<State<GameState>>,
    plan: Res<MusicPlan>,
    paths: Res<GamePaths>,
    mut lib: ResMut<Library>,
    mut menus: ResMut<MenuTracks>,
    menu: Res<crate::menu::Menu>,
    mut layers: ResMut<Layers>,
    mut tracks: Query<&mut Track>,
    mut sources: ResMut<Assets<AudioSource>>,
    log: Res<GameLog>,
) {
    let (key, name) = match state.get() {
        GameState::Boot => return,
        GameState::InGame => {
            layers.shell.clear();
            return;
        }
        GameState::MainMenu | GameState::Loading => {
            if !menus.read {
                menus.read = true;
                let mut ml = rc_menus::Library::new(&paths.game_data());
                menus.main_menu = menu_music(&mut ml, rc_menus::screens::MAIN_MENU, &log);
                menus.loading = menu_music(&mut ml, "XInterfaceCTMenus.CTLoading", &log);
                menus.pause = menu_music(&mut ml, "XInterfaceCTMenus.CTPausePCMenu", &log);
                menus.credits = menu_music(&mut ml, rc_menus::credits::CREDITS_MENU, &log);
                log.line(format!(
                    "music: menus play: main menu {}, loading {}, pause {}",
                    menus.main_menu.as_deref().unwrap_or("nothing"),
                    menus.loading.as_deref().unwrap_or("nothing"),
                    menus
                        .pause
                        .as_deref()
                        .unwrap_or("nothing (the level's music pauses)")
                ));
                if !plan.menu.is_empty() {
                    log.line(format!(
                        "music: music.toml replaces the main menu music with {}",
                        plan.menu
                    ));
                    menus.main_menu = Some(plan.menu.clone());
                }
            }
            if *state.get() == GameState::MainMenu && menu.page == crate::menu::Page::Credits {
                ("credits", menus.credits.clone())
            } else if *state.get() == GameState::MainMenu {
                ("menu", menus.main_menu.clone())
            } else {
                ("loading", menus.loading.clone())
            }
        }
    };
    if layers.shell == key {
        return;
    }
    layers.shell = key.to_string();
    let lib = library(&mut lib, &paths, &log);
    let piece = name.as_deref().and_then(|n| {
        let p = lib.resolve(n, &mut rc_music::Rng::new(1));
        if p.is_none() {
            log.line(format!("music: {n} is not in the music packages"));
        }
        p
    });
    // Everything else stops (a level's overlays and episodes too).
    for l in [Layer::Overlay, Layer::Episode] {
        stop(&mut commands, &mut layers, &mut tracks, l, Fade::NONE);
    }
    match piece {
        Some(p) => {
            log.line(format!("music: {key}: {} ({})", p.name, p.file));
            let v = p.volume.1;
            play(
                &mut commands,
                &mut layers,
                &mut tracks,
                &mut sources,
                &paths,
                &log,
                Layer::Main,
                &p,
                v,
                p.fade_in,
                Fade::NONE,
            );
        }
        None => {
            log.line(format!("music: {key}: no music"));
            stop(
                &mut commands,
                &mut layers,
                &mut tracks,
                Layer::Main,
                Fade::NONE,
            );
        }
    }
}

/// Read the map's music settings while it loads.
/// The map's `LevelInfo.Campaign` (`ELevelCampaign`), 0 when the map
/// doesn't set it (the class default).
pub fn level_campaign(
    pkg: &rc_package::Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
) -> u8 {
    for &a in level.actors.iter().flatten() {
        if !pkg
            .export_class_name(&pkg.exports[a])
            .eq_ignore_ascii_case("LevelInfo")
        {
            continue;
        }
        let Ok(props) = rc_level::actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        return props
            .iter()
            .find(|p| pkg.name(p.name).eq_ignore_ascii_case("Campaign"))
            .and_then(|p| match p.value {
                rc_package::properties::Value::Byte(b) => Some(b),
                _ => None,
            })
            .unwrap_or(0);
    }
    0
}

fn load_level_music(
    mut commands: Commands,
    level: Res<CurrentLevel>,
    paths: Res<GamePaths>,
    plan: Res<MusicPlan>,
    mut lib: ResMut<Library>,
    log: Res<GameLog>,
) {
    let lib = library(&mut lib, &paths, &log);
    let Some(map) = level.map.clone() else {
        return;
    };
    let path = paths.map_path(&map);
    let read = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            rc_package::Package::parse(&d)
                .map(|p| (p, d))
                .map_err(|e| e.to_string())
        })
        .and_then(|(p, d)| {
            rc_level::actors::read_level(&p, &d)
                .map(|l| {
                    // The sound groups' MinCampaign/MaxCampaign compare
                    // with it (`rc_combat::sound_meta`).
                    rc_combat::sound_meta::set_campaign(level_campaign(&p, &d, &l));
                    rc_level::music::read(&p, &d, &l)
                })
                .map_err(|e| e.to_string())
        });
    let mm = match read {
        Ok(m) => m,
        Err(e) => {
            log.line(format!(
                "music: can't read {} ({e}); no level music",
                path.display()
            ));
            rc_level::music::MapMusic::default()
        }
    };
    let d = BattleSettings::default();
    let b = &mm.battle;
    let battle = BattleSettings {
        music: b.music.clone(),
        threshold: b.threshold.map_or(d.threshold, |t| t.max(0) as u32),
        inner: b.inner.unwrap_or(d.inner),
        outer: b.outer.unwrap_or(d.outer),
        criteria: b.criteria.map_or(d.criteria, Criteria::from_byte),
    };
    log.line(format!(
        "music: {map}: battle music {} ({} enemies, {:?}, {:.0}/{:.0} uu)",
        battle.music.as_deref().unwrap_or("none"),
        battle.threshold,
        battle.criteria,
        battle.inner,
        battle.outer
    ));
    let mut rooms = Vec::new();
    for v in mm.volumes {
        if let Some(r) = lib.room(&v.audio_class) {
            log.line(format!(
                "music: {map}: room {} ({}): state {}, overlays {}",
                v.shape.name,
                r.class,
                r.state.as_deref().unwrap_or("-"),
                r.overlays.as_deref().unwrap_or("-")
            ));
            rooms.push((v, r.clone()));
        }
    }
    for f in &mm.failures {
        log.line(format!("music: {map}: volume not read: {f}"));
    }
    let seed = map
        .bytes()
        .fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    let n = rooms.len();
    commands.insert_resource(LevelMusic {
        director: Imuse::new(lib, battle, plan.tuning(), seed),
        rooms,
        inside: vec![false; n],
    });
}

/// Run the level's music director.
#[allow(clippy::too_many_arguments)]
fn level_music(
    mut commands: Commands,
    time: Res<Time>,
    level: Option<ResMut<LevelMusic>>,
    mut cues: MessageReader<MusicCue>,
    threats: Res<MusicThreats>,
    vitals: Res<PlayerVitals>,
    player: Query<&Transform, With<Player>>,
    paths: Res<GamePaths>,
    (mut layers, mut sources): (ResMut<Layers>, ResMut<Assets<AudioSource>>),
    mut tracks: Query<&mut Track>,
    log: Res<GameLog>,
) {
    let Some(mut level) = level else {
        cues.clear();
        return;
    };
    let level = &mut *level;
    let d = &mut level.director;
    for cue in cues.read() {
        match cue.clone() {
            MusicCue::SetState { music, fades } => d.set_state(music.as_deref(), fades_of(fades)),
            MusicCue::PlayEpisode { music, fades } => d.play_episode(&music, fades_of(fades)),
            MusicCue::EnableBattle(on) => d.enable_battle(on),
            MusicCue::BattleParams {
                criteria,
                threshold,
                inner,
                outer,
            } => d.set_battle_params(Criteria::from_byte(criteria), threshold, inner, outer),
            MusicCue::BattleMusic(m) => d.set_battle_music(m.as_deref()),
            MusicCue::ForceBattle => d.force_battle(),
            MusicCue::ForceVictory => d.force_victory(),
        }
    }
    let at = player.single().ok().map(|t| {
        let p = to_unreal(t.translation);
        [p.x, p.y, p.z]
    });
    // Rooms: walking into a volume with music.
    if let Some(p) = at {
        let above = [p[0], p[1], p[2] + 50.0];
        for (i, (v, room)) in level.rooms.iter().enumerate() {
            let inside = v.shape.contains(p) || v.shape.contains(above);
            if inside && !level.inside[i] {
                d.enter_room(&v.shape.name, room);
            }
            level.inside[i] = inside;
        }
    }
    let threats: Vec<Threat> = threats
        .0
        .iter()
        .map(|t| Threat {
            at: t.at,
            engaged: t.engaged,
        })
        .collect();
    d.update(time.delta_secs(), &threats, at, vitals.incapacitated);
    for line in d.take_log() {
        log.line(format!("music: {line}"));
    }
    let cmds = d.take_commands();
    apply(
        cmds,
        Some(d),
        &mut commands,
        &mut layers,
        &mut tracks,
        &mut sources,
        &paths,
        &log,
    );
}

/// Leaving the level: its music stops.
fn leave_level(
    mut commands: Commands,
    mut layers: ResMut<Layers>,
    mut tracks: Query<&mut Track>,
    all: Query<Entity, With<Track>>,
    log: Res<GameLog>,
) {
    commands.remove_resource::<LevelMusic>();
    for e in &all {
        commands.entity(e).try_despawn();
    }
    for l in Layer::ALL {
        stop(&mut commands, &mut layers, &mut tracks, l, Fade::NONE);
    }
    layers.shell.clear();
    log.line("music: left the level; its music stops");
}

/// A menu opened over the level: its music pauses (the original's
/// `PauseLevelMusic`); the pause menu has no music of its own.
fn pause_level_music(sinks: Query<&AudioSink, With<Track>>, log: Res<GameLog>) {
    for s in &sinks {
        s.pause();
    }
    log.line("music: paused with the game");
}

fn resume_level_music(sinks: Query<&AudioSink, With<Track>>, log: Res<GameLog>) {
    for s in &sinks {
        s.play();
    }
    log.line("music: playing again");
}

/// Fades, and the player's music volume, every frame.
fn fades(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<Settings>,
    play: Option<Res<State<Play>>>,
    ducking: Option<Res<rc_game_core::Ducking>>,
    mut tracks: Query<(Entity, &mut Track, Option<&mut AudioSink>)>,
) {
    let paused = play.is_some_and(|p| *p.get() == Play::Paused);
    // Lowered while a radio line plays (the sound engine's ducking), then
    // the original's clamp and the port's master volume.
    let duck = ducking.map_or(1.0, |d| d.music_now());
    let music = settings.audio.music_volume.clamp(0.0, 1.0);
    let master = settings.audio.master_volume.clamp(0.0, 1.0);
    let dt = time.delta_secs();
    for (e, mut t, sink) in &mut tracks {
        if !paused && (t.fading_in || t.fading_out) {
            t.t += dt;
            if t.t >= t.fade.seconds {
                if t.fading_out {
                    commands.entity(e).try_despawn();
                    continue;
                }
                t.fading_in = false;
            }
        }
        if let Some(mut s) = sink {
            let v = (t.gain() * duck * music).clamp(0.0001, 0.9999) * master;
            if (s.volume().to_linear() - v).abs() > 1e-4 {
                s.set_volume(Volume::Linear(v));
            }
        }
    }
}

/// One-shot pieces that ended by themselves: tell the director.
fn watch_ends(
    mut layers: ResMut<Layers>,
    tracks: Query<(), With<Track>>,
    level: Option<ResMut<LevelMusic>>,
) {
    let mut ended = Vec::new();
    for (layer, c) in layers.current.iter_mut() {
        let Some(e) = c.entity else { continue };
        if tracks.contains(e) {
            c.seen = true;
        } else if c.seen {
            ended.push(*layer);
        }
    }
    for l in &ended {
        layers.current.remove(l);
    }
    if let Some(mut level) = level {
        for l in ended {
            level.director.finished(l);
        }
    }
}

/// Log when a piece really starts on the sound card.
fn watch_started(started: Query<&Track, Added<AudioSink>>, log: Res<GameLog>) {
    for t in &started {
        log.line(format!(
            "music: {} started on the sound card ({} layer)",
            t.name,
            t.layer.name()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_plan_reads() {
        let p = MusicPlan::parse(include_str!("../../../data/music/music.toml")).unwrap();
        assert!(p.battle_end_seconds > 0.0);
        assert!(
            p.menu.is_empty(),
            "the shipped file keeps the original menu music"
        );
    }

    #[test]
    fn fade_out_starts_where_the_fade_in_was() {
        let mut t = Track {
            layer: Layer::Main,
            name: "x".into(),
            base: 1.0,
            fade: Fade {
                seconds: 2.0,
                shape: FadeShape::Linear,
            },
            fading_in: true,
            fading_out: false,
            t: 1.0,
        };
        assert!((t.gain() - 0.5).abs() < 1e-4);
        t.fading_in = false;
        t.fading_out = true;
        t.t = 0.0;
        assert!((t.gain() - 1.0).abs() < 1e-4);
    }
}
