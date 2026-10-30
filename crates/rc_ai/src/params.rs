//! A character's AI settings, read from a data file such as
//! `data/enemies/battle_droid.toml`.
//!
//! The file has sections. `[type]` says which of the game's classes the
//! file is for. `[movement]` goes to `rc_movement::MoveParams` (same names
//! as the player's file). `[senses]`, `[combat]`, `[grenades]`, `[spawning]` and
//! `[look]` are read here. Every value has a built-in default (the
//! battle droid's), so a mod's file only needs the lines it changes. A
//! misspelt name is an error naming the line.

use rc_movement::MoveParams;

/// Which enemy this is, and which of the game's classes it stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct EnemyType {
    /// The settings file's name without `.toml`, like `battle_droid`.
    pub id: String,
    /// For the log, like "battle droid".
    pub name: String,
    /// The game's class names a map's spawner may make (its `Prototype`),
    /// compared ignoring letter case. A name ending in `*` matches every
    /// class starting with the rest (`BattleDroid*`).
    pub classes: Vec<String>,
    /// "droid" or "organic": for damage tables and stuns (EMP, flash).
    pub body: String,
}

impl EnemyType {
    pub fn matches(&self, class: &str) -> bool {
        let class = class.to_ascii_lowercase();
        self.classes.iter().any(|c| {
            let c = c.to_ascii_lowercase();
            match c.strip_suffix('*') {
                Some(prefix) => class.starts_with(prefix),
                None => class == c,
            }
        })
    }
}

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
    /// Looks for cover when shot in the open. Super battle droids don't.
    pub uses_cover: bool,
    /// The weapon file left on the ground when it falls, for the player to
    /// pick up. Empty for none.
    pub drops: String,
    /// The original's damage table for this enemy (`DamageVariance`), by
    /// name in `data/combat/damage_types.toml`. Empty for none.
    pub damage_table: String,
    /// Close-up hits (a Geonosian's force pike): reach, uu (0 for none),
    /// damage, its damage type, and seconds between hits.
    pub melee_range: f32,
    pub melee_damage: f32,
    pub melee_damage_type: String,
    pub melee_delay: f32,
    /// Rushes in to hit when its enemy is this close, uu. An enemy with
    /// no gun (`weapon = ""`) always rushes in.
    pub lunge_distance: f32,
}

/// Grenades it throws (`[grenades]`). An empty `grenade` means none (the
/// battle droid's default: the original's droids carry none).
#[derive(Debug, Clone, PartialEq)]
pub struct GrenadeUse {
    /// A grenade file in `data/grenades/` without `.toml`. Empty for none.
    pub grenade: String,
    /// How many it carries.
    pub count: u32,
    /// Chance of throwing at each decision when it may (`GrenadeProbability`).
    pub probability: f32,
    /// Seconds between one fighter's throws, at least (`MinGrenadeTime`).
    pub min_time: f32,
    /// It throws only at enemies between these distances, uu (the
    /// grenade weapon's `ValidRange`).
    pub min_range: f32,
    pub max_range: f32,
    /// It only throws at an enemy it saw within this many seconds.
    pub seen_within: f32,
    /// Seconds the throw takes, during which it doesn't shoot.
    pub throw_time: f32,
    /// The grenade's flight, to aim the arc: fastest throw (uu/s),
    /// downward pull (uu/s²) and blast radius (uu, to keep friends out of
    /// it). The game fills these from the grenade's own file.
    pub speed: f32,
    pub gravity: f32,
    pub radius: f32,
}

impl GrenadeUse {
    pub fn carries(&self) -> bool {
        !self.grenade.is_empty() && self.count > 0
    }
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
    /// Throwing a grenade. Empty for none (it plays the idle animation).
    pub throw: String,
    /// A close-up hit. Empty for none.
    pub melee: String,
    /// Light the character always gets, 0 to 1 of its full colour, on top
    /// of the level's lights (the original lights characters with each
    /// zone's ambient light, which we don't read yet).
    pub ambient: f32,
}

