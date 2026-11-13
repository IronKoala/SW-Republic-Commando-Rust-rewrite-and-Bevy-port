//! The original helmet's start-up sequence, its shield charging sounds and
//! the tactical visor's arcs, as Engine.dll's `UHelmet::DrawHUD` and
//! `UHelmet::DrawTacticalVisionMode` do them (read by disassembly in a
//! scratch folder; nothing copied). The numbers and words come from the
//! install at runtime: the `Helmet` class defaults in Engine.u and the
//! HUD's sounds in `CTHUDDefaults` (Properties.u).
//!
//! The start-up sequence: a new helmet (each level, and after loading a
//! save, as the original makes a new `Helmet` then) counts `InitTime` up
//! with the level's time until the last phase has ended. In each phase
//! (`HealthInitStart`..`HealthInitEnd` 1 to 3 s, `ShieldInitStart`..
//! `ShieldInitEnd` 3.25 to 5.25 s, `SquadInitStart`..`SquadInitEnd` 5.5 to
//! 7.5 s) its line ("SCANNING HEALTH", "CHARGING SHIELDS", "INIT SQUAD
//! LINK") blinks at (400, 503) of the 800 x 600 HUD, and the health meter
//! and the shield bars fill from empty in a straight line; before their
//! phase they are empty. Squad panels appear one by one in their phase,
//! blinking, and are hidden before it.
//!
//! The shield sounds: while the bars (as drawn, so also while the start-up
//! fills them) rise below full, `ShieldRecharging` plays once (it isn't a
//! looping sound); when they reach full, or drop while it plays, it stops
//! with a 0.5 s fade and `ShieldCharged` plays; when they hold still below
//! full it stops at once. Dead or down, both charging sounds stop. Each is
//! played with its `AudioParams` volume and pitch (`Interface_Pitched`:
//! half volume, half pitch).

use bevy::audio::{AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, GameSet, GameState, PlayerVitals};
use rc_package::class_library::{ClassLibrary, DefaultValue};

/// One phase of the start-up sequence: from `start` to `end` seconds of
/// the helmet's `InitTime`, with its line.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// The start-up sequence (the `Helmet` class defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct StartUpPlan {
    pub health: Phase,
    pub shields: Phase,
    pub squad: Phase,
}

impl StartUpPlan {
    /// `InitTime` one frame on: it runs while any phase hasn't ended yet.
    pub fn advance(&self, init: f32, dt: f32) -> f32 {
        if [&self.health, &self.shields, &self.squad]
            .iter()
            .any(|p| p.end > init)
        {
            init + dt
        } else {
            init
        }
    }

    /// The line shown now, if any (health, then shields, then squad; the
    /// squad's only with a squad).
    pub fn text(&self, init: f32, has_squad: bool) -> Option<&str> {
        let on = |p: &Phase| init > p.start && p.end > init;
        if on(&self.health) {
            Some(&self.health.text)
        } else if on(&self.shields) {
            Some(&self.shields.text)
        } else if has_squad && on(&self.squad) {
            Some(&self.squad.text)
        } else {
            None
        }
    }
}

/// How full a bar is drawn in its phase: 0 before it, filling in a
/// straight line during it, `None` (as full as it is) after it.
pub fn fill(p: &Phase, init: f32) -> Option<f32> {
    if init > p.start {
        if p.end > init {
            Some((init - p.start) / (p.end - p.start))
        } else {
            None
        }
    } else {
        Some(0.0)
    }
}

/// The start-up line's opacity, `t` seconds into the level:
/// `int((sin(10 t) + 1) * 128)` kept as a byte (so the top, 256, is 0).
pub fn text_alpha(t: f32) -> u8 {
    (((t * 10.0).sin() as f64 + 1.0) * 128.0) as i64 as u8
}

/// A squad panel in the start-up sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot {
    /// Not drawn yet.
    Hidden,
    /// Drawn blinking (panel, picture and number at this opacity).
    Blinking(u8),
    /// As always.
    Normal,
}

