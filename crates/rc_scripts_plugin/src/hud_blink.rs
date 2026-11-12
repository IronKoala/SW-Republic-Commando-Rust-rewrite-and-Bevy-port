//! Level scripts making part of the HUD blink, to teach the player what it
//! shows (`ACTION_BlinkHUDHealth`, `ACTION_BlinkHUDSquad`,
//! `ACTION_BlinkHUDGrenade`, `ACTION_BlinkHUDShields` and
//! `ACTION_BlinkHUDSquadCommandPrompt`).
//!
//! The original's rules, in our words:
//!
//! - The step (ctgame.u) finds the player and calls `BlinkHUDHealth` (and
//!   so on) with its `BlinkTime`, 0 when the map doesn't set one.
//! - That (CTPlayer) sets the helmet's blink flag for that part, its time,
//!   and how long it has blinked so far to 0.
//! - Each time the helmet draws the HUD (Engine.dll `UHelmet::DrawHUD`),
//!   the time since the last drawing is added. Once it is past `BlinkTime`
//!   the flag is cleared. Until then the part is drawn with the opacity
//!   `rc_menus::hud::blink_alpha` gives for the level's time.
//! - The squad command prompt's flag is cleared when the helmet draws its
//!   text box (`UHelmet::DrawTextInfo`) and nothing reads it, so it shows
//!   nothing.
//!
//! Which elements blink is in `data/ui/hud.toml` (`blink = "health"`).

/// One part of the HUD blinking.
#[derive(Debug, Clone, PartialEq)]
struct Blink {
    kind: &'static str,
    time: f32,
    elapsed: f32,
}

/// The HUD parts the level's scripts are blinking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudBlink {
    blinks: Vec<Blink>,
}

impl HudBlink {
    /// The kind of HUD part a script step blinks (`blinkhudhealth` and so
    /// on, lower case), with `None` for the squad command prompt, which
    /// shows nothing; not a blink step at all: `Err`.
    pub fn kind_of(step: &str) -> Result<Option<&'static str>, ()> {
        match step {
            "blinkhudhealth" => Ok(Some("health")),
            "blinkhudsquad" => Ok(Some("squad")),
            "blinkhudgrenade" => Ok(Some("grenade")),
            "blinkhudshields" => Ok(Some("shields")),
            "blinkhudsquadcommandprompt" => Ok(None),
            _ => Err(()),
        }
    }

    /// Start (or start again) blinking a part for `time` seconds.
    pub fn start(&mut self, kind: &'static str, time: f32) {
        self.blinks.retain(|b| b.kind != kind);
        self.blinks.push(Blink {
            kind,
            time,
            elapsed: 0.0,
        });
    }

    /// The HUD was drawn again, `dt` seconds after the last time: blinks
    /// past their time stop.
    pub fn tick(&mut self, dt: f32) {
        for b in &mut self.blinks {
            b.elapsed += dt;
        }
        self.blinks.retain(|b| b.elapsed <= b.time);
    }

    /// The parts blinking now ("health", "squad", "grenade", "shields").
    pub fn now(&self) -> Vec<&'static str> {
        self.blinks.iter().map(|b| b.kind).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blinks_for_its_time_then_stops() {
        let mut h = HudBlink::default();
        // geo_01a ScriptedTrigger3: BlinkHUDHealth for 15 s.
        h.start(HudBlink::kind_of("blinkhudhealth").unwrap().unwrap(), 15.0);
        h.tick(1.0 / 60.0);
        assert_eq!(h.now(), vec!["health"]);
        for _ in 0..14 * 60 {
            h.tick(1.0 / 60.0);
        }
        assert_eq!(h.now().len(), 1);
        for _ in 0..61 {
            h.tick(1.0 / 60.0);
        }
        assert!(h.now().is_empty());
        // geo_01c's BlinkHUDSquad sets no time: gone by the first drawing.
        h.start("squad", 0.0);
        h.tick(1.0 / 60.0);
        assert!(h.now().is_empty());
        assert_eq!(HudBlink::kind_of("blinkhudsquadcommandprompt"), Ok(None));
        assert!(HudBlink::kind_of("addammo").is_err());
    }
}
