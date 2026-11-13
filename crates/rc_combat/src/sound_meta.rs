//! What the game itself says about each sound we play, kept by the sound's
//! Bevy handle so the sound engine (`rc_scripts_plugin::engine`) can play
//! any sound the original's way, whichever part of the port started it.
//!
//! Every sound read through [`crate::read_sounds`] is registered here with
//! its own `Sound` properties (`Looping`, `Streaming`, `BattleContext`,
//! `MinCampaign`, `MaxCampaign`, `Likelihood`), its length, and the
//! `AudioParams` it names (or the class's default object when it names
//! none, as the game's `USound::GetParams` does). The names, defaults and
//! meanings come from the game's own `engine.u` (the class text and
//! default values) and its `Engine.dll` (read on the user's PC for
//! reference only; nothing from it is copied here).
//!
//! A `SoundMultiple` (a group) picks one member each time it plays; the
//! rules are [`pick_member`], read from `USoundMultiple`'s pick in
//! `Engine.dll` (2026-11-13).

use bevy::asset::AssetId;
use bevy::audio::AudioSource;
use rc_game_core::GamePaths;
use rc_package::properties::{self, Property, Value};
use rc_package::{Package, Reader};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// `AudioParams.Category` (`EAudioGroup`).
pub const MUSIC: u8 = 0;
pub const VOICE: u8 = 1;
pub const SFX: u8 = 2;

/// One sound's facts, as the game's sound engine uses them.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundMeta {
    /// The sound's name in its package (`CA1G1701`), for logs.
    pub name: String,
    /// `Params`: `package.name` in lower case, `None` when it names none
    /// (it then plays with the `AudioParams` class defaults below).
    pub params_name: Option<String>,
    /// `Category`: [`MUSIC`], [`VOICE`] or [`SFX`]. The class default is
    /// music.
    pub category: u8,
    /// `bDisableVoiceCues`: while it plays, battle chatter is held back and
    /// the other sounds are lowered ("ducking").
    pub voice_cues: bool,
    /// `ActorTag`: heard from the actor with this tag instead.
    pub actor_tag: Option<String>,
    pub min_volume: f32,
    pub max_volume: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    /// `NonPositional`: heard the same everywhere.
    pub non_positional: bool,
    /// `Radius` (class default 2000), Unreal units.
    pub radius: f32,
    /// `bDisable3DOcclusion`: the Creative (EAX) wall muffling is skipped.
    pub no_occlusion: bool,
    /// `bDisable3DFalloff`: no quieter with distance.
    pub no_falloff: bool,
    /// `Sound.Looping`: the game clears a caller's request to loop unless
    /// this is set.
    pub looping: bool,
    /// `Sound.Streaming`: read from disk while it plays.
    pub streaming: bool,
    /// Seconds (0 when unknown).
    pub seconds: f32,
    /// `BattleContext`: 0 any time, 1 only in battle, 2 only out of battle.
    pub battle_context: u8,
    pub min_campaign: u8,
    pub max_campaign: u8,
    /// `Likelihood`: how often a group picks it (class default 1).
    pub likelihood: f32,
}

impl Default for SoundMeta {
    /// A sound with no `Params` and the `Sound` class defaults.
    fn default() -> Self {
        SoundMeta {
            name: String::new(),
            params_name: None,
            category: MUSIC,
            voice_cues: false,
            actor_tag: None,
            min_volume: 1.0,
            max_volume: 1.0,
            min_pitch: 1.0,
            max_pitch: 1.0,
            non_positional: false,
            radius: 2000.0,
            no_occlusion: false,
            no_falloff: false,
            looping: false,
            streaming: false,
            seconds: 0.0,
            battle_context: 0,
            min_campaign: 0,
            max_campaign: 255,
            likelihood: 1.0,
        }
    }
}

/// An `AudioParams` object's values (the PC ones; the Xbox ones are never
/// read by the PC game).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamsFacts {
    pub category: u8,
    pub voice_cues: bool,
    pub actor_tag: Option<String>,
    pub min_volume: f32,
    pub max_volume: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
    pub non_positional: bool,
    pub radius: f32,
    pub no_occlusion: bool,
    pub no_falloff: bool,
}

impl Default for ParamsFacts {
    /// `engine.u`'s `AudioParams` defaults: volumes and pitches 1, `Radius`
    /// 2000, everything else 0 or false.
    fn default() -> Self {
        let d = SoundMeta::default();
        ParamsFacts {
            category: d.category,
            voice_cues: false,
            actor_tag: None,
            min_volume: 1.0,
            max_volume: 1.0,
            min_pitch: 1.0,
            max_pitch: 1.0,
            non_positional: false,
            radius: d.radius,
            no_occlusion: false,
            no_falloff: false,
        }
    }
}

