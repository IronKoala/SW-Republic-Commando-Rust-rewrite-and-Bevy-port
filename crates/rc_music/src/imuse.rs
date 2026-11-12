//! What plays when: the game's music rules, called iMuse in its code
//! (`IMuseStateInfo`, `IMuseTrigger`, `ACTION_SetImuseState`).
//!
//! The rules come from the game's own data and script classes. When fights
//! start and end (the "battle engine", with its battle volumes) is native
//! code in `Engine.dll`; it was read from the machine code and is copied
//! here step by step (see "Battle engine" below and `docs/DESIGN.md`
//! under "Music"). What is still our own choice is marked "our choice".
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
//! - **episode**: one-shot pieces (`ACTION_PlayMusic`, the victory
//!   flourish at the end of a fight, the "lost" music). As in the engine,
//!   an episode fades out the other music, ends a fight without a
//!   flourish, is ignored while another episode plays, and when it ends
//!   the room music comes back.
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
#[derive(Debug, Clone, PartialEq)]
pub struct Threat {
    /// Stays the same while it lives (the engine keeps the pawns
    /// themselves in `mBattleBaddies`).
    pub id: u64,
    /// Where it is, in Unreal units.
    pub at: [f32; 3],
    /// Fighting: it has seen or heard the squad.
    pub engaged: bool,
    /// The battle volumes it is in (indexes into the map's list).
    pub zones: Vec<usize>,
}

/// The map's battle volumes as the director needs them each check. The
/// game fills this from the map's `BattleVolume`s and the enemy spawners.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Zones {
    /// The battle volumes the player is in, in map order. The engine takes
    /// the first `EngineBattleVolume` in the player's touching list.
    pub player_in: Vec<usize>,
    /// For each battle volume: every pawn factory inside it is exhausted
    /// (`BattleVolume.AreFactoriesExhausted`, CTGame.u). True for a volume
    /// with none.
    pub exhausted: Vec<bool>,
}

/// Fade times a script gives (`OverrideFadeDefaults`): `fade_in` for the
/// new piece, `fade_out` for the one it replaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeOverride {
    pub fade_in: Fade,
    pub fade_out: Fade,
}

/// The battle engine's own numbers, from `Engine.dll` (they are in its
/// code, not in any data file).
pub mod engine {
    /// The fight check runs when the whole-second clock reaches the next
    /// check time, which is then set one second on
    /// (`mNextBattleUpdateTime = int(appSeconds) + 1`).
    pub const CHECK_SECONDS: f32 = 1.0;
    /// A fight whose enemies are gone from range, but not all dead, ends
    /// this many seconds after the first check that found none.
    pub const END_DELAY: f32 = 2.0;
    /// The battle loop fades out over this when a fight ends without a
    /// flourish (`SetFadeOverrides(5.0, 0, 5.0, 0)`: linear).
    pub const CALM_FADE: f32 = 5.0;
}

/// Our choices where the game's data says nothing (see `data/music/music.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct Tuning {
    /// Fade for the overlay layer when a fight or a script stops it.
    pub overlay_stop_fade: f32,
    /// The "player down" piece when the battle music names none.
    pub player_down: String,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
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
    /// The battle group picked for this fight (`CurrentBattleTrio`: holds
    /// the won and lost music).
    multi: Option<String>,
    /// The battle loop picked from it (`mActiveBattleCueID`).
    looped: Option<String>,
}

