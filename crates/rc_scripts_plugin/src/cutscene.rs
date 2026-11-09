//! Cutscenes: what level scripts ask of the player and the screen.
//!
//! Republic Commando's cutscenes are the game's own world seen through the
//! player's eyes: the prologue (the clone growing up on Kamino), the
//! briefings on the gunship, the epilogue, and moments inside missions.
//! The scripts:
//!
//! - hold the player still (`ACTION_Briefing`), limit where they can look
//!   (`MinYaw`/`MaxYaw`/`MinPitch`/`MaxPitch`, in the world's own angles:
//!   the prologue's rooms each give a range around the way the room's
//!   arrival point faces), hide the HUD (`bDrawHUD`), and let go again
//!   (`bEndBriefing`);
//! - move the player between rooms (`ACTION_TeleportToPoint` on the
//!   player's tag), facing the way the arrival point faces;
//! - stop the player's body moving at all (`ACTION_SetPhysics` to none on
//!   the player, so the newborn clone doesn't fall), and let it move again;
//! - fade the screen to a colour and back (`ACTION_ScreenFade`: a positive
//!   `FadeTime` fades to `FadeColor`, 0 goes there at once, a negative one
//!   fades back from it), and flash it (`ACTION_AnimateVisorFX`: `AddColor`
//!   brightens, `MultiplyColor` darkens, rising over `InTime`, holding
//!   `SustainTime`, falling over `OutTime`);
//! - hide or show the helmet frame (`ACTION_ShowVisor`).
//!
//! What the actions do is our reading of their names, their values in the
//! maps and the class defaults in the game's packages (`rc_probe --class
//! ACTION_Briefing`: limits default to the whole circle, `bDrawHUD` on;
//! `ACTION_ScreenFade`: `FadeTime` 3, black). No game code was read or
//! copied. Where the original does more than we can show yet (bloom on
//! flashes, the baby's and boy's hands in `BriefingWeapon`), the log says
//! "not done yet".
//!
//! The result goes into [`rc_game_core::Cutscene`] (read by the player,
//! the HUD and the screen) and [`rc_game_core::TeleportPlayer`] messages.

use bevy::prelude::*;
use rc_game_core::{Cutscene, GameLog, HeldView, TeleportPlayer};
use rc_level::scripts::{ScriptObject, Val};
use std::f32::consts::TAU;

use crate::LevelScript;

/// Script actions this part carries out. `teleporttopoint` only when it
/// names the player (characters' teleports go to `characters.rs`).
pub const KINDS: &[&str] = &[
    "briefing",
    "screenfade",
    "animatevisorfx",
    "showvisor",
    "setphysics",
];

/// Unreal angle units (65536 to a full turn) to radians.
fn angle(units: f32) -> f32 {
    units * TAU / 65536.0
}

/// A colour value as the maps store it: four bytes, blue, green, red,
/// opacity. Missing: `default`.
fn colour(a: &ScriptObject, key: &str, default: [u8; 4]) -> [u8; 4] {
    match a.get(key) {
        Some(Val::Raw(b)) if b.len() == 4 => [b[0], b[1], b[2], b[3]],
        Some(Val::Struct(s)) => {
            let c = |k: &str, d: u8| match s.get(k) {
                Some(Val::Byte(v)) => *v,
                _ => d,
            };
            [
                c("B", default[0]),
                c("G", default[1]),
                c("R", default[2]),
                c("A", default[3]),
            ]
        }
        _ => default,
    }
}

/// A fade of the whole screen from one colour to another.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fade {
    from: [f32; 4],
    to: [f32; 4],
    start: f32,
    seconds: f32,
}

impl Fade {
    fn at(&self, t: f32) -> [f32; 4] {
        let k = if self.seconds <= 0.0 {
            1.0
        } else {
            ((t - self.start) / self.seconds).clamp(0.0, 1.0)
        };
        let mut out = [0.0; 4];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.from[i] + (self.to[i] - self.from[i]) * k;
        }
        out
    }
}

