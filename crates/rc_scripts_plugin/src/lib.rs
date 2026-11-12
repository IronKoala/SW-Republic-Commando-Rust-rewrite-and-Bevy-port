//! Level scripts in the game. `ScriptsPlugin` reads the map's own scripting
//! while the level loads (`rc_level::scripts`), runs it with
//! `rc_scripts::Director` during play, and carries out what it asks:
//!
//! - events go to doors and lifts by tag (`rc_movers_plugin`), and come
//!   back from them (a door's `OpenedEvent` can start a script);
//! - the map's touch triggers run here (the movers part is told to leave
//!   them alone), so one place sends every trigger's event;
//! - barricades: `ACTION_SetCollision` switches a part's collision off,
//!   a prop blown up (`ACTION_HurtActor`) or removed
//!   (`ACTION_DestroyActor`) loses its collision and is hidden
//!   (`rc_level_plugin::LevelParts`, `MoverPart`);
//! - the player: `ACTION_HurtActor` on the player sends `rc_combat`
//!   `Damage`; bacta dispensers heal while the player holds Use, and send
//!   their map events (`BactaActive.Event`, `BactaRecharging.Event`);
//!   `ACTION_GotoState` switches them off and on;
//! - prompts (`ACTION_ShowPrompt`) become `HudMessage`s;
//! - objectives (`ACTION_ObjectiveUpdate`) are kept in [`Objectives`];
//! - checkpoints send [`CheckpointReached`], held while the player is down;
//! - where every script is goes into save files (`[scripts]`, see
//!   `save.rs`). After a load the scripts carry on from there, and the
//!   level-start events aren't sent again.
//! - level exits: walking into an open exit (a `Teleporter` with a `URL`),
//!   or `ACTION_ChangeLevel` sends a [`ChangeLevel`]; the save part loads
//!   that map and carries the player over. The end-of-campaign debriefing
//!   (`ACTION_ShowDebriefingScreen` with a `NewLevel`) sends a
//!   [`rc_game_core::ShowDebriefing`] instead, holding that `ChangeLevel`
//!   for the game to send after the screen;
//! - enemy spawners (`rc_enemies`) hear every event and switch on by tag;
//!   `ACTION_StopFactorySpawning` and `ACTION_ResetFactory` become
//!   [`FactoryOrder`]s for them, and what they send back (a droid's death
//!   event, a spawner running out) arrives as a `RaiseEvent`.
//! - turrets: `ACTION_AttachToTurret` and `ACTION_SetTurretUsable` become
//!   [`TurretOrder`]s for the turret part (`rc_turrets_plugin`), whose
//!   events (a turret taken or let go) arrive as a `RaiseEvent`.
//!
//! - lines of dialogue and sounds (`ACTION_PlaySound`,
//!   `ACTION_PlayLocalSound`) play from the game's own sound packages, read
//!   while the level loads; a script waiting for a line waits for its real
//!   length (see `sounds.rs`).
//! - characters: actions that name a character, or that belong to the
//!   `ScriptedSequence` a character runs, go to that character (walk
//!   there, turn to the player, play an animation, can't be hurt, ...;
//!   see `characters.rs`). The script waits for walks and animations to
//!   finish.
//!
//! Everything else a script does (camera moves, ...) is logged as "not
//! done yet" with the script's own comment. Every event and effect is
//! logged with a `scripts:` prefix, and
//! every event is sent as a [`ScriptEvent`] message, so other parts
//! (droid factories, the squad, sounds) can hook in by name.

pub mod bacta;
pub mod breakables;
pub mod characters;
pub mod cutscene;
pub mod fx_factories;
pub mod hud_blink;
pub mod markers;
pub mod music;
pub mod objectives;
pub mod placed_emitters;
pub mod projectiles;
pub mod save;
pub mod sight;
pub mod sounds;
pub mod tracker;

use bevy::prelude::*;
use rc_combat::{Damage, DamageKind};
use rc_game_core::save::{RestoringSave, SaveAppExt};
use rc_game_core::{
    playing, units::to_unreal, Action, ChangeLevel, CurrentLevel, GameLog, GamePaths, GameSet,
    GameState, HudMessage, InputPrompts, LevelScriptsRun, LoadingGate, Player, PlayerVitals,
    RaiseEvent, TurretOrder,
};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_level_plugin::{LevelBreakables, LevelMoverData, LevelParts};
use rc_level_viewer::draw::MoverPart;
use rc_movers_plugin::{Happened, LevelMovers, MoverHappened};
use rc_scripts::{Director, Effect, PlayerBody, Settings};
use std::collections::HashSet;

/// The name this part uses in the loading gate and the log.
pub const PART: &str = "scripts";

/// The settings file in the data folders. Optional.
pub const SETTINGS_FILE: &str = "scripts/scripts.toml";

pub struct ScriptsPlugin;

/// Running the level's scripts. Put it after the player's movement, so
/// touch triggers see where the player is this frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptSystems;

