//! One weapon: its numbers (from a data file) and its state while held
//! (ammo, reloading, when it may fire next). Plain Rust, no Bevy.
//!
//! The rules follow the game's own script (`Engine.Weapon`,
//! `Engine.Ammunition`, `CTInventory.CTAmmoRecharging`), described in our
//! own words in docs/DESIGN.md, "Weapons".

use crate::data::Table;
use crate::health::DamageKind;

/// How the trigger works (the script's `EFireMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireMode {
    /// One shot per press (`FM_SemiAuto`, the game's default).
    SemiAuto,
    /// Keeps firing while held (`FM_Automatic`).
    Automatic,
}

/// What a shot is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotKind {
    /// A laser bolt that flies. The game starts it at rest and speeds it up
    /// at `accel` uu/s² until `max_speed` (a `Projectile` with
    /// `bLaserBolt`).
    Bolt {
        accel: f32,
        max_speed: f32,
        /// Seconds before an unfired bolt disappears.
        life_span: f32,
    },
    /// Hits at once along a line up to `range` uu (`bInstantHit` ammo).
    Instant { range: f32 },
    /// Fires a grenade (the weapon's `grenade` file) at `speed` uu/s along
    /// the aim: the DC-17m's anti-armour attachment. It flies, falls and
    /// goes off like a thrown grenade.
    Lobbed { speed: f32 },
}

/// Everything about one kind of weapon. Distances are Unreal units (uu),
/// times are seconds, angles are the game's rotation units (65536 = one
/// full turn) where noted.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponParams {
    /// File name without `.toml`, like `dc17m_blaster`.
    pub id: String,
    pub name: String,
    pub fire_mode: FireMode,
    /// Shortest time between shots (`ShotDelay`).
    pub shot_delay: f32,
    /// Aim error: a shot lands up to this many uu off for every 1000 uu of
    /// distance, sideways and up/down (`AimError`).
    pub aim_error: f32,
    /// Shots in one magazine (`ReloadCount`). 0 means the weapon has no
    /// magazine and recharges instead.
    pub clip_size: u32,
    /// Most ammo carried, magazine included (`MaxAmmo`).
    pub max_ammo: u32,
    /// Ammo when picked up at the start, magazine included
    /// (`PickupAmmoCount`).
    pub start_ammo: u32,
    /// Recharging weapons: shots regained per second (`RechargeRate`).
    pub recharge_rate: f32,
    /// Seconds a reload takes (the first-person `Reload` animation).
    pub reload_time: f32,
    pub shot: ShotKind,
    /// For `Lobbed` shots: the grenade file it fires.
    pub grenade: Option<String>,
    /// The zoom steps, fields of view in degrees across a 4:3 screen like
    /// the game's own setting (`ZoomFOVs` after the first). Each press of
    /// zoom goes to the next, then back out. Empty: no zoom.
    pub zoom_fovs: Vec<f32>,
    /// Damage is multiplied by this while zoomed (`ZoomDamageMultiplier`).
    pub zoom_damage: f32,
    pub damage: f32,
    pub damage_kind: DamageKind,
    /// The damage type class (`MyDamageType`), looked up in the target's
    /// damage table.
    pub damage_type: String,
    /// The melee hit with this weapon in hand: reach in uu and damage
    /// (`MeleeRange`, `MeleeDamage`). `None`: the player's own in
    /// player.toml.
    pub melee: Option<(f32, f32)>,
    /// Where shots start, from the eye: forward, right, up (`FPFireOffset`).
    pub fire_offset: [f32; 3],
    /// The camera kick per shot, in rotation units: up, sideways
    /// (`WeaponKick`). Sideways is random between minus and plus this.
    pub kick: [f32; 2],
    /// How quickly the view springs back after a kick (`RecoilConstant`).
    pub recoil_spring: f32,
    /// How much of the recoil's speed is lost each step (`RecoilDampening`).
    pub recoil_damping: f32,
    /// Sounds, by name in `sound_package`. One fire sound is picked at
    /// random per shot.
    pub sound_package: String,
    pub fire_sounds: Vec<String>,
    pub reload_sound: Option<String>,
    pub empty_sound: Option<String>,
    /// The first-person model: a static mesh in `model_package`.
    pub model_package: String,
    pub model: Option<String>,
    /// Where the model sits in front of the eye: forward, right, up (uu).
    pub model_offset: [f32; 3],
    /// Colour of the bolt, red, green, blue from 0 to 1.
    pub bolt_colour: [f32; 3],
    /// The animated first-person arms holding it (`[arms]`). `None`: the
    /// plain `model` is shown instead.
    pub arms: Option<ArmsParams>,
}