fn flag(pkg: &Package, props: &[Property], n: &str) -> bool {
    matches!(
        properties::find(pkg, props, n).map(|p| &p.value),
        Some(Value::Bool(true))
    )
}

fn float(pkg: &Package, props: &[Property], n: &str) -> Option<f32> {
    match properties::find(pkg, props, n).map(|p| &p.value) {
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    }
}

fn byte(pkg: &Package, props: &[Property], n: &str) -> Option<u8> {
    match properties::find(pkg, props, n).map(|p| &p.value) {
        Some(Value::Byte(b)) => Some(*b),
        _ => None,
    }
}

/// Read an `AudioParams` object's properties.
pub fn params_facts(pkg: &Package, props: &[Property]) -> ParamsFacts {
    let d = ParamsFacts::default();
    ParamsFacts {
        category: byte(pkg, props, "Category").unwrap_or(d.category),
        voice_cues: flag(pkg, props, "bDisableVoiceCues"),
        actor_tag: match properties::find(pkg, props, "ActorTag").map(|p| &p.value) {
            Some(Value::Name(n)) => Some(pkg.name(*n).to_string())
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("None")),
            _ => None,
        },
        min_volume: float(pkg, props, "MinVolume").unwrap_or(d.min_volume),
        max_volume: float(pkg, props, "MaxVolume").unwrap_or(d.max_volume),
        min_pitch: float(pkg, props, "MinPitch").unwrap_or(d.min_pitch),
        max_pitch: float(pkg, props, "MaxPitch").unwrap_or(d.max_pitch),
        non_positional: flag(pkg, props, "NonPositional"),
        radius: float(pkg, props, "Radius").unwrap_or(d.radius),
        no_occlusion: flag(pkg, props, "bDisable3DOcclusion"),
        no_falloff: flag(pkg, props, "bDisable3DFalloff"),
    }
}

/// An export's tagged properties.
pub fn export_properties(pkg: &Package, data: &[u8], index: usize) -> Option<Vec<Property>> {
    let e = pkg.exports.get(index)?;
    let start = e.serial_offset as usize;
    let bytes = data.get(start..start + e.serial_size as usize)?;
    properties::read_properties(pkg, &mut Reader::new(bytes)).ok()
}

/// The `AudioParams` objects of one package, by lower-case name.
type ParamsPackage = HashMap<String, ParamsFacts>;

fn params_cache() -> &'static Mutex<HashMap<String, Arc<ParamsPackage>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<ParamsPackage>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Every `AudioParams` in a sound package (`params_vox`, `params_sfx`),
/// read once and kept.
pub fn params_package(paths: &GamePaths, package: &str) -> Arc<ParamsPackage> {
    let key = package.to_ascii_lowercase();
    if let Some(p) = params_cache().lock().unwrap().get(&key) {
        return p.clone();
    }
    let mut out = ParamsPackage::new();
    let path = paths.game_file(&format!("GameData/Sounds/{package}.uax"));
    if let Ok(data) = std::fs::read(&path) {
        if let Ok(pkg) = Package::parse(&data) {
            for (i, e) in pkg.exports.iter().enumerate() {
                if pkg.export_class_name(e) != "AudioParams" {
                    continue;
                }
                if let Some(props) = export_properties(&pkg, &data, i) {
                    out.insert(
                        pkg.name(e.object_name).to_ascii_lowercase(),
                        params_facts(&pkg, &props),
                    );
                }
            }
        }
    }
    let out = Arc::new(out);
    params_cache().lock().unwrap().insert(key, out.clone());
    out
}

/// The `AudioParams` a sound names (`package.name`, lower case), or the
/// class defaults when it names none or the object can't be read.
pub fn resolve_params(paths: &GamePaths, name: Option<&str>) -> ParamsFacts {
    name.and_then(|n| n.split_once('.'))
        .and_then(|(p, n)| params_package(paths, p).get(n).cloned())
        .unwrap_or_default()
}

