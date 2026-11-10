//! The sounds the first-person arms make: the reload clatter, the click of
//! bringing a weapon up, putting it away, the melee swing and the grenade
//! throw.
//!
//! In the original these aren't in the weapon's script at all. They are
//! `AnimNotify_Sound` events inside the arms animations in
//! `Animations/hudarms.ukx`: each names a sound (such as
//! `GEN_Sound.Foley_Movement.fol_GEN_dc17reload_01`) and a moment through
//! the animation (0 at the start, 1 at the end). The engine plays the sound
//! when the animation passes that moment. `rc_model` reads them with the
//! model (`Model::sounds`); here we watch the arms' animation move on each
//! frame and play every moment it passed.
//!
//! **Our choices:** the sounds play for the player only, at full volume
//! (scaled by the EFFECTS VOLUME setting like every effect), not placed in
//! the level, like the weapon's fire sound. They are quiet while the player
//! is down or a cutscene gives them other hands, and still play when the
//! sniper's zoom hides the arms.

use crate::arms::{Arms, ArmsAnim};
use crate::library::{read_sounds, GameSound};
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths};
use rc_model::Model;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Arms sounds read so far, by `package.name` in lower case.
#[derive(Resource, Default)]
pub struct ArmsSounds {
    sounds: HashMap<String, GameSound>,
    /// Every sound tried, found or not, so a missing one is tried once.
    tried: HashSet<String>,
    /// The last few sounds played, as "weapon Animation package.name", for
    /// the log and self-tests.
    pub played: Vec<String>,
    /// How many have played since the game started.
    pub count: usize,
}

/// The key a sound is kept under, from the dotted path the animation
/// names (`GEN_Sound.Foley_Movement.fol_GEN_dc17reload_01`): its package
/// and its own name, the groups between left out.
pub(crate) fn split(path: &str) -> Option<(String, String)> {
    let (package, rest) = path.split_once('.')?;
    let name = rest.rsplit('.').next().unwrap_or(rest);
    Some((package.to_ascii_lowercase(), name.to_string()))
}

fn key(package: &str, name: &str) -> String {
    format!("{package}.{name}").to_ascii_lowercase()
}

impl ArmsSounds {
    /// Read every sound `model`'s animations name that wasn't tried yet.
    /// Returns lines for the log.
    pub(crate) fn read(
        &mut self,
        weapon: &str,
        model: &Model,
        paths: &GamePaths,
        audio: &mut Assets<AudioSource>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        // Which animations make a sound, for the log.
        let mut per_anim: Vec<String> = ArmsAnim::ALL
            .iter()
            .filter_map(|a| {
                let n = model.sounds.get(a.sequence())?.len();
                Some(format!("{} x{n}", a.sequence()))
            })
            .collect();
        if per_anim.is_empty() {
            per_anim.push("none".into());
        }
        lines.push(format!(
            "arms: {weapon}: animation sounds: {}",
            per_anim.join(", ")
        ));
        let mut by_package: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (_, path) in model.sounds.values().flatten() {
            let Some((package, name)) = split(path) else {
                continue;
            };
            if self.tried.insert(key(&package, &name)) {
                by_package.entry(package).or_default().push(name);
            }
        }
        for (package, names) in by_package {
            let (found, notes) = read_sounds(paths, &package, &names, audio);
            for n in notes {
                lines.push(format!("arms: {weapon}: {n}"));
            }
            lines.push(format!(
                "arms: {weapon}: {} of {} animation sounds read from {package}",
                found.len(),
                names.len()
            ));
            for (name, s) in found {
                self.sounds.insert(key(&package, &name), s);
            }
        }
        lines
    }
}

/// The arms as they were on one frame: weapon shown, animation, seconds in.
pub(crate) type Moment = (Option<String>, ArmsAnim, f32);

/// A stretch of one animation the arms went through this frame, as
/// fractions of its length: sounds after `from` and up to `to` are due.
/// `from` below 0 takes in a sound at the very start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Span {
    pub weapon: String,
    pub anim: ArmsAnim,
    pub from: f32,
    pub to: f32,
}