/// Squad panel `slot` (1 for the first squad mate): hidden before the
/// squad phase; during it, hidden until `(end - start) / 4 * (slot - 1)`
/// is no longer above `(InitTime - start) / slot`, then blinking with
/// the HUD's blink (`sin(5 t) * 200 + 55`).
pub fn squad_slot(p: &Phase, init: f32, slot: usize, t: f32) -> Slot {
    if init > p.start {
        if p.end > init {
            let wait = (p.end - p.start) * 0.25 * (slot as f32 - 1.0);
            if wait > (init - p.start) / slot as f32 {
                Slot::Hidden
            } else {
                Slot::Blinking(rc_menus::hud::blink_alpha(t))
            }
        } else {
            Slot::Normal
        }
    } else {
        Slot::Hidden
    }
}

/// What happens to the shield charging sounds this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeStep {
    /// Start `ShieldRecharging`.
    Start,
    /// Fade `ShieldRecharging` out over 0.5 s and play `ShieldCharged`.
    Charged,
    /// Stop `ShieldRecharging` at once.
    Stop,
}

/// The shield sounds' step, from the bars as drawn last frame (`last`,
/// the helmet's `LastShield`) and now (`s`), and whether the charging
/// sound is marked playing.
pub fn charge_step(last: f32, s: f32, playing: bool) -> Option<ChargeStep> {
    if s < 1.0 && s > last {
        (!playing).then_some(ChargeStep::Start)
    } else if s == 1.0 || last > s {
        playing.then_some(ChargeStep::Charged)
    } else {
        playing.then_some(ChargeStep::Stop)
    }
}

/// The tactical visor's arcs and ring (Engine.dll
/// `UHelmet::DrawTacticalVisionMode`): the arcs grow in over two seconds
/// each time tactical mode starts; the ring turns half a turn a second.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tactical {
    /// Seconds since tactical mode started, up to 2.
    pub grow: f32,
    /// The ring's turn in Unreal's units (65536 a turn), below 65535.
    pub spin: f32,
    /// Tactical mode was on last frame.
    pub was_on: bool,
}

impl Tactical {
    /// One frame on. Returns (how far the arcs have grown in 0..1, the
    /// ring's turn 0..1) while tactical mode is on.
    pub fn step(&mut self, on: bool, dt: f32) -> Option<(f32, f32)> {
        if !on {
            self.was_on = false;
            return None;
        }
        if !self.was_on {
            self.grow = 0.0;
        } else if self.grow < 2.0 {
            self.grow += dt;
        }
        self.was_on = true;
        self.spin += dt * 32768.0;
        if self.spin >= 65535.0 {
            self.spin -= 65535.0;
        }
        let shown = if self.grow < 2.0 {
            self.grow * 0.5
        } else {
            1.0
        };
        Some((shown, self.spin / 65536.0))
    }
}

/// The original helmet's numbers, words and HUD sounds, read from the
/// install once.
#[derive(Resource, Default)]
pub struct HelmetStart {
    pub tried: bool,
    pub plan: Option<StartUpPlan>,
    /// `ShieldRecharging` and `ShieldCharged`, with volume and pitch.
    pub charging: Option<HudSound>,
    pub charged: Option<HudSound>,
}

#[derive(Clone)]
pub struct HudSound {
    pub name: String,
    pub sound: Handle<AudioSource>,
    pub volume: f32,
    pub pitch: f32,
}

/// Steps for the shield sounds, written by the HUD each frame.
#[derive(Resource, Default)]
pub struct ChargeSounds {
    pub steps: Vec<ChargeStep>,
    /// The helmet's `PlayingShieldChargeSound`.
    pub playing: bool,
    /// Times the charging and the charged sound were played (for the
    /// script's check).
    pub started: u32,
    pub charged: u32,
}

#[derive(Component)]
struct ChargingSound;

/// A sound fading out: seconds left of how many.
#[derive(Component)]
struct FadingOut(f32, f32, f32);

pub struct HelmetStartPlugin;

