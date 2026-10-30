//! `data/scripts/scripts.toml`: the values that are our own choices, not
//! the map's, so they can be changed without rebuilding.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// How long a line of dialogue lasts when a script waits for it and
    /// the game couldn't read the sound's own length.
    pub sound_seconds: f32,
    /// Loudness of the sounds and lines scripts play, 0 to 1 (times the
    /// action's own `Volume`).
    pub sound_volume: f32,
    /// How long an animation lasts when a script waits for its end.
    pub anim_seconds: f32,
    /// How long a character takes to walk where a script sends it.
    pub move_seconds: f32,
    /// Seed for random waits.
    pub random_seed: u64,
    /// Tags that mean the player.
    pub player_tags: Vec<String>,
    /// Health of a prop the map gives none.
    pub prop_health: f32,
    /// Most actions one script runs in one frame.
    pub max_actions_per_frame: usize,
    /// How near (uu) the player must stand to use a bacta dispenser.
    pub bacta_range: f32,
    /// Health a bacta dispenser gives per second while used.
    pub bacta_heal_per_second: f32,
    /// Seconds before a used dispenser works again.
    pub bacta_recharge_seconds: f32,
    /// Size (uu) of a level exit (`Teleporter`) the map gives none.
    pub teleporter_radius: f32,
    pub teleporter_height: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound_seconds: 3.0,
            sound_volume: 1.0,
            anim_seconds: 1.5,
            move_seconds: 2.0,
            random_seed: 1138,
            player_tags: vec!["PlayerCommando".into(), "Player".into()],
            prop_health: 100.0,
            max_actions_per_frame: 64,
            bacta_range: 256.0,
            bacta_heal_per_second: 50.0,
            bacta_recharge_seconds: 10.0,
            // engine.u's Teleporter defaults (from NavigationPoint).
            teleporter_radius: 80.0,
            teleporter_height: 100.0,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    timing: Timing,
    #[serde(default)]
    actors: Actors,
    #[serde(default)]
    bacta: Bacta,
    #[serde(default)]
    sound: Sound,
    #[serde(default)]
    exits: Exits,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Sound {
    volume: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Exits {
    teleporter_radius: Option<f32>,
    teleporter_height: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Bacta {
    range: Option<f32>,
    heal_per_second: Option<f32>,
    recharge_seconds: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Timing {
    sound_seconds: Option<f32>,
    anim_seconds: Option<f32>,
    move_seconds: Option<f32>,
    random_seed: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Actors {
    player_tags: Option<Vec<String>>,
    prop_health: Option<f32>,
    max_actions_per_frame: Option<usize>,
}

impl Settings {
    /// Start from `self` and apply the file's values. An unknown name is
    /// an error, so a typo in a mod is reported, not ignored.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let t = f.timing;
        let a = f.actors;
        if let Some(v) = t.sound_seconds {
            self.sound_seconds = v;
        }
        if let Some(v) = t.anim_seconds {
            self.anim_seconds = v;
        }
        if let Some(v) = t.move_seconds {
            self.move_seconds = v;
        }
        if let Some(v) = t.random_seed {
            self.random_seed = v;
        }
        if let Some(v) = a.player_tags {
            self.player_tags = v;
        }
        if let Some(v) = a.prop_health {
            self.prop_health = v;
        }
        if let Some(v) = a.max_actions_per_frame {
            self.max_actions_per_frame = v.max(1);
        }
        let b = f.bacta;
        if let Some(v) = b.range {
            self.bacta_range = v;
        }
        if let Some(v) = b.heal_per_second {
            self.bacta_heal_per_second = v;
        }
        if let Some(v) = b.recharge_seconds {
            self.bacta_recharge_seconds = v;
        }
        if let Some(v) = f.sound.volume {
            self.sound_volume = v.clamp(0.0, 1.0);
        }
        if let Some(v) = f.exits.teleporter_radius {
            self.teleporter_radius = v;
        }
        if let Some(v) = f.exits.teleporter_height {
            self.teleporter_height = v;
        }
        Ok(self)
    }

    /// Whether a tag means the player.
    pub fn is_player(&self, tag: &str) -> bool {
        self.player_tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }
}
