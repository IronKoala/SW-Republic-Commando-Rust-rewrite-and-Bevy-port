//! Consoles, door locks and demolition charges the player uses himself:
//! the original's `ActivateItem`s (`rc_scripts::panels` has their states
//! and rules; this part feeds them the game and carries out what they
//! ask).
//!
//! As the original's `CTPlayer` does it (ctgame.u, ctgame.dll):
//!
//! - **What the player aims at** (`SelectTargetType`, `TGT_Panel`): the
//!   look from the eye (10000 units) meets the item's own shape before
//!   anything else; no squad mate is on its job (its squad marker isn't
//!   active); the player isn't on a turret; it is usable, and usable by
//!   the player; it is nearer the eye than its `PlayerRange`; and the
//!   player is in front of it (`Vector(Rotation)` dotted with the view
//!   below `ActivateDotProdMax`). An armed demolition charge instead is a
//!   plunger target (`TGT_DemolitionReady`) at any distance.
//! - **The HUD** while aiming at one: its prompt (`ActivatePromptText`,
//!   `@` the Use button), the ring filled to its progress
//!   (`FractionComplete`) with its squad marker's icon (none without a
//!   marker), and its clock (`DurationRemaining`, see
//!   `rc_game_core::timer_text`).
//! - **Pressing Use** (`CTPlayer.Use`, case `TGT_Panel`): the item's
//!   `EventUnderway`, then, if it can be used, the hands come up
//!   (`StartHandSignal`) and the player is held (`UsingItem`, see
//!   `rc_game_core::UseHold`). The job starts (`StartSetup`) once the
//!   hands are up, or for a breach or demolition charge when the hands put
//!   the charge on (the animation's `HolsterAttachment` moment). Use must
//!   stay held; letting go stops (`AbortSetup`).
//! - **The plunger** (`TGT_DemolitionReady`): Use brings the hands up with
//!   the plunger (`Detonate`); the charge goes off at the animation's
//!   `FireWeapon` moment.
//! - **What the items ask** ([`rc_scripts::panels::Out`]): events go to
//!   the level's scripts, the transition's sound and effect play, its
//!   damage hurts what is near (characters and breakable props), doors
//!   open, the prop a charge is on is blown away, a used-up charge is
//!   hidden.
//!
//! Not done yet (each said in the log when it would happen): the item's
//! look changing (`Mesh`, `Material` of a state, the charge shown on a
//! breach lock), the AI hearing a breach (`PostStimulusToWorld`), the
//! doors' own trigger behaviour after a slice (`TriggerControl`: they
//! open at once here and stay open), and the helmet's detonate icon over
//! an armed charge.

use bevy::prelude::*;
use rc_combat::{ArmsReadout, DamageKind, Explode, WeaponLibrary};
use rc_effects_plugin::{PreloadEffect, SpawnEffect};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    Action, ActionState, GameLog, HandNotify, InputPrompts, LevelCollision, Manning, MarkerBusy,
    MarkerObsolete, Player, PlayerCamera, UseHold, UseIcon, UseItem, UsePanel,
};
use rc_level_plugin::{ItemShapes, LevelParts};
use rc_movers_plugin::LevelMovers;
use rc_scripts::panels::{DoorAct, Kind, Out, Panels, State, User, AIT_PLUNGER};

use crate::{LevelScript, PART};

/// How far the original's look reaches (`SelectTargetType`'s trace).
pub const LOOK_RANGE: f32 = 10000.0;

/// Seconds to wait for the hands to come up before starting the job
/// anyway (only when the hands' model can't be shown; not the
/// original's, which always has them).
const HANDS_LATE: f32 = 3.0;

/// The level's consoles, door locks and demolition charges, and what the
/// player is doing with one.
#[derive(Resource, Debug, Default)]
pub struct LevelPanels {
    pub panels: Panels,
    /// The one the player is using (`UsingItem`), and seconds since Use
    /// was pressed on it.
    pub using: Option<(usize, f32)>,
    /// An armed charge the plunger is being pushed for, seconds since,
    /// and whether it has gone off.
    pub plunger: Option<(usize, f32, bool)>,
    /// What the look is on now: (item, a plunger target).
    pub aimed: Option<(usize, bool)>,
    /// Lines for the self-tests: every `Out` carried out, in words.
    pub done: Vec<String>,
}

