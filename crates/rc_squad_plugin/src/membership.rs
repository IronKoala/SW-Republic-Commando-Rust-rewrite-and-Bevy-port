//! Squad mates coming into the level, leaving the squad and rejoining it,
//! as the level's scripts say (see `rc_squad::presence`).
//!
//! - [`factory_arrivals`]: a squad mate the map makes with a spawner
//!   (`PawnFactory` with a `Commando..Delta` prototype) comes in when the
//!   spawner's event is sent, out of the squad (Scorch on geo_01a, Fixer
//!   on ras_02b), once.
//! - [`change_membership`]: a script's `ChangeSquadMember`
//!   ([`SquadMembership`]) takes a mate out of the squad or puts them back.
//! - [`follow_script_orders`]: while out of the squad, a mate does what
//!   the scripts tell them ([`CharacterOrder`]): walk to a point or to the
//!   player, turn to the player, wait for the player, be put somewhere,
//!   hide, leave the level. In the squad, the squad leads them: orders that
//!   would walk or turn them are answered at once and logged. Scripted
//!   animations aren't played on squad mates yet (logged, and the script
//!   carries on).

use crate::{log_line, mate_half_height, SquadMate, SquadState};
use bevy::prelude::*;
use rc_game_core::units::to_unreal;
use rc_game_core::{
    CharacterAct, CharacterOrder, CharacterOrderDone, GameLog, Player, ScriptEvent, SquadMembership,
};
use rc_movement::Vec3 as UVec3;
use rc_squad::{Presence, ScriptMove};

/// What a level script waits for a squad mate to do.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptWait {
    pub script: String,
    pub index: usize,
    pub what: Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waiting {
    /// To get where they were sent.
    Walk,
    /// For the player to come within this many uu.
    Player(f32),
}

fn uvec([x, y, z]: [f32; 3]) -> UVec3 {
    UVec3::new(x, y, z)
}

/// Bring in the squad mates whose spawner's event was just sent.
pub(crate) fn factory_arrivals(
    mut state: ResMut<SquadState>,
    mut events: MessageReader<ScriptEvent>,
    log: Option<Res<GameLog>>,
) {
    let state = &mut *state;
    // Not read until the squad exists, so a spawner set off at the level's
    // start still finds it.
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    for ev in events.read() {
        for i in 0..squad.members.len() {
            let Some(t) = state.mates.get_mut(i) else {
                continue;
            };
            let Some(f) = t
                .factories
                .iter()
                .find(|f| {
                    f.tag.eq_ignore_ascii_case(&ev.name) && !t.factories_used.contains(&f.actor)
                })
                .cloned()
            else {
                continue;
            };
            t.factories_used.push(f.actor.clone());
            t.tag = f.pawn_tag.clone();
            t.script = f.script.clone();
            t.event = f.pawn_event.clone();
            t.revive_event = None;
            let feet = f.location - UVec3::new(0.0, 0.0, mate_half_height());
            let name = squad.members[i].settings.name.clone();
            if squad.members[i].presence == Presence::Gone {
                log_line(
                    &log,
                    format!(
                        "{} goes off ({}, sent by {}): makes {name}, called {}",
                        f.actor, ev.name, ev.by, f.pawn_tag
                    ),
                );
            } else {
                // A second copy (yyy_06d's `NewSquad`): we have one body
                // per squad mate, so the one in the level becomes it.
                log_line(
                    &log,
                    format!(
                        "{} goes off ({}, sent by {}): makes another {name}, called {}; \
                         {name} is already in the level, so he moves there and takes the new name \
                         (one body per squad mate)",
                        f.actor, ev.name, ev.by, f.pawn_tag
                    ),
                );
            }
            for e in squad.arrive(i, feet, f.yaw, false) {
                log_line(&log, e.to_string());
            }
        }
    }
}

/// Carry out `ChangeSquadMember`.
pub(crate) fn change_membership(
    mut state: ResMut<SquadState>,
    mut asks: MessageReader<SquadMembership>,
    mates: Query<&SquadMate>,
    mut done: MessageWriter<CharacterOrderDone>,
    log: Option<Res<GameLog>>,
) {
    let state = &mut *state;
    let Some(squad) = state.squad.as_mut() else {
        asks.clear();
        return;
    };
    for a in asks.read() {
        let Ok(mate) = mates.get(a.target) else {
            log_line(&log, format!("{}: not a squad mate; nothing to do", a.by));
            continue;
        };
        let i = mate.index;
        let events = squad.set_in_squad(i, a.join);
        if events.is_empty() {
            log_line(
                &log,
                format!(
                    "{}: {} is already {}",
                    a.by,
                    mate.name,
                    if a.join {
                        "in the squad"
                    } else {
                        "out of the squad"
                    }
                ),
            );
        }
        for e in events {
            log_line(&log, format!("{}: {e}", a.by));
        }
        // Back in the squad: whatever a script was waiting for them to
        // do is over.
        if a.join {
            if let Some(w) = state.script_waits.get_mut(i).and_then(Option::take) {
                done.write(CharacterOrderDone {
                    script: w.script,
                    index: w.index,
                });
            }
        }
    }
}

