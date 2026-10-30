//! Damage types and damage tables: how much of each kind of damage a
//! character really takes.
//!
//! The original names every kind of damage with a class (`CTDamageEnergyDC17`
//! is a kind of `CTDamageEnergy`, which is a kind of `CTDamageType`), and
//! gives each character a table (a `DamageVariance` class in the Properties
//! package) of multipliers for some of those classes. Battle droids, for
//! example, take 3 times melee damage and none from stuns. Both are in
//! `data/combat/damage_types.toml`, read from the game.
//!
//! **Our reading of the rule** (the original's lookup is compiled code): a
//! hit's multiplier is the table's entry for its own type if there is one,
//! else for its parent type, and so on up; with no entry at all it is 1.
//! The game's tables list the specific types before their parents, so this
//! gives what reading them top to bottom gives.

use crate::data::Table;
use bevy::prelude::Component;
use std::collections::HashMap;

/// Every damage type's parent, and every damage table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DamageTypes {
    pub parents: HashMap<String, String>,
    pub tables: HashMap<String, HashMap<String, f32>>,
}

/// The damage table a character uses (its name in `damage_types.toml`).
/// Characters without one take every hit at full strength.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct DamageTable(pub String);

impl DamageTypes {
    /// `[parents]` lists `Type = "Parent"`; every other section is a table
    /// of `Type = multiplier`.
    pub fn from_text(text: &str) -> Result<DamageTypes, String> {
        let mut t = Table::parse(text)?;
        let mut out = DamageTypes::default();
        for name in t.names() {
            let (section, key) = name
                .split_once('.')
                .ok_or_else(|| format!("`{name}` should be inside a [section]"))?;
            if section == "parents" {
                let parent = t
                    .text(&name)?
                    .ok_or_else(|| format!("`{name}` should be text"))?;
                out.parents
                    .insert(key.to_ascii_lowercase(), parent.to_ascii_lowercase());
            } else {
                let m = t
                    .number(&name)?
                    .ok_or_else(|| format!("`{name}` should be a number"))?;
                out.tables
                    .entry(section.to_string())
                    .or_default()
                    .insert(key.to_ascii_lowercase(), m);
            }
        }
        t.finish()?;
        Ok(out)
    }

    /// Whether `ty` is `ancestor` or one of its kinds.
    pub fn is_a(&self, ty: &str, ancestor: &str) -> bool {
        let ancestor = ancestor.to_ascii_lowercase();
        let mut cur = ty.to_ascii_lowercase();
        for _ in 0..32 {
            if cur == ancestor {
                return true;
            }
            match self.parents.get(&cur) {
                Some(p) => cur = p.clone(),
                None => return false,
            }
        }
        false
    }

    /// How much of a hit of type `ty` a character with `table` takes.
    pub fn multiplier(&self, table: Option<&str>, ty: &str) -> f32 {
        let Some(entries) = table.and_then(|t| self.tables.get(t)) else {
            return 1.0;
        };
        let mut cur = ty.to_ascii_lowercase();
        for _ in 0..32 {
            if let Some(m) = entries.get(&cur) {
                return *m;
            }
            match self.parents.get(&cur) {
                Some(p) => cur = p.clone(),
                None => return 1.0,
            }
        }
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_tables_read_and_look_up_by_parent() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/combat/damage_types.toml"
        ))
        .unwrap();
        let d = DamageTypes::from_text(&text).unwrap();
        let bd = Some("DV_DroidsBattleDroidDamage");
        // Melee x3, stun none, a DC-17 bolt counts as energy (x1).
        assert_eq!(d.multiplier(bd, "CTDamageMelee"), 3.0);
        assert_eq!(d.multiplier(bd, "CTDamageStun"), 0.0);
        assert_eq!(d.multiplier(bd, "CTDamageEnergyDC17"), 1.0);
        assert_eq!(d.multiplier(bd, "CTDamageExplosionThermalDet"), 0.75);
        // The squad: the specific DC-17 entry wins over its parent's.
        let sq = Some("DV_OrganicCTSquadDamage");
        assert_eq!(d.multiplier(sq, "CTDamageEnergyDC17Commando"), 0.3);
        assert_eq!(d.multiplier(sq, "CTDamageEnergyDroids"), 0.15);
        assert_eq!(d.multiplier(sq, "CTDamageEnergyDC15s"), 0.45);
        assert_eq!(d.multiplier(None, "CTDamageMelee"), 1.0);
        assert!(d.is_a("CTDamageExplosionSonic", "CTDamageExplosion"));
    }
}
