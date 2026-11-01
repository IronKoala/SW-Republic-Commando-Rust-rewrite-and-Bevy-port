//! The visor's rules, with no Bevy in them so they can be tested alone.
//!
//! The commando's visor has three modes, in this order (the original's
//! `Pawn.VisionPawnUser[0..2]`):
//!
//! - **Normal**: the plain view.
//! - **Tactical**: squad mates glow in their own colour and things to pick
//!   up glow pale blue (each one's `EnhancedVisionColor`; black means no
//!   glow, which is why enemies and walls don't glow). The strength is the
//!   TACTICAL MODE INTENSITY option.
//! - **Low light**: the night view with the helmet lamp on.
//!
//! The TACTICAL VISOR MODE option (the original's `VisorModeDefault`)
//! decides which modes the CYCLE VISOR MODE key (X) steps through. These
//! rules were read from the original's scripts (`PlayerController` and
//! `CTPlayer` in Engine.u and CTGame.u) and rewritten by hand, never
//! copied.

/// Which mode the visor is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VisorMode {
    Normal,
    #[default]
    Tactical,
    LowLight,
}

impl VisorMode {
    /// The original's mode number (0, 1, 2).
    pub fn number(self) -> u8 {
        match self {
            VisorMode::Normal => 0,
            VisorMode::Tactical => 1,
            VisorMode::LowLight => 2,
        }
    }

    pub fn from_number(n: u8) -> Option<VisorMode> {
        match n {
            0 => Some(VisorMode::Normal),
            1 => Some(VisorMode::Tactical),
            2 => Some(VisorMode::LowLight),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            VisorMode::Normal => "normal",
            VisorMode::Tactical => "tactical",
            VisorMode::LowLight => "low light",
        }
    }
}

/// The TACTICAL VISOR MODE option. The original's values, in its order:
/// 0 tactical instead of normal, 1 normal instead of tactical, 2 cycle
/// through all three. Its menu lists them as ON, OFF and CYCLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisorChoice {
    /// ON: tactical and low light; never the plain view.
    TacticalOnly,
    /// OFF: normal and low light; never tactical.
    Off,
    /// CYCLE: normal, tactical, low light. The original's default.
    #[default]
    Cycle,
}

impl VisorChoice {
    pub fn number(self) -> u8 {
        match self {
            VisorChoice::TacticalOnly => 0,
            VisorChoice::Off => 1,
            VisorChoice::Cycle => 2,
        }
    }

    pub fn from_number(n: usize) -> VisorChoice {
        match n {
            0 => VisorChoice::TacticalOnly,
            1 => VisorChoice::Off,
            _ => VisorChoice::Cycle,
        }
    }

    /// The word used in the settings file.
    pub fn word(self) -> &'static str {
        match self {
            VisorChoice::TacticalOnly => "on",
            VisorChoice::Off => "off",
            VisorChoice::Cycle => "cycle",
        }
    }

    pub fn from_word(w: &str) -> Option<VisorChoice> {
        match w.trim().to_ascii_lowercase().as_str() {
            "on" | "tactical" => Some(VisorChoice::TacticalOnly),
            "off" | "normal" => Some(VisorChoice::Off),
            "cycle" => Some(VisorChoice::Cycle),
            _ => None,
        }
    }
}

/// The mode the visor starts in when the player gets a body (a level start
/// or a load): tactical, unless the option is OFF.
pub fn start_mode(choice: VisorChoice) -> VisorMode {
    match choice {
        VisorChoice::Off => VisorMode::Normal,
        _ => VisorMode::Tactical,
    }
}

/// The next mode when CYCLE VISOR MODE is pressed: one step on, wrapping
/// round, skipping normal when the option is ON and tactical when it is
/// OFF.
pub fn next_mode(now: VisorMode, choice: VisorChoice) -> VisorMode {
    let mut n = (now.number() + 1) % 3;
    match choice {
        VisorChoice::TacticalOnly if n == 0 => n = 1,
        VisorChoice::Off if n == 1 => n = 2,
        _ => {}
    }
    VisorMode::from_number(n).unwrap_or_default()
}

/// The mode to switch to when the option changes during play: the
/// original moves off a mode the new option no longer allows (normal under
/// ON goes to tactical; tactical under OFF goes to normal) and leaves the
/// others alone.
pub fn after_option_change(now: VisorMode, choice: VisorChoice) -> VisorMode {
    match (choice, now) {
        (VisorChoice::TacticalOnly, VisorMode::Normal) => VisorMode::Tactical,
        (VisorChoice::Off, VisorMode::Tactical) => VisorMode::Normal,
        _ => now,
    }
}

