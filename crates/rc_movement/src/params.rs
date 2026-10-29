//! The movement numbers, taken from the installed game.
//!
//! Distances are Unreal units ("uu"), times are seconds. Where each number
//! came from is noted next to it. "Defaults" means the class's default
//! property values stored in the game's script packages, read with our own
//! package reader (see docs/RESEARCH.md, "Player movement"). The player is
//! `CTCharacters.PlayerCommando` (named in `System/user.ini`), whose parent
//! classes are CloneCommando, Republic, CTPawn and Engine's Pawn. A value set
//! by a class wins over its parents.

const FIELD_NAMES: [&str; 19] = [
    "ground_speed",
    "accel_rate",
    "jump_z",
    "air_control",
    "walk_speed_ratio",
    "crouch_speed_ratio",
    "back_speed_ratio",
    "side_speed_ratio",
    "gravity_z",
    "ground_friction",
    "terminal_velocity",
    "max_fall_speed",
    "collision_radius",
    "collision_height",
    "crouch_height",
    "crouch_radius",
    "base_eye_height",
    "max_step_height",
    "min_floor_z",
];

/// Everything the movement code needs to know about one kind of character.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveParams {
    /// Top running speed, uu/s.
    pub ground_speed: f32,
    /// Most acceleration the player's input can give, uu/s².
    pub accel_rate: f32,
    /// Upward speed given by a jump, uu/s.
    pub jump_z: f32,
    /// Fraction of `accel_rate` the player keeps in the air.
    pub air_control: f32,
    /// Top speed multiplier while walking (Shift held).
    pub walk_speed_ratio: f32,
    /// Top speed multiplier while crouched.
    pub crouch_speed_ratio: f32,
    /// Top speed multiplier moving straight backwards.
    pub back_speed_ratio: f32,
    /// Top speed multiplier moving straight sideways.
    pub side_speed_ratio: f32,
    /// Gravity, uu/s² (negative is down).
    pub gravity_z: f32,
    /// Ground friction. Higher stops and turns faster.
    pub ground_friction: f32,
    /// Fastest possible fall, uu/s.
    pub terminal_velocity: f32,
    /// Landing faster than this hurts, uu/s. Half of it shakes the view.
    pub max_fall_speed: f32,
    /// Collision box half-width, uu.
    pub collision_radius: f32,
    /// Collision box half-height, uu (the full box is twice this tall).
    pub collision_height: f32,
    /// Collision half-height while crouched, uu.
    pub crouch_height: f32,
    /// Collision half-width while crouched, uu.
    pub crouch_radius: f32,
    /// Eye height above the collision centre when standing, uu.
    pub base_eye_height: f32,
    /// Tallest step the player walks up without jumping, uu.
    pub max_step_height: f32,
    /// A surface is floor if its normal's up part is at least this
    /// (0.7 is a slope of about 45.6 degrees).
    pub min_floor_z: f32,
}

impl MoveParams {
    /// Delta 38, the player character.
    pub fn player_commando() -> Self {
        MoveParams {
            ground_speed: 450.0,      // CloneCommando defaults (Pawn has 600)
            accel_rate: 1024.0,       // CloneCommando defaults (Pawn has 2048)
            jump_z: 475.0,            // CloneCommando defaults
            air_control: 0.35,        // Pawn defaults
            walk_speed_ratio: 0.5,    // Pawn defaults
            crouch_speed_ratio: 0.5,  // Pawn defaults
            back_speed_ratio: 0.6,    // Pawn defaults
            side_speed_ratio: 0.75,   // CloneCommando defaults (Pawn has 0.8)
            gravity_z: -1100.0,       // PhysicsVolume defaults; LevelInfo DefaultGravity agrees
            ground_friction: 8.0,     // PhysicsVolume defaults
            terminal_velocity: 12000.0, // PhysicsVolume defaults
            max_fall_speed: 1200.0,   // Pawn defaults
            collision_radius: 40.0,   // Pawn defaults
            collision_height: 84.0,   // CloneCommando defaults (Pawn has 78)
            crouch_height: 56.0,      // CloneCommando defaults
            crouch_radius: 40.0,      // Pawn defaults
            base_eye_height: 60.0,    // CloneCommando defaults (Pawn has 72)
            max_step_height: 35.0,    // Engine script constant MAXSTEPHEIGHT
            min_floor_z: 0.7,         // Engine script constant MINFLOORZ
        }
    }