/// A weapon's first-person arms: an animated model in the game's
/// animation packages (the weapon class's `Mesh`, like `HudArmsDC17m`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmsParams {
    /// Relative to the game folder.
    pub package: String,
    pub mesh: String,
    /// Field of view the arms are drawn with, degrees across a 4:3 screen
    /// (`DisplayFOV`), whatever the view's own setting or zoom.
    pub fov: f32,
    /// Weapons in the same group swap with the attachment moves
    /// (`AttachHolster`, `AttachLoad`) instead of `Holster` and `Load`.
    pub attachment_group: Option<String>,
    /// Hidden while zoomed in (`bZoomedUsesNoHUDArms`: the sniper scope).
    pub hide_zoomed: bool,
    /// How bright the arms are drawn, 0 to 1.
    pub brightness: f32,
    /// The ammo counter on the gun (see `counter`): the model's digit
    /// pieces, hundreds first, by material name (the ammo class's
    /// `PannerDigit100`, `PannerDigit10`, `PannerDigit1`). Empty: none.
    pub counter_digits: Vec<String>,
    /// The counter's magazine bar piece (`PannerCharge`).
    pub counter_bar: Option<String>,
}

impl WeaponParams {
    /// Read a weapon file. `id` is its name without `.toml`.
    pub fn from_text(id: &str, text: &str) -> Result<WeaponParams, String> {
        let mut t = Table::parse(text)?;
        let need = |v: Option<f32>, name: &str| v.ok_or_else(|| format!("`{name}` is missing"));
        let fire_mode = match t.text("fire.mode")?.as_deref() {
            None | Some("semi_auto") => FireMode::SemiAuto,
            Some("automatic") => FireMode::Automatic,
            Some(o) => {
                return Err(format!(
                    "fire.mode `{o}`: use \"semi_auto\" or \"automatic\""
                ))
            }
        };
        let shot = match t.text("shot.kind")?.as_deref() {
            Some("bolt") => ShotKind::Bolt {
                accel: need(t.number("shot.accel")?, "shot.accel")?,
                max_speed: need(t.number("shot.max_speed")?, "shot.max_speed")?,
                life_span: t.number("shot.life_span")?.unwrap_or(10.0),
            },
            Some("instant") => ShotKind::Instant {
                range: need(t.number("shot.range")?, "shot.range")?,
            },
            Some("lobbed") => ShotKind::Lobbed {
                speed: need(t.number("shot.speed")?, "shot.speed")?,
            },
            Some(o) => {
                return Err(format!(
                    "shot.kind `{o}`: use \"bolt\", \"instant\" or \"lobbed\""
                ))
            }
            None => return Err("`shot.kind` is missing".to_string()),
        };
        let damage_kind = match t.text("shot.damage_kind")? {
            None => DamageKind::Blaster,
            Some(k) => DamageKind::from_name(&k)
                .ok_or_else(|| format!("shot.damage_kind `{k}` is not a known kind"))?,
        };
        let kick = t.vec3("recoil.kick")?.unwrap_or([0.0; 3]);
        let p = WeaponParams {
            id: id.to_string(),
            name: t.text("name")?.unwrap_or_else(|| id.to_string()),
            fire_mode,
            shot_delay: need(t.number("fire.shot_delay")?, "fire.shot_delay")?,
            aim_error: t.number("fire.aim_error")?.unwrap_or(0.0),
            clip_size: t.number("ammo.clip_size")?.unwrap_or(0.0) as u32,
            max_ammo: need(t.number("ammo.max_ammo")?, "ammo.max_ammo")? as u32,
            start_ammo: t
                .number("ammo.start_ammo")?
                .map(|v| v as u32)
                .unwrap_or(u32::MAX),
            recharge_rate: t.number("ammo.recharge_rate")?.unwrap_or(0.0),
            reload_time: t.number("ammo.reload_time")?.unwrap_or(0.0),
            shot,
            grenade: t.text("shot.grenade")?,
            zoom_fovs: t.numbers("fire.zoom_fovs")?.unwrap_or_default(),
            zoom_damage: t.number("fire.zoom_damage")?.unwrap_or(1.0),
            damage_type: t
                .text("shot.damage_type")?
                .unwrap_or_else(|| "CTDamageEnergy".to_string()),
            melee: match (t.number("melee.range")?, t.number("melee.damage")?) {
                (Some(r), Some(d)) => Some((r, d)),
                (None, None) => None,
                _ => return Err("[melee] needs both `range` and `damage`".into()),
            },
            // A lobbed shot's damage is its grenade's.
            damage: if matches!(shot, ShotKind::Lobbed { .. }) {
                t.number("shot.damage")?.unwrap_or(0.0)
            } else {
                need(t.number("shot.damage")?, "shot.damage")?
            },
            damage_kind,
            fire_offset: t.vec3("fire.offset")?.unwrap_or([0.0; 3]),
            kick: [kick[0], kick[1]],
            recoil_spring: t.number("recoil.spring")?.unwrap_or(0.0),
            recoil_damping: t.number("recoil.damping")?.unwrap_or(0.0),
            sound_package: t
                .text("sound.package")?
                .unwrap_or_else(|| "gen_sound".to_string()),
            fire_sounds: t.texts("sound.fire")?.unwrap_or_default(),
            reload_sound: t.text("sound.reload")?,
            empty_sound: t.text("sound.empty")?,
            model_package: t
                .text("model.package")?
                .unwrap_or_else(|| "weapons".to_string()),
            model: t.text("model.mesh")?,
            model_offset: t.vec3("model.offset")?.unwrap_or([0.0; 3]),
            bolt_colour: t.vec3("model.bolt_colour")?.unwrap_or([1.0, 0.3, 0.1]),
            arms: match t.text("arms.mesh")? {
                None => None,
                Some(mesh) => Some(ArmsParams {
                    package: t
                        .text("arms.package")?
                        .unwrap_or_else(|| "GameData/Animations/hudarms.ukx".to_string()),
                    mesh,
                    fov: t.number("arms.fov")?.unwrap_or(85.0),
                    attachment_group: t.text("arms.attachment_group")?,
                    hide_zoomed: t.boolean("arms.hide_zoomed")?.unwrap_or(false),
                    brightness: t.number("arms.brightness")?.unwrap_or(0.75),
                    counter_digits: t.texts("arms.counter_digits")?.unwrap_or_default(),
                    counter_bar: t.text("arms.counter_bar")?,
                }),
            },
        };
        t.finish()?;
        let p = WeaponParams {
            start_ammo: p.start_ammo.min(p.max_ammo),
            ..p
        };
        if matches!(p.shot, ShotKind::Lobbed { .. }) && p.grenade.is_none() {
            return Err("a \"lobbed\" shot needs `shot.grenade`, the grenade file it fires".into());
        }
        if p.shot_delay < 0.0 || p.max_ammo == 0 {
            return Err("fire.shot_delay must be 0 or more and ammo.max_ammo more than 0".into());
        }
        Ok(p)
    }