/// TACTICAL MODE INTENSITY as stored (0 to 1, the original's
/// `TacticalModeIntensity`) from the menu's value number (0 to 10), and
/// back. The original's default is 0.2.
pub fn intensity_from_step(step: usize) -> f32 {
    (step.min(10) as f32) / 10.0
}

pub fn intensity_step(intensity: f32) -> usize {
    (intensity.clamp(0.0, 1.0) * 10.0).round() as usize
}

/// A colour as the original stores it (`Color` struct bytes: blue, green,
/// red, alpha) to red, green, blue (0 to 255). `None` for black, which the
/// original uses to mean "no glow".
pub fn glow_rgb(bgra: &[u8]) -> Option<[u8; 3]> {
    let [b, g, r, ..] = bgra else {
        return None;
    };
    if (*r, *g, *b) == (0, 0, 0) {
        None
    } else {
        Some([*r, *g, *b])
    }
}

/// How strongly something glows at a point on its surface: `facing` is how
/// much the surface faces the eye (1 straight on, 0 side-on), so edges glow
/// most and outline the body. `power` sharpens the edge, `fill` is the
/// share that also covers the middle, and `strength` is the intensity
/// times the data file's scale. Our own shape for the glow; the original's
/// is in its compiled renderer, which we can't read.
pub fn glow_amount(facing: f32, power: f32, fill: f32, strength: f32) -> f32 {
    let edge = (1.0 - facing.abs().clamp(0.0, 1.0)).powf(power.max(0.01));
    (fill + (1.0 - fill) * edge) * strength.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use VisorChoice::*;
    use VisorMode::*;

    #[test]
    fn cycle_steps_through_all_three() {
        assert_eq!(next_mode(Normal, Cycle), Tactical);
        assert_eq!(next_mode(Tactical, Cycle), LowLight);
        assert_eq!(next_mode(LowLight, Cycle), Normal);
    }

    #[test]
    fn on_skips_normal_and_off_skips_tactical() {
        assert_eq!(next_mode(Tactical, TacticalOnly), LowLight);
        assert_eq!(next_mode(LowLight, TacticalOnly), Tactical);
        assert_eq!(next_mode(Normal, Off), LowLight);
        assert_eq!(next_mode(LowLight, Off), Normal);
    }

    #[test]
    fn starts_tactical_unless_off() {
        assert_eq!(start_mode(Cycle), Tactical);
        assert_eq!(start_mode(TacticalOnly), Tactical);
        assert_eq!(start_mode(Off), Normal);
    }

    #[test]
    fn changing_the_option_leaves_a_mode_it_forbids() {
        assert_eq!(after_option_change(Tactical, Off), Normal);
        assert_eq!(after_option_change(Normal, TacticalOnly), Tactical);
        assert_eq!(after_option_change(LowLight, Off), LowLight);
        assert_eq!(after_option_change(Normal, Cycle), Normal);
    }

    #[test]
    fn option_numbers_and_words_round_trip() {
        for c in [TacticalOnly, Off, Cycle] {
            assert_eq!(VisorChoice::from_number(c.number() as usize), c);
            assert_eq!(VisorChoice::from_word(c.word()), Some(c));
        }
        for i in 0..=10 {
            assert_eq!(intensity_step(intensity_from_step(i)), i);
        }
        assert_eq!(intensity_step(0.2), 2);
    }

    #[test]
    fn colours_are_stored_blue_first() {
        // Sev's (Commando07DeltaDefaults) is red.
        assert_eq!(glow_rgb(&[43, 46, 181, 0]), Some([181, 46, 43]));
        assert_eq!(glow_rgb(&[0, 0, 0, 0]), None);
        assert_eq!(glow_rgb(&[1, 2]), None);
    }

    #[test]
    fn edges_glow_more_than_the_middle() {
        let middle = glow_amount(1.0, 2.0, 0.1, 1.0);
        let edge = glow_amount(0.0, 2.0, 0.1, 1.0);
        assert!((middle - 0.1).abs() < 1e-6);
        assert!((edge - 1.0).abs() < 1e-6);
        assert_eq!(glow_amount(0.0, 2.0, 0.1, 0.0), 0.0);
    }
}