/// What the arms passed through between `before` and `now` (`dt` seconds
/// apart). `length` gives an animation's seconds for a weapon.
pub(crate) fn spans(
    before: Option<&Moment>,
    now: &Moment,
    dt: f32,
    length: &dyn Fn(&str, ArmsAnim) -> Option<f32>,
) -> Vec<Span> {
    let mut out = Vec::new();
    let frac = |w: &str, a: ArmsAnim, t: f32| -> Option<f32> {
        let l = length(w, a)?;
        Some(if l > 0.0 { t / l } else { 1.0 })
    };
    let (Some(w), anim, t) = now else {
        return out;
    };
    let to = frac(w, *anim, *t);
    let start = |out: &mut Vec<Span>| {
        if let Some(to) = to {
            out.push(Span {
                weapon: w.clone(),
                anim: *anim,
                from: -1.0,
                to,
            });
        }
    };
    match before {
        Some((Some(bw), ba, bt)) if bw == w && ba == anim => {
            let (Some(from), Some(to)) = (frac(w, *anim, *bt), to) else {
                return out;
            };
            if *t >= *bt {
                out.push(Span {
                    weapon: w.clone(),
                    anim: *anim,
                    from,
                    to,
                });
            } else {
                // Went round again (breathing, walking), or started over
                // (a second shot): the end of the last time round, if it
                // ran to its end, then the start of this one.
                if anim.loops() {
                    out.push(Span {
                        weapon: w.clone(),
                        anim: *anim,
                        from,
                        to: 1.0,
                    });
                }
                start(&mut out);
            }
        }
        Some((Some(bw), ba, bt)) => {
            // Another animation now. One that played once and reached its
            // end this frame (a reload done, a weapon put away) still had
            // its last stretch; one cut short by something else did not.
            if !ba.loops() {
                if let (Some(l), Some(from)) = (length(bw, *ba), frac(bw, *ba, *bt)) {
                    if bt + dt >= l {
                        out.push(Span {
                            weapon: bw.clone(),
                            anim: *ba,
                            from,
                            to: 1.0,
                        });
                    }
                }
            }
            start(&mut out);
        }
        _ => start(&mut out),
    }
    out
}

/// Play the sounds due in `spans`, from the models' animation events.
pub(crate) fn play(
    spans: &[Span],
    model: &dyn Fn(&str) -> Option<std::sync::Arc<Model>>,
    sounds: &mut ArmsSounds,
    commands: &mut Commands,
    log: &Option<Res<GameLog>>,
) {
    for s in spans {
        let Some(m) = model(&s.weapon) else {
            continue;
        };
        let Some(list) = m.sounds.get(s.anim.sequence()) else {
            continue;
        };
        for (t, path) in list {
            if !(*t > s.from && *t <= s.to) {
                continue;
            }
            let handle = split(path)
                .and_then(|(p, n)| sounds.sounds.get(&key(&p, &n)))
                .and_then(|g| g.pick());
            let line = match handle {
                Some(h) => {
                    commands.spawn((
                        AudioPlayer(h),
                        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(1.0)),
                    ));
                    sounds.count += 1;
                    sounds
                        .played
                        .push(format!("{} {} {path}", s.weapon, s.anim.sequence()));
                    let extra = sounds.played.len().saturating_sub(16);
                    sounds.played.drain(..extra);
                    format!(
                        "arms: {} {} at {t:.2}: sound {path}",
                        s.weapon,
                        s.anim.sequence()
                    )
                }
                None => format!(
                    "arms: {} {} at {t:.2}: no sound {path} to play (not in the game files?)",
                    s.weapon,
                    s.anim.sequence()
                ),
            };
            match log {
                Some(l) => l.line(line),
                None => println!("{line}"),
            }
        }
    }
}