impl Plugin for ScriptsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(rc_movers_plugin::TriggersRunElsewhere)
            .insert_resource(LevelScriptsRun)
            .add_message::<ScriptEvent>()
            .add_message::<RaiseEvent>()
            .add_message::<FactoryOrder>()
            .add_message::<CheckpointReached>()
            .add_message::<rc_game_core::CharacterOrder>()
            .add_message::<rc_game_core::CharacterOrderDone>()
            .add_message::<rc_game_core::SquadStanceOrder>()
            .add_message::<rc_game_core::SquadMembership>()
            .add_message::<ChangeLevel>()
            .add_message::<rc_effects_plugin::SpawnEffect>()
            .add_message::<rc_effects_plugin::PreloadEffect>()
            .add_message::<rc_effects_plugin::EffectEnded>()
            .add_message::<projectiles::SpawnProjectile>()
            .add_message::<projectiles::RemoveProjectiles>()
            .init_resource::<projectiles::Projectiles>()
            .add_message::<TurretOrder>()
            .add_message::<rc_game_core::MarkerJob>()
            .init_resource::<rc_game_core::UseClaimed>()
            .add_message::<rc_game_core::PropAnim>()
            .add_message::<rc_game_core::ShowDebriefing>()
            .add_message::<tracker::TrackerAsk>()
            .init_resource::<tracker::ObjectiveTracker>()
            .add_systems(Startup, tracker::load_look)
            .add_save_part::<save::ScriptsSave>()
            .add_save_part::<fx_factories::FxFactoriesSave>()
            .add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(
                Update,
                (build, bacta::prepare)
                    .chain()
                    .run_if(in_state(GameState::Loading)),
            )
            .init_resource::<rc_game_core::UseItem>()
            .add_systems(OnEnter(GameState::InGame), bacta::place_holograms)
            .add_systems(
                Update,
                bacta::show_holograms
                    .in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            )
            .add_systems(
                Update,
                (
                    bacta::heal,
                    bacta::zap,
                    markers::run,
                    breakables::shots,
                    breakables::blasts,
                    breakables::explode,
                    sight::look,
                    run,
                    send_ammo,
                    blink_hud,
                    tracker::track,
                    fx_factories::run,
                    placed_emitters::run,
                    projectiles::fly,
                    breakables::show,
                    characters::cast,
                    sounds::play,
                    hide_parts,
                    cutscene::apply,
                )
                    .chain()
                    .in_set(ScriptSystems)
                    .in_set(GameSet::Simulate)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                markers::apply_states
                    .in_set(ScriptSystems)
                    .in_set(GameSet::Simulate)
                    .after(run)
                    .before(hide_parts)
                    .run_if(playing),
            )
            .add_systems(Update, (sounds::started, sounds::pause, sounds::follow))
            .add_systems(OnEnter(GameState::Loading), cutscene::forget)
            .add_systems(
                OnExit(GameState::InGame),
                (
                    forget,
                    sounds::forget,
                    cutscene::forget,
                    projectiles::forget,
                    tracker::forget,
                ),
            );
    }
}

/// Every event the level's scripting sends, by name (`levelstart`,
/// `SpawnBarricadeDroids`, `BactaUsed`, ...). Read it with a
/// `MessageReader` to hook a part of the game to the map's events.
pub use rc_game_core::ScriptEvent;

pub use sounds::{ScriptSound, ScriptSounds};

/// A script's order to the enemy spawners (see [`rc_game_core::events`]).
pub use rc_game_core::FactoryOrder;

/// A checkpoint was reached: the save part saves the game. `name` is the
/// map's `Checkpoint` actor (unique in the map). Held while the player is
/// down.
pub use rc_game_core::save::CheckpointReached;

/// The mission objectives: the map's list and where each stands
/// (`mission`, see [`objectives`]), and the updates the scripts made, in
/// order: (seconds, `ObjectiveNum`, `ChangeToObjNum`).
#[derive(Resource, Debug, Clone, Default)]
pub struct Objectives {
    pub updates: Vec<(f32, i32, i32)>,
    pub mission: Option<objectives::MissionList>,
}

impl Objectives {
    /// The objective the helmet shows now, in capitals.
    pub fn shown(&self) -> Option<String> {
        self.mission.as_ref()?.shown()
    }
}

struct Dispenser {
    name: String,
    /// Its class (`BactaDispenserGEO`), whose defaults name its effects.
    class: String,
    tag: String,
    location: [f32; 3],
    /// Which way it faces, radians (Unreal yaw): its front, where the
    /// player stands to use it.
    yaw: f32,
    /// Where a character stands to use it (its `NavPts` marker), if any.
    stand: Option<[f32; 3]>,
    enabled: bool,
    active_event: Option<String>,
    used_event: Option<String>,
    in_use: bool,
    recharge_left: f32,
}

/// A bacta dispenser, for the log and self-tests.
#[derive(Debug, Clone)]
pub struct DispenserView {
    pub name: String,
    pub tag: String,
    /// Switched on (scripts switch some off until it's time).
    pub enabled: bool,
    /// Unreal units.
    pub location: [f32; 3],
    /// Where a character stands to use it, if the map says.
    pub stand: Option<[f32; 3]>,
}

/// The level's scripting while it is played.
#[derive(Resource)]
pub struct LevelScript {
    pub director: Director,
    dispensers: Vec<Dispenser>,
    /// Exports of drawn parts to hide.
    hidden: HashSet<usize>,
    /// Checkpoints waiting for the player to be up.
    held: Vec<String>,
    /// Actions on characters waiting to be carried out.
    pub calls: Vec<characters::Call>,
    /// Where scripts can send characters.
    pub places: characters::Places,
    /// Sounds the scripts asked for this frame.
    sounds: Vec<sounds::SoundRequest>,
    /// Ammo the scripts give or take this frame (`ACTION_AddAmmo`).
    ammo: Vec<rc_game_core::pickups::AddAmmo>,
    /// The HUD parts the scripts are blinking (`ACTION_BlinkHUDHealth`
    /// and the rest), read by the HUD.
    pub hud_blink: hud_blink::HudBlink,
    /// Squad marker anchors (charges, consoles, door locks) and what they
    /// set off (see `markers.rs`).
    pub anchors: Vec<markers::Anchor>,
    /// The states of the order markers and their anchors (see
    /// `rc_scripts::marker_states`).
    pub marker_states: rc_scripts::marker_states::MarkerStates,
    /// What their state changes set off, carried out by
    /// `markers::apply_states` this frame.
    pub state_effects: Vec<rc_scripts::marker_states::StateEffect>,
    /// Movers scripts send to a key (`ACTION_MoverToKeyframe`), carried
    /// out by `markers::apply_states`.
    pub mover_keys: Vec<markers::MoverKey>,
    /// Script actions the game finishes after a time: (scripts' clock,
    /// script, action).
    pub timed_done: Vec<(f32, String, usize)>,
    started: bool,
    /// What the cutscene asks of the player and the screen.
    pub scene: cutscene::Scene,
    /// Counts for the self-tests.
    pub stats: PlayStats,
    /// Props weapons can break, and explosions about to go off.
    pub breaking: breakables::Breakables,
    /// What weapons did to props.
    pub props: breakables::PropStats,
}

impl LevelScript {
    /// Just the director, with nothing else of a level: for tests.
    #[cfg(test)]
    pub(crate) fn bare(director: Director) -> Self {
        LevelScript {
            director,
            dispensers: Vec::new(),
            hidden: HashSet::new(),
            held: Vec::new(),
            calls: Vec::new(),
            places: characters::Places::new(Vec::new()),
            sounds: Vec::new(),
            ammo: Vec::new(),
            hud_blink: Default::default(),
            started: true,
            scene: cutscene::Scene::default(),
            stats: PlayStats::default(),
            breaking: breakables::Breakables::new(&[]),
            props: breakables::PropStats::default(),
            anchors: Vec::new(),
            marker_states: Default::default(),
            state_effects: Vec::new(),
            mover_keys: Vec::new(),
            timed_done: Vec::new(),
        }
    }
}

