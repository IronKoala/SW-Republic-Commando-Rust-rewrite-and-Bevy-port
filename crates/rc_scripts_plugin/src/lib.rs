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
//!   `ACTION_ChangeLevel`, or the end-of-campaign debriefing's `NewLevel`
//!   sends a [`ChangeLevel`]; the save part loads that map and carries the
//!   player over;
//! - enemy spawners (`rc_enemies`) hear every event and switch on by tag;
//!   `ACTION_StopFactorySpawning` and `ACTION_ResetFactory` become
//!   [`FactoryOrder`]s for them, and what they send back (a droid's death
//!   event, a spawner running out) arrives as a `RaiseEvent`.
//!
//! Everything else a script does (lines of dialogue, animations,
//! characters walking) is logged as "not done yet" with the script's own
//! comment. Every event and effect is logged with a `scripts:` prefix, and
//! every event is sent as a [`ScriptEvent`] message, so other parts
//! (droid factories, the squad, sounds) can hook in by name.

pub mod save;

use bevy::prelude::*;
use rc_combat::{Damage, DamageKind, Health};
use rc_game_core::save::{RestoringSave, SaveAppExt};
use rc_game_core::{
    playing, units::to_unreal, Action, ActionState, ChangeLevel, CurrentLevel, GameLog,
    GamePaths, GameSet, GameState, HudMessage, LevelScriptsRun, LoadingGate, Player,
    PlayerVitals, RaiseEvent,
};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_level_plugin::{LevelMoverData, LevelParts};
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
            .add_message::<ChangeLevel>()
            .add_save_part::<save::ScriptsSave>()
            .add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(Update, build.run_if(in_state(GameState::Loading)))
            .add_systems(
                Update,
                (bacta, run, hide_parts)
                    .chain()
                    .in_set(ScriptSystems)
                    .in_set(GameSet::Simulate)
                    .run_if(playing),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

/// Every event the level's scripting sends, by name (`levelstart`,
/// `SpawnBarricadeDroids`, `BactaUsed`, ...). Read it with a
/// `MessageReader` to hook a part of the game to the map's events.
pub use rc_game_core::ScriptEvent;

/// A script's order to the enemy spawners (see [`rc_game_core::events`]).
pub use rc_game_core::FactoryOrder;

/// A checkpoint was reached: the save part saves the game. `name` is the
/// map's `Checkpoint` actor (unique in the map). Held while the player is
/// down.
pub use rc_game_core::save::CheckpointReached;

/// The mission objectives the scripts have updated, in order: (seconds,
/// `ObjectiveNum`, `ChangeToObjNum`). What the numbers mean on screen
/// isn't worked out yet.
#[derive(Resource, Debug, Clone, Default)]
pub struct Objectives {
    pub updates: Vec<(f32, i32, i32)>,
}

struct Dispenser {
    name: String,
    tag: String,
    location: [f32; 3],
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
    started: bool,
    /// Counts for the self-tests.
    pub stats: PlayStats,
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
    pub not_done: usize,
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
struct Loaded(MapScripts, Settings);

fn begin(
    mut commands: Commands,
    mut gate: ResMut<LoadingGate>,
    paths: Res<GamePaths>,
    current: Res<CurrentLevel>,
    log: Res<GameLog>,
) {
    gate.wait(PART);
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
                .map(|l| rc_level::scripts::read(&p, &d, &l))
                .map_err(|e| e.to_string())
        });
    match read {
        Ok(m) => {
            for f in &m.failures {
                log.line(format!("{PART}: unreadable: {f}"));
            }
            commands.insert_resource(Loaded(m, settings));
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
    mut gate: ResMut<LoadingGate>,
    log: Res<GameLog>,
) {
    let (Some(loaded), Some(movers)) = (loaded, movers) else {
        return;
    };
    let Loaded(map, settings) = &*loaded;
    let director = Director::new(map, &movers.movers.triggers, settings.clone());
    let s = &director.stats;
    log.line(format!(
        "{PART}: {} scripted triggers and {} sequences with {} actions, {} triggers, {} counters, {} conditions, {} checkpoints, {} props, {} level exits; level start sends {:?}",
        s.scripts,
        s.sequences,
        s.actions,
        s.triggers,
        s.counters,
        s.conditions,
        s.checkpoints,
        s.props,
        s.exits,
        director.start_events()
    ));
    for (name, url, at, open) in director.exits() {
        log.line(format!(
            "{PART}:   level exit {name} at ({:.0}, {:.0}, {:.0}) to {url}{}",
            at[0],
            at[1],
            at[2],
            if open { "" } else { " (closed until the scripts open it)" }
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
            tag: a.obj.tag().to_string(),
            location: a.location,
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
    commands.insert_resource(LevelScript {
        director,
        dispensers,
        hidden: HashSet::new(),
        held: Vec::new(),
        started: false,
        stats: PlayStats::default(),
    });
    commands.insert_resource(Objectives::default());
    commands.remove_resource::<Loaded>();
    gate.done(PART);
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelScript>();
    commands.remove_resource::<Objectives>();
    commands.remove_resource::<Loaded>();
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

/// Bacta dispensers: hold Use near one that is switched on to heal. It
/// sends its "in use" event when you start and its "used" event when you
/// are full.
fn bacta(
    time: Res<Time>,
    script: Option<ResMut<LevelScript>>,
    actions: Res<ActionState>,
    mut player: Query<(&Transform, Option<&mut Health>), With<Player>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    let Ok((t, health)) = player.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    let feet = to_unreal(t.translation);
    let using = actions.pressed(Action::Use);
    let range = script.director.settings.bacta_range;
    let rate = script.director.settings.bacta_heal_per_second;
    let recharge = script.director.settings.bacta_recharge_seconds;
    let now = script.director.time;
    let mut health = health;
    let mut fire = Vec::new();
    let mut healed = 0.0;
    for d in &mut script.dispensers {
        d.recharge_left = (d.recharge_left - dt).max(0.0);
        let dx = feet.x - d.location[0];
        let dy = feet.y - d.location[1];
        let near = (dx * dx + dy * dy).sqrt() < range && (feet.z - d.location[2]).abs() < 256.0;
        let can = near && using && d.enabled && d.recharge_left <= 0.0;
        if can && !d.in_use {
            d.in_use = true;
            log.line(format!(
                "{PART}: t={now:.2} the player uses bacta dispenser {}",
                d.name
            ));
            if let Some(e) = &d.active_event {
                fire.push((e.clone(), d.name.clone()));
            }
        }
        if d.in_use {
            let full = match health.as_deref_mut() {
                Some(h) => {
                    let before = h.current;
                    h.current = (h.current + rate * dt).min(h.max);
                    healed += h.current - before;
                    h.current >= h.max
                }
                None => true,
            };
            if full || !can {
                d.in_use = false;
                if full {
                    d.recharge_left = recharge;
                    log.line(format!(
                        "{PART}: t={now:.2} bacta dispenser {} done: health full",
                        d.name
                    ));
                    if let Some(e) = &d.used_event {
                        fire.push((e.clone(), d.name.clone()));
                    }
                }
            }
        }
    }
    script.stats.bacta_healed += healed;
    for (e, by) in fire {
        script.director.fire(&e, &by);
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
    mut raised: MessageReader<RaiseEvent>,
    mut factories: MessageWriter<FactoryOrder>,
    player: Query<(Entity, &Transform), With<Player>>,
    vitals: Res<PlayerVitals>,
    mut events: MessageWriter<ScriptEvent>,
    (mut checkpoints, mut change): (
        MessageWriter<CheckpointReached>,
        MessageWriter<ChangeLevel>,
    ),
    mut hud: MessageWriter<HudMessage>,
    mut damage: MessageWriter<Damage>,
    restoring: Option<Res<RestoringSave>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
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
    }
    // What the doors did: their own events can start scripts.
    for m in from_movers.read() {
        if let Happened::Event { event, .. } = &m.event {
            script.director.fire(event, &format!("{} (door)", m.name));
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
                log.line(format!(
                    "{PART}: t={now:.2} prop {name} ({tag}) destroyed: no longer solid or drawn{}{}",
                    mesh.map(|m| format!("; should become {m} (not drawn yet)")).unwrap_or_default(),
                    effect.map(|m| format!("; effect {m} (not shown yet)")).unwrap_or_default()
                ));
            }
            Effect::Checkpoint { name } => {
                script.held.push(name);
            }
            Effect::ChangeLevel { url, by } => match rc_scripts::level_url(&url) {
                Some((map, portal)) => {
                    log.line(format!(
                        "{PART}: t={now:.2} {by}: the level is over; on to {map}{}",
                        portal
                            .as_ref()
                            .map(|p| format!(", arriving at {p}"))
                            .unwrap_or_default()
                    ));
                    script.stats.left_for = Some(url);
                    change.write(ChangeLevel { map, portal, by });
                }
                None => log.line(format!(
                    "{PART}: t={now:.2} {by}: ERROR can't read the level address {url:?}; staying"
                )),
            },
            Effect::Action {
                script: who,
                index,
                action,
            } => {
                carry_out(
                    script,
                    &who,
                    index,
                    &action,
                    now,
                    &parts,
                    &mut objectives,
                    &player,
                    &mut hud,
                    &mut damage,
                    &mut factories,
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
    who: &str,
    index: usize,
    a: &ScriptObject,
    now: f32,
    parts: &Option<Res<LevelParts>>,
    objectives: &mut Option<ResMut<Objectives>>,
    player: &Query<(Entity, &Transform), With<Player>>,
    hud: &mut MessageWriter<HudMessage>,
    damage: &mut MessageWriter<Damage>,
    factories: &mut MessageWriter<FactoryOrder>,
    log: &GameLog,
) {
    let kind = a.class.strip_prefix("ACTION_").unwrap_or(&a.class);
    let at = format!("{PART}: t={now:.2} {who} #{index} {kind}");
    match kind.to_ascii_lowercase().as_str() {
        "setcollision" => {
            let tag = a.text("CollideActorTag").unwrap_or("");
            let on = a.flag("bCollideActors", true);
            let n = parts.as_ref().map_or(0, |p| p.set_solid(tag, on));
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
                    "{at}: {tag} collision {on} (not a level part; characters not done yet)"
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
                script.stats.not_done += 1;
                log.line(format!(
                    "{at}: {tag} takes {amount} (characters not done yet)"
                ));
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
                script.stats.not_done += 1;
                log.line(format!("{at}: {tag} (characters not done yet)"));
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
            } else {
                script.stats.not_done += 1;
                log.line(format!("{at}: {tag} to state {state} (not done yet)"));
            }
        }
        "showprompt" => {
            let text = a.text("PromptText").unwrap_or("").to_string();
            if a.flag("bShowOnPC", true) {
                // `@` stands for the button; the original fills in its name.
                let shown = text.replace('@', "USE");
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
        "resetfactory" => {
            let tag = a.text("FactoryTag").unwrap_or("").to_string();
            log.line(format!("{at}: spawners {tag} refilled"));
            script.stats.factory_orders.push(format!("reset {tag}"));
            factories.write(FactoryOrder::Reset { tag });
        }
        "changelevel" => {
            log.line(format!(
                "{at}: on to {}",
                a.text("URL").unwrap_or("(no URL)")
            ));
        }
        "showdebriefingscreen" if a.text("NewLevel").is_some() => {
            script.stats.not_done += 1;
            log.line(format!(
                "{at}: the debriefing \"{}\" (not shown yet), then on to {}",
                a.text("Title").unwrap_or(""),
                a.text("NewLevel").unwrap_or("")
            ));
        }
        "objectiveupdate" => {
            let num = a.int("ObjectiveNum").unwrap_or(0);
            let to = a.int("ChangeToObjNum").unwrap_or(0);
            if let Some(o) = objectives {
                o.updates.push((now, num, to));
            }
            log.line(format!("{at}: objective {num} -> {to}"));
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
