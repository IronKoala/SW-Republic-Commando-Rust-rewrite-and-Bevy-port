//! How a proximity mine behaves, step by step, as the original's script
//! for it says (`ProximityMine` and its parent `ExplosiveCharge` in
//! CTMarkers.u, `MarkerSetProximityMineNative` for the squad marker). Plain
//! Rust, no Bevy: the game part (`rc_scripts_plugin::mines`) feeds it the
//! time, who is near and what hits the mine, and carries out what comes
//! back (sounds, the material, the blast, the barrel).
//!
//! The steps (the script's states):
//!
//! 1. **Unplaced** (`ChargeUnplaced`): a see-through hologram on the
//!    barrel, waiting for someone to set it. Damage to it goes to the
//!    barrel instead; if the barrel is gone, so is the mine.
//! 2. **Placed** (`ChargePlaced`): set, not yet armed. From now on any
//!    damage sets it off (`bTriggeredByDamage`).
//! 3. **Armed** (`ChargeArmed`): armed by whoever set it; the mine is now on
//!    their side. After `ChargeArmed.Duration` seconds it plays
//!    `ActivateSound` and starts watching.
//! 4. **Watching** (`ProximityDetect`): every 0.25 s it looks for an enemy of
//!    its side within `TriggerRadius` who isn't crouched or dead. One found:
//!    `AlertSound` (a chirp) and on to Alert.
//! 5. **Alert** (`ProximityAlert`): every 0.2 s it looks again. Still there:
//!    another chirp, and once `AlertTimeLimit` seconds have passed since the
//!    first chirp, it explodes. Gone: back to watching.
//! 6. **Exploded** (`Explode`): `ChargeExplode` (effect, sound, a blast of
//!    `Damage` within `DamageRadius`), the barrel is destroyed, the mine
//!    is gone and so is its squad marker.
//!
//! "Every 0.25 s" is the script's own `NextCheckTime`: a check happens on
//! the first frame after that time.

/// Where a mine is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Unplaced,
    Placed,
    /// Seconds left before it starts watching.
    Armed {
        left: f32,
    },
    Watching,
    /// Time (the mine's clock) of the first chirp.
    Alert {
        since: f32,
    },
    Exploded,
    /// Taken away without exploding (its barrel went first).
    Removed,
}

/// Something the game must do for a mine.
#[derive(Debug, Clone, PartialEq)]
pub enum MineEvent {
    /// Entered Placed: `ChargePlaced`'s sound and material.
    Placed,
    /// Entered Armed: `ChargeArmed`'s sound and material.
    Armed,
    /// Started watching: play `ActivateSound`.
    Watching,
    /// Saw an enemy: play `AlertSound`.
    Chirp,
    /// Explodes now: `ChargeExplode`, destroy the barrel, hide the mine.
    Explode,
    /// Gone without exploding (its barrel broke while it was unplaced).
    Removed,
}

/// One mine's running state.
#[derive(Debug, Clone, PartialEq)]
pub struct MineRun {
    pub phase: Phase,
    /// The mine's own clock (seconds since the level started running it).
    pub now: f32,
    next_check: f32,
    /// Side it is on (the game's `TeamIndex`): set by whoever arms it.
    pub team: i32,
    pub trigger_radius: f32,
    pub alert_time_limit: f32,
    /// `ChargeArmed.Duration`.
    pub arm_wait: f32,
}

impl MineRun {
    pub fn new(team: i32, trigger_radius: f32, alert_time_limit: f32, arm_wait: f32) -> Self {
        MineRun {
            phase: Phase::Unplaced,
            now: 0.0,
            next_check: 0.0,
            team,
            trigger_radius,
            alert_time_limit,
            arm_wait,
        }
    }

    /// Still in the level (not exploded or taken away).
    pub fn present(&self) -> bool {
        !matches!(self.phase, Phase::Exploded | Phase::Removed)
    }

    /// Set but not exploded: damage sets it off.
    pub fn set(&self) -> bool {
        matches!(
            self.phase,
            Phase::Placed | Phase::Armed { .. } | Phase::Watching | Phase::Alert { .. }
        )
    }