/// What the scripts did in the game, for the log and the self-tests.
#[derive(Debug, Clone, Default)]
pub struct PlayStats {
    pub parts_switched_off: Vec<String>,
    pub props_destroyed: Vec<String>,
    pub player_hurt: f32,
    pub bacta_healed: f32,
    pub prompts: Vec<String>,
    pub checkpoints: Vec<String>,
    /// Orders sent to enemy spawners: `stop TAG` or `reset TAG`.
    pub factory_orders: Vec<String>,
    /// Where the level was left for (the map's own address), if it was.
    pub left_for: Option<String>,
    /// Events sent into the scripts by other parts (droids, spawners).
    pub raised: Vec<String>,
    /// Sounds and lines played, by name, in order.
    pub sounds_played: Vec<String>,
    /// How many of them started on the sound card.
    pub sounds_started: usize,
    /// Sounds asked for that weren't loaded.
    pub sounds_missing: Vec<String>,
    /// Music actions passed on to the music.
    pub music_cues: usize,
    /// Objective tracker steps passed on to the tracker.
    pub tracker_asks: usize,
    /// The sounds played from a place in the level (not heard everywhere).
    pub sounds_placed: Vec<String>,
    pub not_done: usize,
    /// Orders sent to characters: `KIND WHO`.
    pub character_orders: Vec<String>,
    /// `PlayPropAnim`s on the player's own body (not drawn in first
    /// person, as in the original).
    pub player_body_anims: Vec<String>,
    /// Characters that finished an order the script waited for:
    /// `SCRIPT #INDEX`.
    pub character_done: Vec<String>,
    /// `MakeInvulnerable`: `WHO true|false`.
    pub invulnerable: Vec<String>,
    pub characters_hurt: usize,
    pub squad_orders: Vec<String>,
    /// `ChangeSquadMember`: `WHO joins|leaves`.
    pub squad_members: Vec<String>,
    /// Line-of-sight triggers the player set off by looking, by name.
    pub sight_seen: Vec<String>,
    /// Characters an action named that weren't found.
    pub no_character: Vec<String>,
    /// Orders sent to turrets: `attach TAG`, `detach TAG`, `usable TAG`,
    /// `unusable TAG`.
    pub turret_orders: Vec<String>,
    /// Ammo given or taken (`ACTION_AddAmmo`): `WeaponClass +N`.
    pub add_ammo: Vec<String>,
    /// HUD blinks started (`ACTION_BlinkHUD...`): `Kind seconds`.
    pub hud_blinks: Vec<String>,
    /// Squad marker job steps carried out: `ANCHOR Step`.
    pub marker_steps: Vec<String>,
    /// Locked doors opened by a breach or a slice (mover names).
    pub doors_unlocked: Vec<String>,
    /// Props a `GotoState` switched: `NAME State: what happened`.
    pub prop_states: Vec<String>,
    /// Order markers and anchors scripts sent to a state ("tag state").
    pub marker_states: Vec<String>,
    /// Movers scripts sent to a key ("tag key").
    pub mover_keys: Vec<String>,
}

impl LevelScript {
    /// The map's bacta dispensers.
    pub fn dispensers(&self) -> Vec<DispenserView> {
        self.dispensers
            .iter()
            .map(|d| DispenserView {
                name: d.name.clone(),
                tag: d.tag.clone(),
                enabled: d.enabled,
                location: d.location,
                stand: d.stand,
            })
            .collect()
    }
}

/// The map's scripting as read while loading, waiting for the movers'
/// triggers.
#[derive(Resource)]
struct Loaded(MapScripts, Settings, Vec<rc_level::scripts::Place>);

fn begin(
    mut commands: Commands,
    mut gate: ResMut<LoadingGate>,
    paths: Res<GamePaths>,
    current: Res<CurrentLevel>,
    log: Res<GameLog>,
) {
    gate.wait(PART);
    commands.remove_resource::<bacta::BactaAssets>();
    commands.remove_resource::<bacta::BactaStats>();
    let settings = load_settings(&paths, &log);
    let Some(map) = &current.map else {
        gate.fail(PART, "no map chosen");
        return;
    };
    let path = paths.map_path(map);
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
                    let m = rc_level::scripts::read(&p, &d, &l);
                    // What the map's objective tracker steps point at.
                    let targets = tracker::read_targets(&p, &d, &l, &m, &paths, &log);
                    (m, rc_level::scripts::places(&p, &d, &l), targets)
                })
                .map_err(|e| e.to_string())
        });
    match read {
        Ok((m, places, targets)) => {
            for f in &m.failures {
                log.line(format!("{PART}: unreadable: {f}"));
            }
            commands.insert_resource(targets);
            commands.insert_resource(Loaded(m, settings, places));
        }
        Err(e) => {
            // No scripts is not fatal: the level plays without them.
            log.line(format!(
                "{PART}: ERROR reading {}: {e}; no level scripts",
                path.display()
            ));
            gate.done(PART);
        }
    }
}

fn load_settings(paths: &GamePaths, log: &GameLog) -> Settings {
    let Some(path) = paths.data_file(SETTINGS_FILE) else {
        log.line(format!("{PART}: no {SETTINGS_FILE}; built-in settings"));
        return Settings::default();
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| Settings::default().with_overrides(&t))
    {
        Ok(s) => {
            log.line(format!("{PART}: settings from {}", path.display()));
            s
        }
        Err(e) => {
            log.line(format!(
                "{PART}: ERROR in {}: {e}; built-in settings",
                path.display()
            ));
            Settings::default()
        }
    }
}

fn struct_event(o: &ScriptObject, key: &str) -> Option<String> {
    match o.structure(key)?.get("Event") {
        Some(Val::Text(t)) if !t.eq_ignore_ascii_case("None") => Some(t.clone()),
        _ => None,
    }
}