impl LevelPanels {
    pub fn new(panels: Panels) -> Self {
        LevelPanels {
            panels,
            ..Default::default()
        }
    }
}

/// Preload every effect and sound the items can need, so the first bang
/// doesn't stall the game. Runs while loading.
pub(crate) fn preload(panels: &Panels, out: &mut MessageWriter<PreloadEffect>) {
    for p in &panels.list {
        for (_, i) in &p.infos {
            if i.effect.is_some() || i.sound.is_some() {
                out.write(PreloadEffect {
                    effect: i.effect.clone(),
                    sound: i.sound.as_deref().map(path),
                });
            }
        }
    }
}

fn v3(a: [f32; 3]) -> rc_movement::Vec3 {
    rc_movement::Vec3::new(a[0], a[1], a[2])
}

fn path(p: &str) -> Vec<String> {
    p.split('.').map(str::to_string).collect()
}

/// What the look meets: the item nearest the eye whose shape the line
/// from `eye` along `dir` (Unreal units) hits before the level.
fn look_hit(
    panels: &Panels,
    shapes: &ItemShapes,
    level: Option<&LevelCollision>,
    eye: rc_movement::Vec3,
    dir: rc_movement::Vec3,
) -> Option<(usize, f32)> {
    let end = eye + dir * LOOK_RANGE;
    let wall = level
        .and_then(|l| l.0.sweep(rc_movement::Vec3::ZERO, eye, end))
        .map_or(1.0, |h| h.time);
    let mut best: Option<(usize, f32)> = None;
    for (i, p) in panels.list.iter().enumerate() {
        if p.gone || p.hidden {
            continue;
        }
        let Some(t) = shapes.line_hit(p.export, eye, end) else {
            continue;
        };
        // The item itself may be part of the level's collision: a hit on
        // it at the same place counts.
        if t <= wall + 1e-4 && best.is_none_or(|(_, b)| t < b) {
            best = Some((i, t));
        }
    }
    best
}

/// The `TGT_Panel` conditions after the look has met item `i`.
fn panel_target(
    panels: &Panels,
    busy: &MarkerBusy,
    i: usize,
    eye: rc_movement::Vec3,
    dir: rc_movement::Vec3,
) -> bool {
    let p = &panels.list[i];
    let marker_active = p
        .marker
        .as_deref()
        .is_some_and(|_| busy.0.contains(&p.name.to_ascii_lowercase()));
    let d = rc_movement::Vec3::new(
        p.location[0] - eye.x,
        p.location[1] - eye.y,
        p.location[2] - eye.z,
    );
    let f = p.facing();
    let len = dir.length().max(1e-6);
    let dot = (f[0] * dir.x + f[1] * dir.y + f[2] * dir.z) / len;
    // `HasRequiredInventory`: no item in the game's maps needs any
    // (`AmmunitionRequired` is None in all of them).
    let has_inventory = p.ammo_required.is_none();
    !marker_active
        && p.usable
        && p.usable_by_player
        && has_inventory
        && p.player_range > d.length()
        && dot < p.dot_max
}