/// The moment the arms are at now.
pub(crate) fn moment(arms: &Arms) -> Moment {
    (arms.shown.clone(), arms.anim, arms.time)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(_: &str, a: ArmsAnim) -> Option<f32> {
        match a {
            ArmsAnim::Reload => Some(1.5),
            ArmsAnim::Breathe => Some(2.0),
            ArmsAnim::Holster => Some(0.5),
            ArmsAnim::Load => Some(0.5),
            ArmsAnim::Fire => Some(0.2),
            _ => None,
        }
    }

    fn at(w: &str, a: ArmsAnim, t: f32) -> Moment {
        (Some(w.to_string()), a, t)
    }

    fn span(w: &str, anim: ArmsAnim, from: f32, to: f32) -> Span {
        Span {
            weapon: w.into(),
            anim,
            from,
            to,
        }
    }

    #[test]
    fn splits_the_dotted_path() {
        assert_eq!(
            split("GEN_Sound.Foley_Movement.fol_GEN_dc17reload_01"),
            Some(("gen_sound".into(), "fol_GEN_dc17reload_01".into()))
        );
        assert_eq!(
            split("GEN_Sound.wep_x"),
            Some(("gen_sound".into(), "wep_x".into()))
        );
        assert_eq!(split("nothing"), None);
    }

    #[test]
    fn going_on_covers_the_stretch_between() {
        let s = spans(
            Some(&at("r", ArmsAnim::Reload, 0.3)),
            &at("r", ArmsAnim::Reload, 0.45),
            0.15,
            &len,
        );
        assert_eq!(s.len(), 1);
        assert!((s[0].from - 0.2).abs() < 1e-5 && (s[0].to - 0.3).abs() < 1e-5);
    }

    #[test]
    fn a_new_animation_includes_its_very_start() {
        let s = spans(
            Some(&at("r", ArmsAnim::Breathe, 0.5)),
            &at("r", ArmsAnim::Reload, 0.01),
            0.01,
            &len,
        );
        // Breathing loops, so no tail for it; the reload from its start.
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].anim, ArmsAnim::Reload);
        assert!(s[0].from < 0.0);
    }

    #[test]
    fn a_finished_reload_plays_its_last_stretch() {
        let s = spans(
            Some(&at("r", ArmsAnim::Reload, 1.49)),
            &at("r", ArmsAnim::Breathe, 0.0),
            0.02,
            &len,
        );
        assert_eq!(s[0], span("r", ArmsAnim::Reload, 1.49 / 1.5, 1.0));
        assert_eq!(s[1].anim, ArmsAnim::Breathe);
    }

    #[test]
    fn a_reload_cut_short_does_not() {
        let s = spans(
            Some(&at("r", ArmsAnim::Reload, 0.5)),
            &at("r", ArmsAnim::Holster, 0.02),
            0.02,
            &len,
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].anim, ArmsAnim::Holster);
    }

    #[test]
    fn the_old_weapon_finishes_going_away() {
        let s = spans(
            Some(&at("rifle", ArmsAnim::Holster, 0.49)),
            &at("pistol", ArmsAnim::Load, 0.02),
            0.02,
            &len,
        );
        assert_eq!(s[0].weapon, "rifle");
        assert_eq!(s[0].anim, ArmsAnim::Holster);
        assert_eq!(s[1].weapon, "pistol");
        assert_eq!(s[1].anim, ArmsAnim::Load);
    }

    #[test]
    fn breathing_round_again_covers_both_ends() {
        let s = spans(
            Some(&at("r", ArmsAnim::Breathe, 1.99)),
            &at("r", ArmsAnim::Breathe, 0.01),
            0.02,
            &len,
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].to, 1.0);
        assert!(s[1].from < 0.0);
    }

    #[test]
    fn a_second_shot_starts_over() {
        let s = spans(
            Some(&at("r", ArmsAnim::Fire, 0.1)),
            &at("r", ArmsAnim::Fire, 0.02),
            0.02,
            &len,
        );
        assert_eq!(s.len(), 1);
        assert!(s[0].from < 0.0);
    }

    #[test]
    fn nothing_in_hand_nothing_heard() {
        let s = spans(
            Some(&at("r", ArmsAnim::Reload, 0.3)),
            &(None, ArmsAnim::Breathe, 0.0),
            0.02,
            &len,
        );
        assert!(s.is_empty());
    }
}