/// Once the level part has read the triggers, build the director.
fn build(
    mut commands: Commands,
    loaded: Option<Res<Loaded>>,
    movers: Option<Res<LevelMoverData>>,
    breakable: Option<Res<LevelBreakables>>,
    mut gate: ResMut<LoadingGate>,
    paths: Res<GamePaths>,
    audio: Option<ResMut<Assets<bevy::audio::AudioSource>>>,
    mut preload: MessageWriter<rc_effects_plugin::PreloadEffect>,
    log: Res<GameLog>,
) {
    let (Some(loaded), Some(movers)) = (loaded, movers) else {
        return;
    };
    let Loaded(map, settings, places) = &*loaded;
    let mut director = Director::new(map, &movers.movers.triggers, settings.clone());
    // Props weapons can break get the game's full damage rules.
    let breakable = breakable.map(|b| b.0.clone()).unwrap_or_default();
    director.add_breakables(&breakable);
    // Their explosions and sounds are read now, not at the first bang.
    breakables::preload(&breakable, &mut preload);
    // Characters really walk and animate in the game: scripts wait for them.
    director.game_runs_characters = true;
    let places = characters::Places::new(places.clone());
    // The lines and sounds the scripts play, and how long each lasts.
    // Without audio (tests) there's nothing to read them into.
    match audio {
        Some(mut audio) => {
            let s = sounds::load(map, &paths, &mut audio, &log);
            for (name, secs) in &s.lengths {
                director.set_sound_length(name, *secs);
            }
            commands.insert_resource(s);
        }
        None => log.line(format!("{PART}: no audio; script sounds not read")),
    }
    let s = &director.stats;
    log.line(format!(
        "{PART}: {} scripted triggers and {} sequences with {} actions, {} triggers, {} line-of-sight triggers, {} counters, {} conditions, {} checkpoints, {} props ({} weapons can break), {} level exits; level start sends {:?}",
        s.scripts,
        s.sequences,
        s.actions,
        s.triggers,
        s.sight_triggers,
        s.counters,
        s.conditions,
        s.checkpoints,
        s.props,
        s.breakables,
        s.exits,
        director.start_events()
    ));
    for (name, url, at, open) in director.exits() {
        log.line(format!(
            "{PART}:   level exit {name} at ({:.0}, {:.0}, {:.0}) to {url}{}",
            at[0],
            at[1],
            at[2],
            if open {
                ""
            } else {
                " (closed until the scripts open it)"
            }
        ));
    }
    let dispensers: Vec<Dispenser> = map
        .actors
        .iter()
        .filter(|a| {
            a.obj
                .class
                .to_ascii_lowercase()
                .starts_with("bactadispenser")
        })
        .map(|a| Dispenser {
            name: a.obj.name.clone(),
            class: a.obj.class.clone(),
            tag: a.obj.tag().to_string(),
            location: a.location,
            yaw: match a.obj.get("Rotation") {
                Some(Val::Raw(b)) if b.len() >= 8 => {
                    i32::from_le_bytes([b[4], b[5], b[6], b[7]]) as f32 * std::f32::consts::TAU
                        / 65536.0
                }
                _ => 0.0,
            },
            stand: match a.obj.get("NavPts") {
                Some(Val::List(l)) => l.first().and_then(|n| {
                    map.actors
                        .iter()
                        .find(|b| b.obj.name.eq_ignore_ascii_case(n))
                        .map(|b| b.location)
                }),
                _ => None,
            },
            enabled: true,
            active_event: struct_event(&a.obj, "BactaActive"),
            used_event: struct_event(&a.obj, "BactaRecharging"),
            in_use: false,
            recharge_left: 0.0,
        })
        .collect();
    for d in &dispensers {
        log.line(format!(
            "{PART}:   bacta dispenser {} (tag {}) at ({:.0}, {:.0}, {:.0}), sends {:?} when used and {:?} when done",
            d.name, d.tag, d.location[0], d.location[1], d.location[2], d.active_event, d.used_event
        ));
    }
    log.line(format!(
        "{PART}: {} places characters can be sent to",
        places.len()
    ));
    let anchors = markers::read_anchors(map);
    let marker_states = rc_scripts::marker_states::MarkerStates::read(map);
    log.line(format!(
        "{PART}: {} order markers and anchors with states",
        marker_states.actors.len()
    ));
    for a in &anchors {
        log.line(format!(
            "{PART}:   marker anchor {} ({}): started {:?}, armed {:?}, done {:?}{}{}",
            a.name,
            a.class,
            a.started,
            a.armed,
            a.done,
            a.destroys
                .as_ref()
                .map(|(p, _)| format!(", blows away {p}"))
                .unwrap_or_default(),
            if a.doors.is_empty() {
                String::new()
            } else {
                format!(", opens doors {:?}", a.doors)
            }
        ));
    }
    // Effect spawners: read now, their effects too, so the first doesn't
    // stall the game.
    let fx = fx_factories::read(map);
    fx_factories::log_list(&fx, &log);
    let mut wanted: Vec<&str> = fx
        .iter()
        .filter(|f| f.kind == rc_effects::factory::Kind::Effect)
        .map(|f| f.prototype.as_str())
        .collect();
    wanted.sort_unstable_by_key(|n| n.to_ascii_lowercase());
    wanted.dedup_by_key(|n| n.to_ascii_lowercase());
    for w in wanted {
        preload.write(rc_effects_plugin::PreloadEffect {
            effect: Some(w.to_string()),
            sound: None,
        });
    }
    commands.insert_resource(fx_factories::LevelFxFactories::new(fx));
    // Emitters placed in the map (geo_01a's dust under the landing
    // gunship): found by their class, their effects read now.
    let emitters = match rc_effects::read::Library::open(&paths.game_data()) {
        Ok(lib) => placed_emitters::read(map, &|c: &str| lib.is_emitter(c)),
        Err(e) => {
            log.line(format!(
                "{PART}: effect packages not read ({e}); no placed emitters"
            ));
            Vec::new()
        }
    };
    placed_emitters::log_list(&emitters, &log);
    let mut classes: Vec<&str> = emitters.iter().map(|e| e.class.as_str()).collect();
    classes.sort_unstable_by_key(|n| n.to_ascii_lowercase());
    classes.dedup_by_key(|n| n.to_ascii_lowercase());
    for c in classes {
        preload.write(rc_effects_plugin::PreloadEffect {
            effect: Some(c.to_string()),
            sound: None,
        });
    }
    commands.insert_resource(placed_emitters::LevelEmitters::new(emitters));
    commands.insert_resource(LevelScript {
        director,
        dispensers,
        hidden: HashSet::new(),
        held: Vec::new(),
        calls: Vec::new(),
        places,
        sounds: Vec::new(),
        ammo: Vec::new(),
        hud_blink: Default::default(),
        anchors,
        marker_states,
        state_effects: Vec::new(),
        mover_keys: Vec::new(),
        timed_done: Vec::new(),
        started: false,
        scene: cutscene::Scene::default(),
        stats: PlayStats::default(),
        breaking: breakables::Breakables::new(&breakable),
        props: breakables::PropStats::default(),
    });
    let mission = objectives::MissionList::from_map(map);
    if let Some(m) = &mission {
        log.line(format!(
            "{PART}: {} mission objectives, revealed by {}",
            m.list.len(),
            m.tag
        ));
    }
    commands.insert_resource(Objectives {
        mission,
        ..Objectives::default()
    });
    commands.remove_resource::<Loaded>();
    gate.done(PART);
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelScript>();
    commands.remove_resource::<fx_factories::LevelFxFactories>();
    commands.remove_resource::<placed_emitters::LevelEmitters>();
    commands.remove_resource::<Objectives>();
    commands.remove_resource::<Loaded>();
    commands.remove_resource::<tracker::TrackerTargets>();
}

