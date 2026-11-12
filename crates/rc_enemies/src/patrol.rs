//! Enemies walking the patrol route the map gives them
//! (`PatrolRouteFromFactory` / `PatrolRoute`), with `rc_ai::PatrolRun`'s
//! rules: each step becomes a script-style order for the fighter
//! ([`ScriptGoal`]) and the next step starts when it is done. Events go
//! into the level ([`RaiseEvent`]). geo_03c's sonic turret gunner runs to
//! the turret this way and its `GunnerGetOnTurret` event puts it on.
//!
//! While a level script's order is under way the patrol waits for it.
//! Not done: fighting on the way (`ShootWhileMoving`: the fighter follows
//! the route without fighting either way), the original's weighing of the
//! patrol against its other plans (`PatrolPriority`), and keeping a patrol
//! in saves (a loaded fighter doesn't patrol).

use crate::{scripted::ScriptedAnim, Droid, Fight, Looks, PART};
use bevy::prelude::*;
use rc_ai::{PatrolStep, ScriptGoal};
use rc_combat::Invulnerable;
use rc_game_core::{GameLog, RaiseEvent};

/// Take each patrolling fighter on to its next step when the last is done.
pub(crate) fn walk(
    mut fight: ResMut<Fight>,
    mut droids: Query<&mut Droid>,
    looks: Res<Looks>,
    mut raise: MessageWriter<RaiseEvent>,
    mut commands: Commands,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let now = fight.battle.time;
    let ids: Vec<u32> = fight.patrols.keys().copied().collect();
    for id in ids {
        let Some(f) = fight.battle.fighters.iter().find(|f| f.id == id) else {
            // Dead or gone.
            fight.patrols.remove(&id);
            continue;
        };
        // Something under way: ours or a level script's.
        if f.script.is_some() || fight.script_waits.contains_key(&id) {
            continue;
        }
        let entity = fight.entities.get(&id).copied();
        let at = format!("{PART}: t={now:.1}s droid {id} patrol");
        // Steps that take no time are done at once, up to one that does.
        for _ in 0..16 {
            let rng = &mut fight.patrol_rng;
            let Some(run) = fight.patrols.get_mut(&id) else {
                break;
            };
            let step = run.next(&mut |n| rng.next_u32() as usize % n.max(1));
            let finished = run.finished();
            let Some(step) = step else {
                fight.patrols.remove(&id);
                log.line(format!("{at} finished"));
                break;
            };
            let Some(f) = fight.battle.fighter_mut(id) else {
                break;
            };
            match step {
                PatrolStep::Move { to, run } => {
                    let d = (to - f.pawn.location).length_2d();
                    f.follow_script(ScriptGoal::MoveTo {
                        to,
                        walk: !run,
                        tolerance: 0.0,
                        face: None,
                    });
                    fight
                        .stats
                        .scripted
                        .push(format!("patrol {id} walk {d:.0}"));
                    log.line(format!(
                        "{at} {} to ({:.0}, {:.0}, {:.0}), {d:.0} uu away",
                        if run { "runs" } else { "walks" },
                        to.x,
                        to.y,
                        to.z
                    ));
                    break;
                }
                PatrolStep::Face(yaw) => {
                    f.follow_script(ScriptGoal::Face(yaw));
                    break;
                }
                PatrolStep::Wait(secs) => {
                    f.follow_script(ScriptGoal::Hold { until: now + secs });
                    break;
                }
                PatrolStep::Anim(name) => {
                    let mut droid = entity.and_then(|e| droids.get_mut(e).ok());
                    let clip = droid.as_ref().and_then(|d| {
                        looks
                            .by_type
                            .get(&d.kind)
                            .and_then(|t| t.look.named_clip(&name))
                    });
                    match (clip, droid.as_mut()) {
                        (Some(clip), Some(d)) => {
                            let sa = ScriptedAnim {
                                name: name.clone(),
                                clip,
                                looping: false,
                                frame: 0.0,
                            };
                            let secs = sa.seconds();
                            f.follow_script(ScriptGoal::Hold { until: now + secs });
                            d.scripted = Some(sa);
                            d.frame = 0.0;
                            log.line(format!("{at} plays {name} ({secs:.2} s)"));
                            break;
                        }
                        _ => log.line(format!("{at} has no animation {name}; skipped")),
                    }
                }
                PatrolStep::Event(name) => {
                    fight
                        .stats
                        .scripted
                        .push(format!("patrol {id} event {name}"));
                    log.line(format!("{at} sends {name}"));
                    raise.write(RaiseEvent {
                        name,
                        by: format!("patrol of droid {id}"),
                    });
                }
                PatrolStep::Invulnerable(on) => {
                    if let Some(e) = entity {
                        if on {
                            commands.entity(e).insert(Invulnerable);
                        } else {
                            commands.entity(e).remove::<Invulnerable>();
                        }
                    }
                }
                // Our fighters don't fight while following an order.
                PatrolStep::Fights(_) => {}
            }
            if finished {
                fight.patrols.remove(&id);
                log.line(format!("{at} finished"));
                break;
            }
        }
    }
}