/// A sound's facts from its `Sound` properties, its `Params` and its
/// length.
pub fn meta_of(
    pkg: &Package,
    props: &[Property],
    name: &str,
    params_name: Option<String>,
    params: &ParamsFacts,
    seconds: f32,
) -> SoundMeta {
    let d = SoundMeta::default();
    SoundMeta {
        name: name.to_string(),
        params_name,
        category: params.category,
        voice_cues: params.voice_cues,
        actor_tag: params.actor_tag.clone(),
        min_volume: params.min_volume,
        max_volume: params.max_volume,
        min_pitch: params.min_pitch,
        max_pitch: params.max_pitch,
        non_positional: params.non_positional,
        radius: params.radius,
        no_occlusion: params.no_occlusion,
        no_falloff: params.no_falloff,
        looping: flag(pkg, props, "Looping"),
        streaming: flag(pkg, props, "Streaming"),
        seconds,
        battle_context: byte(pkg, props, "BattleContext").unwrap_or(d.battle_context),
        min_campaign: byte(pkg, props, "MinCampaign").unwrap_or(d.min_campaign),
        max_campaign: byte(pkg, props, "MaxCampaign").unwrap_or(d.max_campaign),
        likelihood: float(pkg, props, "Likelihood").unwrap_or(d.likelihood),
    }
}

type Registry = RwLock<HashMap<AssetId<AudioSource>, Arc<SoundMeta>>>;

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(Default::default)
}

/// Remember what the game says about the sound behind `id`.
pub fn register(id: AssetId<AudioSource>, meta: SoundMeta) {
    registry().write().unwrap().insert(id, Arc::new(meta));
}

/// What the game says about the sound behind `id`, if it was read from
/// the game's packages.
pub fn meta(id: AssetId<AudioSource>) -> Option<Arc<SoundMeta>> {
    registry().read().unwrap().get(&id).cloned()
}

/// The level's `LevelInfo.Campaign` (`ELevelCampaign`: 0 Geonosis, 1 the
/// ship, 2 Kashyyyk), which group picks compare with `MinCampaign` and
/// `MaxCampaign`. Set when a level loads.
static CAMPAIGN: AtomicU8 = AtomicU8::new(0);

pub fn set_campaign(c: u8) {
    CAMPAIGN.store(c, Ordering::Relaxed);
}

pub fn campaign() -> u8 {
    CAMPAIGN.load(Ordering::Relaxed)
}

/// A random number from 0 to 1 for the sound engine's picks (the game's
/// `appSRand`). One shared generator, like the game's.
pub fn srand() -> f32 {
    static STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let mut x = STATE.load(Ordering::Relaxed);
    loop {
        let mut y = x;
        y ^= y << 13;
        y ^= y >> 7;
        y ^= y << 17;
        match STATE.compare_exchange_weak(x, y, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (y >> 40) as f32 / (1u64 << 24) as f32,
            Err(now) => x = now,
        }
    }
}

/// A group's own pick settings and its state between plays
/// (`SoundMultiple.SelectionStyle`, `FirstSound`, and the transient
/// `bFirstTime` and `WhichSound`).
#[derive(Debug, Default)]
pub struct GroupPick {
    /// `SelectionStyle = SS_InOrder`: the members play one after another.
    pub in_order: bool,
    /// `FirstSound`: played once, the first time the group plays.
    pub first: Option<bevy::prelude::Handle<AudioSource>>,
    state: Mutex<GroupState>,
}

#[derive(Debug)]
struct GroupState {
    first_time: bool,
    which: usize,
}

impl Default for GroupState {
    fn default() -> Self {
        GroupState {
            first_time: true,
            which: 0,
        }
    }
}

impl GroupPick {
    pub fn new(in_order: bool, first: Option<bevy::prelude::Handle<AudioSource>>) -> Self {
        GroupPick {
            in_order,
            first,
            state: Mutex::default(),
        }
    }

    /// Start over: the game's `bFirstTime` and `WhichSound` are transient,
    /// so they start fresh each time the group's package is loaded (with
    /// each level).
    pub fn reset(&self) {
        *self.state.lock().unwrap() = GroupState::default();
    }
}

/// What a group member's filter needs: `BattleContext`, `MinCampaign`,
/// `MaxCampaign` and `Likelihood`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberWeight {
    pub battle_context: u8,
    pub min_campaign: u8,
    pub max_campaign: u8,
    pub likelihood: f32,
}

impl MemberWeight {
    pub fn of(m: &SoundMeta) -> Self {
        MemberWeight {
            battle_context: m.battle_context,
            min_campaign: m.min_campaign,
            max_campaign: m.max_campaign,
            likelihood: m.likelihood,
        }
    }

