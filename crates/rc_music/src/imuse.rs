//! What plays when: the game's music rules, called iMuse in its code
//! (`IMuseStateInfo`, `IMuseTrigger`, `ACTION_SetImuseState`).
//!
//! The rules come from the game's own data and script classes; the engine
//! code that runs them is native, so how it times things is our reading.
//! What the data says, and what we decided where it doesn't (marked
//! "our choice"), is in `docs/DESIGN.md` under "Music".
//!
//! Three layers play at once:
//! - **main**: the room music (the "state", usually a looping suspense
//!   piece), or the battle loop during a fight, or the "player down" loop
//!   while the player waits for a squad mate. One at a time; a change fades
//!   the old piece out (its own `FadeOutTime`) and the new one in (its own
//!   `FadeInTime`), unless a script gives its own fade times.
//! - **overlay**: short pieces now and then over the room music, from the
//!   room's overlay group. The first waits `OverlayStartupDelay` seconds,
//!   each next one a random `OverlayMinSpacing`..`OverlayMaxSpacing`
//!   seconds after the last ended. Not during fights.
//! - **episode**: one-shot pieces on top of everything (`ACTION_PlayMusic`,
//!   the victory flourish at the end of a fight).
//!
//! The director doesn't play sound itself: it hands out [`Command`]s, and
//! the game tells it when a one-shot piece has ended ([`Imuse::finished`]).

use crate::library::{Fade, MusicLibrary, Piece, Rng, RoomMusic};
use std::collections::HashSet;
use std::sync::Arc;

/// A sound layer (see the module notes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Main,
    Overlay,
    Episode,
}

impl Layer {
    pub const ALL: [Layer; 3] = [Layer::Main, Layer::Overlay, Layer::Episode];
    pub fn name(self) -> &'static str {
        match self {
            Layer::Main => "main",
            Layer::Overlay => "overlay",
            Layer::Episode => "episode",
        }
    }
}

/// What the game should do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Start `piece` on `layer` at loudness `volume` (before the player's
    /// music volume), fading in over `fade_in`. Whatever played on the
    /// layer fades out over `fade_out_old`.
    Play {
        layer: Layer,
        piece: Piece,
        volume: f32,
        fade_in: Fade,
        fade_out_old: Fade,
    },
    /// Fade out whatever plays on `layer`.
    Stop { layer: Layer, fade: Fade },
}

/// What counts towards a fight (the engine's `EBattleEngineBattleCriteria`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criteria {
    /// Enemies that are fighting (have seen or heard the squad), anywhere.
    Attack,
    /// Living enemies near the player, fighting or not.
    Proximity,
    /// Enemies that are fighting and near the player (the engine default).
    AttackAndProximity,
}

impl Criteria {
    pub fn from_byte(b: u8) -> Criteria {
        match b {
            0 => Criteria::Attack,
            1 => Criteria::Proximity,
            _ => Criteria::AttackAndProximity,
        }
    }
}

/// The battle engine's settings: the map's `LevelInfo` (`BattleMusic`,
/// `BattleBaddieThreshold`, `BattleRadiusInner`, `BattleRadiusOuter`,
/// `BattleCriteria`), changed later by scripts.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleSettings {
    /// The battle group ("megamulti") to pick fights' music from.
    pub music: Option<String>,
    /// How many enemies start a fight.
    pub threshold: u32,
    /// Enemies this close (Unreal units) start a fight...
    pub inner: f32,
    /// ...and the fight lasts while any is this close.
    pub outer: f32,
    pub criteria: Criteria,
}

impl Default for BattleSettings {
    /// The engine's `LevelInfo` defaults.
    fn default() -> Self {
        BattleSettings {
            music: None,
            threshold: 3,
            inner: 1250.0,
            outer: 2500.0,
            criteria: Criteria::AttackAndProximity,
        }
    }
}

/// An enemy, as the music sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threat {
    /// Where it is, in Unreal units.
    pub at: [f32; 3],
    /// Fighting: it has seen or heard the squad.
    pub engaged: bool,
}

/// Fade times a script gives (`OverrideFadeDefaults`): `fade_in` for the
/// new piece, `fade_out` for the one it replaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeOverride {
    pub fade_in: Fade,
    pub fade_out: Fade,
}

/// Our choices where the game's data says nothing (see `data/music/music.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct Tuning {
    /// Seconds between battle checks (the engine keeps an
    /// `mNextBattleUpdateTime`; its spacing isn't stored).
    pub battle_check_seconds: f32,
    /// A fight ends after this many seconds with no enemy in range, so it
    /// doesn't flicker on and off at the edge.
    pub battle_end_seconds: f32,
    /// Fade for the overlay layer when a fight or a script stops it.
    pub overlay_stop_fade: f32,
    /// The "player down" piece when the battle music names none.
    pub player_down: String,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            battle_check_seconds: 0.5,
            battle_end_seconds: 3.0,
            overlay_stop_fade: 1.0,
            player_down: "musGEN_playerincap_lp".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Main {
    Silent,
    State,
    Battle,
    Down,
}

