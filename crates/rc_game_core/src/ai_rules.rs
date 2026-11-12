//! Who the AI fights and how far it sees: the rules level scripts change.
//!
//! In the original every character has a team (`Pawn.TeamIndex`, 0 to 9)
//! and every team says of each other team whether it is an enemy, neutral
//! or an ally (`CTTeamInfo.RelationTowards`). A computer-run character
//! (`CTBot`) picks its enemy in `ULocalSituation::Assess` (ctgame.dll,
//! read with a disassembler, nothing copied): of everyone alive and a
//! valid enemy, within its pawn's `SightRadius`, it counts only those its
//! own team treats as an enemy **and** whose `bPerceivedAsThreat` is on,
//! that it can see or hear; it takes the nearest, except that an enemy a
//! script suggested (`ACTION_SuggestEnemy`) is taken whenever it is seen.
//!
//! Level scripts change these values:
//!
//! - `ACTION_SetTeamIndex` sets `TeamIndex` (an action that names no
//!   team sets 0, the neutral team);
//! - `ACTION_SetThreat` sets `bPerceivedAsThreat`. **Not found:** the
//!   flag's class default is off, and the only compiled code found that
//!   switches it on is a character waking from a shut-down or perched
//!   state; where ordinary characters get it is not known yet. Here every
//!   character starts with it on (so everyone fights as before) and only a
//!   script switches it off;
//! - `ACTION_SetSightRadius` sets `SightRadius`, or puts back the class
//!   default (`ResetToDefaults`);
//! - `ACTION_SuggestEnemy` sets the bot's suggested enemy (`None` clears
//!   it).
//!
//! [`AiRules`] carries those values on a character, [`TeamTable`] the
//! teams. The parts that run characters (`rc_enemies` for droids and clone
//! troopers, `rc_squad_plugin` for squad mates) read them; the level
//! scripts part (`rc_scripts_plugin`) writes them.

use bevy::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

/// How many teams the game has (`MAXTEAMS` in Core.u).
pub const TEAMS: usize = 10;

/// `ETeamRelationship` (Core.u): `TEAM_Enemy`, `TEAM_Neutral`, `TEAM_Ally`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Enemy,
    Neutral,
    Ally,
}

impl Relation {
    /// The game's byte value (0 enemy, 1 neutral, 2 ally).
    pub fn from_byte(b: u8) -> Option<Relation> {
        match b {
            0 => Some(Relation::Enemy),
            1 => Some(Relation::Neutral),
            2 => Some(Relation::Ally),
            _ => None,
        }
    }

    fn letter(c: char) -> Option<Relation> {
        match c.to_ascii_uppercase() {
            'E' => Some(Relation::Enemy),
            'N' => Some(Relation::Neutral),
            'A' => Some(Relation::Ally),
            _ => None,
        }
    }
}

/// Every team's relation towards every team: `rel[a][b]` is how a
/// character of team `a` treats one of team `b`. Read from
/// `data/ai/teams.toml`, then changed by the map's own `TeamOverride`
/// actors ([`TeamTable::set_team`]).
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct TeamTable {
    pub rel: [[Relation; TEAMS]; TEAMS],
}

#[derive(Deserialize)]
struct TeamsFile {
    relations: BTreeMap<String, String>,
}

impl TeamTable {
    /// Read `teams.toml`: a `[relations]` table with one line per team,
    /// `"N" = "EAAAEEEEEE"`, one letter per team 0 to 9 (E enemy, N
    /// neutral, A ally). Teams it leaves out treat everyone as an enemy
    /// (the game's default relation is `TEAM_Enemy`).
    pub fn from_toml(text: &str) -> Result<TeamTable, String> {
        let file: TeamsFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut rel = [[Relation::Enemy; TEAMS]; TEAMS];
        for (k, v) in &file.relations {
            let a: usize = k
                .trim()
                .parse()
                .ok()
                .filter(|a| *a < TEAMS)
                .ok_or_else(|| format!("team \"{k}\" should be 0 to 9"))?;
            let letters: Vec<char> = v.chars().filter(|c| !c.is_whitespace()).collect();
            if letters.len() != TEAMS {
                return Err(format!(
                    "team {a}: \"{v}\" should have {TEAMS} letters (E, N or A)"
                ));
            }
            for (b, c) in letters.into_iter().enumerate() {
                rel[a][b] = Relation::letter(c)
                    .ok_or_else(|| format!("team {a}: '{c}' should be E, N or A"))?;
            }
        }
        Ok(TeamTable { rel })
    }

    /// How team `a` treats team `b`. Teams outside 0 to 9 are neutral to
    /// everyone (the game looks no relation up for them).
    pub fn relation(&self, a: u8, b: u8) -> Relation {
        self.rel
            .get(a as usize)
            .and_then(|r| r.get(b as usize))
            .copied()
            .unwrap_or(Relation::Neutral)
    }

