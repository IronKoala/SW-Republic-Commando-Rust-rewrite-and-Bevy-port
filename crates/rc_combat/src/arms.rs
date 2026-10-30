//! The first-person arms ("HUD arms"): which of the weapon's animations
//! plays, and when. Plain Rust, no Bevy; `arms_view` draws the result.
//!
//! The game holds each player weapon as one animated model in
//! `GameData/Animations/hudarms.ukx` (the commando's arms and the gun
//! together, `HudArmsDC17m` and so on), drawn at the eye and turned with
//! the view. The rules below follow the game's own script
//! (`Engine.Weapon`: `PlayIdleAnim`, `PlayFiring`, `PlayReloading`,
//! `TweenDown`, `PlaySelect`), described in our own words in
//! docs/DESIGN.md, "First-person arms":
//!
//! - Standing still: `ActionBreathe` over and over. After 15 seconds still
//!   (`PlayIdleTime`), `Idle` plays once (looking the gun over), then
//!   breathing again.
//! - Moving slower than 226 uu/s: `Walk`; faster: `Run`.
//! - Zoomed in: `AimBreathe`, `Aim` while moving, `AimFire` per shot, when
//!   the weapon has them.
//! - Each shot plays `Fire` from the start; a reload plays `Reload`; a
//!   thrown grenade `ThrowGrenade`; a melee hit `MeleeAttack`.
//! - Switching: the old weapon plays `Holster` (down out of view), then
//!   the new one `Load` (up into view). Between two DC-17m attachments the
//!   game plays `AttachHolster` and `AttachLoad` instead (taking the
//!   attachment off and clicking the new one on).
//!
//! **Our choices:** the game picks the next movement animation only when
//! the current one ends; we change at once when the player starts or
//! stops moving, so the arms don't keep breathing for up to 2.7 s after
//! setting off. There's no blending between animations yet.

/// One of the arms' animations. The names are the game's sequence names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArmsAnim {
    Breathe,
    Idle,
    Walk,
    Run,
    Fire,
    Reload,
    Holster,
    Load,
    AttachHolster,
    AttachLoad,
    Throw,
    Melee,
    Aim,
    AimBreathe,
    AimFire,
}

impl ArmsAnim {
    pub const ALL: [ArmsAnim; 15] = [
        ArmsAnim::Breathe,
        ArmsAnim::Idle,
        ArmsAnim::Walk,
        ArmsAnim::Run,
        ArmsAnim::Fire,
        ArmsAnim::Reload,
        ArmsAnim::Holster,
        ArmsAnim::Load,
        ArmsAnim::AttachHolster,
        ArmsAnim::AttachLoad,
        ArmsAnim::Throw,
        ArmsAnim::Melee,
        ArmsAnim::Aim,
        ArmsAnim::AimBreathe,
        ArmsAnim::AimFire,
    ];

    /// The sequence's name in the game's animation sets.
    pub fn sequence(self) -> &'static str {
        match self {
            ArmsAnim::Breathe => "ActionBreathe",
            ArmsAnim::Idle => "Idle",
            ArmsAnim::Walk => "Walk",
            ArmsAnim::Run => "Run",
            ArmsAnim::Fire => "Fire",
            ArmsAnim::Reload => "Reload",
            ArmsAnim::Holster => "Holster",
            ArmsAnim::Load => "Load",
            ArmsAnim::AttachHolster => "AttachHolster",
            ArmsAnim::AttachLoad => "AttachLoad",
            ArmsAnim::Throw => "ThrowGrenade",
            ArmsAnim::Melee => "MeleeAttack",
            ArmsAnim::Aim => "Aim",
            ArmsAnim::AimBreathe => "AimBreathe",
            ArmsAnim::AimFire => "AimFire",
        }
    }

    /// Breathing, walking and running repeat; everything else plays once.
    pub fn loops(self) -> bool {
        matches!(
            self,
            ArmsAnim::Breathe
                | ArmsAnim::Walk
                | ArmsAnim::Run
                | ArmsAnim::Aim
                | ArmsAnim::AimBreathe
        )
    }

    fn movement(self) -> bool {
        self.loops() || self == ArmsAnim::Idle
    }
}

/// Seconds standing still before `Idle` plays (`PlayIdleTime`).
pub const IDLE_AFTER: f32 = 15.0;
/// Moving slower than this (uu/s) is walking, faster is running: the
/// script's `VelocitySq < 51076`.
pub const RUN_FROM: f32 = 226.0;
/// Slower than this counts as standing still (`VelocitySq >= 0.04`).
pub const STILL_BELOW: f32 = 0.2;