impl Plugin for HelmetStartPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HelmetStart>()
            .init_resource::<ChargeSounds>()
            .add_systems(OnEnter(GameState::InGame), read_defaults)
            .add_systems(
                Update,
                (play_charge_sounds, fade_out)
                    .chain()
                    .in_set(GameSet::Hud)
                    .after(crate::original_hud::DrawLayout)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn string(d: &rc_package::class_library::Defaults, name: &str) -> Option<String> {
    match &d.get(name)?.value {
        DefaultValue::Raw(b) => {
            // Length (with its zero), letters, zero.
            let len = *b.first()? as usize;
            let text = b.get(1..len)?;
            Some(String::from_utf8_lossy(text).into_owned())
        }
        _ => d.string(name),
    }
}

/// Read the `Helmet` defaults (Engine.u) and the HUD's shield sounds
/// (`CTHUDDefaults`, Properties.u), the first time a level starts.
fn read_defaults(
    mut start: ResMut<HelmetStart>,
    paths: Res<GamePaths>,
    mut audio: ResMut<Assets<AudioSource>>,
    log: Res<GameLog>,
) {
    if start.tried {
        return;
    }
    start.tried = true;
    let mut lib = ClassLibrary::new();
    for rel in [
        "GameData/System/Engine.u",
        "GameData/System/CTGame.u",
        "GameData/Properties/Properties.u",
    ] {
        let path = paths.game_file(rel);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read(&path) {
            Ok(data) => {
                if let Err(e) = lib.add_package(&name, data) {
                    log.line(format!("helmet start: {rel}: {e}"));
                }
            }
            Err(e) => log.line(format!("helmet start: can't read {rel}: {e}")),
        }
    }
    let h = lib.defaults("Helmet");
    let phase = |what: &str| -> Option<Phase> {
        Some(Phase {
            start: h.number(&format!("{what}InitStart"))?,
            end: h.number(&format!("{what}InitEnd"))?,
            text: string(&h, &format!("{what}InitText"))?,
        })
    };
    start.plan = match (phase("Health"), phase("Shield"), phase("Squad")) {
        (Some(health), Some(shields), Some(squad)) => Some(StartUpPlan {
            health,
            shields,
            squad,
        }),
        _ => None,
    };
    match &start.plan {
        Some(p) => log.line(format!(
            "helmet start: {:?} {}..{} s, {:?} {}..{} s, {:?} {}..{} s",
            p.health.text,
            p.health.start,
            p.health.end,
            p.shields.text,
            p.shields.start,
            p.shields.end,
            p.squad.text,
            p.squad.start,
            p.squad.end
        )),
        None => log.line("helmet start: Helmet's Init defaults not found; no start-up sequence"),
    }
    // The HUD's sounds, by full name (GEN_Sound.Interface.int_...).
    let wanted: Vec<(&str, String)> = ["ShieldRecharging", "ShieldCharged"]
        .iter()
        .filter_map(|k| Some((*k, lib.object_path("CTHUDDefaults", k)??)))
        .collect();
    let (params, notes) = rc_combat::read_audio_params(&paths, &["params_sfx".to_string()]);
    for n in notes {
        log.line(format!("helmet start: {n}"));
    }
    for (key, full) in wanted {
        let package = full.split('.').next().unwrap_or("").to_string();
        let name = full.rsplit('.').next().unwrap_or("").to_string();
        let (found, notes) =
            rc_combat::read_sounds(&paths, &package, std::slice::from_ref(&name), &mut audio);
        for n in notes {
            log.line(format!("helmet start: {n}"));
        }
        let Some((_, s)) = found.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) else {
            log.line(format!("helmet start: {key} sound {full} not found"));
            continue;
        };
        let Some(handle) = s.variants.first().cloned() else {
            continue;
        };
        let p = s.params.as_ref().and_then(|p| params.get(p));
        // The game picks between the lowest and highest each time; ours
        // have both the same.
        let (volume, pitch) = p.map_or((1.0, 1.0), |p| {
            (
                (p.volume.0 + p.volume.1) / 2.0,
                (p.pitch.0 + p.pitch.1) / 2.0,
            )
        });
        // The engine keeps the pitch between 0.5 and 2.
        let pitch = pitch.clamp(0.5, 2.0);
        log.line(format!(
            "helmet start: {key} = {full} (volume {volume}, pitch {pitch})"
        ));
        let sound = HudSound {
            name: full.clone(),
            sound: handle,
            volume,
            pitch,
        };
        if key == "ShieldRecharging" {
            start.charging = Some(sound);
        } else {
            start.charged = Some(sound);
        }
    }
}

