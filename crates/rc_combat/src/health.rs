//! Health, shields and damage, shared by the player, the squad and enemies.
//!
//! The rules are the game's (`Engine.Pawn.TakeDamage` and `Pawn.Tick`, in
//! our own words):
//!
//! - Damage goes to the shield first. Whatever the shield can't take goes
//!   to health.
//! - Any hit on the shield stops it recharging for `recharge_delay`
//!   seconds. After that it refills at `recharge_rate` per second.
//! - At 0 health the character is down. What "down" means (dying, or a
//!   commando waiting to be revived) is up to whoever owns the character.
//!
//! Not done yet: the game also multiplies damage by where it hit (droid
//! heads and arms) and by the target's damage-type table (`DamageVariance`
//! classes in the Properties package).

use bevy::prelude::*;

/// What caused some damage. The game has a damage-type class per weapon
/// family (`CTDamageEnergy`, `CTDamageExplosion`, ...); this is the short
/// list we need so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageKind {
    /// Blaster bolts (`CTDamageEnergy` and its variants).
    Blaster,
    Explosion,
    Melee,
    Fall,
    Other,
}

impl DamageKind {
    pub fn from_name(name: &str) -> Option<DamageKind> {
        Some(match name {
            "blaster" => DamageKind::Blaster,
            "explosion" => DamageKind::Explosion,
            "melee" => DamageKind::Melee,
            "fall" => DamageKind::Fall,
            "other" => DamageKind::Other,
            _ => return None,
        })
    }
}

/// Health points. The player has 200, a commando squad mate and a battle
/// droid 100 (see docs/RESEARCH.md, "Weapons and damage").
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    /// Full health. The game fills health to the maximum when a character
    /// appears (`Pawn.PostBeginPlay`).
    pub fn new(max: f32) -> Self {
        Health { current: max, max }
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }
}

/// A personal energy shield, as commandos wear.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Shield {
    pub current: f32,
    pub max: f32,
    /// Shield regained per second once it is recharging.
    pub recharge_rate: f32,
    /// Seconds after a hit before it starts recharging.
    pub recharge_delay: f32,
    /// Seconds since the shield was last hit.
    pub since_hit: f32,
}

impl Shield {
    /// A full shield.
    pub fn new(max: f32, recharge_rate: f32, recharge_delay: f32) -> Self {
        Shield {
            current: max,
            max,
            recharge_rate,
            recharge_delay,
            since_hit: f32::INFINITY,
        }
    }

    /// Let `dt` seconds pass. The game recharges only once the delay since
    /// the last hit is over.
    pub fn tick(&mut self, dt: f32) {
        self.since_hit += dt;
        if self.since_hit > self.recharge_delay && self.current < self.max {
            self.current = (self.current + self.recharge_rate * dt).min(self.max);
        }
    }
}

/// Which side a character is on. Shots don't care (friendly fire is
/// possible, as in the game); AI uses it to pick targets.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    Republic,
    Separatist,
    Neutral,
}

/// Marks a character whose health reached 0. Added once, with [`Died`].
#[derive(Component, Debug, Clone, Copy)]
pub struct Dead;

/// How one hit was split between shield and health.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Taken {
    pub to_shield: f32,
    pub to_health: f32,
    /// Health reached 0 with this hit (and wasn't 0 before).
    pub killed: bool,
}

/// Apply `amount` of damage: shield first, then health.
pub fn take_damage(health: &mut Health, shield: Option<&mut Shield>, amount: f32) -> Taken {
    if amount <= 0.0 {
        return Taken::default();
    }
    let mut rest = amount;
    let mut to_shield = 0.0;
    if let Some(s) = shield {
        // Any hit on the shield restarts its recharge delay, even one that
        // breaks it (the script sets the delay before checking).
        s.since_hit = 0.0;
        to_shield = rest.min(s.current.max(0.0));
        s.current -= to_shield;
        rest -= to_shield;
    }
    let was_alive = !health.is_dead();
    let to_health = rest;
    health.current -= to_health;
    Taken {
        to_shield,
        to_health,
        killed: was_alive && health.is_dead(),
    }
}

/// "Hurt this": send it with `MessageWriter<Damage>`. Positions and
/// directions are in Bevy space.
#[derive(Message, Debug, Clone)]
pub struct Damage {
    pub target: Entity,
    pub amount: f32,
    pub kind: DamageKind,
    /// Who did it, if anyone.
    pub instigator: Option<Entity>,
    /// Where it hit.
    pub point: Vec3,
    /// Which way the shot or blast was going.
    pub direction: Vec3,
}

/// Sent once when a character's health reaches 0. Nothing is despawned:
/// the character's owner decides what happens.
#[derive(Message, Debug, Clone)]
pub struct Died {
    pub entity: Entity,
    pub killer: Option<Entity>,
}

/// The game's difficulty setting for damage the player takes. The original
/// keeps it in `user.ini`: easy 0.25, medium 1, hard 1.75
/// (`DamageModifierEasy/Medium/Hard`). **Guess:** where the game applies
/// it is in compiled code we don't read; we apply it to damage the player
/// takes only.
#[derive(Resource, Debug, Clone, Copy)]
pub struct Difficulty {
    pub damage_to_player: f32,
}

impl Default for Difficulty {
    fn default() -> Self {
        Difficulty {
            damage_to_player: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_takes_damage_first_then_health() {
        let mut h = Health::new(200.0);
        let mut s = Shield::new(100.0, 20.0, 3.0);
        let t = take_damage(&mut h, Some(&mut s), 70.0);
        assert_eq!((t.to_shield, t.to_health), (70.0, 0.0));
        let t = take_damage(&mut h, Some(&mut s), 70.0);
        assert_eq!((t.to_shield, t.to_health), (30.0, 40.0));
        assert_eq!((s.current, h.current), (0.0, 160.0));
        let t = take_damage(&mut h, Some(&mut s), 500.0);
        assert!(t.killed && h.is_dead());
        let t = take_damage(&mut h, Some(&mut s), 5.0);
        assert!(!t.killed, "only the hit that kills counts as killing");
    }

    #[test]
    fn shield_waits_then_recharges() {
        let mut h = Health::new(200.0);
        let mut s = Shield::new(100.0, 20.0, 3.0);
        take_damage(&mut h, Some(&mut s), 100.0);
        for _ in 0..29 {
            s.tick(0.1);
        }
        assert_eq!(s.current, 0.0, "nothing in the first 3 s");
        for _ in 0..21 {
            s.tick(0.1);
        }
        assert!(
            (s.current - 40.0).abs() < 2.1,
            "20 a second for 2 s: {}",
            s.current
        );
        for _ in 0..100 {
            s.tick(0.1);
        }
        assert_eq!(s.current, 100.0);
    }
}
