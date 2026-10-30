//! rc_game: the game. One program with the menus, settings, controls,
//! on-screen display and music, which every other part of the game plugs
//! into (see docs/PLUGINS.md).
//!
//! Usage:
//!   rc_game [--settings FILE] [--map NAME] [--script] [--frames N] [--log-input]
//!
//! `--settings` picks the settings file (default `settings.toml` in the
//! folder you run from; written with defaults if missing). `--map` skips the
//! menus and loads that map. `--script` plays a fixed route through the menus
//! and a level instead of reading the keyboard, checks every step, and exits
//! with code 0 if all passed. `--frames N` closes after N frames.
//! `--log-input` writes every button press and the move, look and gyro
//! numbers to the log, to check a controller.
//! Everything is logged to `logs/rc_game.log`.

mod hud;
mod input;
mod levels;
mod menu;
mod music;
mod original_hud;
mod original_menu;
mod panini;
mod player;
mod rebind;
mod script;
mod settings;
mod theme;

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowMode};
use rc_game_core::{CorePlugin, CurrentLevel, GameLog, GamePaths, GameSet, GameState};
use settings::{Settings, SettingsPath};
use std::path::PathBuf;

const LOG_PATH: &str = "logs/rc_game.log";

#[derive(Resource)]
struct ExitAfter(Option<u32>);

/// Starting straight into a map (`--map`).
#[derive(Resource)]
struct StartMap(Option<String>);

fn boot(
    start: Res<StartMap>,
    mut current: ResMut<CurrentLevel>,
    mut next: ResMut<NextState<GameState>>,
    log: Res<GameLog>,
) {
    match &start.0 {
        Some(m) => {
            log.line(format!("boot: --map {m}, skipping the menus"));
            current.map = Some(m.clone());
            next.set(GameState::Loading);
        }
        None => next.set(GameState::MainMenu),
    }
}

fn log_states(
    state: Res<State<GameState>>,
    play: Option<Res<State<rc_game_core::Play>>>,
    log: Res<GameLog>,
) {
    if state.is_changed() {
        log.line(format!("state: {:?}", state.get()));
    }
    if let Some(p) = play {
        if p.is_changed() {
            log.line(format!("state: play {:?}", p.get()));
        }
    }
}

fn count_frames(
    mut frames: Local<u32>,
    exit_after: Res<ExitAfter>,
    mut exit: MessageWriter<AppExit>,
    log: Res<GameLog>,
) {
    *frames += 1;
    if let Some(n) = exit_after.0 {
        if *frames >= n {
            log.line(format!("{n} frames done, closing"));
            exit.write(AppExit::Success);
        }
    }
}

/// The menu camera clears the screen itself when there is no 3D view under
/// it (in the menus), and draws on top of the 3D view during play.
fn menu_camera_clear(
    views: Query<(), With<rc_game_core::PlayerCamera>>,
    mut ui: Query<&mut Camera, With<IsDefaultUiCamera>>,
) {
    let menus = views.is_empty();
    for mut c in &mut ui {
        let clears = matches!(c.clear_color, ClearColorConfig::Default);
        if clears != menus {
            c.clear_color = if menus {
                ClearColorConfig::Default
            } else {
                ClearColorConfig::None
            };
        }
    }
}