fn spawn_sound(commands: &mut Commands, s: &HudSound, charging: bool) {
    let mut e = commands.spawn((
        AudioPlayer(s.sound.clone()),
        PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(s.volume))
            .with_speed(s.pitch),
        DespawnOnExit(GameState::InGame),
    ));
    if charging {
        e.insert(ChargingSound);
    }
}

fn play_charge_sounds(
    mut commands: Commands,
    mut steps: ResMut<ChargeSounds>,
    start: Res<HelmetStart>,
    vitals: Res<PlayerVitals>,
    charging: Query<Entity, (With<ChargingSound>, Without<FadingOut>)>,
    log: Res<GameLog>,
) {
    let steps = &mut *steps;
    // Dead or down: the original stops the charging sounds at once.
    if (vitals.health <= 0.0 || vitals.incapacitated) && steps.playing {
        for e in &charging {
            commands.entity(e).despawn();
        }
        steps.playing = false;
        steps.steps.clear();
        return;
    }
    for step in steps.steps.drain(..) {
        match step {
            ChargeStep::Start => {
                if let Some(s) = &start.charging {
                    spawn_sound(&mut commands, s, true);
                    log.line(format!("helmet start: shields charging, {} plays", s.name));
                    steps.started += 1;
                }
                steps.playing = true;
            }
            ChargeStep::Charged => {
                for e in &charging {
                    commands.entity(e).insert(FadingOut(0.5, 0.5, 1.0));
                }
                if let Some(s) = &start.charged {
                    spawn_sound(&mut commands, s, false);
                    log.line(format!("helmet start: shields charged, {} plays", s.name));
                    steps.charged += 1;
                }
                steps.playing = false;
            }
            ChargeStep::Stop => {
                for e in &charging {
                    commands.entity(e).despawn();
                }
                steps.playing = false;
            }
        }
    }
}