/// What happened this frame.
#[derive(Debug, Clone, Default)]
pub struct ArmsInput {
    pub dt: f32,
    /// The weapon in hand (its file id), `None` for empty hands.
    pub weapon: Option<String>,
    /// Ground speed, uu/s.
    pub speed: f32,
    pub zoomed: bool,
    pub fired: bool,
    pub reload_started: bool,
    pub threw: bool,
    pub melee: bool,
}

/// What the arms are doing: which weapon is drawn, which animation plays,
/// how far into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Arms {
    /// The weapon drawn now (it can lag the one in hand while the old one
    /// goes down).
    pub shown: Option<String>,
    pub anim: ArmsAnim,
    /// Seconds into `anim`.
    pub time: f32,
    /// The weapon coming up next, while the shown one goes down.
    pending: Option<String>,
    /// Seconds standing still, for `Idle`.
    still: f32,
}

impl Default for Arms {
    fn default() -> Self {
        Arms {
            shown: None,
            anim: ArmsAnim::Breathe,
            time: 0.0,
            pending: None,
            still: 0.0,
        }
    }
}

/// What the rules need to know about the weapons' animations: the length
/// in seconds of `anim` for `weapon`, `None` if its set doesn't have it.
/// And which "attachment group" a weapon is in (the DC-17m and its sniper
/// and anti-armour attachments share one).
pub trait ArmsClips {
    fn length(&self, weapon: &str, anim: ArmsAnim) -> Option<f32>;
    fn group(&self, weapon: &str) -> Option<&str>;
}

impl Arms {
    /// Move on by one frame: pick the animation and advance its time.
    pub fn step(&mut self, input: &ArmsInput, clips: &dyn ArmsClips) {
        let has = |w: &Option<String>, a: ArmsAnim| {
            w.as_deref().and_then(|w| clips.length(w, a)).is_some()
        };
        if input.speed < STILL_BELOW && !input.zoomed {
            self.still += input.dt;
        } else {
            self.still = 0.0;
        }

        // Switching weapons.
        if input.weapon != self.shown {
            if self.shown.is_none() || !has(&self.shown, ArmsAnim::Holster) {
                // Nothing to put away: bring the new one straight up.
                self.bring_up(input.weapon.clone(), false, clips);
            } else if self.pending.as_ref() != input.weapon.as_ref()
                || !matches!(self.anim, ArmsAnim::Holster | ArmsAnim::AttachHolster)
            {
                let attach = same_group(clips, &self.shown, &input.weapon)
                    && has(&self.shown, ArmsAnim::AttachHolster);
                self.pending = input.weapon.clone();
                if !matches!(self.anim, ArmsAnim::Holster | ArmsAnim::AttachHolster) {
                    self.play(if attach {
                        ArmsAnim::AttachHolster
                    } else {
                        ArmsAnim::Holster
                    });
                }
            }
        } else if self.pending.is_some() {
            // Switched back before the old one was down: keep it.
            self.pending = None;
            self.play(self.idle_choice(input, clips));
        }

        let putting_away = matches!(self.anim, ArmsAnim::Holster | ArmsAnim::AttachHolster);
        if !putting_away {
            // One thing per frame, the most important first.
            let event = if input.melee {
                Some(ArmsAnim::Melee)
            } else if input.threw {
                Some(ArmsAnim::Throw)
            } else if input.reload_started {
                Some(ArmsAnim::Reload)
            } else if input.fired {
                Some(if input.zoomed && has(&self.shown, ArmsAnim::AimFire) {
                    ArmsAnim::AimFire
                } else {
                    ArmsAnim::Fire
                })
            } else {
                None
            };
            if let Some(a) = event.filter(|a| has(&self.shown, *a)) {
                self.play(a);
            }
        }

        self.time += input.dt;
        let length = self
            .shown
            .as_deref()
            .and_then(|w| clips.length(w, self.anim));
        let ended = length.is_none_or(|l| self.time >= l);
        if self.anim.movement() {
            let want = self.idle_choice(input, clips);
            if want != self.anim && !(self.anim == ArmsAnim::Idle && want == ArmsAnim::Breathe) {
                self.play(want);
            } else if ended && !self.anim.loops() {
                // `Idle` finished: back to breathing, and count again.
                self.still = 0.0;
                self.play(ArmsAnim::Breathe);
            } else if ended {
                self.time = length.map_or(0.0, |l| if l > 0.0 { self.time % l } else { 0.0 });
            }
        } else if ended {
            if putting_away {
                let attach = self.anim == ArmsAnim::AttachHolster;
                let next = self.pending.take();
                self.bring_up(next, attach, clips);
            } else {
                self.play(self.idle_choice(input, clips));
            }
        }
    }