fn main() -> AppExit {
    let mut settings_file: Option<PathBuf> = None;
    let mut map = None;
    let mut use_script = false;
    let mut frames = None;
    let mut log_input = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--settings" => settings_file = args.next().map(PathBuf::from),
            "--map" => map = args.next(),
            "--script" => use_script = true,
            "--frames" => frames = args.next().and_then(|n| n.parse().ok()),
            "--log-input" => log_input = true,
            other => {
                eprintln!("unknown option {other}");
                eprintln!(
                    "usage: rc_game [--settings FILE] [--map NAME] [--script] [--frames N] [--log-input]"
                );
                return AppExit::from_code(2);
            }
        }
    }
    let log = match GameLog::create(std::path::Path::new(LOG_PATH)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't create {LOG_PATH}: {e}");
            return AppExit::from_code(1);
        }
    };
    log.line(format!("rc_game {}", env!("CARGO_PKG_VERSION")));

    // A scripted run uses its own settings file, so it never changes yours.
    let settings_path = settings_file.unwrap_or_else(|| {
        PathBuf::from(if use_script {
            "logs/rc_game_script_settings.toml"
        } else {
            "settings.toml"
        })
    });
    let settings = match Settings::load_or_create(&settings_path) {
        Ok((s, note)) => {
            log.line(note);
            s
        }
        Err(e) => {
            log.line(format!("ERROR in the settings file {e}"));
            return AppExit::from_code(1);
        }
    };

    log.line(format!(
        "difficulty: {:?} (damage to the player x{:.2})",
        settings.game.difficulty,
        settings.game.difficulty.damage_to_player()
    ));
    let (tried, found) = settings::find_game_folder(&settings);
    for t in &tried {
        log.line(format!("game folder: tried {}", t.display()));
    }
    let game_dir = match &found {
        Some(d) => {
            log.line(format!("game folder: using {}", d.display()));
            d.clone()
        }
        None => {
            log.line("game folder: NOT FOUND; set [game] folder in the settings file");
            tried.first().cloned().unwrap_or_default()
        }
    };
    let paths = GamePaths::new(game_dir, settings.data_dirs());
    // A scripted run saves into its own folder, emptied first, so it never
    // touches your saves and always starts the same way.
    let save_folder = if use_script {
        let d = PathBuf::from("logs/script_saves");
        let _ = std::fs::remove_dir_all(&d);
        d
    } else if settings.save.folder.trim().is_empty() {
        rc_save::file::default_folder(rc_save::file::System::this(), |k| std::env::var(k).ok())
    } else {
        PathBuf::from(settings.save.folder.trim())
    };
    for d in &paths.data_dirs {
        log.line(format!(
            "data folder: {}{}",
            d.display(),
            if d.is_dir() { "" } else { " (missing)" }
        ));
    }

    let bindings = paths
        .data_file(&settings.controls.bindings)
        .ok_or_else(|| {
            format!(
                "{} not found in the data folders",
                settings.controls.bindings
            )
        })
        .and_then(|p| {
            let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            log.line(format!("controls from {}", p.display()));
            input::Bindings::parse(&t).map_err(|e| format!("{}: {e}", p.display()))
        });
    let base_bindings = match bindings {
        Ok(b) => b,
        Err(e) => {
            log.line(format!("ERROR in the controls file: {e}"));
            return AppExit::from_code(1);
        }
    };
    // The player's own changes, saved by the game next to the settings file.
    let user_bindings = settings_path
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join(&settings.controls.user_bindings);
    let bindings = match std::fs::read_to_string(&user_bindings) {
        Ok(t) => match base_bindings.with_changes(&t) {
            Ok(b) => {
                log.line(format!("controls: your changes from {}", user_bindings.display()));
                b
            }
            Err(e) => {
                log.line(format!(
                    "ERROR in {}: {e}; using the default controls",
                    user_bindings.display()
                ));
                base_bindings.clone()
            }
        },
        Err(_) => base_bindings.clone(),
    };
    log.line(format!(
        "controls: {} keyboard, {} mouse, {} gamepad bindings; touch {}",
        bindings.keys.len(),
        bindings.mouse.len(),
        bindings.pad.len(),
        if bindings.touch { "on" } else { "off" }
    ));
    let button_names = match paths.data_file("input/button_names.toml") {
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| rc_pads::ButtonNames::parse(&t))
        {
            Ok(n) => n,
            Err(e) => {
                log.line(format!("ERROR in {}: {e}; prompts use plain names", p.display()));
                rc_pads::ButtonNames::default()
            }
        },
        None => {
            log.line("controls: no input/button_names.toml; prompts use plain names");
            rc_pads::ButtonNames::default()
        }
    };

    // Controllers: SDL if it starts (it reads gyros and knows every kind of
    // controller), otherwise Bevy's own gilrs.
    #[cfg(all(feature = "sdl", not(target_os = "android")))]
    let sdl = match rc_pads::sdl::SdlPads::open() {
        Ok(p) => {
            for n in &p.notes {
                log.line(n);
            }
            Some(p)
        }
        Err(e) => {
            log.line(format!("pads: {e}; using gilrs (no gyro)"));
            None
        }
    };
    #[cfg(not(all(feature = "sdl", not(target_os = "android"))))]
    let sdl: Option<()> = {
        log.line("pads: built without SDL; using gilrs (no gyro)");
        None
    };

    let theme = match paths.data_file("ui/theme.toml") {
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| theme::Theme::parse(&t))
        {
            Ok(t) => t,
            Err(e) => {
                log.line(format!(
                    "ERROR in {}: {e}; using the built-in look",
                    p.display()
                ));
                theme::Theme::default()
            }
        },
        None => theme::Theme::default(),
    };
    let music_plan = match paths.data_file("music/music.toml") {
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| music::MusicPlan::parse(&t))
        {
            Ok(m) => m,
            Err(e) => {
                log.line(format!("ERROR in {}: {e}; no music", p.display()));
                music::MusicPlan::default()
            }
        },
        None => {
            log.line("music: no music/music.toml; no music");
            music::MusicPlan::default()
        }
    };
    let (level_list, notes) = levels::load(&paths);
    for n in notes {
        log.line(n);
    }

    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Star Wars Republic Commando (Rust and Bevy)".into(),
            resolution: (settings.video.width, settings.video.height).into(),
            mode: if settings.video.fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            present_mode: if settings.video.vsync {
                PresentMode::AutoVsync
            } else {
                PresentMode::AutoNoVsync
            },
            ..default()
        }),
        ..default()
    });
    // SDL reads the controllers instead of gilrs; never both, or every
    // button would count twice.
    app.add_plugins(if sdl.is_some() {
        plugins.disable::<bevy::gilrs::GilrsPlugin>()
    } else {
        plugins
    })
    .add_plugins(CorePlugin)
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(log)
    .insert_resource(paths)
    .insert_resource(rc_save::SaveFolder(save_folder))
    .insert_resource(menu::GamePathsFound(found.is_some()))
    .insert_resource(player::FieldOfView(settings.video.fov))
    .insert_resource(input::LookSettings::from_settings(&settings))
    .insert_resource(settings.gyro.clone())
    .insert_resource(button_names)
    .insert_resource(bindings)
    .insert_resource(input::BaseBindings(base_bindings))
    .insert_resource(rebind::UserBindingsPath(user_bindings))
    .insert_resource(input::LogInput(log_input))
    .insert_resource(theme)
    .insert_resource(music_plan)
    .insert_resource(level_list)
    .insert_resource(settings)
    .insert_resource(SettingsPath(settings_path))
    .insert_resource(StartMap(map))
    .insert_resource(ExitAfter(frames))
    .add_plugins(rc_pads::PadsPlugin)
    .add_plugins((
        menu::MenuPlugin,
        original_menu::OriginalMenuPlugin,
        hud::HudPlugin,
        original_hud::OriginalHudPlugin,
        music::MusicPlugin,
        player::PlayerPlugin,
        panini::PaniniViewPlugin,
    ))
    .add_systems(Startup, |mut commands: Commands| {
        // The camera for menus and the display; the 3D view has its own.
        commands.spawn((
            Camera2d,
            Camera {
                order: 2,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            IsDefaultUiCamera,
        ));
    })
    .add_systems(OnEnter(GameState::Boot), boot)
    .add_systems(
        Update,
        (
            log_states,
            menu::apply_settings,
            count_frames,
            menu_camera_clear,
        )
            .in_set(GameSet::Hud),
    );

    // PLUGINS FROM OTHER PARTS (see docs/PLUGINS.md).
    app.add_plugins(rc_level_plugin::LevelPlugin);
    app.add_plugins(rc_movers_plugin::MoversPlugin);
    app.add_plugins(rc_scripts_plugin::ScriptsPlugin);
    app.add_plugins(rc_combat::CombatPlugin);
    app.add_plugins(rc_enemies::EnemiesPlugin);
    app.add_plugins(rc_squad_plugin::SquadPlugin);
    app.add_plugins(rc_save::SavePlugin);
    app.configure_sets(
        Update,
        player::PlayerMoveSet.before(rc_squad_plugin::SquadSet),
    );
    app.configure_sets(
        Update,
        rc_scripts_plugin::ScriptSystems.after(player::PlayerMoveSet),
    );

    #[cfg(all(feature = "sdl", not(target_os = "android")))]
    match sdl {
        Some(p) => rc_pads::sdl::add_to(&mut app, p),
        None => {
            app.insert_resource(rc_pads::PadReader::Gilrs);
        }
    }
    #[cfg(not(all(feature = "sdl", not(target_os = "android"))))]
    app.insert_resource(rc_pads::PadReader::Gilrs);

    if use_script {
        app.add_plugins(script::ScriptPlugin);
    } else {
        app.add_plugins(input::DeviceInputPlugin);
    }
    app.run()
}