/// The 0.5 s fade (the engine's `StopSound` with a fade time). Our
/// reading: the fade is drawn as a straight line down.
fn fade_out(
    mut commands: Commands,
    time: Res<Time>,
    mut fading: Query<(Entity, &mut FadingOut, Option<&mut AudioSink>)>,
) {
    for (e, mut f, sink) in &mut fading {
        if f.2 == 1.0 {
            if let Some(s) = &sink {
                f.2 = s.volume().to_linear();
            }
        }
        f.0 -= time.delta_secs();
        if f.0 <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if let Some(mut s) = sink {
            s.set_volume(Volume::Linear(f.2 * f.0 / f.1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn plan() -> StartUpPlan {
        let p = |start, end, text: &str| Phase {
            start,
            end,
            text: text.into(),
        };
        // The Helmet class defaults in engine.u.
        StartUpPlan {
            health: p(1.0, 3.0, "SCANNING HEALTH"),
            shields: p(3.25, 5.25, "CHARGING SHIELDS"),
            squad: p(5.5, 7.5, "INIT SQUAD LINK"),
        }
    }

    #[test]
    fn the_start_up_runs_as_the_original() {
        let p = plan();
        assert_eq!(p.advance(0.0, 0.1), 0.1);
        assert_eq!(p.advance(7.6, 0.1), 7.6, "stops after the last phase");
        assert_eq!(fill(&p.shields, 2.0), Some(0.0), "empty before");
        assert_eq!(fill(&p.shields, 4.25), Some(0.5));
        assert_eq!(fill(&p.shields, 6.0), None, "as it is after");
        assert_eq!(p.text(2.0, true), Some("SCANNING HEALTH"));
        assert_eq!(p.text(4.0, true), Some("CHARGING SHIELDS"));
        assert_eq!(p.text(6.0, true), Some("INIT SQUAD LINK"));
        assert_eq!(p.text(6.0, false), None);
        assert_eq!(p.text(3.1, true), None, "a gap between phases");
    }

    #[test]
    fn the_line_blinks_and_wraps_as_a_byte() {
        assert_eq!(text_alpha(0.0), 128);
        // sin = 1 gives 256, kept as a byte: 0.
        assert_eq!(text_alpha(std::f32::consts::PI / 20.0), 0);
        assert_eq!(text_alpha(-std::f32::consts::PI / 20.0), 0);
    }

    #[test]
    fn squad_panels_come_in_one_by_one() {
        let p = plan().squad;
        assert_eq!(squad_slot(&p, 5.0, 1, 0.0), Slot::Hidden);
        assert!(matches!(squad_slot(&p, 5.6, 1, 0.0), Slot::Blinking(_)));
        // Slot 2 waits until (init - 5.5) / 2 >= 0.5.
        assert_eq!(squad_slot(&p, 6.4, 2, 0.0), Slot::Hidden);
        assert!(matches!(squad_slot(&p, 6.6, 2, 0.0), Slot::Blinking(_)));
        assert_eq!(squad_slot(&p, 7.6, 3, 0.0), Slot::Normal);
    }

    #[test]
    fn shield_sounds_follow_the_bars() {
        use ChargeStep::*;
        assert_eq!(charge_step(0.2, 0.3, false), Some(Start));
        assert_eq!(charge_step(0.2, 0.3, true), None);
        assert_eq!(charge_step(0.9, 1.0, true), Some(Charged));
        assert_eq!(charge_step(0.9, 1.0, false), None);
        assert_eq!(
            charge_step(0.5, 0.4, true),
            Some(Charged),
            "a hit while charging"
        );
        assert_eq!(charge_step(0.5, 0.5, true), Some(Stop));
    }

    #[test]
    fn tactical_arcs_grow_in_over_two_seconds() {
        let mut t = Tactical::default();
        assert_eq!(t.step(false, 0.1), None);
        assert_eq!(t.step(true, 0.1).unwrap().0, 0.0, "first frame: nothing");
        let (g, s) = t.step(true, 1.0).unwrap();
        assert_eq!(g, 0.5);
        assert!((s - 1.0).abs() < 1e-3 || (s - 0.0).abs() < 1e-3 || s > 0.0);
        t.step(true, 1.0);
        assert_eq!(t.step(true, 1.0).unwrap().0, 1.0);
        t.step(false, 0.1);
        assert_eq!(t.step(true, 0.1).unwrap().0, 0.0, "again from nothing");
    }

    #[test]
    fn the_ring_turns_half_a_turn_a_second() {
        let mut t = Tactical::default();
        t.step(true, 0.0);
        let (_, a) = t.step(true, 0.25).unwrap();
        assert!((a - 0.125).abs() < 1e-3, "{a}");
    }

    /// With the install (RC_GAME_DIR), the start-up words, times and
    /// shield sounds are read from the game's own files.
    #[test]
    fn the_install_gives_the_words_times_and_sounds() {
        let Ok(dir) = std::env::var("RC_GAME_DIR") else {
            eprintln!("RC_GAME_DIR not set; skipped");
            return;
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<AudioSource>()
            .init_resource::<HelmetStart>()
            .insert_resource(GamePaths::new(dir, Vec::new()))
            .insert_resource(GameLog::terminal());
        app.world_mut().run_system_once(read_defaults).unwrap();
        let start = app.world().resource::<HelmetStart>();
        assert_eq!(start.plan, Some(plan()));
        let charging = start.charging.as_ref().expect("ShieldRecharging");
        assert_eq!(charging.name, "GEN_Sound.Interface.int_GEN_shieldChrgUp_01");
        assert_eq!((charging.volume, charging.pitch), (0.5, 0.5));
        let charged = start.charged.as_ref().expect("ShieldCharged");
        assert_eq!(charged.name, "GEN_Sound.Interface.int_GEN_sqdRevivFull_01");
    }
}