/// Drive the items: what the player aims at and uses, their timers, and
/// what they ask for. Runs after the bacta dispensers (which reset the
/// "Use is taken" flag) and before the squad hears Use.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    time: Res<Time>,
    panels: Option<ResMut<LevelPanels>>,
    script: Option<ResMut<LevelScript>>,
    actions: Res<ActionState>,
    (player, camera): (
        Query<Option<&Manning>, With<Player>>,
        Query<&GlobalTransform, With<PlayerCamera>>,
    ),
    (shapes, level, parts, movers): (
        Option<Res<ItemShapes>>,
        Option<Res<LevelCollision>>,
        Option<Res<LevelParts>>,
        Option<Res<LevelMovers>>,
    ),
    (mut claimed, mut use_item, busy): (
        ResMut<rc_game_core::UseClaimed>,
        ResMut<UseItem>,
        Res<MarkerBusy>,
    ),
    (mut notifies, arms, prompts, lib): (
        MessageReader<HandNotify>,
        Option<Res<ArmsReadout>>,
        Option<Res<InputPrompts>>,
        Option<Res<WeaponLibrary>>,
    ),
    (mut effects, mut explode, mut obsolete): (
        MessageWriter<SpawnEffect>,
        MessageWriter<Explode>,
        MessageWriter<MarkerObsolete>,
    ),
    log: Res<GameLog>,
    mut was_hands: Local<Option<String>>,
) {
    let Some(mut script) = script else {
        notifies.clear();
        return;
    };
    let Some(mut lp) = panels else {
        notifies.clear();
        for (tag, state, at) in std::mem::take(&mut script.panel_states) {
            script.stats.not_done += 1;
            log.line(format!("{at}: {tag} to state {state} (not done yet)"));
        }
        return;
    };
    let lp = &mut *lp;
    let script = &mut *script;
    if !script.started {
        notifies.clear();
        return;
    }
    let now = script.director.time;
    if !lp.panels.begun() {
        lp.panels.begin();
        log.line(format!(
            "{PART}: t={now:.2} {} consoles, door locks and charges start",
            lp.panels.list.len()
        ));
    }
    // A squad mate finished a job on one (`markers.rs`): it is done with,
    // as the original's item is after its user finished.
    for a in &script.anchors {
        if !a.sent[2] {
            continue;
        }
        if let Some(i) = lp.panels.find(&a.name) {
            let p = &mut lp.panels.list[i];
            if p.usable && p.user.is_none() && finished_by_squad(p.kind, p.state) {
                p.usable = false;
                log.line(format!(
                    "{PART}: t={now:.2} {} was done by the squad: the player can't use it",
                    p.name
                ));
            }
        }
    }
    // The level's scripts move items to a state (`ACTION_GotoState`).
    for (tag, state, at) in std::mem::take(&mut script.panel_states) {
        let mut n = 0;
        for i in 0..lp.panels.list.len() {
            let p = &lp.panels.list[i];
            if !p.tag.eq_ignore_ascii_case(&tag) {
                continue;
            }
            n += 1;
            match State::named(p.kind, &state) {
                Some(s) => {
                    log.line(format!("{at}: {} goes to state {}", p.name, s.name()));
                    lp.panels.goto(i, s);
                }
                None => log.line(format!(
                    "{at}: {} has no state {state}; stays {}",
                    p.name,
                    p.state.name()
                )),
            }
        }
        if n == 0 {
            script.stats.not_done += 1;
            log.line(format!("{at}: {tag} to state {state} (not done yet)"));
        }
    }
    lp.panels.busy = busy.0.clone();
    let dt = time.delta_secs().min(0.25);
    let on_turret = player.single().ok().flatten().is_some();
    let look = camera
        .single()
        .ok()
        .map(|c| (to_unreal(c.translation()), to_unreal(c.forward().as_vec3())));
    let use_down = actions.pressed(Action::Use);
    let use_pressed = actions.just_pressed(Action::Use);
    let notes: Vec<HandNotify> = notifies.read().cloned().collect();

    // What the look is on (`SelectTargetType`); not while using one
    // (`UsingItem` ignores it).
    if lp.using.is_none() && lp.plunger.is_none() {
        lp.aimed = None;
        if let (Some((eye, dir)), Some(shapes), false) = (look, shapes.as_deref(), on_turret) {
            if let Some((i, _)) = look_hit(&lp.panels, shapes, level.as_deref(), eye, dir) {
                if panel_target(&lp.panels, &busy, i, eye, dir) {
                    lp.aimed = Some((i, false));
                } else if lp.panels.list[i].item_type == AIT_PLUNGER {
                    lp.aimed = Some((i, true));
                }
            }
        }
    }

    // Use pressed on one.
    if use_pressed && lp.using.is_none() && lp.plunger.is_none() {
        match lp.aimed {
            Some((i, false)) => {
                let name = lp.panels.list[i].name.clone();
                if lp.panels.press_use(i) {
                    lp.using = Some((i, 0.0));
                    log.line(format!(
                        "{PART}: t={now:.2} the player uses {name} ({}): hands {}",
                        lp.panels.list[i].kind.what(),
                        lp.panels.list[i].hands().0
                    ));
                } else {
                    log.line(format!(
                        "{PART}: t={now:.2} the player pressed Use on {name}: busy or not usable"
                    ));
                }
            }
            Some((i, true)) => {
                lp.plunger = Some((i, 0.0, false));
                log.line(format!(
                    "{PART}: t={now:.2} the player pushes the plunger for {}",
                    lp.panels.list[i].name
                ));
            }
            None => {}
        }
    }

    // Using one: start the job when the hands are ready, keep Use held.
    let mut holding = false;
    if let Some((i, since)) = lp.using {
        let since = since + dt;
        lp.using = Some((i, since));
        let (hands, waits_for_attach) = lp.panels.list[i].hands();
        let hands_id = rc_combat::hands::arms_id(hands);
        let has_model = lib
            .as_deref()
            .and_then(|l| l.hands.as_ref())
            .is_some_and(|h| h.for_arms(&hands_id).is_some());
        let shown = arms
            .as_deref()
            .is_some_and(|a| a.weapon.as_deref() == Some(hands_id.as_str()));
        let attached = notes
            .iter()
            .any(|n| n.using == hands && n.notify.ends_with("HolsterAttachment"));
        let p = &lp.panels.list[i];
        if !p.activating && p.user.is_none() && use_down {
            let start = if !has_model || arms.is_none() {
                Some("no hands to show")
            } else if waits_for_attach {
                attached.then_some("the hands put the charge on")
            } else if shown {
                Some("the hands are up")
            } else {
                None
            }
            .or_else(|| (since >= HANDS_LATE).then_some("the hands never came up"));
            if let Some(why) = start {
                log.line(format!(
                    "{PART}: t={now:.2} {} starts ({why})",
                    lp.panels.list[i].name
                ));
                lp.panels.start_setup(i, User::Player);
            }
        }
        holding = use_down;
    }
    lp.panels.tick(dt, holding);
    if let Some((i, _)) = lp.using {
        let p = &lp.panels.list[i];
        if !use_down {
            // Let go (`StopUse`): a job under way was stopped by the tick
            // (`BeginActivating` runs `AbortSetup`).
            log.line(format!(
                "{PART}: t={now:.2} the player let go of {} at {:.0}% ({} left)",
                p.name,
                p.fraction * 100.0,
                rc_game_core::timer_text(p.remaining).unwrap_or_else(|| "none".into())
            ));
            lp.using = None;
        } else if p.user.is_some() && !p.activating {
            log.line(format!(
                "{PART}: t={now:.2} the player is done with {}: now {}",
                p.name,
                p.state.name()
            ));
            lp.using = None;
        }
    }

    // The plunger: fires at the hands' `FireWeapon` moment.
    if let Some((i, since, fired)) = lp.plunger {
        let since = since + dt;
        let has_model = lib
            .as_deref()
            .and_then(|l| l.hands.as_ref())
            .is_some_and(|h| h.for_arms(&rc_combat::hands::arms_id("plunger")).is_some());
        let fire_now = !fired
            && (notes
                .iter()
                .any(|n| n.using == "plunger" && n.notify.ends_with("FireWeapon"))
                || !has_model
                || arms.is_none()
                || since >= HANDS_LATE);
        let mut fired = fired;
        if fire_now {
            log.line(format!(
                "{PART}: t={now:.2} the plunger fires {}",
                lp.panels.list[i].name
            ));
            lp.panels.fire_plunger(i);
            fired = true;
        }
        // The hands go down once the `Detonate` move is over.
        let over = fired
            && arms.as_deref().is_none_or(|a| {
                a.weapon.as_deref() != Some("hands:plunger") || a.animation != "Load"
            });
        lp.plunger = if over { None } else { Some((i, since, fired)) };
    }

    // What the items asked for.
    let outs = lp.panels.take();
    carry_out(
        outs,
        lp,
        script,
        parts.as_deref(),
        level.as_deref(),
        movers.as_deref(),
        lib.as_deref(),
        (&mut effects, &mut explode, &mut obsolete),
        &log,
    );

    // The HUD and the player.
    let mut hold = None;
    let mut panel = UsePanel::Hidden;
    let mut icon = UseIcon::Default;
    let mut timer = None;
    let mut prompt = None;
    let mut hands = None;
    if let Some((i, _)) = lp.using {
        let p = &lp.panels.list[i];
        hold = Some(UseHold {
            stand: p.stand(),
            look_at: [p.location[0], p.location[1], p.location[2] + p.hud_z_offset],
        });
        panel = UsePanel::Working(p.fraction);
        icon = icon_of(p);
        timer = Some(p.remaining);
        hands = Some(p.hands().0.to_string());
    } else if lp.plunger.is_some() {
        hands = Some("plunger".to_string());
    } else if let Some((i, false)) = lp.aimed {
        let p = &lp.panels.list[i];
        panel = UsePanel::Ready(p.fraction);
        icon = icon_of(p);
        timer = Some(p.remaining);
        let key = prompts
            .as_ref()
            .and_then(|p| p.label(Action::Use))
            .unwrap_or("USE");
        prompt = Some(p.prompt.replace('@', &format!("'{}'", key.to_uppercase())));
    }
    let ours = hold.is_some() || panel != UsePanel::Hidden || hands.is_some();
    if ours {
        // Use is the item's, not a squad order.
        claimed.0 = true;
        use_item.hold = hold;
        use_item.panel = panel;
        use_item.icon = icon;
        use_item.timer = timer;
        if prompt.is_some() {
            use_item.prompt = prompt;
        }
        if hands.is_some() {
            use_item.hands = hands.clone();
        }
    } else {
        use_item.hold = None;
        use_item.icon = UseIcon::Default;
        use_item.timer = None;
    }
    if *was_hands != hands {
        log.line(format!(
            "{PART}: t={now:.2} hands {}",
            hands.as_deref().unwrap_or("away")
        ));
        *was_hands = hands;
    }
}

