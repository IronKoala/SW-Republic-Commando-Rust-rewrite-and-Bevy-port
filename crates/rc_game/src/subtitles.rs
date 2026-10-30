//! SUBTITLES: the words of each line the level scripts say, shown at the
//! bottom of the screen while the line plays.
//!
//! The words come from the original game's `GameData/System/subtitles*.int`
//! files, read at start-up (never copied into the repository). Each file
//! pairs a sound name with its text by number:
//! `SubtitleSound[12]="CA1G1500"` and `SubtitleText[12]="Clone Advisor 1:
//! ..."`. A line shows while its sound plays (at least `MIN_SECONDS`).
//!
//! PLACEHOLDER look: Bevy's built-in font in our theme's colours, in the
//! same style as the rest of the stand-in display (`hud.rs`), until the
//! display is drawn with the original's fonts.

use crate::settings::Settings;
use crate::theme::Theme;
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, GameSet, GameState};
use rc_scripts_plugin::ScriptSound;
use std::collections::HashMap;

/// A line stays up at least this long, so a short one can be read.
const MIN_SECONDS: f32 = 1.5;
/// The most lines shown at once (the newest at the bottom).
const MAX_LINES: usize = 3;

/// The words of each line, by lower-case sound name.
#[derive(Resource, Debug, Default)]
pub struct Subtitles {
    pub by_sound: HashMap<String, String>,
}

impl Subtitles {
    /// Read every `subtitles*.int` file in the game's `System` folder.
    pub fn load(paths: &GamePaths, log: &GameLog) -> Subtitles {
        let dir = paths.game_file("GameData/System");
        let mut out = Subtitles::default();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            log.line(format!(
                "subtitles: can't read {}; no subtitles",
                dir.display()
            ));
            return out;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_ascii_lowercase())
                    .is_some_and(|n| n.starts_with("subtitles") && n.ends_with(".int"))
            })
            .collect();
        files.sort();
        for f in &files {
            match std::fs::read(f) {
                Ok(bytes) => {
                    let found = parse(&crate::levels::decode_int(&bytes));
                    log.line(format!(
                        "subtitles: {} lines from {}",
                        found.len(),
                        f.display()
                    ));
                    out.by_sound.extend(found);
                }
                Err(e) => log.line(format!("subtitles: can't read {}: {e}", f.display())),
            }
        }
        if files.is_empty() {
            log.line(format!(
                "subtitles: no subtitles*.int in {}; no subtitles",
                dir.display()
            ));
        }
        out
    }
}