#[derive(Debug, Clone)]
struct Overlays {
    group: String,
    first: Option<String>,
    min: f32,
    max: f32,
    /// When the next one starts; `None` while one plays.
    next_at: Option<f32>,
}

#[derive(Debug, Clone)]
struct Fight {
    /// The battle group picked for this fight (holds the won/lost music).
    multi: Option<String>,
    forced: bool,
    /// When enemies were last in range.
    last_in_range: f32,
}

/// The music director for one level.
pub struct Imuse {
    lib: Arc<MusicLibrary>,
    pub tuning: Tuning,
    rng: Rng,
    time: f32,
    /// The room music ("state"), by name.
    state: Option<String>,
    main: Main,
    playing: [Option<Piece>; 3],
    overlays: Option<Overlays>,
    pub battle: BattleSettings,
    pub battle_enabled: bool,
    fight: Option<Fight>,
    down: bool,
    next_check: f32,
    used_rooms: HashSet<String>,
    out: Vec<Command>,
    log: Vec<String>,
}

fn slot(l: Layer) -> usize {
    match l {
        Layer::Main => 0,
        Layer::Overlay => 1,
        Layer::Episode => 2,
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

impl Imuse {
    pub fn new(lib: Arc<MusicLibrary>, battle: BattleSettings, tuning: Tuning, seed: u64) -> Imuse {
        Imuse {
            lib,
            tuning,
            rng: Rng::new(seed),
            time: 0.0,
            state: None,
            main: Main::Silent,
            playing: [None, None, None],
            overlays: None,
            battle,
            battle_enabled: true,
            fight: None,
            down: false,
            next_check: 0.0,
            used_rooms: HashSet::new(),
            out: Vec::new(),
            log: Vec::new(),
        }
    }

    /// Commands since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.out)
    }

    /// Log lines since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    pub fn in_fight(&self) -> bool {
        self.fight.is_some()
    }

    /// The piece playing on a layer, as far as the director knows.
    pub fn playing(&self, layer: Layer) -> Option<&Piece> {
        self.playing[slot(layer)].as_ref()
    }

    fn note(&mut self, s: String) {
        self.log.push(format!("t={:.1} {s}", self.time));
    }

    fn play(&mut self, layer: Layer, piece: Piece, fades: Option<FadeOverride>) {
        let old = self.playing[slot(layer)].take();
        let fade_out_old = match (fades, &old) {
            (Some(f), _) => f.fade_out,
            (None, Some(o)) => o.fade_out,
            (None, None) => Fade::NONE,
        };
        let fade_in = fades.map_or(piece.fade_in, |f| f.fade_in);
        let volume = self.rng.range(piece.volume.0, piece.volume.1);
        self.note(format!(
            "{}: {} ({}{}, fade in {:.1}s){}",
            layer.name(),
            piece.name,
            if piece.looping { "loops" } else { "once" },
            if volume < 0.999 {
                format!(", volume {volume:.2}")
            } else {
                String::new()
            },
            fade_in.seconds,
            old.as_ref()
                .map(|o| format!(", {} fades out over {:.1}s", o.name, fade_out_old.seconds))
                .unwrap_or_default()
        ));
        self.playing[slot(layer)] = Some(piece.clone());
        self.out.push(Command::Play {
            layer,
            piece,
            volume,
            fade_in,
            fade_out_old,
        });
    }

    fn stop(&mut self, layer: Layer, fade: Option<Fade>) {
        if let Some(old) = self.playing[slot(layer)].take() {
            let fade = fade.unwrap_or(old.fade_out);
            self.note(format!(
                "{}: {} stops (fade out {:.1}s)",
                layer.name(),
                old.name,
                fade.seconds
            ));
            self.out.push(Command::Stop { layer, fade });
        }
    }

    /// Resolve a name to a piece, logging names the library doesn't know.
    fn piece(&mut self, name: &str) -> Option<Piece> {
        let lib = self.lib.clone();
        let p = lib.resolve(name, &mut self.rng);
        if p.is_none() {
            self.note(format!("music {name}: not in the music packages; skipped"));
        }
        p
    }

    /// Put the room music back on the main layer (after a fight, or when
    /// the player gets up).
    fn resume_state(&mut self, fades: Option<FadeOverride>) {
        match self.state.clone().and_then(|s| self.piece(&s)) {
            Some(p) => {
                self.main = Main::State;
                self.play(Layer::Main, p, fades);
            }
            None => {
                self.main = Main::Silent;
                self.stop(Layer::Main, fades.map(|f| f.fade_out));
            }
        }
    }

    fn stop_overlays(&mut self) {
        let fade = Fade {
            seconds: self.tuning.overlay_stop_fade,
            ..Fade::NONE
        };
        self.stop(Layer::Overlay, Some(fade));
        if let Some(o) = &mut self.overlays {
            o.next_at = None;
        }
    }

    // ---- What scripts and the level ask for ----

    /// `ACTION_SetImuseState`: new room music (`None`: silence). Overlays
    /// stop: the script takes over. During a fight or while the player is
    /// down the new music waits until that's over.
    pub fn set_state(&mut self, name: Option<&str>, fades: Option<FadeOverride>) {
        self.note(format!(
            "script sets the music state to {}",
            name.unwrap_or("silence")
        ));
        self.state = name.map(str::to_string);
        self.overlays = None;
        self.stop_overlays();
        if self.fight.is_some() || self.down {
            self.note("(waits for the fight to end)".into());
            return;
        }
        self.resume_state(fades);
    }

    /// `ACTION_PlayMusic` (and a `MusicTrigger`'s episode): a one-shot
    /// piece on top of everything.
    pub fn play_episode(&mut self, name: &str, fades: Option<FadeOverride>) {
        if let Some(p) = self.piece(name) {
            self.play(Layer::Episode, p, fades);
        }
    }

    /// The player walked into a volume whose `AudioClass` has music.
    /// `volume` names the volume (for "once only").
    pub fn enter_room(&mut self, volume: &str, room: &RoomMusic) {
        let key = volume.to_ascii_lowercase();
        if room.once_only && self.used_rooms.contains(&key) {
            return;
        }
        self.used_rooms.insert(key);
        self.note(format!(
            "entered {volume} ({}): state {}, overlays {}",
            room.class,
            room.state.as_deref().unwrap_or("-"),
            room.overlays.as_deref().unwrap_or("-")
        ));
        if let Some(s) = &room.state {
            let same = self
                .state
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(s));
            self.state = Some(s.clone());
            if !same && self.fight.is_none() && !self.down {
                self.resume_state(None);
            }
        }
        if let Some(g) = &room.overlays {
            let first = self.lib.group(g).and_then(|g| g.first.clone());
            self.overlays = Some(Overlays {
                group: g.clone(),
                first,
                min: room.min_spacing,
                max: room.max_spacing.max(room.min_spacing),
                next_at: if self.playing(Layer::Overlay).is_some() {
                    None
                } else {
                    Some(self.time + room.startup_delay)
                },
            });
        }
    }

    /// `ACTION_EnableBattleEngine`. Switching off during a fight ends it
    /// without a flourish.
    pub fn enable_battle(&mut self, on: bool) {
        self.note(format!("battle engine {}", if on { "on" } else { "off" }));
        self.battle_enabled = on;
        if !on && self.fight.is_some() {
            self.end_fight(false);
        }
    }

    /// `ACTION_SetBattleEngineParams`.
    pub fn set_battle_params(
        &mut self,
        criteria: Criteria,
        threshold: u32,
        inner: f32,
        outer: f32,
    ) {
        self.note(format!(
            "battle settings: {criteria:?}, {threshold} enemies, {inner:.0}/{outer:.0} uu"
        ));
        self.battle.criteria = criteria;
        self.battle.threshold = threshold;
        self.battle.inner = inner;
        self.battle.outer = outer;
    }

    /// `ACTION_SetBattleMusicMulti`.
    pub fn set_battle_music(&mut self, name: Option<&str>) {
        self.note(format!("battle music is now {}", name.unwrap_or("none")));
        self.battle.music = name.map(str::to_string);
    }

    /// `ACTION_ForceBattleMusic`: a fight starts now and lasts until a
    /// script ends it.
    pub fn force_battle(&mut self) {
        match &mut self.fight {
            Some(f) => f.forced = true,
            None => self.start_fight(true),
        }
    }

    /// `ACTION_ForceVictoryFlourish`: the fight ends with its flourish.
    pub fn force_victory(&mut self) {
        if self.fight.is_some() {
            self.end_fight(true);
        } else {
            self.note("victory flourish asked for, but no fight is on".into());
        }
    }

    // ---- Fights ----

    fn start_fight(&mut self, forced: bool) {
        let Some(music) = self.battle.music.clone() else {
            self.note("a fight, but the level has no battle music".into());
            return;
        };
        // A "megamulti" holds battle groups; a battle group holds loops.
        let lib = self.lib.clone();
        let multi = match lib.group(&music) {
            Some(g) if g.members.iter().any(|m| lib.group(m).is_some()) => {
                lib.pick_member(g, &mut self.rng)
            }
            Some(_) => Some(music.clone()),
            None => None,
        };
        self.note(format!(
            "fight starts{} ({music} -> {})",
            if forced { " (script)" } else { "" },
            multi.as_deref().unwrap_or("?")
        ));
        self.fight = Some(Fight {
            multi: multi.clone(),
            forced,
            last_in_range: self.time,
        });
        self.stop_overlays();
        if self.down {
            return;
        }
        if let Some(p) = multi.as_deref().and_then(|m| self.piece(m)) {
            self.main = Main::Battle;
            self.play(Layer::Main, p, None);
        }
    }

    fn end_fight(&mut self, won: bool) {
        let Some(f) = self.fight.take() else {
            return;
        };
        self.note(format!("fight over{}", if won { ", won" } else { "" }));
        if won {
            let flourish = f
                .multi
                .as_deref()
                .and_then(|m| self.lib.group(m))
                .and_then(|g| g.won.clone());
            if let Some(w) = flourish {
                self.play_episode(&w, None);
            }
        }
        if !self.down {
            self.resume_state(None);
        }
        if let Some(o) = &mut self.overlays {
            if o.next_at.is_none() {
                o.next_at = Some(self.time + o.min);
            }
        }
    }

    fn check_fight(&mut self, threats: &[Threat], player: Option<[f32; 3]>) {
        let b = &self.battle;
        let counts = |radius: f32| {
            threats
                .iter()
                .filter(|t| match b.criteria {
                    Criteria::Attack => t.engaged,
                    Criteria::Proximity => player.is_some_and(|p| dist(p, t.at) <= radius),
                    Criteria::AttackAndProximity => {
                        t.engaged && player.is_some_and(|p| dist(p, t.at) <= radius)
                    }
                })
                .count() as u32
        };
        let (near, far) = (counts(b.inner), counts(b.outer));
        let threshold = b.threshold.max(1);
        match &mut self.fight {
            None => {
                if self.battle_enabled && near >= threshold && self.battle.music.is_some() {
                    self.note(format!("{near} enemies within {:.0} uu", self.battle.inner));
                    self.start_fight(false);
                }
            }
            Some(f) => {
                if far > 0 {
                    f.last_in_range = self.time;
                } else if !f.forced && self.time - f.last_in_range >= self.tuning.battle_end_seconds
                {
                    self.note(format!("no enemies within {:.0} uu", self.battle.outer));
                    self.end_fight(true);
                }
            }
        }
    }

    // ---- Time ----

    /// Advance by `dt` seconds. `threats` are the living enemies,
    /// `player` where the player is (Unreal units), `down` whether the
    /// player is down waiting for a squad mate.
    pub fn update(&mut self, dt: f32, threats: &[Threat], player: Option<[f32; 3]>, down: bool) {
        self.time += dt;
        if down != self.down {
            self.down = down;
            if down {
                let lost = self
                    .battle
                    .music
                    .as_deref()
                    .and_then(|m| self.lib.group(m))
                    .and_then(|g| g.lost.clone())
                    .unwrap_or_else(|| self.tuning.player_down.clone());
                self.note(format!("player down: {lost}"));
                self.stop_overlays();
                if let Some(p) = self.piece(&lost) {
                    self.main = Main::Down;
                    self.play(Layer::Main, p, None);
                }
            } else {
                self.note("player up again".into());
                let multi = self.fight.as_ref().and_then(|f| f.multi.clone());
                match multi.and_then(|m| self.piece(&m)) {
                    Some(p) => {
                        self.main = Main::Battle;
                        self.play(Layer::Main, p, None);
                    }
                    None => self.resume_state(None),
                }
            }
        }
        if self.time >= self.next_check {
            self.next_check = self.time + self.tuning.battle_check_seconds;
            self.check_fight(threats, player);
        }
        // Overlays, between fights.
        if self.fight.is_none() && !self.down {
            let due = self
                .overlays
                .as_ref()
                .is_some_and(|o| o.next_at.is_some_and(|t| self.time >= t));
            if due {
                let o = self.overlays.as_mut().unwrap();
                o.next_at = None;
                let name = o.first.take().unwrap_or_else(|| o.group.clone());
                match self.piece(&name) {
                    Some(p) => self.play(Layer::Overlay, p, None),
                    None => self.finished(Layer::Overlay),
                }
            }
        }
    }

    /// A one-shot piece on `layer` came to its end (or couldn't be
    /// played).
    pub fn finished(&mut self, layer: Layer) {
        if let Some(p) = self.playing[slot(layer)].take() {
            self.note(format!("{}: {} ended", layer.name(), p.name));
        }
        match layer {
            Layer::Overlay => {
                if let Some(o) = &mut self.overlays {
                    let wait = self.rng.range(o.min, o.max);
                    o.next_at = Some(self.time + wait);
                }
            }
            Layer::Main => {
                // A one-shot state piece doesn't come back after a fight.
                if self.main == Main::State {
                    self.state = None;
                    self.main = Main::Silent;
                }
            }
            Layer::Episode => {}
        }
    }
}