/// A visor flash: a colour over the screen whose opacity rises, holds and
/// falls.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Flash {
    /// Colour and the opacity at its height.
    colour: [f32; 4],
    start: f32,
    rise: f32,
    hold: f32,
    fall: f32,
}

impl Flash {
    fn strength(&self, t: f32) -> f32 {
        let t = t - self.start;
        if t < 0.0 {
            0.0
        } else if t < self.rise {
            t / self.rise.max(1e-3)
        } else if t < self.rise + self.hold {
            1.0
        } else if t < self.rise + self.hold + self.fall {
            1.0 - (t - self.rise - self.hold) / self.fall.max(1e-3)
        } else {
            0.0
        }
    }
    fn over(&self, t: f32) -> bool {
        t > self.start + self.rise + self.hold + self.fall
    }
}

/// Lay one colour over another (both with their own opacity).
fn over(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 1e-6 {
        return [0.0; 4];
    }
    let mut out = [0.0, 0.0, 0.0, a];
    for i in 0..3 {
        out[i] = (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    }
    out
}

/// The cutscene's state while a level plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    held: Option<HeldView>,
    hide_hud: bool,
    hide_visor: bool,
    briefing_weapon: Option<String>,
    frozen: bool,
    fade: Option<Fade>,
    flashes: Vec<Flash>,
    teleports: Vec<TeleportPlayer>,
    /// What was done, for the log and the self-tests.
    pub done: Vec<String>,
}

impl Scene {
    /// The colour over the screen at time `t` (the scripts' clock).
    pub fn tint(&self, t: f32) -> [f32; 4] {
        let mut c = self.fade.map(|f| f.at(t)).unwrap_or([0.0; 4]);
        for f in &self.flashes {
            let s = f.strength(t);
            if s > 0.0 {
                let mut top = f.colour;
                top[3] *= s;
                c = over(top, c);
            }
        }
        c
    }

    /// Whether the player is held (for the self-tests).
    pub fn held(&self) -> Option<HeldView> {
        self.held
    }