/// Whether a squad mate's finished job (`markers.rs` sent its "done")
/// leaves the item in a state the player can't use it from.
fn finished_by_squad(kind: Kind, state: State) -> bool {
    match kind {
        Kind::Console => state != State::Hacked,
        Kind::SliceLock | Kind::BreachLock => state == State::Locked,
        Kind::Charge => matches!(state, State::Unplaced | State::Placed),
    }
}

fn icon_of(p: &rc_scripts::panels::Panel) -> UseIcon {
    match p.marker_icon {
        Some([x, y]) => UseIcon::Square([x as u32, y as u32]),
        None => UseIcon::Nothing,
    }
}

/// Carry out what the items asked for.
#[allow(clippy::too_many_arguments)]
fn carry_out(
    outs: Vec<Out>,
    lp: &mut LevelPanels,
    script: &mut LevelScript,
    parts: Option<&LevelParts>,
    level: Option<&LevelCollision>,
    movers: Option<&LevelMovers>,
    lib: Option<&WeaponLibrary>,
    (effects, explode, obsolete): (
        &mut MessageWriter<SpawnEffect>,
        &mut MessageWriter<Explode>,
        &mut MessageWriter<MarkerObsolete>,
    ),
    log: &GameLog,
) {
    let now = script.director.time;
    for o in outs {
        let line = match &o {
            Out::Transition {
                panel,
                info_name,
                info,
                at,
            } => {
                let mut said = vec![format!("{panel} {info_name}")];
                if let Some(e) = &info.event {
                    script.director.fire(e, panel);
                    said.push(format!("sends {e}"));
                }
                mark_sent(script, panel, info_name);
                let point = [0, 1, 2].map(|k| at[k] + info.effect_offset[k]);
                if info.effect.is_some() || info.sound.is_some() {
                    effects.write(SpawnEffect {
                        effect: info.effect.clone().unwrap_or_default(),
                        at: to_bevy(v3(point)),
                        sound: info.sound.as_deref().map(path),
                        by: panel.clone(),
                        ..default()
                    });
                    if let Some(e) = &info.effect {
                        said.push(format!("effect {e}"));
                    }
                    if let Some(s) = &info.sound {
                        said.push(format!("sound {s}"));
                    }
                }
                if info.damage > 0.0 && info.radius > 0.0 {
                    let ty = info.damage_type.clone().unwrap_or_default();
                    explode.write(Explode {
                        point: to_bevy(v3(*at)),
                        damage: info.damage,
                        radius: info.radius,
                        damage_type: ty.clone(),
                        kind: DamageKind::Explosion,
                        instigator: None,
                        what: panel.clone(),
                    });
                    if let Some(parts) = parts {
                        crate::breakables::hurt_around(
                            script,
                            parts,
                            level,
                            lib,
                            Vec3::from_array(*at),
                            info.damage,
                            info.radius,
                            &ty,
                            None,
                            &format!("{panel}'s blast"),
                            log,
                        );
                    }
                    said.push(format!(
                        "hurts {} within {} ({ty})",
                        info.damage, info.radius
                    ));
                }
                if info.mesh.is_some() || info.material.is_some() {
                    said.push(format!(
                        "look {}{} (not shown yet)",
                        info.mesh.as_deref().unwrap_or(""),
                        info.material
                            .as_deref()
                            .map(|m| format!(" material {m} on {}", info.material_index))
                            .unwrap_or_default()
                    ));
                }
                said.join(", ")
            }
            Out::Event { panel, event } => {
                script.director.fire(event, panel);
                mark_sent(script, panel, "EventUnderway");
                format!("{panel} sends {event} (its EventUnderway)")
            }
            Out::Marker { panel, obsolete: o } => {
                let marker = lp
                    .panels
                    .find(panel)
                    .and_then(|i| lp.panels.list[i].marker.clone())
                    .unwrap_or_default();
                obsolete.write(MarkerObsolete {
                    anchor: panel.clone(),
                    obsolete: *o,
                });
                format!(
                    "{panel}'s squad marker {marker} {}",
                    if *o { "is done with" } else { "is back" }
                )
            }
            Out::Doors { panel, doors, act } => {
                let mut said = Vec::new();
                for d in doors {
                    match act {
                        DoorAct::Lock => said.push(format!("{d} stays shut")),
                        DoorAct::Unlock | DoorAct::Open => {
                            let opened = movers.is_some_and(|m| m.open_named(d));
                            if opened {
                                script.stats.doors_unlocked.push(d.clone());
                            }
                            said.push(format!(
                                "{d} {}",
                                if opened {
                                    "opens"
                                } else {
                                    "not found among the doors"
                                }
                            ));
                        }
                    }
                }
                format!("{panel} {act:?}: {}", said.join(", "))
            }
            Out::DestroyProp { panel, prop } => {
                if !script.director.destroy_prop_named(prop) {
                    let export = script
                        .anchors
                        .iter()
                        .find(|a| a.name.eq_ignore_ascii_case(panel))
                        .and_then(|a| a.destroys.as_ref())
                        .and_then(|(_, e)| *e);
                    if let Some(export) = export {
                        if let Some(p) = parts {
                            p.set_solid_export(export, false);
                        }
                        script.hidden.insert(export);
                        script.stats.props_destroyed.push(prop.clone());
                    }
                }
                format!("{panel} blows away {prop}: no longer solid or drawn")
            }
            Out::DeductAmmo { panel, ammo, .. } => {
                format!("{panel} takes one {ammo} (no item in the game's maps needs any)")
            }
            Out::Mesh { panel, mesh } => format!(
                "{panel} shows {} (not shown yet)",
                mesh.as_deref().unwrap_or("its own mesh")
            ),
            Out::Hidden { panel, hidden } => {
                set_drawn(lp, script, parts, panel, !*hidden);
                format!("{panel} {}", if *hidden { "hidden" } else { "shown" })
            }
            Out::Gone { panel } => {
                set_drawn(lp, script, parts, panel, false);
                format!("{panel} is gone")
            }
            Out::Stimulus { panel, what } => {
                format!("{panel}: the AI would hear {what} (no AI hearing for it yet)")
            }
        };
        log.line(format!("{PART}: t={now:.2} {line}"));
        lp.done.push(line);
    }
}