fn text_of(v: Option<&Val>) -> String {
    match v {
        Some(Val::Text(t)) => t.clone(),
        Some(Val::Float(f)) => format!("{f}"),
        Some(Val::Int(i)) => format!("{i}"),
        Some(Val::Byte(b)) => format!("{b}"),
        Some(Val::Bool(b)) => format!("{b}"),
        _ => String::new(),
    }
}

/// Step the director and carry out what it asks.
#[allow(clippy::too_many_arguments)]
fn run(
    time: Res<Time>,
    script: Option<ResMut<LevelScript>>,
    movers: Option<Res<LevelMovers>>,
    parts: Option<Res<LevelParts>>,
    mut objectives: Option<ResMut<Objectives>>,
    mut from_movers: MessageReader<MoverHappened>,
    // One tuple: Bevy takes at most 16 inputs per system.
    (mut raised, mut finished): (
        MessageReader<RaiseEvent>,
        MessageReader<rc_game_core::CharacterOrderDone>,
    ),
    (mut factories, mut turrets, mut collisions): (
        MessageWriter<FactoryOrder>,
        MessageWriter<TurretOrder>,
        MessageWriter<rc_game_core::CharacterCollision>,
    ),
    player: Query<(Entity, &Transform), With<Player>>,
    vitals: Res<PlayerVitals>,
    mut events: MessageWriter<ScriptEvent>,
    (mut checkpoints, mut change, mut debrief, mut music, mut tracker_asks): (
        MessageWriter<CheckpointReached>,
        MessageWriter<ChangeLevel>,
        MessageWriter<rc_game_core::ShowDebriefing>,
        MessageWriter<rc_game_core::MusicCue>,
        MessageWriter<tracker::TrackerAsk>,
    ),
    mut hud: MessageWriter<HudMessage>,
    mut damage: MessageWriter<Damage>,
    (restoring, prompts): (Option<Res<RestoringSave>>, Option<Res<InputPrompts>>),
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    // The Use button's name on the device used last, for prompts' `@`.
    let use_key = prompts
        .as_ref()
        .and_then(|p| p.label(Action::Use))
        .unwrap_or("USE")
        .to_string();
    // A save is being put back: its [scripts] section says where the
    // scripts are, so nothing starts or runs until it has.
    if restoring.is_some() {
        return;
    }
    let script = &mut *script;
    if !script.started {
        script.started = true;
        log.line(format!(
            "{PART}: level start: sending {:?}",
            script.director.start_events()
        ));
        script.director.start();
        let fx = script.marker_states.start();
        script.state_effects.extend(fx);
    }
    // What the doors did: their own events can start scripts.
    for m in from_movers.read() {
        if let Happened::Event { event, .. } = &m.event {
            script.director.fire(event, &format!("{} (door)", m.name));
        }
    }
    // Characters that finished what a script waits for.
    for d in finished.read() {
        if script.director.action_done(&d.script, d.index) {
            log.line(format!(
                "{PART}: t={:.2} {} #{}: the character is done; the script carries on",
                script.director.time, d.script, d.index
            ));
            script
                .stats
                .character_done
                .push(format!("{} #{}", d.script, d.index));
        }
    }
    // Events from other parts' actors (a droid's death, a spawner done).
    for r in raised.read() {
        log.line(format!(
            "{PART}: t={:.2} {} sends {}",
            script.director.time, r.by, r.name
        ));
        script.stats.raised.push(r.name.clone());
        script.director.fire(&r.name, &r.by);
    }
    let body = player.single().ok().map(|(_, t)| {
        let feet = to_unreal(t.translation);
        PlayerBody {
            location: [feet.x, feet.y, feet.z + 84.0],
            radius: 40.0,
            half_height: 84.0,
        }
    });
    let dt = time.delta_secs().min(0.25);
    let effects = script.director.step(dt, body);
    let now = script.director.time;
    for e in effects {
        match e {
            Effect::Note(n) => log.line(format!("{PART}: t={now:.2} {n}")),
            Effect::Event { name, by } => {
                if !by.ends_with("(door)") {
                    if let Some(m) = &movers {
                        let n = m.send(&name);
                        if n > 0 {
                            log.line(format!(
                                "{PART}: t={now:.2} {name} opens {n} doors or lifts"
                            ));
                        }
                    }
                }
                if let Some(i) = objectives
                    .as_mut()
                    .and_then(|o| o.mission.as_mut())
                    .and_then(|m| m.event(&name))
                {
                    log.line(format!("{PART}: t={now:.2} objective {i} revealed"));
                }
                events.write(ScriptEvent { name, by });
            }
            Effect::Untrigger { name, by } => {
                if let Some(m) = &movers {
                    m.release(&name);
                }
                script.director.untrigger(&name);
                log.line(format!(
                    "{PART}: t={now:.2} {by}: the player left; {name} is over"
                ));
            }
            Effect::PropDestroyed {
                export,
                name,
                tag,
                mesh,
                effect,
            } => {
                if let Some(p) = &parts {
                    p.set_solid_export(export, false);
                }
                script.hidden.insert(export);
                script.stats.props_destroyed.push(tag.clone());
                // Explosions go off 0.1 to 0.2 s later, as in the game.
                let delay = 0.1 + 0.1 * ((export as f32 * 0.618).fract());
                let explodes = script.breaking.broke(export, now, delay);
                script.breaking.stepped.push((export, true));
                let sound = script
                    .breaking
                    .get(export)
                    .and_then(|b| b.destroyed.sound.clone());
                log.line(format!(
                    "{PART}: t={now:.2} prop {name} ({tag}) destroyed: no longer solid or drawn{}{}{}{}",
                    if explodes { "; explodes in a moment" } else { "" },
                    mesh.map(|m| format!("; becomes {m}")).unwrap_or_default(),
                    effect.map(|m| format!("; effect {m}")).unwrap_or_default(),
                    sound.map(|m| format!("; sound {m}")).unwrap_or_default()
                ));
            }
            Effect::PropDamaged {
                export,
                name,
                tag,
                removed,
            } => {
                script.breaking.stepped.push((export, false));
                if removed {
                    if let Some(p) = &parts {
                        p.set_solid_export(export, false);
                    }
                    script.hidden.insert(export);
                    script.stats.props_destroyed.push(tag.clone());
                }
                let look = script
                    .breaking
                    .get(export)
                    .and_then(|b| b.damaged.mesh.clone());
                log.line(format!(
                    "{PART}: t={now:.2} prop {name} ({tag}) damaged{}",
                    if removed {
                        "; it has no damaged look, so it is gone (no longer solid or drawn)"
                            .to_string()
                    } else {
                        look.map(|m| format!("; should become {m} (not drawn yet)"))
                            .unwrap_or_default()
                    }
                ));
            }
            Effect::Checkpoint { name } => {
                script.held.push(name);
            }
            Effect::ChangeLevel {
                url,
                by,
                debriefing,
            } => match rc_scripts::level_url(&url) {
                Some((map, portal)) => {
                    log.line(format!(
                        "{PART}: t={now:.2} {by}: the level is over; on to {map}{}",
                        portal
                            .as_ref()
                            .map(|p| format!(", arriving at {p}"))
                            .unwrap_or_default()
                    ));
                    script.stats.left_for = Some(url);
                    let then = ChangeLevel { map, portal, by };
                    match debriefing {
                        // The campaign's end: the game shows the
                        // debriefing first and goes on at CONTINUE.
                        Some(d) => {
                            log.line(format!(
                                "{PART}: t={now:.2} debriefing {:?} ({}, picture {}) before going on",
                                d.title, d.menu_class, d.picture
                            ));
                            debrief.write(rc_game_core::ShowDebriefing {
                                menu_class: d.menu_class,
                                title: d.title,
                                text: d.text,
                                picture: d.picture,
                                then,
                            });
                        }
                        None => {
                            change.write(then);
                        }
                    }
                }
                None => log.line(format!(
                    "{PART}: t={now:.2} {by}: ERROR can't read the level address {url:?}; staying"
                )),
            },
            Effect::Action {
                script: who,
                index,
                action,
                owner,
                waits,
            } => {
                let call = characters::Call {
                    script: who,
                    index,
                    kind: action
                        .class
                        .strip_prefix("ACTION_")
                        .unwrap_or(&action.class)
                        .to_ascii_lowercase(),
                    action,
                    owner,
                    waits,
                    deadline: now + script.director.settings.character_grace_seconds,
                };
                // Cutscenes: holding the player, fades, the player's
                // teleports.
                if cutscene::KINDS.contains(&call.kind.as_str()) || call.kind == "teleporttopoint" {
                    let at = format!(
                        "{PART}: t={now:.2} {} #{} {}",
                        call.script, call.index, call.kind
                    );
                    let settings = &script.director.settings;
                    let places = &script.places;
                    if script.scene.take(
                        &call.action,
                        now,
                        |t| settings.is_player(t),
                        |d| places.find(d).map(|p| (p.location, p.yaw)),
                        &at,
                        &log,
                    ) {
                        continue;
                    }
                }
                if characters::KINDS.contains(&call.kind.as_str()) {
                    script.calls.push(call);
                    continue;
                }
                if let Some(cue) = music::cue(&call.action) {
                    log.line(format!(
                        "{PART}: t={now:.2} {} #{} {}: music {cue:?}",
                        call.script, call.index, call.kind
                    ));
                    script.stats.music_cues += 1;
                    music.write(cue);
                    continue;
                }
                if let Some(tag) = tracker::asked(&call.action) {
                    script.stats.tracker_asks += 1;
                    tracker_asks.write(tracker::TrackerAsk {
                        tag,
                        from: format!("t={now:.2} {} #{}", call.script, call.index),
                    });
                    continue;
                }
                carry_out(
                    script,
                    call,
                    now,
                    &parts,
                    &mut objectives,
                    &player,
                    &mut hud,
                    &mut damage,
                    &mut factories,
                    &mut turrets,
                    &mut collisions,
                    &use_key,
                    &log,
                );
            }
        }
    }
    // Checkpoints wait while the player is down, as in the original.
    if !vitals.incapacitated && !script.held.is_empty() {
        for name in std::mem::take(&mut script.held) {
            log.line(format!(
                "{PART}: t={now:.2} checkpoint {name}: save the game"
            ));
            script.stats.checkpoints.push(name.clone());
            checkpoints.write(CheckpointReached { name });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn carry_out(
    script: &mut LevelScript,
    call: characters::Call,
    now: f32,
    parts: &Option<Res<LevelParts>>,
    objectives: &mut Option<ResMut<Objectives>>,
    player: &Query<(Entity, &Transform), With<Player>>,
    hud: &mut MessageWriter<HudMessage>,
    damage: &mut MessageWriter<Damage>,
    factories: &mut MessageWriter<FactoryOrder>,
    turrets: &mut MessageWriter<TurretOrder>,
    collisions: &mut MessageWriter<rc_game_core::CharacterCollision>,
    use_key: &str,
    log: &GameLog,
) {
    let a = &call.action;
    let kind = a.class.strip_prefix("ACTION_").unwrap_or(&a.class);
    let at = format!("{PART}: t={now:.2} {} #{} {kind}", call.script, call.index);
    match kind.to_ascii_lowercase().as_str() {
        "setcollision" => {
            let tag = a.text("CollideActorTag").unwrap_or("");
            let on = a.flag("bCollideActors", true);
            let n = parts.as_ref().map_or(0, |p| p.set_solid(tag, on));
            // Characters with that tag too (the parts that own them apply
            // it; nothing happens if there are none).
            if !script.director.is_teleporter(tag) && !tag.is_empty() {
                collisions.write(rc_game_core::CharacterCollision {
                    tag: tag.to_string(),
                    on,
                });
            }
            if script.director.is_teleporter(tag) {
                log.line(format!(
                    "{at}: level exit {tag} {}",
                    if on { "open" } else { "closed" }
                ));
            } else if n > 0 {
                if !on {
                    script.stats.parts_switched_off.push(tag.to_string());
                }
                log.line(format!(
                    "{at}: {tag} {} ({n} parts)",
                    if on { "solid again" } else { "no longer solid" }
                ));
            } else {
                script.stats.not_done += 1;
                log.line(format!(
                    "{at}: {tag} collision {on} (not a level part; sent to the characters with that tag)"
                ));
            }
        }
        "hurtactor" => {
            let tag = a.text("TargetTag").unwrap_or("");
            let amount = a.float("amount").unwrap_or(0.0);
            if script.director.settings.is_player(tag) {
                if let Ok((e, t)) = player.single() {
                    damage.write(Damage {
                        target: e,
                        amount,
                        kind: if a.text("Type").is_some_and(|t| t.contains("Explosion")) {
                            DamageKind::Explosion
                        } else {
                            DamageKind::Other
                        },
                        instigator: None,
                        point: t.translation,
                        direction: Vec3::NEG_Y,
                    });
                    script.stats.player_hurt += amount;
                    log.line(format!("{at}: the player takes {amount} damage"));
                }
            } else if script.director.props_standing(tag) == 0
                && !script
                    .stats
                    .props_destroyed
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(tag))
            {
                // Not a prop: characters with that tag.
                script.calls.push(call.clone());
            }
        }
        "destroyactor" | "sethidden" => {
            let tag = if kind.eq_ignore_ascii_case("destroyactor") {
                a.text("DestroyTag")
            } else {
                a.text("HideActorTag")
            }
            .unwrap_or("");
            let hide = kind.eq_ignore_ascii_case("destroyactor") || a.flag("bHidden", false);
            let mut n = 0;
            if let Some(p) = parts {
                for (export, _, t, _, _) in p.list() {
                    if t.eq_ignore_ascii_case(tag) {
                        n += 1;
                        if hide {
                            script.hidden.insert(export);
                        } else {
                            script.hidden.remove(&export);
                        }
                        if kind.eq_ignore_ascii_case("destroyactor") {
                            p.set_solid_export(export, false);
                        }
                    }
                }
            }
            if n == 0 {
                // Not a level part: characters with that tag.
                script.calls.push(call.clone());
            } else {
                log.line(format!(
                    "{at}: {tag} {} ({n} parts)",
                    if hide { "hidden" } else { "shown" }
                ));
            }
        }
        "gotostate" => {
            let tag = a.text("TargetTag").unwrap_or("");
            let state = a.text("State").unwrap_or("");
            let on = !state.eq_ignore_ascii_case("Disabled");
            let mut n = 0;
            for d in script
                .dispensers
                .iter_mut()
                .filter(|d| d.tag.eq_ignore_ascii_case(tag))
            {
                d.enabled = on;
                n += 1;
            }
            if n > 0 {
                log.line(format!(
                    "{at}: bacta dispenser {tag} {}",
                    if on { "switched on" } else { "switched off" }
                ));
            }
            // Order markers and their anchors (charges, consoles, door
            // locks).
            let (k, fx) = script.marker_states.goto_state(tag, state);
            if k > 0 {
                log.line(format!(
                    "{at}: {k} order markers or anchors tagged {tag} to {state}"
                ));
                script.stats.marker_states.push(format!("{tag} {state}"));
            }
            script.state_effects.extend(fx);
            n += k;
            // Props (engine.u `Prop`'s states).
            let props = script.director.props_goto_state(tag, state);
            for (name, what) in &props {
                log.line(format!(
                    "{at}: prop {name} (tag {tag}) to {state:?}: {what}"
                ));
                script
                    .stats
                    .prop_states
                    .push(format!("{name} {state}: {what}"));
            }
            n += props.len();
            if n == 0 {
                script.stats.not_done += 1;
                log.line(format!("{at}: {tag} to state {state} (not done yet)"));
            }
        }
        "movertokeyframe" => {
            let tag = a.text("MoverTag").unwrap_or("").to_string();
            let key = a.int("KeyFrame").unwrap_or(0).max(0) as usize;
            log.line(format!("{at}: movers {tag} to key {key}"));
            script.mover_keys.push(markers::MoverKey {
                tag,
                key,
                waits: call.waits,
                script: call.script.clone(),
                index: call.index,
            });
        }
        "setcancancelmarker" => {
            let tag = a.text("MarkerTag").unwrap_or("");
            let can = a.flag("bCanCancel", true);
            let names = script.marker_states.markers_tagged(tag);
            if names.is_empty() {
                log.line(format!("{at}: {tag}: no order marker has that tag"));
            }
            for name in names {
                log.line(format!(
                    "{at}: {name} {} be cancelled once under way",
                    if can { "can" } else { "can't" }
                ));
                script
                    .state_effects
                    .push(rc_scripts::marker_states::StateEffect::CanCancel { name, can });
            }
        }
        "showmarker" => {
            // Only the marker's icons (engine.u `AlwaysVisible`,
            // `NotifyTargeted`): the port doesn't draw order markers' icons.
            let tag = a.text("MarkerTag").unwrap_or("");
            log.line(format!(
                "{at}: {tag} icons {} (not done yet: the port draws no marker icons)",
                if a.flag("bShow", true) {
                    "shown"
                } else {
                    "hidden"
                }
            ));
            script.stats.not_done += 1;
        }
        "showprompt" => {
            let text = a.text("PromptText").unwrap_or("").to_string();
            if a.flag("bShowOnPC", true) {
                // `@` stands for the Use button; the original fills in its
                // name, as we do (F on the keyboard, X on an Xbox pad, ...).
                let shown = text.replace('@', use_key);
                hud.write(HudMessage {
                    text: shown.clone(),
                    seconds: a.float("PromptTime").unwrap_or(5.0),
                });
                script.stats.prompts.push(shown.clone());
                log.line(format!("{at}: shows \"{shown}\""));
            }
        }
        "stopfactoryspawning" => {
            let tag = a.text("FactoryTag").unwrap_or("").to_string();
            let for_good = a.flag("PermanentlyDisableFactory", false);
            let remove_offspring = a.flag("DestroyFactoryOffspring", false);
            log.line(format!(
                "{at}: spawners {tag} stop{}{}",
                if for_good { " for good" } else { "" },
                if remove_offspring {
                    ", what they made is removed"
                } else {
                    ""
                }
            ));
            script.stats.factory_orders.push(format!("stop {tag}"));
            factories.write(FactoryOrder::Stop {
                tag,
                for_good,
                remove_offspring,
            });
        }
        // Turrets are the turret part's (rc_turrets_plugin).
        "attachtoturret" => {
            let turret = a.text("DestinationTag").unwrap_or("").to_string();
            let attach = a.flag("Attach", true);
            log.line(format!(
                "{at}: the characters running {} get {} turret {turret}",
                call.owner.as_deref().unwrap_or("(no sequence)"),
                if attach { "on" } else { "off" }
            ));
            script.stats.turret_orders.push(format!(
                "{} {turret}",
                if attach { "attach" } else { "detach" }
            ));
            turrets.write(TurretOrder::Attach {
                turret,
                attach,
                owner: call.owner.clone(),
                by: format!("{} #{}", call.script, call.index),
            });
        }
        "setturretusable" => {
            let turret = a.text("TurretTag").unwrap_or("").to_string();
            let usable = a.flag("IsPlayerUsable", true);
            log.line(format!(
                "{at}: turrets {turret} {} by the player",
                if usable { "usable" } else { "not usable" }
            ));
            script.stats.turret_orders.push(format!(
                "{} {turret}",
                if usable { "usable" } else { "unusable" }
            ));
            turrets.write(TurretOrder::SetUsable {
                turret,
                usable,
                by: format!("{} #{}", call.script, call.index),
            });
        }
        "resetfactory" => {
            let tag = a.text("FactoryTag").unwrap_or("").to_string();
            log.line(format!("{at}: spawners {tag} refilled"));
            script.stats.factory_orders.push(format!("reset {tag}"));
            factories.write(FactoryOrder::Reset { tag });
        }
        "playsound" | "playlocalsound" => {
            if a.flag("bDisabled", false) {
                log.line(format!("{at}: switched off in the map; not played"));
            } else {
                let volume = script.director.settings.sound_volume;
                script.sounds.push(sounds::request(
                    &call.script,
                    call.index,
                    a,
                    volume,
                    call.owner.clone(),
                ));
            }
        }
        // Read by the HUD (rc_game's original HUD and data/ui/hud.toml).
        k if hud_blink::HudBlink::kind_of(k).is_ok() => {
            let time = a.float("BlinkTime").unwrap_or(0.0);
            let what = kind.strip_prefix("BlinkHUD").unwrap_or(kind);
            script.stats.hud_blinks.push(format!("{what} {time}"));
            match hud_blink::HudBlink::kind_of(k) {
                Ok(Some(part)) => {
                    log.line(format!("{at}: the HUD's {part} blinks for {time} s"));
                    script.hud_blink.start(part, time);
                }
                _ => log.line(format!(
                    "{at}: blink the squad command prompt for {time} s: the original shows nothing for it"
                )),
            }
        }
        // Handed to the combat part (rc_combat::script_ammo), which keeps
        // the count between none and the most carried.
        "addammo" => {
            let class = a.text("WhichWeapon").unwrap_or("").to_string();
            let amount = a.int("AmmoAmount").unwrap_or(0);
            if class.is_empty() {
                log.line(format!("{at}: no weapon named; nothing happens"));
            } else {
                log.line(format!("{at}: {amount:+} for the player's {class}"));
                script.stats.add_ammo.push(format!("{class} {amount:+}"));
                script.ammo.push(rc_game_core::pickups::AddAmmo {
                    weapon_class: class,
                    amount,
                    by: format!("{} #{}", call.script, call.index),
                });
            }
        }
        "changelevel" => {
            log.line(format!(
                "{at}: on to {}",
                a.text("URL").unwrap_or("(no URL)")
            ));
        }
        "showdebriefingscreen" if a.text("NewLevel").is_some() => {
            log.line(format!(
                "{at}: the debriefing \"{}\", then on to {}",
                a.text("Title").unwrap_or(""),
                a.text("NewLevel").unwrap_or("")
            ));
        }
        "objectiveupdate" => {
            let num = a.int("ObjectiveNum").unwrap_or(0);
            let to = a.int("ChangeToObjNum").unwrap_or(0);
            let action = a.int("UpdateAction").unwrap_or(0).clamp(0, 2) as u8;
            if let Some(o) = objectives {
                o.updates.push((now, num, to));
                if let Some(m) = &mut o.mission {
                    m.update(action, num, to);
                }
            }
            log.line(format!(
                "{at}: objective {num} {} {to}",
                ["completed", "failed", "changed to"][action as usize]
            ));
        }
        _ => {
            script.stats.not_done += 1;
            let what: Vec<String> = a
                .values
                .iter()
                .filter(|(k, _)| k.as_str() != "Comment")
                .map(|(k, v)| format!("{k}={}", text_of(Some(v))))
                .collect();
            let comment = a
                .text("Comment")
                .map(|c| format!(" \"{c}\""))
                .unwrap_or_default();
            log.line(format!("{at} {}{comment} (not done yet)", what.join(" ")));
        }
    }
}

/// The HUD parts the scripts blink run their time out (the original adds
/// the time between drawings of the HUD).
fn blink_hud(script: Option<ResMut<LevelScript>>, time: Res<Time>) {
    let Some(mut script) = script else { return };
    if script.hud_blink != hud_blink::HudBlink::default() {
        script.hud_blink.tick(time.delta_secs());
    }
}

/// Hand the ammo the scripts gave or took to the combat part.
fn send_ammo(
    script: Option<ResMut<LevelScript>>,
    mut out: MessageWriter<rc_game_core::pickups::AddAmmo>,
) {
    let Some(mut script) = script else { return };
    if !script.ammo.is_empty() {
        out.write_batch(std::mem::take(&mut script.ammo));
    }
}

/// Hide the drawn parts scripts removed.
fn hide_parts(script: Option<Res<LevelScript>>, mut parts: Query<(&MoverPart, &mut Visibility)>) {
    let Some(script) = script else { return };
    if script.hidden.is_empty() {
        return;
    }
    for (p, mut v) in &mut parts {
        if script.hidden.contains(&p.0) && *v != Visibility::Hidden {
            *v = Visibility::Hidden;
        }
    }
}