    /// Someone set it (the squad marker's `PlaceEvent`, sent by the
    /// placing animation).
    pub fn place(&mut self) -> Vec<MineEvent> {
        if self.phase != Phase::Unplaced {
            return Vec::new();
        }
        self.phase = Phase::Placed;
        vec![MineEvent::Placed]
    }

    /// Someone on side `team` armed it (the marker's `ArmEvent`:
    /// `ProximityMine.Activate`). An unplaced mine is placed first.
    pub fn arm(&mut self, team: i32) -> Vec<MineEvent> {
        let mut out = Vec::new();
        if self.phase == Phase::Unplaced {
            out.extend(self.place());
        }
        if self.phase != Phase::Placed {
            return out;
        }
        self.team = team;
        self.phase = Phase::Armed {
            left: self.arm_wait,
        };
        out.push(MineEvent::Armed);
        out
    }

    /// Damage reached the mine. Once set, any damage sets it off; before
    /// that the damage belongs to the barrel (`true` back: pass it on).
    pub fn hurt(&mut self) -> (Vec<MineEvent>, bool) {
        if self.set() {
            self.phase = Phase::Exploded;
            return (vec![MineEvent::Explode], false);
        }
        (Vec::new(), self.phase == Phase::Unplaced)
    }

    /// Its barrel is broken. An unplaced mine goes with it; a set one stays
    /// (the barrel's own blast reaches it through `hurt`).
    pub fn barrel_gone(&mut self) -> Vec<MineEvent> {
        if self.phase == Phase::Unplaced {
            self.phase = Phase::Removed;
            return vec![MineEvent::Removed];
        }
        Vec::new()
    }

    /// Advance by `dt` seconds. `enemy_near(team, radius)` says whether an
    /// enemy of side `team` who isn't crouched or dead is within `radius`
    /// uu; it is only asked when a check is due.
    pub fn tick(
        &mut self,
        dt: f32,
        enemy_near: &mut dyn FnMut(i32, f32) -> bool,
    ) -> Vec<MineEvent> {
        self.now += dt;
        let mut out = Vec::new();
        match self.phase {
            Phase::Armed { left } => {
                let left = left - dt;
                if left <= 0.0 {
                    // Timer(): ActivateSound, then ProximityDetect.
                    self.phase = Phase::Watching;
                    out.push(MineEvent::Watching);
                } else {
                    self.phase = Phase::Armed { left };
                }
            }
            Phase::Watching => {
                if self.trigger_radius > 1.0 && self.now > self.next_check {
                    self.next_check = self.now + 0.25;
                    if enemy_near(self.team, self.trigger_radius) {
                        out.push(MineEvent::Chirp);
                        self.phase = Phase::Alert { since: self.now };
                    }
                }
            }
            Phase::Alert { since } => {
                if self.trigger_radius > 1.0 && self.now > self.next_check {
                    self.next_check = self.now + 0.2;
                    if enemy_near(self.team, self.trigger_radius) {
                        out.push(MineEvent::Chirp);
                        if self.now >= since + self.alert_time_limit {
                            self.phase = Phase::Exploded;
                            out.push(MineEvent::Explode);
                        }
                    } else {
                        self.phase = Phase::Watching;
                    }
                }
            }
            _ => {}
        }
        out
    }
}

/// The squad marker's side of setting a mine (`MarkerSetProximityMine`):
/// the placing animation sends `PlaceEvent` (`MproxMinePlace`), then
/// `ArmEvent` (`ProxMineArm`); the first places the mine, the second arms
/// it. Each only counts once and in that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerStep {
    #[default]
    WaitingToPlace,
    WaitingToArm,
    Finished,
}