    /// Carry out a cutscene action. `player_place` finds a place by tag
    /// (location, yaw). Returns false if the action isn't one of ours.
    pub fn take(
        &mut self,
        a: &ScriptObject,
        now: f32,
        is_player: impl Fn(&str) -> bool,
        place: impl Fn(&str) -> Option<([f32; 3], f32)>,
        at: &str,
        log: &GameLog,
    ) -> bool {
        let kind = a
            .class
            .strip_prefix("ACTION_")
            .unwrap_or(&a.class)
            .to_ascii_lowercase();
        match kind.as_str() {
            "briefing" => {
                if a.flag("bEndBriefing", false) {
                    self.held = None;
                    self.hide_hud = false;
                    self.briefing_weapon = None;
                    log.line(format!("{at}: the briefing is over; the player can move"));
                    self.done.push("briefing end".into());
                } else {
                    let range = |lo: &str, hi: &str| {
                        let l = a
                            .float(lo)
                            .or(a.int(lo).map(|i| i as f32))
                            .unwrap_or(-32768.0);
                        let h = a
                            .float(hi)
                            .or(a.int(hi).map(|i| i as f32))
                            .unwrap_or(32768.0);
                        (h - l < 65536.0).then(|| (angle(l), angle(h)))
                    };
                    let view = HeldView {
                        yaw: range("MinYaw", "MaxYaw"),
                        pitch: range("MinPitch", "MaxPitch"),
                        weapon_change: a.flag("bOkToChangeWeapon", false),
                    };
                    self.held = Some(view);
                    self.hide_hud = !a.flag("bDrawHUD", true);
                    let deg = |r: Option<(f32, f32)>| {
                        r.map(|(l, h)| {
                            format!("{:.0} to {:.0} deg", l.to_degrees(), h.to_degrees())
                        })
                        .unwrap_or_else(|| "free".into())
                    };
                    log.line(format!(
                        "{at}: the player is held; look left-right {}, up-down {}; HUD {}",
                        deg(view.yaw),
                        deg(view.pitch),
                        if self.hide_hud { "hidden" } else { "shown" }
                    ));
                    if let Some(w) = a.text("BriefingWeapon") {
                        if a.flag("bSetBriefingWeapon", false) {
                            log.line(format!(
                                "{at}: the player holds {w} (those hands aren't drawn yet: arms and gun hidden)"
                            ));
                            self.briefing_weapon = Some(w.to_string());
                        }
                    }
                    self.done.push("briefing".into());
                }
                true
            }
            "screenfade" => {
                let t = a.float("FadeTime").unwrap_or(3.0);
                let [b, g, r, al] = colour(a, "FadeColor", [0, 0, 0, 255]);
                let c = [
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    al as f32 / 255.0,
                ];
                let current = self.fade.map(|f| f.at(now)).unwrap_or([0.0; 4]);
                let fade = if t < 0.0 {
                    // Back from the colour (from where the screen is now,
                    // or from the colour if nothing covers it).
                    let from = if current[3] > 0.0 { current } else { c };
                    Fade {
                        from,
                        to: [from[0], from[1], from[2], 0.0],
                        start: now,
                        seconds: -t,
                    }
                } else {
                    let from = if current[3] > 0.0 {
                        current
                    } else {
                        [c[0], c[1], c[2], 0.0]
                    };
                    Fade {
                        from,
                        to: c,
                        start: now,
                        seconds: t,
                    }
                };
                log.line(format!(
                    "{at}: the screen fades {} over {:.1} s",
                    if t < 0.0 { "back in" } else { "out" },
                    t.abs()
                ));
                self.done.push(format!("fade {t}"));
                self.fade = Some(fade);
                true
            }
            "animatevisorfx" => {
                let rise = a.float("InTime").unwrap_or(0.0);
                let hold = a.float("SustainTime").unwrap_or(0.0);
                let fall = a.float("OutTime").unwrap_or(0.0);
                let mut parts = Vec::new();
                if a.flag("AffectAddColor", false) {
                    // Adding a colour brightens: shown as that colour over
                    // the screen, as strong as its brightest part.
                    let [b, g, r, _] = colour(a, "AddColor", [0, 0, 0, 0]);
                    let m = r.max(g).max(b) as f32 / 255.0;
                    if m > 0.0 {
                        self.flashes.push(Flash {
                            colour: [
                                r as f32 / 255.0 / m,
                                g as f32 / 255.0 / m,
                                b as f32 / 255.0 / m,
                                m,
                            ],
                            start: now,
                            rise,
                            hold,
                            fall,
                        });
                        parts.push("brightens");
                    }
                }
                if a.flag("AffectMultiplyColor", false) {
                    // Multiplying by a colour darkens: shown as black over
                    // the screen, as strong as the colour is dark.
                    let [b, g, r, _] = colour(a, "MultiplyColor", [0, 0, 0, 0]);
                    let light = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
                    if light < 1.0 {
                        self.flashes.push(Flash {
                            colour: [0.0, 0.0, 0.0, 1.0 - light],
                            start: now,
                            rise,
                            hold,
                            fall,
                        });
                        parts.push("darkens");
                    }
                }
                if a.flag("AffectBloomFilter", false) {
                    parts.push("glows (bloom not done yet)");
                }
                log.line(format!(
                    "{at}: visor flash {} (up {rise} s, holds {hold} s, down {fall} s)",
                    if parts.is_empty() {
                        "with nothing to show".to_string()
                    } else {
                        parts.join(", ")
                    }
                ));
                self.done.push("flash".into());
                true
            }
            "showvisor" => {
                let on = a.flag("bVisor", false);
                self.hide_visor = !on;
                log.line(format!(
                    "{at}: helmet frame {}",
                    if on { "shown" } else { "hidden" }
                ));
                self.done.push(format!("visor {on}"));
                true
            }
            "setphysics" => {
                let who = a.text("ActorTag").unwrap_or("");
                if !is_player(who) {
                    return false;
                }
                // UE2 physics modes: 0 none, 1 walking, 2 falling, ...
                let mode = match a.get("NewPhysicsMode") {
                    Some(Val::Byte(b)) => *b,
                    Some(Val::Int(i)) => *i as u8,
                    _ => 0,
                };
                self.frozen = mode == 0;
                log.line(format!(
                    "{at}: the player's body {}",
                    if self.frozen {
                        "stops moving (no physics)"
                    } else {
                        "moves again"
                    }
                ));
                self.done.push(format!("physics {mode}"));
                true
            }
            "teleporttopoint" => {
                let who = a.text("ActorToTeleportTag").unwrap_or("");
                if !is_player(who) {
                    return false;
                }
                let dest = a.text("DestinationTag").unwrap_or("");
                match place(dest) {
                    Some((location, yaw)) => {
                        log.line(format!(
                            "{at}: the player moves to {dest} ({:.0}, {:.0}, {:.0}) facing {:.0} deg",
                            location[0],
                            location[1],
                            location[2],
                            yaw.to_degrees()
                        ));
                        self.teleports.push(TeleportPlayer {
                            location,
                            yaw,
                            by: at.to_string(),
                        });
                        self.done.push(format!("teleport {dest}"));
                    }
                    None => log.line(format!("{at}: no place {dest} in the map; skipped")),
                }
                true
            }
            _ => false,
        }
    }
}