    /// Whether the member may play now: in battle a "not in battle"
    /// member (2) is skipped, out of battle a "battle only" one (1); and
    /// the level's campaign must be within its range.
    fn allowed(&self, battle: bool, campaign: u8) -> bool {
        let context_ok = if battle {
            self.battle_context != 2
        } else {
            self.battle_context != 1
        };
        context_ok && campaign >= self.min_campaign && campaign <= self.max_campaign
    }
}

impl Default for MemberWeight {
    fn default() -> Self {
        MemberWeight::of(&SoundMeta::default())
    }
}

/// What a group plays this time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    /// Its `FirstSound`.
    First,
    /// Member number `n`.
    Member(usize),
}

/// The game's group pick (`USoundMultiple`, Engine.dll):
/// - `SS_InOrder`: the next member, round and round;
/// - else, the first time, its `FirstSound` when it has one;
/// - else a weighted random pick: the `Likelihood`s of the members allowed
///   now (see [`MemberWeight`]) are added up, a random point `r` below the
///   total is chosen (`rand` is 0 to 1), and the first allowed member whose
///   running total passes `r` plays. When none does, the first member
///   plays.
pub fn pick_member(
    group: &GroupPick,
    members: &[MemberWeight],
    battle: bool,
    campaign: u8,
    rand: f32,
) -> Option<Picked> {
    if members.is_empty() {
        return None;
    }
    let mut s = group.state.lock().unwrap();
    if group.in_order {
        let i = s.which % members.len();
        s.which = if i + 1 == members.len() { 0 } else { i + 1 };
        return Some(Picked::Member(i));
    }
    if group.first.is_some() && s.first_time {
        s.first_time = false;
        return Some(Picked::First);
    }
    let total: f32 = members
        .iter()
        .filter(|m| m.allowed(battle, campaign))
        .map(|m| m.likelihood)
        .sum();
    let r = rand * total;
    let mut sum = 0.0f32;
    for (i, m) in members.iter().enumerate() {
        if !m.allowed(battle, campaign) {
            continue;
        }
        sum += m.likelihood;
        if sum > r {
            return Some(Picked::Member(i));
        }
    }
    Some(Picked::Member(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(ctx: u8, min: u8, max: u8, l: f32) -> MemberWeight {
        MemberWeight {
            battle_context: ctx,
            min_campaign: min,
            max_campaign: max,
            likelihood: l,
        }
    }

    #[test]
    fn in_order_goes_round() {
        let g = GroupPick::new(true, None);
        let m = [MemberWeight::default(); 3];
        let got: Vec<_> = (0..5)
            .map(|_| pick_member(&g, &m, false, 0, 0.5).unwrap())
            .collect();
        use Picked::Member as M;
        assert_eq!(got, vec![M(0), M(1), M(2), M(0), M(1)]);
    }

    #[test]
    fn first_sound_plays_once() {
        let g = GroupPick::new(false, Some(Default::default()));
        let m = [MemberWeight::default(); 2];
        assert_eq!(pick_member(&g, &m, false, 0, 0.9), Some(Picked::First));
        assert_eq!(pick_member(&g, &m, false, 0, 0.9), Some(Picked::Member(1)));
        g.reset();
        assert_eq!(pick_member(&g, &m, false, 0, 0.9), Some(Picked::First));
    }

    #[test]
    fn weights_and_filters() {
        let g = GroupPick::new(false, None);
        // Battle-only, any time (weight 3), out-of-battle only.
        let m = [w(1, 0, 255, 1.0), w(0, 0, 255, 3.0), w(2, 0, 255, 1.0)];
        // Out of battle the total is 4: below 3 member 1, else member 2.
        assert_eq!(pick_member(&g, &m, false, 0, 0.5), Some(Picked::Member(1)));
        assert_eq!(pick_member(&g, &m, false, 0, 0.9), Some(Picked::Member(2)));
        // In battle: member 0 first (1 of 4).
        assert_eq!(pick_member(&g, &m, true, 0, 0.1), Some(Picked::Member(0)));
        // A campaign outside every range falls back to member 0.
        let m = [w(0, 1, 1, 1.0), w(0, 2, 2, 1.0)];
        assert_eq!(pick_member(&g, &m, false, 0, 0.5), Some(Picked::Member(0)));
        assert_eq!(pick_member(&g, &m, false, 2, 0.5), Some(Picked::Member(1)));
    }

    #[test]
    fn srand_stays_in_range() {
        for _ in 0..1000 {
            let r = srand();
            assert!((0.0..1.0).contains(&r));
        }
    }
}
