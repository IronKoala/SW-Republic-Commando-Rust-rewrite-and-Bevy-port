use super::file::System as Os;
use super::file::{
    default_folder, list, read, slot_path, time_text, write_safely, Header, SaveFile, FORMAT,
};
use super::*;
use bevy::state::app::StatesPlugin;
use rc_game_core::CorePlugin;
use std::collections::HashSet;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rc_save_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn slot_file_names_round_trip() {
    for s in [
        SaveSlot::Quick,
        SaveSlot::Checkpoint,
        SaveSlot::Named("before_the_bridge".into()),
    ] {
        assert_eq!(SaveSlot::from_file_name(&s.file_name()), Some(s));
    }
    // Nothing that could leave the save folder gets into a file name.
    assert_eq!(
        SaveSlot::Named("../../GameData/x".into()).file_name(),
        "save_GameDatax.toml"
    );
    assert_eq!(SaveSlot::from_file_name("settings.toml"), None);
}

#[test]
fn save_folder_on_each_system() {
    let env = |k: &str| match k {
        "APPDATA" => Some(r"C:\Users\me\AppData\Roaming".to_string()),
        "HOME" => Some("/home/me".to_string()),
        _ => None,
    };
    assert_eq!(
        default_folder(Os::Windows, env),
        PathBuf::from(r"C:\Users\me\AppData\Roaming")
            .join("rc_game")
            .join("saves")
    );
    assert_eq!(
        default_folder(Os::Linux, env),
        PathBuf::from("/home/me/.local/share/rc_game/saves")
    );
    let xdg = |k: &str| (k == "XDG_DATA_HOME").then(|| "/data".to_string());
    assert_eq!(
        default_folder(Os::Linux, xdg),
        PathBuf::from("/data/rc_game/saves")
    );
    assert_eq!(default_folder(Os::Android, env), PathBuf::from("saves"));
    assert_eq!(
        default_folder(Os::Windows, |_| None),
        PathBuf::from("saves")
    );
}

#[test]
fn time_text_is_a_calendar_date() {
    assert_eq!(time_text(0), "1970-01-01 00:00 UTC");
    assert_eq!(time_text(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(time_text(1_793_299_200), "2026-10-29 18:40 UTC");
}

fn sample() -> SaveFile {
    let mut parts = toml::Table::new();
    parts.insert(
        "player".into(),
        toml::Value::Table(toml::from_str("location = [1.5, -2.0, 48.0]\nyaw = 0.5\n").unwrap()),
    );
    parts.insert(
        "checkpoints".into(),
        toml::Value::Table(toml::from_str("reached = ['level start']\n").unwrap()),
    );
    SaveFile {
        header: Header {
            format: FORMAT,
            game_version: "0.1.0".into(),
            map: "geo_01a".into(),
            checkpoint: Some("level start".into()),
            saved_at: 1_793_299_200,
            saved_at_text: time_text(1_793_299_200),
            play_seconds: 12.5,
        },
        parts,
    }
}

#[test]
fn file_text_reads_back_and_keeps_part_order() {
    let s = sample();
    let text = s.to_text(&["player", "checkpoints"]).unwrap();
    assert!(text.starts_with("# Star Wars"));
    let save = text.find("[save]").unwrap();
    let player = text.find("[player]").unwrap();
    let cps = text.find("[checkpoints]").unwrap();
    assert!(save < player && player < cps, "{text}");
    assert_eq!(SaveFile::parse(&text).unwrap(), s);
}

#[test]
fn bad_files_say_why() {
    assert!(SaveFile::parse("[player]\nyaw = 1\n")
        .unwrap_err()
        .contains("no [save]"));
    let newer = "[save]\nformat = 99\n";
    assert!(SaveFile::parse(newer)
        .unwrap_err()
        .contains("newer version"));
    assert!(SaveFile::parse("not toml at all = = =").is_err());
}

#[test]
fn writing_replaces_the_old_file_and_lists_newest_first() {
    let dir = temp_dir("list");
    let mut a = sample();
    a.header.saved_at = 100;
    let mut b = sample();
    b.header.saved_at = 200;
    b.header.map = "ras_01a".into();
    write_safely(&slot_path(&dir, &SaveSlot::Quick), &a.to_text(&[]).unwrap()).unwrap();
    write_safely(
        &slot_path(&dir, &SaveSlot::Checkpoint),
        &b.to_text(&[]).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("save_broken.toml"), "nonsense").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a save").unwrap();
    let (list, bad) = list(&dir);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].slot, SaveSlot::Checkpoint);
    assert_eq!(list[0].map, "ras_01a");
    assert_eq!(bad.len(), 1, "{bad:?}");
    // No temporary file left behind.
    assert!(!dir.join("quicksave.toml.writing").exists());
    // Writing again replaces it.
    let mut c = sample();
    c.header.saved_at = 300;
    write_safely(&slot_path(&dir, &SaveSlot::Quick), &c.to_text(&[]).unwrap()).unwrap();
    assert_eq!(
        read(&slot_path(&dir, &SaveSlot::Quick))
            .unwrap()
            .header
            .saved_at,
        300
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// A whole game in miniature: a part with one number, saved and loaded
// through the plugin the way the real game does it.

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
struct Score(u32);

#[derive(Serialize, Deserialize)]
struct ScoreSave {
    score: u32,
}

impl SavePart for ScoreSave {
    const KEY: &'static str = "score";
    fn save(world: &mut World) -> Option<Self> {
        world
            .get_resource::<Score>()
            .map(|s| ScoreSave { score: s.0 })
    }
    fn load(world: &mut World, data: Self) -> Result<(), String> {
        world.insert_resource(Score(data.score));
        Ok(())
    }
}

/// The level "loads" instantly: whenever we're in Loading, go to InGame and
/// start the score from scratch, as a fresh map would.
fn fake_level(mut next: ResMut<NextState<GameState>>, mut score: ResMut<Score>) {
    next.set(GameState::InGame);
    score.0 = 0;
}

fn game(dir: &std::path::Path) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, CorePlugin))
        .insert_resource(GameLog::terminal())
        .insert_resource(SaveFolder(dir.to_path_buf()))
        .insert_resource(Score(0))
        .add_plugins(SavePlugin)
        .add_save_part::<ScoreSave>()
        .add_systems(OnEnter(GameState::Loading), fake_level);
    app.world_mut().resource_mut::<CurrentLevel>().map = Some("geo_01a".into());
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::Loading);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::InGame
    );
    app
}

