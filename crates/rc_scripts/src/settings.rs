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
    /// Unreal units: how far a placed sound carries at full loudness when
    /// the game's own settings for it (`AudioParams`) give no `Radius`.
    pub sound_radius: f32,
    /// How long an animation lasts when a script waits for its end.
    pub anim_seconds: f32,
    /// How long a character takes to walk where a script sends it, when
    /// no game carries it out (the headless tools).
    pub move_seconds: f32,
    /// In the game: the longest a script waits for a character to arrive
    /// where it sent it before it carries on anyway.
    pub move_timeout_seconds: f32,
    /// In the game: the longest a script waits for an animation to end.
    pub anim_timeout_seconds: f32,
    /// In the game: the longest a script waits for a character to turn.
    pub turn_timeout_seconds: f32,
    /// In the game: how long an action waits for its characters to appear
    /// (a factory's character comes a frame after its event) before it is
    /// skipped.
    pub character_grace_seconds: f32,
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
    /// Units in front of a dispenser the player is pulled to while using
    /// it (the game's `PlayerOffset`).
    pub bacta_player_offset: f32,
    /// Pull the player into place and hold them there while they use a
    /// dispenser (the game's `bLockPlayerLocation`).
    pub bacta_lock_player: bool,
    /// How fast the pull moves the player (units a second) and turns them
    /// to face the dispenser (degrees a second).
    pub bacta_pull_speed: f32,
    pub bacta_turn_speed: f32,
    /// The squad marker hologram shown at a dispenser's squad spot, as
    /// `Package.Group.Mesh`; empty for none.
    pub bacta_hologram: String,
    /// How far (uu) the crosshair reaches to show the hologram.
    pub bacta_hologram_range: f32,
    /// The hologram's colour (sRGB, 0 to 255).
    pub bacta_hologram_colour: [u8; 3],
    /// Seconds the hologram takes to fade in or out.
    pub bacta_hologram_fade_seconds: f32,
    /// How bright the hologram is drawn, 0 to 1.
    pub bacta_hologram_brightness: f32,
    /// The line shown while you can use a dispenser; `@` is the Use
    /// button. Empty: the game's own (BactaDispenser `ActivatePromptText`).
    pub bacta_prompt: String,
    /// Sounds (`Package.Group.Name`, empty for none): the crosshair moves
    /// onto a dispenser; healing (repeats); health full.
    pub bacta_sound_aim: String,
    pub bacta_sound_healing: String,
    pub bacta_sound_full: String,
    /// Size (uu) of a level exit (`Teleporter`) the map gives none.
    pub teleporter_radius: f32,
    pub teleporter_height: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound_seconds: 3.0,
            sound_volume: 1.0,
            // Engine.u's AudioParams default Radius.
            sound_radius: 2000.0,
            anim_seconds: 1.5,
            move_seconds: 2.0,
            move_timeout_seconds: 30.0,
            anim_timeout_seconds: 10.0,
            turn_timeout_seconds: 2.0,
            character_grace_seconds: 1.0,
            random_seed: 1138,
            player_tags: vec!["PlayerCommando".into(), "Player".into()],
            prop_health: 100.0,
            max_actions_per_frame: 64,
            bacta_range: 256.0,
            bacta_heal_per_second: 50.0,
            bacta_recharge_seconds: 10.0,
            // BactaDispenserDefaults (Properties/properties.u).
            bacta_player_offset: 90.0,
            // BactaDispenser (System/ctgame.u).
            bacta_lock_player: true,
            bacta_pull_speed: 300.0,
            bacta_turn_speed: 360.0,
            // MarkerGetBacta's Participants IconMesh (System/ctgame.u).
            bacta_hologram: "MarkerIcons.Revive.reviveSM1BactaDispenser".into(),
            // SquadMarker's TargetingDistance (System/engine.u).
            bacta_hologram_range: 3000.0,
            // SquadMarker's EnabledColor and OmitColor (System/engine.u).
            bacta_hologram_colour: [122, 206, 255],
            bacta_hologram_fade_seconds: 0.25,
            bacta_prompt: String::new(),
            bacta_hologram_brightness: 0.6,
            // CTHUDDefaults (Properties/properties.u): TargetMarker,
            // HealthRecharging, HealthCharged.
            bacta_sound_aim: "GEN_Sound.Interface.int_GEN_retCrossZoom_01".into(),
            bacta_sound_healing: "GEN_Sound.Interface.int_GEN_hudHlthChrgng_02".into(),
            bacta_sound_full: "GEN_Sound.Interface.int_GEN_hudHlthChrgd_01".into(),
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
    default_radius: Option<f32>,
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
    player_offset: Option<f32>,
    lock_player: Option<bool>,
    pull_speed: Option<f32>,
    turn_speed: Option<f32>,
    hologram: Option<String>,
    hologram_range: Option<f32>,
    hologram_colour: Option<[u8; 3]>,
    hologram_fade_seconds: Option<f32>,
    prompt: Option<String>,
    hologram_brightness: Option<f32>,
    sound_aim: Option<String>,
    sound_healing: Option<String>,
    sound_full: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Timing {
    sound_seconds: Option<f32>,
    anim_seconds: Option<f32>,
    move_seconds: Option<f32>,
    move_timeout_seconds: Option<f32>,
    anim_timeout_seconds: Option<f32>,
    turn_timeout_seconds: Option<f32>,
    character_grace_seconds: Option<f32>,
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
        if let Some(v) = t.move_timeout_seconds {
            self.move_timeout_seconds = v;
        }
        if let Some(v) = t.anim_timeout_seconds {
            self.anim_timeout_seconds = v;
        }
        if let Some(v) = t.turn_timeout_seconds {
            self.turn_timeout_seconds = v;
        }
        if let Some(v) = t.character_grace_seconds {
            self.character_grace_seconds = v;
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
        if let Some(v) = b.player_offset {
            self.bacta_player_offset = v;
        }
        if let Some(v) = b.lock_player {
            self.bacta_lock_player = v;
        }
        if let Some(v) = b.pull_speed {
            self.bacta_pull_speed = v.max(1.0);
        }
        if let Some(v) = b.turn_speed {
            self.bacta_turn_speed = v.max(1.0);
        }
        if let Some(v) = b.hologram {
            self.bacta_hologram = v;
        }
        if let Some(v) = b.hologram_range {
            self.bacta_hologram_range = v;
        }
        if let Some(v) = b.hologram_colour {
            self.bacta_hologram_colour = v;
        }
        if let Some(v) = b.hologram_fade_seconds {
            self.bacta_hologram_fade_seconds = v.max(0.0);
        }
        if let Some(v) = b.prompt {
            self.bacta_prompt = v;
        }
        if let Some(v) = b.hologram_brightness {
            self.bacta_hologram_brightness = v.clamp(0.0, 1.0);
        }
        if let Some(v) = b.sound_aim {
            self.bacta_sound_aim = v;
        }
        if let Some(v) = b.sound_healing {
            self.bacta_sound_healing = v;
        }
        if let Some(v) = b.sound_full {
            self.bacta_sound_full = v;
        }
        if let Some(v) = f.sound.volume {
            self.sound_volume = v.clamp(0.0, 1.0);
        }
        if let Some(v) = f.sound.default_radius {
            self.sound_radius = v.max(1.0);
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