    pub fn recharges(&self) -> bool {
        self.clip_size == 0
    }
}

/// What happened when the trigger was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Not pulled, or still waiting for the shot delay.
    Nothing,
    /// A shot left the weapon.
    Fired,
    /// Pulled with no ammo at all: the game plays the empty click.
    Empty,
    /// The magazine was empty, so a reload started instead.
    Reloading,
}

/// A held weapon's changing state.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponState {
    /// Shots in the magazine. For recharging weapons, the charge (whole
    /// shots and part of the next).
    pub clip: f32,
    /// Ammo outside the magazine (always 0 for recharging weapons).
    pub reserve: u32,
    /// Seconds until the next shot is allowed.
    pub cooldown: f32,
    /// Seconds of reload left, if reloading.
    pub reloading: Option<f32>,
    /// Shots fired since picking it up, for logs and tests.
    pub shots_fired: u32,
}

impl WeaponState {
    /// A freshly picked-up weapon with its starting ammo, magazine full.
    pub fn new(p: &WeaponParams) -> Self {
        if p.recharges() {
            return WeaponState {
                clip: p.max_ammo as f32,
                reserve: 0,
                cooldown: 0.0,
                reloading: None,
                shots_fired: 0,
            };
        }
        let clip = p.clip_size.min(p.start_ammo);
        WeaponState {
            clip: clip as f32,
            reserve: p.start_ammo - clip,
            cooldown: 0.0,
            reloading: None,
            shots_fired: 0,
        }
    }