/// How a fight ends (the engine's argument to its end-of-battle code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightEnd {
    /// No enemy left in range, but not all of the fight's enemies dead:
    /// the battle loop fades out and the room music comes back.
    Calm,
    /// Every enemy of the fight dead (and the battle volume's factories
    /// spent): the group's "won" flourish.
    Won,
    /// The player died: the group's "lost" music.
    Lost,
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
    /// The enemies taking part (`mBattleBaddies`).
    baddies: Vec<u64>,
    /// The battle volume of the fight (`BattleZone`).
    zone: Option<usize>,
    zones: Zones,
    player_dead: bool,
    /// The last update's enemies and player, for script actions that run
    /// the fight check between updates.
    last_threats: Vec<Threat>,
    last_player: Option<[f32; 3]>,
    /// When a fight without enemies in range ends (0: not set). Like the
    /// engine's, it is only cleared by a check that finds enemies in range.
    end_at: f32,
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
            baddies: Vec::new(),
            zone: None,
            zones: Zones::default(),
            player_dead: false,
            last_threats: Vec::new(),
            last_player: None,
            end_at: 0.0,
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
        if self.fight.is_some() || self.down || self.episode_playing() {
            self.note("(waits for the fight or episode to end)".into());
            return;
        }
        self.resume_state(fades);
    }

    fn episode_playing(&self) -> bool {
        self.playing(Layer::Episode).is_some()
    }

    /// `ACTION_PlayMusic` (and a `MusicTrigger`'s episode). Engine
    /// (`UAudioSubsystem::PlayMusicEpisode`): ignored while another episode
    /// plays; otherwise it plays, every other piece fades out, overlays
    /// stop and a fight ends without a flourish. The room music comes back
    /// when it ends ([`Imuse::finished`]).
    pub fn play_episode(&mut self, name: &str, fades: Option<FadeOverride>) {
        self.episode(name, fades, false);
    }

    /// The engine's episode start. `clear_state`: the room music becomes
    /// silence (the "lost" music does this).
    fn episode(&mut self, name: &str, fades: Option<FadeOverride>, clear_state: bool) -> bool {
        if let Some(e) = self.playing(Layer::Episode) {
            self.note(format!("{name}: skipped, episode {} still plays", e.name));
            return false;
        }
        let Some(p) = self.piece(name) else {
            return false;
        };
        self.play(Layer::Episode, p, fades);
        self.stop(Layer::Main, None);
        self.main = Main::Silent;
        if clear_state {
            self.state = None;
        }
        self.stop_overlays();
        if self.fight.take().is_some() {
            self.note("fight over (an episode took over)".into());
        }
        true
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
    /// without a flourish (engine: end of battle, "calm").
    pub fn enable_battle(&mut self, on: bool) {
        self.note(format!("battle engine {}", if on { "on" } else { "off" }));
        if !on {
            self.end_fight(FightEnd::Calm);
        }
        self.battle_enabled = on;
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

    /// The battle volumes, as they are now (see [`Zones`]).
    pub fn set_zones(&mut self, zones: Zones) {
        self.zones = zones;
    }

    /// The player is dead (not just down).
    pub fn set_player_dead(&mut self, dead: bool) {
        self.player_dead = dead;
    }

    /// `ACTION_ForceBattleMusic` (engine: `TryForceBattleZone`). Only
    /// works inside a battle volume: the volume the player is in becomes
    /// the fight's, and the fight starts if an enemy is in it or one of
    /// its factories still has enemies to make. Otherwise nothing happens.
    pub fn force_battle(&mut self) {
        self.zone = self.zones.player_in.first().copied();
        let Some(z) = self.zone else {
            self.note("script forces battle music, but the player is in no battle volume".into());
            return;
        };
        self.note(format!("script forces battle music in battle volume {z}"));
        self.try_start(true);
        if self.fight.is_none() {
            self.zone = None;
        }
    }

    /// `ACTION_ForceVictoryFlourish` (engine: `TryForceBattleVictory`):
    /// the fight ends with its flourish.
    pub fn force_victory(&mut self) {
        if self.fight.is_some() {
            self.end_fight(FightEnd::Won);
        } else {
            self.note("victory flourish asked for, but no fight is on".into());
        }
    }

    // ---- Battle engine (Engine.dll) ----
    //
    // Copied from the machine code of the engine's iMuse functions (the
    // fight check at 0x103ac860, the timed update at 0x103acda0, start at
    // 0x103acc50, end at 0x103abbf0, the battle volume lookup at
    // 0x103abb70 and the "all dead" test at 0x103ab6f0). Only the names
    // are ours.

    /// The fight check (engine 0x103ac860). `forced` counts as being in a
    /// fight. Adds the enemies that count to the fight's list.
    fn check(&mut self, threats: &[Threat], player: Option<[f32; 3]>, forced: bool) -> bool {
        let in_fight = self.fight.is_some() || forced;
        let b = self.battle.clone();
        let radius = if in_fight { b.outer } else { b.inner };
        let zone = self.zone;
        let player_in_zone = zone.is_some_and(|z| self.zones.player_in.contains(&z));
        let (mut engaged_n, mut near_n) = (0u32, 0u32);
        for t in threats {
            let t_in_zone = zone.is_some_and(|z| t.zones.contains(&z));
            if zone.is_some() && in_fight && player_in_zone {
                // In a fight inside its battle volume: every enemy in the
                // volume counts, near or not, fighting or not.
                if t_in_zone {
                    engaged_n += 1;
                    near_n += 1;
                    self.add_baddie(t.id);
                }
                continue;
            }
            if zone.is_some() && !in_fight && !t_in_zone {
                continue;
            }
            let mut engaged = false;
            if matches!(b.criteria, Criteria::Attack | Criteria::AttackAndProximity) && t.engaged {
                engaged_n += 1;
                engaged = true;
            }
            let mut near = false;
            let looks = b.criteria == Criteria::Proximity
                || (b.criteria == Criteria::AttackAndProximity && engaged);
            if looks {
                if let Some(p) = player {
                    let d2 = (t.at[0] - p[0]).powi(2)
                        + (t.at[1] - p[1]).powi(2)
                        + (t.at[2] - p[2]).powi(2);
                    if radius * radius > d2 {
                        near_n += 1;
                        near = true;
                    }
                }
            }
            if (b.criteria == Criteria::Attack && engaged)
                || (b.criteria == Criteria::Proximity && near)
                || (engaged && near)
            {
                self.add_baddie(t.id);
            }
        }
        // In a fight, one enemy keeps it going.
        let threshold = if in_fight { 1 } else { b.threshold };
        if player.is_none() || self.player_dead {
            return false;
        }
        if let (Some(z), true, true) = (zone, in_fight, player_in_zone) {
            return engaged_n > 0 || !self.zone_exhausted(z);
        }
        match b.criteria {
            Criteria::Attack => engaged_n >= threshold,
            Criteria::Proximity => near_n >= threshold,
            Criteria::AttackAndProximity => engaged_n >= threshold && near_n >= threshold,
        }
    }

    fn add_baddie(&mut self, id: u64) {
        if !self.baddies.contains(&id) {
            self.baddies.push(id);
        }
    }

    fn zone_exhausted(&self, z: usize) -> bool {
        self.zones.exhausted.get(z).copied().unwrap_or(true)
    }

    /// Start a fight (engine 0x103acc50): needs battle music, no episode
    /// playing and no fight on. If the player is in a battle volume, that
    /// becomes the fight's volume and the check runs again counting only
    /// what is in it.
    fn try_start_with(&mut self, threats: &[Threat], player: Option<[f32; 3]>, forced: bool) {
        if self.battle.music.is_none() || self.episode_playing() || self.fight.is_some() {
            return;
        }
        self.zone = self.zones.player_in.first().copied();
        if let Some(z) = self.zone {
            self.baddies.clear();
            if !self.check(threats, player, forced) {
                self.zone = None;
                return;
            }
            self.note(format!("fight in battle volume {z}"));
        }
        self.start_fight(forced);
    }

    fn try_start(&mut self, forced: bool) {
        let threats = std::mem::take(&mut self.last_threats);
        let player = self.last_player;
        self.try_start_with(&threats, player, forced);
        self.last_threats = threats;
    }

    fn start_fight(&mut self, forced: bool) {
        let Some(music) = self.battle.music.clone() else {
            return;
        };
        // A "megamulti" holds battle groups; a battle group holds loops
        // (engine: `SelectSound` on the battle music, then on the group).
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
        let looped = multi.as_deref().and_then(|m| self.piece(m));
        self.fight = Some(Fight {
            multi: multi.clone(),
            looped: looped.as_ref().map(|p| p.name.clone()),
        });
        self.stop_overlays();
        if self.down {
            return;
        }
        if let Some(p) = looped {
            self.main = Main::Battle;
            self.play(Layer::Main, p, None);
        }
    }

    /// End the fight (engine 0x103abbf0). Nothing happens while an
    /// episode plays. "Won" and "lost" play the group's music as an
    /// episode when it has one (the room music comes back when it ends;
    /// "lost" also leaves the room music silent); otherwise, and for
    /// "calm", the battle loop fades out over 5 s and the room music
    /// comes back.
    fn end_fight(&mut self, how: FightEnd) {
        let Some(f) = self.fight.clone() else {
            return;
        };
        if self.episode_playing() {
            self.note(format!("fight would end ({how:?}), but an episode plays"));
            return;
        }
        self.note(format!("fight over ({how:?})"));
        let group = f.multi.as_deref().and_then(|m| self.lib.group(m)).cloned();
        let mut how = how;
        match how {
            FightEnd::Won => match group.as_ref().and_then(|g| g.won.clone()) {
                Some(w) if self.episode(&w, None, false) => {}
                _ => how = FightEnd::Calm,
            },
            FightEnd::Lost => match group.as_ref().and_then(|g| g.lost.clone()) {
                Some(l) if self.episode(&l, None, true) => {}
                _ => {
                    self.note("no lost music: the room music becomes silence".into());
                    self.state = None;
                    how = FightEnd::Calm;
                }
            },
            FightEnd::Calm => {}
        }
        if how == FightEnd::Calm {
            self.fight = None;
            if !self.down {
                let room = self.state.clone().and_then(|s| self.piece(&s));
                let calm = Fade {
                    seconds: engine::CALM_FADE,
                    shape: crate::library::FadeShape::Linear,
                };
                match room {
                    Some(p) => {
                        let fades = FadeOverride {
                            fade_in: p.fade_in,
                            fade_out: calm,
                        };
                        self.main = Main::State;
                        self.play(Layer::Main, p, Some(fades));
                    }
                    None => {
                        self.main = Main::Silent;
                        self.stop(Layer::Main, Some(calm));
                    }
                }
            }
            self.restart_overlays();
        }
        self.baddies.clear();
        self.zone = None;
    }

    /// Overlays carry on: the next one after a random wait between the
    /// room's spacings (engine 0x103ab5e0).
    fn restart_overlays(&mut self) {
        if self.fight.is_some() || self.down || self.episode_playing() {
            return;
        }
        if self.playing(Layer::Overlay).is_some() {
            return;
        }
        if let Some(o) = &mut self.overlays {
            let wait = self.rng.range(o.min, o.max);
            o.next_at = Some(self.time + wait);
        }
    }

    /// The timed battle update (engine 0x103acda0), once a second.
    fn battle_tick(&mut self, threats: &[Threat], player: Option<[f32; 3]>) {
        if self.down || !self.battle_enabled {
            return;
        }
        let in_fight = self.fight.is_some();
        if in_fight {
            // Drop the fight's enemies that died.
            self.baddies
                .retain(|id| threats.iter().any(|t| t.id == *id));
        }
        let ok = self.check(threats, player, false);
        if !in_fight {
            if ok {
                self.try_start_with(threats, player, false);
            } else {
                self.baddies.clear();
            }
            return;
        }
        if ok {
            self.end_at = 0.0;
            return;
        }
        // Every enemy of the fight dead, and its battle volume (if any)
        // has nothing left to make: won, at once.
        let all_dead = self.baddies.is_empty() && self.zone.is_none_or(|z| self.zone_exhausted(z));
        let how = if all_dead {
            self.end_fight(FightEnd::Won);
            FightEnd::Won
        } else if player.is_none() || self.player_dead {
            FightEnd::Lost
        } else {
            FightEnd::Calm
        };
        if self.end_at == 0.0 {
            match how {
                FightEnd::Calm => self.end_at = self.time + engine::END_DELAY,
                FightEnd::Won => self.end_at = self.time,
                FightEnd::Lost => {}
            }
        }
        if self.time > self.end_at {
            self.end_fight(how);
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
            } else if self.player_dead {
                // Engine 0x103acba0: the room music becomes silence and a
                // fight ends "lost".
                self.note("player dead".into());
                self.state = None;
                if self.fight.is_some() {
                    self.end_fight(FightEnd::Lost);
                } else {
                    self.resume_state(None);
                }
            } else {
                self.note("player up again".into());
                // Engine 0x103acba0: a fight carries on with its loop if
                // the fight check still holds, else it ends calmly.
                if self.fight.is_some() {
                    if self.check(threats, player, false) {
                        let looped = self.fight.as_ref().and_then(|f| f.looped.clone());
                        if let Some(p) = looped.and_then(|l| self.piece(&l)) {
                            self.main = Main::Battle;
                            self.play(Layer::Main, p, None);
                        }
                    } else {
                        self.end_fight(FightEnd::Calm);
                    }
                } else {
                    self.resume_state(None);
                    self.restart_overlays();
                }
            }
        }
        self.last_threats = threats.to_vec();
        self.last_player = player;
        if self.time >= self.next_check {
            // The engine compares whole seconds: int(now) >= next, then
            // next = int(now) + 1.
            self.next_check = self.time.floor() + engine::CHECK_SECONDS;
            self.battle_tick(threats, player);
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
            Layer::Episode => {
                // Engine 0x103ac040 -> 0x103aba10: the room music comes
                // back and overlays carry on.
                if self.fight.is_none() && !self.down {
                    self.resume_state(None);
                    self.restart_overlays();
                }
            }
        }
    }
}
