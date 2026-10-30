//! Level scripts taking an enemy over: the [`CharacterOrder`]s the level
//! scripts part sends for characters it found by their [`ScriptTags`]
//! (our droids carry them), carried out with `rc_ai`'s script orders
//! ([`ScriptGoal`]), and a [`CharacterOrderDone`] back when the script
//! waits for it.
//!
//! - `MoveTo`, `MoveToPlayer`, `TurnToPlayer`, `WaitForPlayer`: the
//!   fighter follows the order instead of its own plans until done;
//! - `PlayAnim`: the animation by name from the character's own set, held
//!   still for its real length (looping ones until the next order, or
//!   until it is hurt); one the set doesn't have is logged and done at
//!   once;
//! - `WaitForAnimEnd`: done when the script's animation ends;
//! - `Teleport`, `Hidden`, `Remove` (no death event).

use crate::{Droid, Fight, Looks, PART};
use bevy::prelude::*;
use rc_ai::ScriptGoal;
use rc_anim::Clip;
use rc_game_core::{CharacterAct, CharacterOrder, CharacterOrderDone, GameLog};
use rc_movement::Vec3;

/// An animation a script plays on a droid.
#[derive(Debug, Clone)]
pub struct ScriptedAnim {
    pub name: String,
    pub clip: Clip,
    pub looping: bool,
    pub frame: f32,
}

impl ScriptedAnim {
    /// Its length, seconds.
    pub fn seconds(&self) -> f32 {
        self.clip.num_frames / self.clip.rate.max(1.0)
    }

    pub fn advance(&mut self, dt: f32) {
        let n = self.clip.num_frames.max(1.0);
        self.frame = if self.looping {
            (self.frame + dt * self.clip.rate).rem_euclid(n)
        } else {
            (self.frame + dt * self.clip.rate).min(n - 1.0)
        };
    }
}

fn v(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// Report finished orders, then take this frame's new ones.
pub(crate) fn follow_orders(
    mut fight: ResMut<Fight>,
    mut orders: MessageReader<CharacterOrder>,
    mut done: MessageWriter<CharacterOrderDone>,
    mut droids: Query<(&mut Droid, &mut Visibility)>,
    looks: Res<Looks>,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let now = fight.battle.time;
    for id in std::mem::take(&mut fight.script_done) {
        if let Some((script, index)) = fight.script_waits.remove(&id) {
            if let Some(f) = fight.battle.fighters.iter().find(|f| f.id == id) {
                let p = f.pawn.location;
                fight.stats.scripted.push(format!(
                    "done {id} {script}#{index} at {:.0} {:.0} {:.0}",
                    p.x, p.y, p.z
                ));
            }
            done.write(CharacterOrderDone { script, index });
        }
    }
    for o in orders.read() {
        let Ok((mut droid, mut vis)) = droids.get_mut(o.target) else {
            continue; // not one of ours
        };
        let id = droid.id;
        let Some(f) = fight.battle.fighter_mut(id) else {
            continue;
        };
        // A new order replaces one the script stopped waiting for.
        fight.script_waits.remove(&id);
        let at = format!("{PART}: t={now:.1}s droid {id} ({} #{})", o.script, o.index);
        let mut finished = false;
        let note = |what: String| format!("{what} {id} {}#{}", o.script, o.index);
        let mut noted = Vec::new();
        match &o.act {
            CharacterAct::MoveTo { to, walk, face } => {
                droid.scripted = None;
                f.follow_script(ScriptGoal::MoveTo {
                    to: v(*to),
                    walk: *walk,
                    tolerance: 0.0,
                    face: *face,
                });
                let d = (v(*to) - f.pawn.location).length_2d();
                noted.push(note(format!("walk {d:.0}")));
                log.line(format!(
                    "{at} {} to ({:.0}, {:.0}, {:.0}), {d:.0} uu away",
                    if *walk { "walks" } else { "runs" },
                    to[0],
                    to[1],
                    to[2]
                ));
            }
            CharacterAct::MoveToPlayer { tolerance } => {
                droid.scripted = None;
                f.follow_script(ScriptGoal::MoveToTarget {
                    tolerance: *tolerance,
                });
                log.line(format!(
                    "{at} goes to the player (within {tolerance:.0} uu)"
                ));
            }
            CharacterAct::TurnToPlayer => {
                noted.push(note("turn".into()));
                f.follow_script(ScriptGoal::TurnToTarget);
                log.line(format!("{at} turns to the player"));
            }
            CharacterAct::WaitForPlayer { distance } => {
                f.follow_script(ScriptGoal::WaitForTarget {
                    distance: *distance,
                });
                log.line(format!("{at} waits for the player within {distance:.0} uu"));
            }
            CharacterAct::PlayAnim { name, looping } => {
                let clip = looks
                    .by_type
                    .get(&droid.kind)
                    .and_then(|t| t.look.named_clip(name));
                match clip {
                    Some(clip) => {
                        let sa = ScriptedAnim {
                            name: name.clone(),
                            clip,
                            looping: *looping,
                            frame: 0.0,
                        };
                        let secs = sa.seconds();
                        noted.push(note(format!("anim {name} {secs:.2}")));
                        f.follow_script(ScriptGoal::Hold {
                            until: if *looping { f32::MAX } else { now + secs },
                        });
                        log.line(format!(
                            "{at} plays {name} ({secs:.2} s{})",
                            if *looping { ", looping" } else { "" }
                        ));
                        droid.scripted = Some(sa);
                        droid.frame = 0.0;
                    }
                    None => {
                        noted.push(note(format!("no anim {name}")));
                        log.line(format!(
                            "{at} has no animation {name} in its set ({}); skipped",
                            droid.kind
                        ));
                        finished = true;
                    }
                }
            }
            CharacterAct::WaitForAnimEnd => match &droid.scripted {
                Some(sa) if !sa.looping => {
                    log.line(format!("{at} waits for {} to end", sa.name));
                }
                _ => finished = true,
            },
            CharacterAct::Teleport { to, yaw } => {
                f.teleport(v(*to), *yaw);
                log.line(format!(
                    "{at} teleported to ({:.0}, {:.0}, {:.0})",
                    to[0], to[1], to[2]
                ));
                finished = true;
            }
            CharacterAct::Hidden(h) => {
                *vis = if *h {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
                log.line(format!("{at} {}", if *h { "hidden" } else { "shown" }));
                finished = true;
            }
            CharacterAct::Remove => {
                noted.push(note("removed".into()));
                if let Some(e) = fight.battle.remove(id) {
                    fight.queued.push(e);
                }
                log.line(format!("{at} removed (no death event)"));
                finished = true;
            }
        }
        fight.stats.scripted.extend(noted);
        if o.waits {
            if finished {
                done.write(CharacterOrderDone {
                    script: o.script.clone(),
                    index: o.index,
                });
            } else {
                fight.script_waits.insert(id, (o.script.clone(), o.index));
            }
        }
    }
}