    /// Which repeating animation fits how the player moves.
    fn idle_choice(&self, input: &ArmsInput, clips: &dyn ArmsClips) -> ArmsAnim {
        let has = |a: ArmsAnim| {
            self.shown
                .as_deref()
                .and_then(|w| clips.length(w, a))
                .is_some()
        };
        let aim = |a: ArmsAnim, otherwise: ArmsAnim| {
            if input.zoomed && has(a) {
                a
            } else {
                otherwise
            }
        };
        if input.speed >= RUN_FROM {
            aim(ArmsAnim::Aim, ArmsAnim::Run)
        } else if input.speed >= STILL_BELOW {
            aim(ArmsAnim::Aim, ArmsAnim::Walk)
        } else if !input.zoomed && self.still >= IDLE_AFTER && has(ArmsAnim::Idle) {
            ArmsAnim::Idle
        } else {
            aim(ArmsAnim::AimBreathe, ArmsAnim::Breathe)
        }
    }

    fn bring_up(&mut self, weapon: Option<String>, attach: bool, clips: &dyn ArmsClips) {
        self.shown = weapon;
        self.pending = None;
        let has = |a| {
            self.shown
                .as_deref()
                .and_then(|w| clips.length(w, a))
                .is_some()
        };
        self.play(if attach && has(ArmsAnim::AttachLoad) {
            ArmsAnim::AttachLoad
        } else {
            ArmsAnim::Load
        });
    }

    fn play(&mut self, anim: ArmsAnim) {
        self.anim = anim;
        self.time = 0.0;
    }
}