    /// Whole shots in the magazine.
    pub fn shots_in_clip(&self) -> u32 {
        self.clip.floor().max(0.0) as u32
    }

    /// Let `dt` seconds pass: the shot delay counts down, a reload may
    /// finish, and recharging weapons regain charge. Returns true when a
    /// reload finished.
    pub fn tick(&mut self, p: &WeaponParams, dt: f32) -> bool {
        // May go below 0 while the trigger is held, so the time left over
        // in a frame counts toward the next shot (see `trigger`).
        self.cooldown = (self.cooldown - dt).max(-p.shot_delay.max(dt));
        if p.recharges() {
            self.clip = (self.clip + p.recharge_rate * dt).min(p.max_ammo as f32);
        }
        if let Some(left) = self.reloading {
            let left = left - dt;
            if left > 0.0 {
                self.reloading = Some(left);
            } else {
                self.reloading = None;
                let want = p.clip_size - self.shots_in_clip();
                let moved = want.min(self.reserve);
                self.clip += moved as f32;
                self.reserve -= moved;
                return true;
            }
        }
        false
    }

    /// Start a reload if it makes sense: the weapon reloads, isn't already
    /// reloading, the magazine isn't full, and there is spare ammo. The
    /// game's rule (`Weapon.ForceReload`) is the same.
    pub fn start_reload(&mut self, p: &WeaponParams) -> bool {
        if p.recharges()
            || self.reloading.is_some()
            || self.shots_in_clip() >= p.clip_size
            || self.reserve == 0
        {
            return false;
        }
        self.reloading = Some(p.reload_time);
        true
    }

