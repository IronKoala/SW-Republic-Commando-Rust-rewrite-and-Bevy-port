//! A character's AI settings, read from a data file such as
//! `data/enemies/battle_droid.toml`.
//!
//! The file has sections. `[movement]` goes to `rc_movement::MoveParams`
//! (same names as the player's file). `[senses]`, `[combat]`,
//! `[spawning]` and `[look]` are read here. Every value has a built-in default (the
//! battle droid's), so a mod's file only needs the lines it changes. A
//! misspelt name is an error naming the line.

use rc_movement::MoveParams;

#[derive(Debug, Clone, PartialEq)]
pub struct Senses {
    pub sight_radius: f32,
    pub peripheral_vision: f32,
    pub auto_detect_radius: f32,
    pub enemy_memory_time: f32,
    pub decision_interval: f32,
    pub reaction_time: f32,
    /// Degrees per second.
    pub turn_rate: f32,
    /// Hears shots this far away, uu.
    pub hearing_radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combat {
    pub health: f32,
    pub accuracy: f32,
    /// Degrees of aim error at accuracy 0.
    pub max_aim_error: f32,
    pub weapon: String,
    pub burst_count: u32,
    pub shot_delay: f32,
    pub burst_pause_min: f32,
    pub burst_pause_max: f32,
    pub weapon_range: f32,
    pub max_aim_yaw: f32,
    pub ready_distance: f32,
    pub safe_distance: f32,
    pub move_interval: f32,
    pub max_maneuver_time: f32,
    pub cover_boredom_time: f32,
    pub cover_search_radius: f32,
    pub lost_sight_reposition: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spawning {
    pub activate_unscripted_within: f32,
    pub max_alive_total: u32,
}

/// How the character looks: its animated mesh and which of its animations
/// to play for each thing it does. Only the game's Bevy side uses this.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The animation package, relative to the game folder.
    pub package: String,
    pub mesh: String,
    pub idle: String,
    pub walk: String,
    pub run: String,
    pub walk_back: String,
    pub step_left: String,
    pub step_right: String,
    pub fire: String,
    pub crouch: String,
    pub crouch_fire: String,
    pub death: String,
    /// Light the character always gets, 0 to 1 of its full colour, on top
    /// of the level's lights (the original lights characters with each
    /// zone's ambient light, which we don't read yet).
    pub ambient: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiParams {
    pub movement: MoveParams,
    pub senses: Senses,
    pub combat: Combat,
    pub spawning: Spawning,
    pub look: Look,
}

impl AiParams {
    /// The battle droid (geo_01a's BattleDroidTM), from the game's class
    /// defaults where they exist. The same numbers as
    /// `data/enemies/battle_droid.toml`.
    pub fn battle_droid() -> Self {
        let mut m = MoveParams::player_commando();
        m.ground_speed = 375.0;
        m.accel_rate = 1000.0;
        m.walk_speed_ratio = 0.52;
        m.collision_radius = 35.0;
        m.collision_height = 80.0;
        m.crouch_radius = 35.0;
        m.crouch_height = 50.0;
        m.base_eye_height = 60.0;
        m.side_speed_ratio = 0.8;
        m.jump_z = 0.0;
        AiParams {
            movement: m,
            senses: Senses {
                sight_radius: 5000.0,
                peripheral_vision: 0.2,
                auto_detect_radius: 200.0,
                enemy_memory_time: 15.0,
                decision_interval: 0.5,
                reaction_time: 0.5,
                turn_rate: 110.0,
                hearing_radius: 2500.0,
            },
            combat: Combat {
                health: 100.0,
                accuracy: 0.4,
                max_aim_error: 10.0,
                weapon: "e5_blaster_tm".into(),
                burst_count: 3,
                shot_delay: 0.1,
                burst_pause_min: 1.0,
                burst_pause_max: 2.0,
                weapon_range: 7000.0,
                max_aim_yaw: 60.0,
                ready_distance: 800.0,
                safe_distance: 1200.0,
                move_interval: 400.0,
                max_maneuver_time: 10.0,
                cover_boredom_time: 15.0,
                cover_search_radius: 1500.0,
                lost_sight_reposition: 2.0,
            },
            spawning: Spawning {
                activate_unscripted_within: 3000.0,
                max_alive_total: 12,
            },
            look: Look {
                package: "GameData/Animations/droid.ukx".into(),
                mesh: "BattleDroid".into(),
                idle: "ActionBreathe".into(),
                walk: "WalkForward".into(),
                run: "RunForward".into(),
                walk_back: "WalkBackward".into(),
                step_left: "StepLeft".into(),
                step_right: "StepRight".into(),
                fire: "Fire".into(),
                crouch: "CrouchBreathe".into(),
                crouch_fire: "CrouchFire".into(),
                death: "Death".into(),
                ambient: 0.5,
            },
        }
    }

    /// Start from `self` and apply a settings file.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        let mut section = String::new();
        let mut movement_lines = String::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                movement_lines.push('\n');
                continue;
            }
            if let Some(s) = line.strip_prefix('[') {
                section = s.trim_end_matches(']').trim().to_string();
                movement_lines.push('\n');
                continue;
            }
            if section == "movement" {
                movement_lines.push_str(line);
                movement_lines.push('\n');
                continue;
            }
            movement_lines.push('\n');
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected `name = value`, got `{line}`"))?;
            let (key, value) = (key.trim(), value.trim());
            self.set(&section, key, value).map_err(|e| format!("line {n}: {e}"))?;
        }
        // Line numbers stay right because every line was kept (blank or not).
        self.movement = self.movement.with_overrides(&movement_lines)?;
        Ok(self)
    }

    fn set(&mut self, section: &str, key: &str, value: &str) -> Result<(), String> {
        let num = || -> Result<f32, String> {
            value.parse().map_err(|_| format!("`{value}` is not a number"))
        };
        let s = &mut self.senses;
        let c = &mut self.combat;
        let p = &mut self.spawning;
        let l = &mut self.look;
        let text = || value.trim_matches('"').to_string();
        match (section, key) {
            ("senses", "sight_radius") => s.sight_radius = num()?,
            ("senses", "peripheral_vision") => s.peripheral_vision = num()?,
            ("senses", "auto_detect_radius") => s.auto_detect_radius = num()?,
            ("senses", "enemy_memory_time") => s.enemy_memory_time = num()?,
            ("senses", "decision_interval") => s.decision_interval = num()?,
            ("senses", "reaction_time") => s.reaction_time = num()?,
            ("senses", "turn_rate") => s.turn_rate = num()?,
            ("senses", "hearing_radius") => s.hearing_radius = num()?,
            ("combat", "health") => c.health = num()?,
            ("combat", "accuracy") => c.accuracy = num()?,
            ("combat", "max_aim_error") => c.max_aim_error = num()?,
            ("combat", "weapon") => c.weapon = text(),
            ("combat", "burst_count") => c.burst_count = num()?.max(1.0) as u32,
            ("combat", "shot_delay") => c.shot_delay = num()?,
            ("combat", "burst_pause_min") => c.burst_pause_min = num()?,
            ("combat", "burst_pause_max") => c.burst_pause_max = num()?,
            ("combat", "weapon_range") => c.weapon_range = num()?,
            ("combat", "max_aim_yaw") => c.max_aim_yaw = num()?,
            ("combat", "ready_distance") => c.ready_distance = num()?,
            ("combat", "safe_distance") => c.safe_distance = num()?,
            ("combat", "move_interval") => c.move_interval = num()?,
            ("combat", "max_maneuver_time") => c.max_maneuver_time = num()?,
            ("combat", "cover_boredom_time") => c.cover_boredom_time = num()?,
            ("combat", "cover_search_radius") => c.cover_search_radius = num()?,
            ("combat", "lost_sight_reposition") => c.lost_sight_reposition = num()?,
            ("spawning", "activate_unscripted_within") => p.activate_unscripted_within = num()?,
            ("spawning", "max_alive_total") => p.max_alive_total = num()?.max(0.0) as u32,
            ("look", "package") => l.package = text(),
            ("look", "mesh") => l.mesh = text(),
            ("look", "idle") => l.idle = text(),
            ("look", "walk") => l.walk = text(),
            ("look", "run") => l.run = text(),
            ("look", "walk_back") => l.walk_back = text(),
            ("look", "step_left") => l.step_left = text(),
            ("look", "step_right") => l.step_right = text(),
            ("look", "fire") => l.fire = text(),
            ("look", "crouch") => l.crouch = text(),
            ("look", "crouch_fire") => l.crouch_fire = text(),
            ("look", "death") => l.death = text(),
            ("look", "ambient") => l.ambient = num()?,
            _ => return Err(format!("unknown setting `{key}` in [{section}]")),
        }
        Ok(())
    }

    /// Every setting as `section.name = value`, for the log.
    pub fn describe(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .movement
            .fields()
            .into_iter()
            .map(|(k, v)| format!("movement.{k} = {v}"))
            .collect();
        out.push(format!("{:?}", self.senses));
        out.push(format!("{:?}", self.combat));
        out.push(format!("{:?}", self.spawning));
        out.push(format!("{:?}", self.look));
        out
    }
}

/// Cut a `#` comment, but not a `#` inside quotes.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}