/// `(lower-case sound name, text)` from a subtitles file's
/// `SubtitleSound[n]` and `SubtitleText[n]` lines, paired by number within
/// each `[section]`.
pub fn parse(text: &str) -> HashMap<String, String> {
    let mut sounds: HashMap<(usize, String), String> = HashMap::new();
    let mut texts: HashMap<(usize, String), String> = HashMap::new();
    let mut section = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section += 1;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((name, rest)) = key.trim().split_once('[') else {
            continue;
        };
        let Some(number) = rest.strip_suffix(']') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value)
            .to_string();
        let slot = (section, number.trim().to_string());
        match name.trim().to_ascii_lowercase().as_str() {
            "subtitlesound" => {
                sounds.insert(slot, value);
            }
            "subtitletext" => {
                texts.insert(slot, value);
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for (slot, sound) in sounds {
        if let Some(t) = texts.remove(&slot) {
            if !sound.is_empty() && !t.is_empty() {
                out.insert(sound.to_ascii_lowercase(), t);
            }
        }
    }
    out
}

pub struct SubtitlesPlugin;

impl Plugin for SubtitlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .add_systems(OnEnter(GameState::InGame), spawn_area)
            .add_systems(
                Update,
                show.in_set(GameSet::Hud)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn load(mut commands: Commands, paths: Res<GamePaths>, log: Res<GameLog>) {
    commands.insert_resource(Subtitles::load(&paths, &log));
}

/// Where the lines go: the bottom middle of the display's area, above the
/// shield and ammunition readouts.
#[derive(Component)]
struct SubtitleArea;

/// One line on screen: the sound it belongs to, and how long it has shown.
#[derive(Component)]
struct SubtitleLine {
    sound: Entity,
    shown: f32,
}

fn spawn_area(mut commands: Commands, settings: Res<Settings>) {
    let max = settings.video.hud_max_aspect;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                max_width: if max > 0.0 {
                    Val::Vh(100.0 * max)
                } else {
                    Val::Auto
                },
                left: Val::Auto,
                right: Val::Auto,
                margin: UiRect::horizontal(Val::Auto),
                justify_content: JustifyContent::Center,
                ..default()
            },
            DespawnOnExit(GameState::InGame),
            GlobalZIndex(2),
        ))
        .with_children(|root| {
            root.spawn((
                SubtitleArea,
                Node {
                    position_type: PositionType::Absolute,
                    // Above the bottom readouts (which end about 15% up).
                    bottom: Val::VMin(18.0),
                    max_width: Val::Percent(70.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::VMin(0.6),
                    ..default()
                },
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    settings: Res<Settings>,
    subtitles: Option<Res<Subtitles>>,
    theme: Res<Theme>,
    time: Res<Time>,
    started: Query<(Entity, &ScriptSound), Added<ScriptSound>>,
    sounds: Query<(), With<ScriptSound>>,
    area: Query<Entity, With<SubtitleArea>>,
    mut lines: Query<(Entity, &mut SubtitleLine)>,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs();
    let mut on_screen = 0;
    for (e, mut l) in &mut lines {
        l.shown += dt;
        let still_playing = sounds.contains(l.sound);
        if !settings.game.subtitles || (!still_playing && l.shown >= MIN_SECONDS) {
            commands.entity(e).despawn();
        } else {
            on_screen += 1;
        }
    }
    let Ok(area) = area.single() else {
        return;
    };
    for (e, s) in &started {
        if !s.speech {
            continue;
        }
        let Some(text) = subtitles
            .as_ref()
            .and_then(|t| t.by_sound.get(&s.name.to_ascii_lowercase()))
        else {
            log.line(format!("subtitles: no words for {}", s.name));
            continue;
        };
        if !settings.game.subtitles {
            continue;
        }
        if on_screen >= MAX_LINES {
            // The oldest line makes room (it is the first child).
            if let Some((old, _)) = lines.iter().min_by(|a, b| b.1.shown.total_cmp(&a.1.shown)) {
                commands.entity(old).despawn();
            }
        } else {
            on_screen += 1;
        }
        log.line(format!("subtitles: {}: \"{text}\"", s.name));
        commands.entity(area).with_children(|p| {
            p.spawn((
                SubtitleLine { sound: e, shown: 0.0 },
                Node {
                    padding: UiRect::axes(Val::VMin(1.0), Val::VMin(0.3)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                Text::new(text.clone()),
                TextLayout::justify(Justify::Center),
                theme.font(theme.small),
                TextColor(theme.text),
            ));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_sounds_with_their_words_by_number() {
        let text = "[Subtitles]\r\n\
            SubtitleSound[0]=\"CA1G1500\"\r\n\
            SubtitleText[0]=\"Clone Advisor 1: Delta Squad, report.\"\r\n\
            SubtitleText[1]=\"No sound for this one.\"\r\n\
            SubtitleSound[2]=CA1G1501\r\n\
            SubtitleText[2]=Unquoted words\r\n\
            [Other]\r\n\
            SubtitleSound[0]=\"X\"\r\n\
            SubtitleText[0]=\"Another section's line 0\"\r\n";
        let s = parse(text);
        assert_eq!(s.len(), 3);
        assert_eq!(s["ca1g1500"], "Clone Advisor 1: Delta Squad, report.");
        assert_eq!(s["ca1g1501"], "Unquoted words");
        assert_eq!(s["x"], "Another section's line 0");
    }

    fn app() -> App {
        let mut app = App::new();
        let mut subs = Subtitles::default();
        subs.by_sound.insert("line1".into(), "Sev: Copy that.".into());
        app.add_plugins(MinimalPlugins)
            .insert_resource(Settings::default())
            .insert_resource(Theme::default())
            .insert_resource(GameLog::terminal())
            .insert_resource(subs)
            .add_systems(Update, show);
        app.world_mut().spawn(SubtitleArea);
        app
    }

    fn shown(app: &mut App) -> Vec<String> {
        let mut q = app.world_mut().query_filtered::<&Text, With<SubtitleLine>>();
        q.iter(app.world()).map(|t| t.0.clone()).collect()
    }

    fn say(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .spawn(ScriptSound {
                name: name.into(),
                source: "Sev".into(),
                speech: true,
            })
            .id()
    }

    #[test]
    fn a_line_shows_while_it_plays_and_only_with_the_option_on() {
        let mut app = app();
        let line = say(&mut app, "LINE1");
        app.update();
        assert_eq!(shown(&mut app), ["Sev: Copy that."]);
        // A line with no words in the files shows nothing.
        say(&mut app, "unknown");
        app.update();
        assert_eq!(shown(&mut app).len(), 1);
        // Switched off: the line goes, and new ones don't show.
        app.world_mut().resource_mut::<Settings>().game.subtitles = false;
        app.update();
        assert!(shown(&mut app).is_empty());
        app.world_mut().despawn(line);
        say(&mut app, "line1");
        app.update();
        assert!(shown(&mut app).is_empty());
    }
}