impl MarkerStep {
    /// What an event with this step does: `Some(true)` places, `Some(false)`
    /// arms, `None` nothing (out of order or already done).
    pub fn take(&mut self, placing: bool) -> Option<bool> {
        match (*self, placing) {
            (MarkerStep::WaitingToPlace, true) => {
                *self = MarkerStep::WaitingToArm;
                Some(true)
            }
            (MarkerStep::WaitingToArm, false) => {
                *self = MarkerStep::Finished;
                Some(false)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mine() -> MineRun {
        MineRun::new(1, 512.0, 1.5, 3.0)
    }

    fn run(m: &mut MineRun, secs: f32, near: bool) -> Vec<MineEvent> {
        let mut out = Vec::new();
        let steps = (secs / 0.05).round() as usize;
        for _ in 0..steps {
            out.extend(m.tick(0.05, &mut |_, _| near));
        }
        out
    }

    #[test]
    fn unplaced_mine_waits_and_passes_damage_on() {
        let mut m = mine();
        assert!(run(&mut m, 5.0, true).is_empty());
        let (ev, pass) = m.hurt();
        assert!(ev.is_empty() && pass);
        assert_eq!(m.barrel_gone(), vec![MineEvent::Removed]);
        assert!(!m.present());
    }

    #[test]
    fn armed_mine_waits_then_watches() {
        let mut m = mine();
        assert_eq!(m.place(), vec![MineEvent::Placed]);
        assert_eq!(m.arm(0), vec![MineEvent::Armed]);
        assert_eq!(m.team, 0);
        // Nothing for the arming wait (3 s), even with an enemy there.
        assert!(run(&mut m, 2.9, true).is_empty());
        let ev = run(&mut m, 0.2, false);
        assert_eq!(ev, vec![MineEvent::Watching]);
        assert_eq!(m.phase, Phase::Watching);
    }

    #[test]
    fn enemy_staying_near_sets_it_off_after_the_alert_time() {
        let mut m = mine();
        m.arm(0);
        run(&mut m, 3.05, false);
        assert_eq!(m.phase, Phase::Watching);
        let ev = run(&mut m, 1.0, true);
        assert!(ev.contains(&MineEvent::Chirp));
        assert!(!ev.contains(&MineEvent::Explode));
        let ev = run(&mut m, 1.0, true);
        assert_eq!(ev.last(), Some(&MineEvent::Explode));
        assert_eq!(m.phase, Phase::Exploded);
        // The alert lasted at least 1.5 s.
        let chirps = ev.iter().filter(|e| **e == MineEvent::Chirp).count();
        assert!(chirps >= 2, "{ev:?}");
    }

    #[test]
    fn enemy_leaving_resets_the_alert() {
        let mut m = mine();
        m.arm(0);
        run(&mut m, 3.05, false);
        run(&mut m, 1.0, true);
        assert!(matches!(m.phase, Phase::Alert { .. }));
        run(&mut m, 0.5, false);
        assert_eq!(m.phase, Phase::Watching);
        // A fresh alert needs the full 1.5 s again.
        let ev = run(&mut m, 1.2, true);
        assert!(!ev.contains(&MineEvent::Explode));
    }

    #[test]
    fn set_mine_explodes_when_hurt() {
        let mut m = mine();
        m.place();
        let (ev, pass) = m.hurt();
        assert_eq!(ev, vec![MineEvent::Explode]);
        assert!(!pass);
        // An exploded mine does nothing more.
        assert!(m.arm(0).is_empty());
        assert!(run(&mut m, 5.0, true).is_empty());
        // And a set mine stays when its barrel goes.
        let mut m = mine();
        m.place();
        assert!(m.barrel_gone().is_empty());
        assert!(m.present());
    }

    #[test]
    fn arming_an_unplaced_mine_places_it_first() {
        let mut m = mine();
        assert_eq!(m.arm(0), vec![MineEvent::Placed, MineEvent::Armed]);
    }

    #[test]
    fn marker_steps_go_in_order() {
        let mut s = MarkerStep::default();
        assert_eq!(s.take(false), None);
        assert_eq!(s.take(true), Some(true));
        assert_eq!(s.take(true), None);
        assert_eq!(s.take(false), Some(false));
        assert_eq!(s.take(false), None);
        assert_eq!(s, MarkerStep::Finished);
    }
}