    pub fn hostile(&self, a: u8, b: u8) -> bool {
        self.relation(a, b) == Relation::Enemy
    }

    /// A map's `TeamOverride` actor replaces team `index` with its own
    /// `RelationTowards` (`GameReplicationInfo.UseTeam`); relations it
    /// doesn't set are the default, `TEAM_Enemy`.
    pub fn set_team(&mut self, index: usize, relations: &[Option<u8>; TEAMS]) {
        if index >= TEAMS {
            return;
        }
        for (b, r) in relations.iter().enumerate() {
            self.rel[index][b] = r.and_then(Relation::from_byte).unwrap_or(Relation::Enemy);
        }
    }

    /// Whom each team treats as an enemy, as a plain grid.
    pub fn hostile_grid(&self) -> [[bool; TEAMS]; TEAMS] {
        let mut g = [[false; TEAMS]; TEAMS];
        for (a, row) in self.rel.iter().enumerate() {
            for (b, r) in row.iter().enumerate() {
                g[a][b] = *r == Relation::Enemy;
            }
        }
        g
    }
}

/// The player's team: `CTGameInfoSP.PickTeam` puts every
/// `PlayerController` on team 1.
pub const PLAYER_TEAM: u8 = 1;

/// What the AI needs to know about one character that level scripts can
/// change. See the module notes.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct AiRules {
    /// The game's class (`BattleDroid`, `CloneCommando`, ...), for
    /// `ACTION_SetSightRadius`'s `AffectPawnsOfClass`, which takes the
    /// class and every class under it.
    pub class: String,
    /// The classes above `class`, nearest first, once the game's classes
    /// have been read (`rc_enemies` fills it in); `None` until then.
    pub parents: Option<Vec<String>>,
    /// `TeamIndex`.
    pub team: u8,
    /// `bPerceivedAsThreat`.
    pub threat: bool,
    /// `SightRadius` when a script set it; `None` is the character's own.
    pub sight_radius: Option<f32>,
    /// The enemy a script suggested (`ACTION_SuggestEnemy`).
    pub suggested_enemy: Option<Entity>,
}

impl AiRules {
    pub fn new(class: impl Into<String>, team: u8) -> Self {
        AiRules {
            class: class.into(),
            parents: None,
            team,
            threat: true,
            sight_radius: None,
            suggested_enemy: None,
        }
    }

    /// UnrealScript's `IsA`: its class is `class` or under it (letter case
    /// ignored).
    pub fn is_a(&self, class: &str) -> bool {
        self.class.eq_ignore_ascii_case(class)
            || self
                .parents
                .iter()
                .flatten()
                .any(|p| p.eq_ignore_ascii_case(class))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> TeamTable {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/ai/teams.toml"
        ))
        .unwrap();
        TeamTable::from_toml(&text).unwrap()
    }

    #[test]
    fn the_games_teams_read_from_the_data_file() {
        let t = table();
        // Droids (team 5) and the player's side (1, 2) are enemies.
        assert!(t.hostile(5, 1) && t.hostile(5, 2) && t.hostile(2, 5));
        // The squad and the player are allies.
        assert_eq!(t.relation(2, 1), Relation::Ally);
        // The neutral team treats everyone as neutral.
        assert!((0..10).all(|b| t.relation(0, b) == Relation::Neutral));
        // The berserker team is everyone's enemy, itself included.
        assert!((0..10).all(|b| t.hostile(9, b)));
    }

    #[test]
    fn a_map_override_replaces_a_teams_relations() {
        // geo_01a's TeamOverride2: team 7 neutral to 1, 2, 6, ally to
        // 4, 5, 7.
        let mut t = table();
        let mut r = [None; TEAMS];
        for (b, v) in [(1, 1), (2, 1), (4, 2), (5, 2), (6, 1), (7, 2)] {
            r[b] = Some(v);
        }
        t.set_team(7, &r);
        assert_eq!(t.relation(7, 1), Relation::Neutral);
        assert_eq!(t.relation(7, 5), Relation::Ally);
        assert!(t.hostile(7, 0) && t.hostile(7, 3));
    }

    #[test]
    fn is_a_takes_the_class_and_its_parents() {
        let mut r = AiRules::new("Commando07Delta", 2);
        assert!(r.is_a("commando07delta") && !r.is_a("CloneCommando"));
        r.parents = Some(vec!["CloneCommando".into(), "CTPawn".into()]);
        assert!(r.is_a("CloneCommando") && r.is_a("ctpawn") && !r.is_a("BattleDroid"));
    }

    #[test]
    fn bad_team_lines_are_refused() {
        assert!(TeamTable::from_toml("[relations]\n\"10\" = \"EEEEEEEEEE\"").is_err());
        assert!(TeamTable::from_toml("[relations]\n\"1\" = \"EEE\"").is_err());
        assert!(TeamTable::from_toml("[relations]\n\"1\" = \"XEEEEEEEEE\"").is_err());
    }
}