/// What the character says or sounds like: sound names in `package`
/// (`GameData/Sounds/<package>.uax`). One is picked at random each time.
/// Empty lists play nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sounds {
    pub package: String,
    /// On first seeing the enemy.
    pub spotted: Vec<String>,
    pub hurt: Vec<String>,
    pub death: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiParams {
    pub kind: EnemyType,
    pub sounds: Sounds,
    pub movement: MoveParams,
    pub senses: Senses,
    pub combat: Combat,
    pub grenades: GrenadeUse,
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
            kind: EnemyType {
                id: "battle_droid".into(),
                name: "battle droid".into(),
                classes: vec!["BattleDroid*".into()],
                body: "droid".into(),
            },
            sounds: Sounds {
                package: "droid_sound".into(),
                ..Default::default()
            },
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
                uses_cover: true,
                drops: "e5_blaster_pickup".into(),
                damage_table: "DV_DroidsBattleDroidDamage".into(),
                melee_range: 0.0,
                melee_damage: 0.0,
                melee_damage_type: "CTDamageMelee".into(),
                melee_delay: 1.5,
                lunge_distance: 0.0,
            },
            grenades: GrenadeUse {
                grenade: String::new(),
                count: 0,
                probability: 0.3,
                min_time: 10.0,
                min_range: 600.0,
                max_range: 2500.0,
                seen_within: 3.0,
                throw_time: 1.0,
                speed: 2200.0,
                gravity: 1100.0,
                radius: 600.0,
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
                throw: String::new(),
                melee: String::new(),
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
            self.set(&section, key, value)
                .map_err(|e| format!("line {n}: {e}"))?;
        }
        // Line numbers stay right because every line was kept (blank or not).
        self.movement = self.movement.with_overrides(&movement_lines)?;
        Ok(self)
    }

    fn set(&mut self, section: &str, key: &str, value: &str) -> Result<(), String> {
        let num = || -> Result<f32, String> {
            value
                .parse()
                .map_err(|_| format!("`{value}` is not a number"))
        };
        let s = &mut self.senses;
        let c = &mut self.combat;
        let p = &mut self.spawning;
        let l = &mut self.look;
        let text = || value.trim_matches('"').to_string();
        let boolean = || match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("`{value}` should be true or false")),
        };
        let list = || -> Result<Vec<String>, String> {
            let inner = value
                .strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .ok_or_else(|| format!("`{value}` should be a list like [\"A\", \"B\"]"))?;
            Ok(inner
                .split(',')
                .map(|x| x.trim().trim_matches('"').to_string())
                .filter(|x| !x.is_empty())
                .collect())
        };
        let k = &mut self.kind;
        let snd = &mut self.sounds;
        let g = &mut self.grenades;
        match (section, key) {
            ("grenades", "grenade") => g.grenade = text(),
            ("grenades", "count") => g.count = num()?.max(0.0) as u32,
            ("grenades", "probability") => g.probability = num()?.clamp(0.0, 1.0),
            ("grenades", "min_time") => g.min_time = num()?,
            ("grenades", "min_range") => g.min_range = num()?,
            ("grenades", "max_range") => g.max_range = num()?,
            ("grenades", "seen_within") => g.seen_within = num()?,
            ("grenades", "throw_time") => g.throw_time = num()?,
            ("grenades", "speed") => g.speed = num()?,
            ("grenades", "gravity") => g.gravity = num()?,
            ("grenades", "radius") => g.radius = num()?,
            ("sound", "package") => snd.package = text(),
            ("sound", "spotted") => snd.spotted = list()?,
            ("sound", "hurt") => snd.hurt = list()?,
            ("sound", "death") => snd.death = list()?,
            ("type", "id") => k.id = text(),
            ("type", "name") => k.name = text(),
            ("type", "classes") => k.classes = list()?,
            ("type", "body") => {
                k.body = text();
                if !matches!(k.body.as_str(), "droid" | "organic") {
                    return Err("body should be \"droid\" or \"organic\"".into());
                }
            }
            ("combat", "uses_cover") => c.uses_cover = boolean()?,
            ("combat", "drops") => c.drops = text(),
            ("combat", "damage_table") => c.damage_table = text(),
            ("combat", "melee_range") => c.melee_range = num()?,
            ("combat", "melee_damage") => c.melee_damage = num()?,
            ("combat", "melee_damage_type") => c.melee_damage_type = text(),
            ("combat", "melee_delay") => c.melee_delay = num()?,
            ("combat", "lunge_distance") => c.lunge_distance = num()?,
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
            ("look", "throw") => l.throw = text(),
            ("look", "melee") => l.melee = text(),
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
        out.push(format!("{:?}", self.kind));
        out.push(format!("{:?}", self.senses));
        out.push(format!("{:?}", self.combat));
        out.push(format!("{:?}", self.grenades));
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