fn same_group(clips: &dyn ArmsClips, a: &Option<String>, b: &Option<String>) -> bool {
    match (
        a.as_deref().and_then(|w| clips.group(w)),
        b.as_deref().and_then(|w| clips.group(w)),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The DC-17m's real lengths (frames at 30 per second), its sniper
    /// attachment, and the DC-15s pistol, which has no attachment moves.
    struct Clips(HashMap<(&'static str, ArmsAnim), f32>);

    impl Clips {
        fn new() -> Clips {
            let mut m = HashMap::new();
            let f = |n: f32| n / 30.0;
            for w in ["rifle", "sniper"] {
                for (a, n) in [
                    (ArmsAnim::Breathe, 81.0),
                    (ArmsAnim::Idle, 61.0),
                    (ArmsAnim::Walk, 25.0),
                    (ArmsAnim::Run, 22.0),
                    (ArmsAnim::Fire, 3.0),
                    (ArmsAnim::Reload, 45.0),
                    (ArmsAnim::Holster, 11.0),
                    (ArmsAnim::Load, 20.0),
                    (ArmsAnim::AttachHolster, 34.0),
                    (ArmsAnim::AttachLoad, 34.0),
                    (ArmsAnim::Throw, 29.0),
                    (ArmsAnim::Melee, 34.0),
                    (ArmsAnim::AimBreathe, 79.0),
                    (ArmsAnim::Aim, 25.0),
                    (ArmsAnim::AimFire, 3.0),
                ] {
                    m.insert((w, a), f(n));
                }
            }
            for (a, n) in [
                (ArmsAnim::Breathe, 80.0),
                (ArmsAnim::Fire, 10.0),
                (ArmsAnim::Holster, 14.0),
                (ArmsAnim::Load, 18.0),
                (ArmsAnim::Walk, 25.0),
            ] {
                m.insert(("pistol", a), f(n));
            }
            Clips(m)
        }
    }

    impl ArmsClips for Clips {
        fn length(&self, weapon: &str, anim: ArmsAnim) -> Option<f32> {
            self.0
                .iter()
                .find(|((w, a), _)| *w == weapon && *a == anim)
                .map(|(_, l)| *l)
        }
        fn group(&self, weapon: &str) -> Option<&str> {
            (weapon != "pistol").then_some("dc17m")
        }
    }

    fn input(weapon: &str) -> ArmsInput {
        ArmsInput {
            dt: 1.0 / 30.0,
            weapon: Some(weapon.into()),
            ..Default::default()
        }
    }

    /// Run `seconds` of frames, returning the animations seen in order.
    fn run(arms: &mut Arms, clips: &Clips, i: &ArmsInput, seconds: f32) -> Vec<ArmsAnim> {
        let mut seen: Vec<ArmsAnim> = vec![];
        for _ in 0..(seconds * 30.0).round() as usize {
            arms.step(i, clips);
            if seen.last() != Some(&arms.anim) {
                seen.push(arms.anim);
            }
        }
        seen
    }

    #[test]
    fn first_weapon_comes_up_then_breathes() {
        let c = Clips::new();
        let mut a = Arms::default();
        let seen = run(&mut a, &c, &input("rifle"), 2.0);
        assert_eq!(seen, [ArmsAnim::Load, ArmsAnim::Breathe]);
        assert_eq!(a.shown.as_deref(), Some("rifle"));
    }

    #[test]
    fn idle_after_fifteen_seconds_still_then_breathes_again() {
        let c = Clips::new();
        let mut a = Arms::default();
        let seen = run(&mut a, &c, &input("rifle"), 18.0);
        assert_eq!(
            seen,
            [
                ArmsAnim::Load,
                ArmsAnim::Breathe,
                ArmsAnim::Idle,
                ArmsAnim::Breathe
            ]
        );
    }

    #[test]
    fn walking_and_running_follow_speed() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        let walk = ArmsInput {
            speed: 150.0,
            ..input("rifle")
        };
        assert_eq!(run(&mut a, &c, &walk, 0.5), [ArmsAnim::Walk]);
        let fast = ArmsInput {
            speed: 300.0,
            ..input("rifle")
        };
        assert_eq!(run(&mut a, &c, &fast, 0.5), [ArmsAnim::Run]);
        assert_eq!(run(&mut a, &c, &input("rifle"), 0.5), [ArmsAnim::Breathe]);
    }

    #[test]
    fn each_shot_restarts_fire_and_reload_plays_through() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        let shot = ArmsInput {
            fired: true,
            ..input("rifle")
        };
        a.step(&shot, &c);
        assert_eq!((a.anim, a.time), (ArmsAnim::Fire, 1.0 / 30.0));
        a.step(&input("rifle"), &c);
        a.step(&shot, &c);
        assert_eq!((a.anim, a.time), (ArmsAnim::Fire, 1.0 / 30.0));
        let reload = ArmsInput {
            reload_started: true,
            ..input("rifle")
        };
        a.step(&reload, &c);
        let seen = run(&mut a, &c, &input("rifle"), 1.6);
        assert_eq!(seen, [ArmsAnim::Reload, ArmsAnim::Breathe]);
    }

    #[test]
    fn switching_holsters_then_loads() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        let seen = run(&mut a, &c, &input("pistol"), 1.5);
        assert_eq!(seen, [ArmsAnim::Holster, ArmsAnim::Load, ArmsAnim::Breathe]);
        assert_eq!(a.shown.as_deref(), Some("pistol"));
    }

    #[test]
    fn attachments_swap_with_attach_moves() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        let seen = run(&mut a, &c, &input("sniper"), 3.0);
        assert_eq!(
            seen,
            [
                ArmsAnim::AttachHolster,
                ArmsAnim::AttachLoad,
                ArmsAnim::Breathe
            ]
        );
    }

    #[test]
    fn shots_while_putting_away_dont_interrupt() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        a.step(&input("pistol"), &c);
        let shot = ArmsInput {
            fired: true,
            ..input("pistol")
        };
        a.step(&shot, &c);
        assert_eq!(
            (a.anim, a.shown.as_deref()),
            (ArmsAnim::Holster, Some("rifle"))
        );
    }

    #[test]
    fn zoomed_uses_aim_moves_when_the_weapon_has_them() {
        let c = Clips::new();
        let mut a = Arms::default();
        run(&mut a, &c, &input("rifle"), 1.0);
        let z = ArmsInput {
            zoomed: true,
            ..input("rifle")
        };
        assert_eq!(run(&mut a, &c, &z, 0.5), [ArmsAnim::AimBreathe]);
        a.step(
            &ArmsInput {
                fired: true,
                ..z.clone()
            },
            &c,
        );
        assert_eq!(a.anim, ArmsAnim::AimFire);
        // The pistol has no aim moves: it breathes as usual.
        let mut p = Arms::default();
        run(&mut p, &c, &input("pistol"), 1.0);
        let zp = ArmsInput {
            zoomed: true,
            ..input("pistol")
        };
        assert_eq!(run(&mut p, &c, &zp, 0.5), [ArmsAnim::Breathe]);
    }
}
