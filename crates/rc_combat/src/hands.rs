//! The hands the player uses things with: the original's `HandSignal`
//! "weapon" (`CTInventory.HandSignal`, mesh `HudArms.HudArmsHands`).
//!
//! In the game, using something (a bacta dispenser, a door panel) switches
//! to this weapon: the gun in hand plays `Holster`, the hands come up with
//! the use's `LoadAnim`, play its `FireAnim` again and again while Use is
//! held (`CTPlayer` state `UsingItem` fires the weapon each time it goes
//! back to idle), and go down with its `HolsterAnim` when the use ends,
//! before the last gun comes back up with `Load`
//! (`Pawn.SwitchToLastWeapon`). Which three moves depends on the thing
//! used (`HandSignal.MarkerHandSetup`); a bacta dispenser's are
//! `BactaLoad`, `BactaLoop` and `BactaHolster` (`SetupBactaHealer`).
//!
//! Everything is read from `data/combat/hand_signal.toml`: the `[arms]`
//! section as in a weapon file, and one section per use naming its three
//! moves. [`rc_game_core::UseItem::hands`] says which use is playing.

use crate::data::Table;
use crate::weapon::ArmsParams;
use std::collections::BTreeMap;

/// The arms id the hands are known by for one use (`hands:bacta`): each
/// use is its own model to the arms, with its moves under the usual names.
pub fn arms_id(using: &str) -> String {
    format!("hands:{using}")
}

/// The three moves of one use, by their names in the animation set.
#[derive(Debug, Clone, PartialEq)]
pub struct HandMoves {
    /// Coming up into view (`LoadAnim`).
    pub load: String,
    /// Repeated while Use is held (`FireAnim`).
    pub fire: String,
    /// Going down out of view (`HolsterAnim`).
    pub holster: String,
}

/// `data/combat/hand_signal.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct HandSignal {
    pub arms: ArmsParams,
    /// By use name (`bacta`).
    pub uses: BTreeMap<String, HandMoves>,
}

impl HandSignal {
    pub fn from_text(text: &str) -> Result<HandSignal, String> {
        let mut t = Table::parse(text)?;
        let arms = ArmsParams::read(&mut t)?.ok_or("`arms.mesh` is missing")?;
        let sections: Vec<String> = {
            let mut v: Vec<String> = t
                .names()
                .into_iter()
                .filter_map(|n| n.split_once('.').map(|(s, _)| s.to_string()))
                .filter(|s| s != "arms")
                .collect();
            v.dedup();
            v
        };
        let mut uses = BTreeMap::new();
        for s in sections {
            let mut need = |k: &str| {
                t.text(&format!("{s}.{k}"))?
                    .ok_or_else(|| format!("[{s}] needs `{k}`"))
            };
            let moves = HandMoves {
                load: need("load")?,
                fire: need("fire")?,
                holster: need("holster")?,
            };
            uses.insert(s, moves);
        }
        t.finish()?;
        Ok(HandSignal { arms, uses })
    }

    /// The arms settings and moves for an arms id made by [`arms_id`].
    pub fn for_arms(&self, id: &str) -> Option<(&ArmsParams, &HandMoves)> {
        let using = id.strip_prefix("hands:")?;
        Some((&self.arms, self.uses.get(using)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shipped_file() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/combat/hand_signal.toml"
        ))
        .unwrap();
        let h = HandSignal::from_text(&text).unwrap();
        assert_eq!(h.arms.mesh, "HudArmsHands");
        assert_eq!(h.arms.fov, 85.0);
        let (_, m) = h.for_arms(&arms_id("bacta")).unwrap();
        assert_eq!(
            (m.load.as_str(), m.fire.as_str(), m.holster.as_str()),
            ("BactaLoad", "BactaLoop", "BactaHolster")
        );
        assert!(h.for_arms("dc17m_blaster").is_none());
    }

    #[test]
    fn a_use_without_all_three_moves_is_an_error() {
        let e = HandSignal::from_text("[arms]\nmesh = \"X\"\n[bacta]\nload = \"A\"\n").unwrap_err();
        assert!(e.contains("fire"), "{e}");
    }
}
