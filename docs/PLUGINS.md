# Plugging a part into the game

The game program is `rc_game`. Each part of the game (the level, weapons,
enemies, the squad, the menus) is a Bevy **plugin**: a bundle of systems that
`rc_game` switches on with one line. Parts don't call each other; they share
the small set of types in `crates/rc_game_core`.

## Adding your part

1. Make your part a library crate (say `rc_weapons`) with a public plugin:

   ```rust
   pub struct WeaponsPlugin;
   impl Plugin for WeaponsPlugin {
       fn build(&self, app: &mut App) { /* systems */ }
   }
   ```

2. Depend on `rc_game_core` (path `../rc_game_core`) and on Bevy with only
   the features you need. Don't add `DefaultPlugins`, a window, `CorePlugin`
   or a camera for the player: `rc_game` does that. Your own test program
   (like `rc_walk`) can keep adding them.

3. In `crates/rc_game/Cargo.toml` add your crate, and in
   `crates/rc_game/src/main.rs` add your plugin to the list marked
   `PLUGINS FROM OTHER PARTS`. That list is the only place `rc_game` names
   your crate.

## The rules every part follows

| Need | Use |
|---|---|
| Run only during play, not in menus or paused | `.run_if(playing)` |
| Order within a frame | put systems in `GameSet::Simulate` (gameplay), `GameSet::View` (cameras, things that follow), `GameSet::Hud` |
| Set up for a map | `OnEnter(GameState::Loading)`, read `CurrentLevel` and `GamePaths` |
| Tell the menus loading is slow | `LoadingGate::wait("weapons")` then `done("weapons")`, or `fail(...)` |
| Clean up when the map ends | spawn with `DespawnOnExit(GameState::InGame)` |
| Find a game file | `GamePaths::game_file("GameData/Sounds/x.uax")`, `map_path("geo_01a")`. Never `RC_GAME_DIR` or a Steam path. |
| Find our own data file (moddable) | `GamePaths::data_file("weapons/dc15s.toml")`: a mod's copy wins over `data/` |
| Player input | read `ActionState` (`pressed(Action::Fire)`, `movement`, `look`). Never read keys, mouse or gamepad. |
| Collide or trace | `LevelCollision` (Unreal units) |
| Where the player is | the entity with `Player`; the view is the entity with `PlayerCamera` |
| Show numbers on screen | write `PlayerVitals`, `WeaponReadout`, `SquadReadout`, or send a `HudMessage` |
| Units | `units::to_bevy` / `units::to_unreal` (50 Unreal units = 1 Bevy unit) |
| Logging | `Res<GameLog>`: `log.line("weapons: fired dc15s, 1 hit")` |

A new action (a button the player presses) is added to `Action` in
`rc_game_core/src/actions.rs` and given default keys in
`data/input/bindings.toml`.

## Who writes what

| Part | Writes | Reads |
|---|---|---|
| shell (`rc_game`): menus, settings, input, display, music | `GameState`, `Play`, `CurrentLevel`, `ActionState`, `GamePaths` | the readouts |
| level | `LevelCollision`, the map's look | `CurrentLevel`, `GamePaths` |
| player | the `Player` body, the `PlayerCamera` | `ActionState`, `LevelCollision` |
| weapons and combat | `WeaponReadout`, `PlayerVitals` | `ActionState`, `PlayerCamera`, `LevelCollision` |
| enemies | their own entities | `Player`, `LevelCollision` |
| squad | `SquadReadout` | `ActionState`, `Player` |