/// Carry out level scripts' orders to squad mates, and answer the ones
/// the scripts wait on when they're done.
pub(crate) fn follow_script_orders(
    mut state: ResMut<SquadState>,
    mut orders: MessageReader<CharacterOrder>,
    mut done: MessageWriter<CharacterOrderDone>,
    mut mates: Query<(&SquadMate, &mut Visibility)>,
    player: Query<&Transform, With<Player>>,
    log: Option<Res<GameLog>>,
) {
    let state = &mut *state;
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    let n = squad.members.len();
    if state.script_waits.len() != n {
        state.script_waits.resize(n, None);
    }
    let player_feet = player.single().ok().map(|t| to_unreal(t.translation));
    // Waits that are over.
    for i in 0..n {
        let Some(w) = state.script_waits[i].clone() else {
            continue;
        };
        let m = &squad.members[i];
        let over = match w.what {
            Waiting::Walk => m.script_move.is_none(),
            Waiting::Player(d) => player_feet.is_some_and(|p| (p - m.pawn.feet()).length() <= d),
        } || m.presence != Presence::Away;
        if over {
            state.script_waits[i] = None;
            log_line(
                &log,
                format!(
                    "{} done with {} #{} ({:?}) at ({:.0}, {:.0}, {:.0})",
                    m.settings.name,
                    w.script,
                    w.index,
                    w.what,
                    m.pawn.feet().x,
                    m.pawn.feet().y,
                    m.pawn.feet().z
                ),
            );
            done.write(CharacterOrderDone {
                script: w.script,
                index: w.index,
            });
        }
    }
    for o in orders.read() {
        let Ok((mate, mut vis)) = mates.get_mut(o.target) else {
            continue; // not a squad mate
        };
        let i = mate.index;
        let Some(m) = squad.members.get(i) else {
            continue;
        };
        // A new order replaces one the script stopped waiting for.
        state.script_waits[i] = None;
        let name = m.settings.name.clone();
        let at = format!("{} #{}: {name}", o.script, o.index);
        let away = m.presence == Presence::Away && !m.down;
        let mut wait: Option<Waiting> = None;
        match &o.act {
            CharacterAct::Teleport { to, yaw } => {
                let feet = uvec(*to) - UVec3::new(0.0, 0.0, mate_half_height());
                squad.script_teleport(i, feet, *yaw);
                log_line(
                    &log,
                    format!(
                        "{at} is put at ({:.0}, {:.0}, {:.0})",
                        feet.x, feet.y, feet.z
                    ),
                );
            }
            CharacterAct::Hidden(h) => {
                *vis = if *h {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
                log_line(
                    &log,
                    format!("{at} {}", if *h { "hidden" } else { "shown" }),
                );
            }
            CharacterAct::Remove => {
                for e in squad.remove(i) {
                    log_line(&log, format!("{at}: {e}"));
                }
            }
            act if !away => {
                log_line(
                    &log,
                    format!(
                        "{at} {}: {act:?} skipped",
                        if m.down {
                            "is down"
                        } else if m.presence == Presence::InSquad {
                            "is in the squad (the squad leads them)"
                        } else {
                            "is not in the level"
                        }
                    ),
                );
            }
            CharacterAct::MoveTo { to, walk, face } => {
                let feet = uvec(*to) - UVec3::new(0.0, 0.0, mate_half_height());
                let d = (feet - m.pawn.feet()).length_2d();
                squad.script_move(
                    i,
                    ScriptMove {
                        to: feet,
                        walk: *walk,
                        face: *face,
                        index: o.index,
                    },
                );
                log_line(
                    &log,
                    format!(
                        "{at} {} to ({:.0}, {:.0}, {:.0}), {d:.0} uu away",
                        if *walk { "walks" } else { "runs" },
                        feet.x,
                        feet.y,
                        feet.z
                    ),
                );
                wait = Some(Waiting::Walk);
            }
            CharacterAct::MoveToPlayer { tolerance } => {
                match player_feet {
                    Some(p) => {
                        let from = m.pawn.feet();
                        let d = p - from;
                        let len = d.length_2d();
                        let stop = if len > *tolerance {
                            p - UVec3::new(d.x / len * tolerance, d.y / len * tolerance, 0.0)
                        } else {
                            from
                        };
                        squad.script_move(
                            i,
                            ScriptMove {
                                to: stop,
                                walk: false,
                                face: Some(d.y.atan2(d.x)),
                                index: o.index,
                            },
                        );
                        log_line(
                        &log,
                        format!("{at} goes to the player (within {tolerance:.0} uu), {len:.0} uu away"),
                    );
                        wait = Some(Waiting::Walk);
                    }
                    None => log_line(&log, format!("{at}: no player; skipped")),
                }
            }
            CharacterAct::TurnToPlayer => {
                if let Some(p) = player_feet {
                    let d = p - m.pawn.feet();
                    squad.script_face(i, d.y.atan2(d.x));
                }
                log_line(&log, format!("{at} turns to the player"));
            }
            CharacterAct::WaitForPlayer { distance } => {
                log_line(
                    &log,
                    format!("{at} waits for the player within {distance:.0} uu"),
                );
                wait = Some(Waiting::Player(*distance));
            }
            CharacterAct::PlayAnim { name: anim, .. } => {
                log_line(
                    &log,
                    format!(
                        "{at} should play {anim} (squad mates' scripted animations aren't played yet); skipped"
                    ),
                );
            }
            CharacterAct::WaitForAnimEnd => {}
        }
        if o.waits {
            match wait {
                Some(what) => {
                    state.script_waits[i] = Some(ScriptWait {
                        script: o.script.clone(),
                        index: o.index,
                        what,
                    })
                }
                None => {
                    done.write(CharacterOrderDone {
                        script: o.script.clone(),
                        index: o.index,
                    });
                }
            }
        }
    }
}