fn results(app: &mut App) -> Vec<SaveResult> {
    app.world_mut()
        .resource_mut::<Messages<SaveResult>>()
        .drain()
        .collect()
}

#[test]
fn save_change_load_puts_everything_back() {
    let dir = temp_dir("round_trip");
    let mut app = game(&dir);
    // The start of the level was saved as a checkpoint.
    assert!(slot_path(&dir, &SaveSlot::Checkpoint).exists());
    assert_eq!(
        app.world().resource::<Checkpoints>().reached,
        vec![LEVEL_START.to_string()]
    );

    // Play a bit, reach a checkpoint, quicksave.
    app.world_mut().resource_mut::<Score>().0 = 7;
    app.world_mut().write_message(CheckpointReached {
        name: "bridge".into(),
    });
    app.update();
    app.world_mut().write_message(CheckpointReached {
        name: "bridge".into(),
    });
    let mut keys = HashSet::new();
    keys.insert(Action::QuickSave);
    app.world_mut()
        .resource_mut::<ActionState>()
        .update(keys, Vec2::ZERO, Vec2::ZERO);
    app.update();
    let r = results(&mut app);
    assert!(r.iter().all(|r| r.outcome.is_ok()), "{r:?}");
    assert!(r.iter().any(|r| r.slot == SaveSlot::Quick && !r.loading));
    let quick = read(&slot_path(&dir, &SaveSlot::Quick)).unwrap();
    assert_eq!(quick.header.checkpoint.as_deref(), Some("bridge"));
    assert_eq!(quick.part("score").unwrap()["score"].as_integer(), Some(7));
    assert_eq!(
        app.world().resource::<Checkpoints>().reached,
        vec![LEVEL_START.to_string(), "bridge".to_string()],
        "a checkpoint reached twice counts once"
    );
    assert_eq!(
        app.world().resource::<SaveList>().newest().unwrap().slot,
        SaveSlot::Quick
    );

    // Things change, then quickload.
    app.world_mut().resource_mut::<Score>().0 = 99;
    app.world_mut()
        .resource_mut::<Checkpoints>()
        .reached
        .push("later".into());
    app.world_mut()
        .resource_mut::<ActionState>()
        .update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
    app.update();
    let mut keys = HashSet::new();
    keys.insert(Action::QuickLoad);
    app.world_mut()
        .resource_mut::<ActionState>()
        .update(keys.clone(), Vec2::ZERO, Vec2::ZERO);
    app.update();
    // Still held next frame: no longer "just pressed", so it loads once.
    app.world_mut()
        .resource_mut::<ActionState>()
        .update(keys, Vec2::ZERO, Vec2::ZERO);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(*app.world().resource::<Score>(), Score(7));
    assert_eq!(
        app.world().resource::<Checkpoints>().reached,
        vec![LEVEL_START.to_string(), "bridge".to_string()]
    );
    assert!(!app.world().contains_resource::<RestoringSave>());
    // Loading a save doesn't overwrite the checkpoint with a new "level
    // start".
    let cp = read(&slot_path(&dir, &SaveSlot::Checkpoint)).unwrap();
    assert_eq!(cp.header.checkpoint.as_deref(), Some("bridge"));

    // Saving again gives the same file apart from the time.
    app.world_mut().write_message(SaveGame {
        slot: SaveSlot::Named("again".into()),
    });
    app.update();
    let again = read(&slot_path(&dir, &SaveSlot::Named("again".into()))).unwrap();
    assert_eq!(again.parts, quick.parts);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loading_a_missing_save_says_so_and_stays_put() {
    let dir = temp_dir("missing");
    let mut app = game(&dir);
    app.world_mut().write_message(LoadGame {
        slot: SaveSlot::Quick,
    });
    app.update();
    let r = results(&mut app);
    let load = r.iter().find(|r| r.loading).unwrap();
    assert!(load.outcome.as_ref().unwrap_err().contains("no quicksave"));
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::InGame
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_from_the_menu_is_refused() {
    let dir = temp_dir("menu");
    let mut app = game(&dir);
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::MainMenu);
    app.update();
    app.world_mut().write_message(SaveGame {
        slot: SaveSlot::Quick,
    });
    app.update();
    let r = results(&mut app);
    assert!(r.iter().any(|r| !r.loading && r.outcome.is_err()), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn differences_between_sections() {
    let a: toml::Value = toml::from_str("a = [1.0, 2.0]\nb = 'x'\n[c]\nd = 3\n").unwrap();
    let b: toml::Value = toml::from_str("a = [1.5, 2.0]\nb = 'x'\n[c]\nd = 3\n").unwrap();
    let c: toml::Value = toml::from_str("a = [1.0, 2.0]\nb = 'y'\n[c]\nd = 3\n").unwrap();
    assert_eq!(super::file::largest_difference(&a, &b), Some(0.5));
    assert_eq!(super::file::largest_difference(&a, &c), None);
}