    /// Check the trigger. `held` is whether fire is held this frame,
    /// `pressed` whether it went down this frame (semi-automatic weapons
    /// need a fresh press for every shot).
    pub fn trigger(&mut self, p: &WeaponParams, held: bool, pressed: bool) -> Trigger {
        let wants = match p.fire_mode {
            FireMode::Automatic => held,
            FireMode::SemiAuto => pressed,
        };
        if !wants || self.reloading.is_some() {
            self.cooldown = self.cooldown.max(0.0);
            return Trigger::Nothing;
        }
        if self.clip < 1.0 {
            if self.start_reload(p) {
                return Trigger::Reloading;
            }
            // Only a fresh press clicks, so holding the trigger on an empty
            // gun doesn't click every frame.
            return if pressed {
                Trigger::Empty
            } else {
                Trigger::Nothing
            };
        }
        if self.cooldown > 1e-6 {
            return Trigger::Nothing;
        }
        self.clip -= 1.0;
        // Adding the delay (instead of setting it) keeps the fire rate the
        // same at any frame rate: 0.1 s is exactly 10 shots a second even
        // when frames don't divide it evenly. **Our choice:** the game
        // sets the next shot time from the current frame, which loses the
        // left-over part of a frame each shot.
        self.cooldown += p.shot_delay;
        self.shots_fired += 1;
        Trigger::Fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const RIFLE: &str = r#"
name = "Test rifle"
[fire]
mode = "automatic"
shot_delay = 0.1
aim_error = 15
[ammo]
clip_size = 60
max_ammo = 300
start_ammo = 120
reload_time = 1.5
[shot]
kind = "bolt"
accel = 60000
max_speed = 10000
damage = 20
"#;

    pub(crate) const PISTOL: &str = r#"
[fire]
shot_delay = 0.2
[ammo]
max_ammo = 5
recharge_rate = 1.5
[shot]
kind = "bolt"
accel = 40000
max_speed = 8000
damage = 55
"#;

    #[test]
    fn automatic_fire_rate_and_magazine() {
        let p = WeaponParams::from_text("rifle", RIFLE).unwrap();
        let mut w = WeaponState::new(&p);
        assert_eq!((w.shots_in_clip(), w.reserve), (60, 60));
        // Hold the trigger for one second at 100 steps per second.
        let mut shots = 0;
        for i in 0..100 {
            if w.trigger(&p, true, i == 0) == Trigger::Fired {
                shots += 1;
            }
            w.tick(&p, 0.01);
        }
        assert_eq!(shots, 10, "0.1 s between shots is 10 shots a second");
        assert_eq!(w.shots_in_clip(), 50);
    }

    #[test]
    fn empty_magazine_reloads_from_reserve() {
        let p = WeaponParams::from_text("rifle", RIFLE).unwrap();
        let mut w = WeaponState::new(&p);
        w.clip = 0.0;
        assert_eq!(w.trigger(&p, true, true), Trigger::Reloading);
        // No shots during the 1.5 s reload.
        let mut t = 0.0;
        while w.reloading.is_some() {
            assert_eq!(w.trigger(&p, true, false), Trigger::Nothing);
            w.tick(&p, 0.05);
            t += 0.05;
        }
        assert!((t - 1.5f32).abs() < 0.051, "reload took {t}");
        assert_eq!((w.shots_in_clip(), w.reserve), (60, 0));
        // Fire everything; then an empty click, and no reload.
        w.clip = 0.0;
        assert_eq!(w.trigger(&p, true, true), Trigger::Empty);
        assert!(!w.start_reload(&p));
    }

    #[test]
    fn semi_auto_needs_a_press_and_recharges() {
        let p = WeaponParams::from_text("pistol", PISTOL).unwrap();
        let mut w = WeaponState::new(&p);
        assert_eq!(w.trigger(&p, true, true), Trigger::Fired);
        w.tick(&p, 0.5);
        assert_eq!(
            w.trigger(&p, true, false),
            Trigger::Nothing,
            "held, not pressed"
        );
        assert_eq!(w.trigger(&p, true, true), Trigger::Fired);
        // Below one whole shot it clicks; 1.5 shots a second come back.
        w.clip = 0.5;
        w.tick(&p, 0.2);
        assert_eq!(w.trigger(&p, true, true), Trigger::Empty);
        w.tick(&p, 0.2);
        assert!((w.clip - 1.1).abs() < 1e-4, "{}", w.clip);
        assert_eq!(w.trigger(&p, true, true), Trigger::Fired);
        w.tick(&p, 10.0);
        assert_eq!(w.clip, 5.0);
    }

    #[test]
    fn data_mistakes_are_reported() {
        let e = WeaponParams::from_text("x", "[fire]\nshot_delay = 1\n").unwrap_err();
        assert_eq!(e, "`shot.kind` is missing");
        let e = WeaponParams::from_text("x", &format!("{RIFLE}\nbogus = 1\n")).unwrap_err();
        assert!(e.contains("unknown setting `shot.bogus`"), "{e}");
    }
}