    /// Read a settings file, starting from `self` and changing only the
    /// values the file names. The format is one `name = number` per line,
    /// with `#` starting a comment and `[section]` lines ignored. Names are
    /// the field names above. An unknown name or a bad number is an error,
    /// with its line number, so a typo in a mod is reported, not ignored.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let n = i + 1;
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected `name = number`, got `{line}`"))?;
            let key = key.trim();
            let value: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("line {n}: `{}` is not a number", value.trim()))?;
            let slot = self
                .field_mut(key)
                .ok_or_else(|| format!("line {n}: unknown setting `{key}`"))?;
            *slot = value;
        }
        Ok(self)
    }

    /// Every setting with its name, in file order.
    pub fn fields(&self) -> Vec<(&'static str, f32)> {
        let mut c = self.clone();
        FIELD_NAMES
            .iter()
            .map(|&n| (n, *c.field_mut(n).expect("listed field")))
            .collect()
    }

    fn field_mut(&mut self, name: &str) -> Option<&mut f32> {
        Some(match name {
            "ground_speed" => &mut self.ground_speed,
            "accel_rate" => &mut self.accel_rate,
            "jump_z" => &mut self.jump_z,
            "air_control" => &mut self.air_control,
            "walk_speed_ratio" => &mut self.walk_speed_ratio,
            "crouch_speed_ratio" => &mut self.crouch_speed_ratio,
            "back_speed_ratio" => &mut self.back_speed_ratio,
            "side_speed_ratio" => &mut self.side_speed_ratio,
            "gravity_z" => &mut self.gravity_z,
            "ground_friction" => &mut self.ground_friction,
            "terminal_velocity" => &mut self.terminal_velocity,
            "max_fall_speed" => &mut self.max_fall_speed,
            "collision_radius" => &mut self.collision_radius,
            "collision_height" => &mut self.collision_height,
            "crouch_height" => &mut self.crouch_height,
            "crouch_radius" => &mut self.crouch_radius,
            "base_eye_height" => &mut self.base_eye_height,
            "max_step_height" => &mut self.max_step_height,
            "min_floor_z" => &mut self.min_floor_z,
            _ => return None,
        })
    }

    /// Eye height above the centre while crouched. The game's script uses
    /// the smaller of 80% of the crouch height and the crouch height minus 10.
    pub fn crouched_eye_height(&self) -> f32 {
        (0.8 * self.crouch_height).min(self.crouch_height - 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_from_text() {
        let text = "# a mod\n[player]\nground_speed = 900  # twice as fast\njump_z=600\n";
        let p = MoveParams::player_commando().with_overrides(text).unwrap();
        assert_eq!(p.ground_speed, 900.0);
        assert_eq!(p.jump_z, 600.0);
        assert_eq!(p.accel_rate, 1024.0);
    }

    #[test]
    fn overrides_report_mistakes() {
        let base = MoveParams::player_commando();
        let e = base.clone().with_overrides("\ngrund_speed = 1").unwrap_err();
        assert_eq!(e, "line 2: unknown setting `grund_speed`");
        let e = base.with_overrides("jump_z = high").unwrap_err();
        assert_eq!(e, "line 1: `high` is not a number");
    }

    #[test]
    fn every_field_is_named() {
        let p = MoveParams::player_commando();
        let f = p.fields();
        assert_eq!(f.len(), FIELD_NAMES.len());
        assert_eq!(f[0], ("ground_speed", 450.0));
    }
}