/// Show or hide an item, and its collision with it.
fn set_drawn(
    lp: &LevelPanels,
    script: &mut LevelScript,
    parts: Option<&LevelParts>,
    panel: &str,
    shown: bool,
) {
    let Some(i) = lp.panels.find(panel) else {
        return;
    };
    let export = lp.panels.list[i].export;
    if shown {
        script.hidden.remove(&export);
    } else {
        script.hidden.insert(export);
    }
    if let Some(p) = parts {
        p.set_solid_export(export, shown);
    }
}

/// The squad marker side (`markers.rs`) mustn't send again what the
/// player's own use already sent: mark the step done on the anchor.
fn mark_sent(script: &mut LevelScript, panel: &str, info_name: &str) {
    let step = match info_name {
        "EventUnderway" | "ActiveState" | "ChargePlaced" => 0,
        "ChargeArmed" => 1,
        "HackedState" | "DoorUnlockedState" | "DoorDamagedState" | "ChargeExplode" => 2,
        _ => return,
    };
    if let Some(a) = script
        .anchors
        .iter_mut()
        .find(|a| a.name.eq_ignore_ascii_case(panel))
    {
        for k in 0..=step {
            a.sent[k] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayStats;
    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;
    use rc_level::scripts::MapScripts;
    use rc_scripts::panels::{Info, Panel};
    use rc_scripts::{Director, Settings};
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    /// A console 150 units ahead of the player along X, facing back at
    /// him: one second of slicing (Easy), then "Hacked".
    fn console() -> Panel {
        let mut p = Panel::new("HackTerminalTest0", Kind::Console);
        p.export = 7;
        p.location = [150.0, 0.0, 60.0];
        p.rotation = [0, 32768, 0];
        p.default_duration = 1.0;
        p.marker = Some("MarkerHackTerminal9".into());
        p.marker_icon = Some([0, 5]);
        p.event_underway = Some("SliceStarted".into());
        p.infos = vec![
            (
                "ActiveState".into(),
                Info {
                    duration: 1.0,
                    ..Info::default()
                },
            ),
            (
                "HackedState".into(),
                Info {
                    event: Some("Hacked".into()),
                    sound: Some("GEN_Sound.Interface.int_GEN_HackedConsole_01".into()),
                    ..Info::default()
                },
            ),
        ];
        p
    }

    fn world(item: Panel) -> World {
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        w.insert_resource(Time::<()>::default());
        w.insert_resource(LevelScript {
            shakes: Default::default(),
            panel_states: Vec::new(),
            director: Director::new(&MapScripts::default(), &[], Settings::default()),
            dispensers: vec![],
            hidden: Default::default(),
            held: vec![],
            calls: vec![],
            places: Default::default(),
            sounds: vec![],
            ammo: vec![],
            hud_blink: Default::default(),
            started: true,
            scene: crate::cutscene::Scene::default(),
            stats: PlayStats::default(),
            breaking: Default::default(),
            anchors: vec![],
            props: Default::default(),
        });
        let tris: HashMap<usize, Vec<[[f32; 3]; 3]>> = [(
            item.export,
            crate::breakables::tests::cube([140.0, -20.0, 20.0], [160.0, 20.0, 100.0]),
        )]
        .into();
        w.insert_resource(ItemShapes::new(tris));
        w.insert_resource(LevelPanels::new(Panels::new(vec![item])));
        w.insert_resource(ActionState::default());
        w.init_resource::<rc_game_core::UseClaimed>();
        w.init_resource::<UseItem>();
        w.init_resource::<MarkerBusy>();
        w.init_resource::<Messages<HandNotify>>();
        w.init_resource::<Messages<SpawnEffect>>();
        w.init_resource::<Messages<Explode>>();
        w.init_resource::<Messages<MarkerObsolete>>();
        w.spawn(Player);
        // The eye 60 up at the origin, looking along X.
        let eye = to_bevy(rc_movement::Vec3::new(0.0, 0.0, 60.0));
        let ahead = to_bevy(rc_movement::Vec3::new(1.0, 0.0, 60.0));
        w.spawn((
            PlayerCamera,
            GlobalTransform::from(Transform::from_translation(eye).looking_at(ahead, Vec3::Y)),
        ));
        w
    }

    fn frame(w: &mut World, use_held: bool, dt: f32) {
        let now: HashSet<Action> = if use_held {
            [Action::Use].into()
        } else {
            HashSet::new()
        };
        w.resource_mut::<ActionState>()
            .update(now, Vec2::ZERO, Vec2::ZERO);
        w.resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(dt));
        w.run_system_once(run).unwrap();
        w.resource_mut::<LevelScript>().director.step(dt, None);
    }

    fn sent(w: &World) -> Vec<String> {
        w.resource::<LevelScript>()
            .director
            .history
            .iter()
            .map(|(_, e)| e.clone())
            .collect()
    }

    #[test]
    fn aiming_shows_the_prompt_ring_icon_and_clock() {
        let mut w = world(console());
        frame(&mut w, false, 0.1);
        let u = w.resource::<UseItem>().clone();
        assert_eq!(u.panel, UsePanel::Ready(0.0));
        assert_eq!(u.icon, UseIcon::Square([0, 5]));
        assert_eq!(u.timer, Some(1.0));
        assert!(u
            .prompt
            .as_deref()
            .unwrap()
            .starts_with("PRESS AND HOLD 'USE'"));
        assert!(u.hold.is_none());
        // Use is the console's, not a squad order.
        assert!(w.resource::<rc_game_core::UseClaimed>().0);
    }

    #[test]
    fn holding_use_slices_the_console() {
        let mut w = world(console());
        frame(&mut w, false, 0.1);
        frame(&mut w, true, 0.1);
        assert!(
            sent(&w).contains(&"SliceStarted".to_string()),
            "{:?}",
            sent(&w)
        );
        let u = w.resource::<UseItem>().clone();
        assert!(u.hold.is_some(), "the player is held");
        assert_eq!(u.hands.as_deref(), Some("slice"));
        for _ in 0..12 {
            frame(&mut w, true, 0.1);
        }
        assert!(sent(&w).contains(&"Hacked".to_string()), "{:?}", sent(&w));
        let lp = w.resource::<LevelPanels>();
        assert_eq!(lp.panels.list[0].state, State::Hacked);
        assert!(lp.using.is_none());
        assert!(
            lp.done
                .iter()
                .any(|l| l.contains("int_GEN_HackedConsole_01")),
            "{:?}",
            lp.done
        );
        // The squad marker is done with.
        let n = w.resource::<Messages<MarkerObsolete>>().len();
        assert!(n > 0);
    }

    #[test]
    fn letting_go_stops_and_the_time_is_kept() {
        let mut w = world(console());
        frame(&mut w, false, 0.1);
        for _ in 0..5 {
            frame(&mut w, true, 0.1);
        }
        frame(&mut w, false, 0.1);
        let lp = w.resource::<LevelPanels>();
        let p = &lp.panels.list[0];
        assert!(lp.using.is_none());
        assert!(!p.activating);
        // A console keeps its progress (`bSavesProgress`), back in Standby.
        assert_eq!(p.state, State::Standby);
        assert!(!sent(&w).contains(&"Hacked".to_string()));
        assert!(w.resource::<UseItem>().hold.is_none());
    }

    #[test]
    fn a_squad_mate_on_it_keeps_the_player_off() {
        let mut w = world(console());
        w.resource_mut::<MarkerBusy>()
            .0
            .insert("hacktterminaltest0".replace("tt", "t"));
        frame(&mut w, false, 0.1);
        assert_eq!(w.resource::<UseItem>().panel, UsePanel::Hidden);
        frame(&mut w, true, 0.1);
        assert!(!sent(&w).contains(&"SliceStarted".to_string()));
    }

    #[test]
    fn from_behind_or_too_far_it_is_not_a_target() {
        let mut p = console();
        // Facing away from the player.
        p.rotation = [0, 0, 0];
        let mut w = world(p);
        frame(&mut w, false, 0.1);
        assert_eq!(w.resource::<UseItem>().panel, UsePanel::Hidden);
        let mut p = console();
        p.player_range = 100.0;
        let mut w = world(p);
        frame(&mut w, false, 0.1);
        assert_eq!(w.resource::<UseItem>().panel, UsePanel::Hidden);
    }
}