/// Hand the cutscene's state to the rest of the game.
pub(crate) fn apply(
    script: Option<ResMut<LevelScript>>,
    mut cut: ResMut<Cutscene>,
    mut teleport: MessageWriter<TeleportPlayer>,
) {
    let Some(mut script) = script else {
        if *cut != Cutscene::default() {
            *cut = Cutscene::default();
        }
        return;
    };
    let now = script.director.time;
    let scene = &mut script.scene;
    for t in scene.teleports.drain(..) {
        teleport.write(t);
    }
    scene.flashes.retain(|f| !f.over(now));
    let want = Cutscene {
        held: scene.held,
        frozen: scene.frozen,
        tint: scene.tint(now),
        hide_hud: scene.hide_hud,
        hide_visor: scene.hide_visor,
        briefing_weapon: scene.briefing_weapon.clone(),
    };
    if *cut != want {
        *cut = want;
    }
}

/// Nothing held or covering the screen once the level is left.
pub(crate) fn forget(mut cut: ResMut<Cutscene>) {
    *cut = Cutscene::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::Vals;

    fn action(kind: &str, values: &[(&str, Val)]) -> ScriptObject {
        ScriptObject {
            export: 0,
            class: format!("ACTION_{kind}"),
            name: kind.into(),
            values: values
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Vals>(),
        }
    }

    fn take(s: &mut Scene, a: &ScriptObject, now: f32) -> bool {
        let log = GameLog::terminal();
        s.take(
            a,
            now,
            |t| t.eq_ignore_ascii_case("PlayerCommando"),
            |t| (t == "GymTeleporter").then_some(([1.0, 2.0, 3.0], 1.5)),
            "test",
            &log,
        )
    }

    /// The prologue's nursery: held, looking within the map's range,
    /// the HUD off.
    #[test]
    fn briefing_holds_and_limits_the_look() {
        let mut s = Scene::default();
        let a = action(
            "Briefing",
            &[
                ("MinYaw", Val::Float(-3000.0)),
                ("MaxYaw", Val::Float(24000.0)),
                ("MinPitch", Val::Float(-3000.0)),
                ("MaxPitch", Val::Float(8000.0)),
                ("bDrawHUD", Val::Bool(false)),
            ],
        );
        assert!(take(&mut s, &a, 0.0));
        let h = s.held.unwrap();
        let (lo, hi) = h.yaw.unwrap();
        assert!((lo - angle(-3000.0)).abs() < 1e-5 && (hi - angle(24000.0)).abs() < 1e-5);
        assert!(s.hide_hud);
        // No limits given: free that way.
        assert!(take(&mut s, &action("Briefing", &[]), 1.0));
        assert_eq!(s.held.unwrap().yaw, None);
        assert!(!s.hide_hud);
        assert!(take(
            &mut s,
            &action("Briefing", &[("bEndBriefing", Val::Bool(true))]),
            2.0
        ));
        assert!(s.held.is_none());
    }

    /// Fade out to black over 2 s, then back in over 1 s.
    #[test]
    fn fades_out_and_back() {
        let mut s = Scene::default();
        assert!(take(
            &mut s,
            &action("ScreenFade", &[("FadeTime", Val::Float(2.0))]),
            10.0
        ));
        assert_eq!(s.tint(10.0)[3], 0.0);
        assert!((s.tint(11.0)[3] - 0.5).abs() < 1e-5);
        assert_eq!(s.tint(13.0), [0.0, 0.0, 0.0, 1.0]);
        assert!(take(
            &mut s,
            &action("ScreenFade", &[("FadeTime", Val::Float(-1.0))]),
            13.0
        ));
        assert!((s.tint(13.5)[3] - 0.5).abs() < 1e-5);
        assert_eq!(s.tint(14.0)[3], 0.0);
        // A fade back in with nothing covering starts from black.
        let mut s = Scene::default();
        take(
            &mut s,
            &action("ScreenFade", &[("FadeTime", Val::Float(-2.0))]),
            0.0,
        );
        assert_eq!(s.tint(0.0)[3], 1.0);
    }

    /// The prologue's white flash between rooms: up 2 s, holds 2 s, down 2 s.
    #[test]
    fn white_flash_rises_holds_and_falls() {
        let mut s = Scene::default();
        let a = action(
            "AnimateVisorFX",
            &[
                ("AddColor", Val::Raw(vec![255, 255, 255, 0])),
                ("AffectAddColor", Val::Bool(true)),
                ("InTime", Val::Float(2.0)),
                ("SustainTime", Val::Float(2.0)),
                ("OutTime", Val::Float(2.0)),
            ],
        );
        assert!(take(&mut s, &a, 0.0));
        assert!((s.tint(1.0)[3] - 0.5).abs() < 1e-5);
        assert_eq!(s.tint(3.0), [1.0, 1.0, 1.0, 1.0]);
        assert!((s.tint(5.0)[3] - 0.5).abs() < 1e-5);
        assert_eq!(s.tint(7.0)[3], 0.0);
    }

    /// Only the player's teleports and physics are ours.
    #[test]
    fn player_teleport_and_physics() {
        let mut s = Scene::default();
        let tp = action(
            "TeleportToPoint",
            &[
                ("ActorToTeleportTag", Val::Text("PlayerCommando".into())),
                ("DestinationTag", Val::Text("GymTeleporter".into())),
            ],
        );
        assert!(take(&mut s, &tp, 0.0));
        assert_eq!(s.teleports.len(), 1);
        assert_eq!(s.teleports[0].location, [1.0, 2.0, 3.0]);
        let other = action(
            "TeleportToPoint",
            &[("ActorToTeleportTag", Val::Text("Sev".into()))],
        );
        assert!(!take(&mut s, &other, 0.0));
        let freeze = action(
            "SetPhysics",
            &[("ActorTag", Val::Text("PlayerCommando".into()))],
        );
        assert!(take(&mut s, &freeze, 0.0));
        assert!(s.frozen);
        let walk = action(
            "SetPhysics",
            &[
                ("ActorTag", Val::Text("PlayerCommando".into())),
                ("NewPhysicsMode", Val::Byte(1)),
            ],
        );
        assert!(take(&mut s, &walk, 0.0));
        assert!(!s.frozen);
    }
}
