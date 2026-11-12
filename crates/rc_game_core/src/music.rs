//! How other parts talk to the music (`rc_game`'s `music.rs`).
//!
//! - The level scripts send [`MusicCue`]s: the map's own music actions
//!   (`ACTION_SetImuseState`, `ACTION_PlayMusic`, the battle engine ones).
//! - The enemies keep [`MusicThreats`] up to date: where each living enemy
//!   is and whether it is fighting. The music decides from these, with the
//!   map's own battle settings, when a fight's music starts and ends.

use bevy::prelude::{Message, Resource};

/// Fade times a script gave (`OverrideFadeDefaults`). Shapes are the
/// engine's `EMusicFadeType` numbers (0 linear, 1 sine, 2 log, 3 inverse
/// log).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MusicFades {
    pub fade_in: f32,
    pub fade_in_shape: u8,
    pub fade_out: f32,
    pub fade_out_shape: u8,
}

/// A music action from a level script. Names are the game's own sound
/// object names (`musGEO_suspensestate03_lp`, `GEO_battle_megamulti`).
#[derive(Message, Debug, Clone, PartialEq)]
pub enum MusicCue {
    /// `ACTION_SetImuseState`: new room music; `None` for silence.
    SetState {
        music: Option<String>,
        fades: Option<MusicFades>,
    },
    /// `ACTION_PlayMusic`: a one-shot piece on top.
    PlayEpisode {
        music: String,
        fades: Option<MusicFades>,
    },
    /// `ACTION_EnableBattleEngine`.
    EnableBattle(bool),
    /// `ACTION_SetBattleEngineParams` (criteria as the engine numbers it:
    /// 0 attack, 1 proximity, 2 both).
    BattleParams {
        criteria: u8,
        threshold: u32,
        inner: f32,
        outer: f32,
    },
    /// `ACTION_SetBattleMusicMulti`.
    BattleMusic(Option<String>),
    /// `ACTION_ForceBattleMusic`.
    ForceBattle,
    /// `ACTION_ForceVictoryFlourish`.
    ForceVictory,
}

/// One living enemy, as the music sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MusicThreat {
    /// Stays the same while it lives (a fight remembers its enemies).
    pub id: u64,
    /// Its centre, in Unreal units.
    pub at: [f32; 3],
    /// Half-size of its collision box (radius, radius, height).
    pub extent: [f32; 3],
    /// It has seen or heard the squad.
    pub engaged: bool,
}

/// Every living enemy. Written by the enemies part each frame of play.
#[derive(Resource, Debug, Clone, Default)]
pub struct MusicThreats(pub Vec<MusicThreat>);

/// One enemy factory (a map's `PawnFactory`), for battle volumes: a fight
/// inside a `BattleVolume` lasts until every factory in it is exhausted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MusicFactory {
    /// Where it stands, in Unreal units.
    pub at: [f32; 3],
    /// `Factory.IsExhausted` (CTGame.u): never for one that makes enemies
    /// forever; yes once switched off after having been on, or once it has
    /// made its whole capacity.
    pub exhausted: bool,
}

/// Every enemy factory on the map. Written by the enemies part.
#[derive(Resource, Debug, Clone, Default)]
pub struct MusicFactories(pub Vec<MusicFactory>);
