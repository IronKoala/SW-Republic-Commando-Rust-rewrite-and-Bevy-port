//! The sound engine: every sound the port plays (weapons, characters,
//! level scripts, map loops, menus) goes through here and is played the
//! way the original's `UAudioSubsystem` (Engine.dll) and its OpenAL
//! driver (`ALAudio.dll`) play it. Read by disassembly on the cloud
//! scratch and the user's PC, 2026-11-13; the decompiled code is never
//! committed and nothing is copied from it.
//!
//! A part plays a sound by spawning Bevy's `AudioPlayer` with
//! `PlaybackSettings` (and `SoundAt` for a place in the level,
//! `rc_game_core::SoundBy` for who plays it). The same frame, before Bevy
//! starts it, the engine:
//!
//! 1. **Rolls its volume and pitch** (`PlayUSound`): volume `MinVolume +
//!    (MaxVolume - MinVolume) × random`, not played below 0.0001, at most 1;
//!    pitch the same from `MinPitch`/`MaxPitch`, kept within 0.5 to 2, and
//!    the OpenAL driver keeps it within 0.7 to 1.2. A loop is played once
//!    unless the sound itself is marked `Looping`.
//! 2. **Checks the listener can hear it** (`APlayerController::CheckHearSound`
//!    and `AController::CanHearSound`): a sound whose `AudioParams` name an
//!    `ActorTag` comes from that actor; a voice from a dead character isn't
//!    played; a placed sound starts only when the listener is closer than
//!    `Radius × 100`, times the rooms share (`0.85 ^ rooms apart²`) when a
//!    wall is between.
//! 3. **Queues it.** Then, every frame (`UAudioSubsystem::Update`), the
//!    queued and the playing sounds are given a priority
//!    ([`priority`]), sorted (the C library's `qsort`, [`sort`]), and the
//!    channels (AUDIO CHANNELS, at most 32) go to the most important:
//!    sounds at or below the cut-off priority stop unless a level script
//!    started them, bullet impacts are skipped when no channel is free,
//!    long faraway muffled effects are skipped, and each queued sound gets
//!    a free channel, takes over a channel of the same speaker, or takes
//!    the least important one ([`find_source`]). Queued sounds that got no
//!    channel are dropped.
//! 4. **Lowers the rest while a radio line plays** ([`Duck`]): while any
//!    sound whose `AudioParams` set `bDisableVoiceCues` plays, music goes
//!    to 0.5, effects and other voices to 0.25, over half a second, and the
//!    characters' battle chatter is held back (`Ducking`).
//! 5. **Sets each sound's loudness** (`FAbstractSource::UpdateVolume`,
//!    `UALAudioSubsystem`): rolled volume × lowering × its fade × its
//!    category's slider (MUSIC, VOICE, SOUND FX), kept within 0.0001 and
//!    0.9999, then the distance rule of [`crate::sounds::Placed`].
//! 6. **EAX** (the AUDIO menu's EAX row, `rc_game_core::EaxOn`): the
//!    muffling of a placed sound behind walls, in millibels
//!    ([`occlusion_at_start`], [`occlusion_each_frame`]), for the room
//!    part to play.

use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings, SpatialAudioSink};
use bevy::audio::{SpatialScale, Volume};
use bevy::prelude::*;
use rc_combat::sound_meta::{self, SoundMeta, MUSIC, SFX, VOICE};
use rc_combat::Dead;
use rc_game_core::units::to_unreal;
use rc_game_core::{
    Ducking, EaxOcclusion, EaxOn, MusicTrack, Play, PlayerCamera, ScriptTags, SoundAt, SoundBy,
    SoundFade, SoundMix,
};
use rc_level_plugin::LevelHearing;
use rc_movement::Vec3 as UVec3;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::sounds::{sound_clock, Placed};

/// Core.dll's `GAudioMaxRadiusMultiplier`.
pub const MAX_RADIUS_MULTIPLIER: f32 = 100.0;

/// The original's most channels (`UALAudioSubsystem::Init` caps its
/// `Channels` at 32).
pub const MAX_CHANNELS: u32 = 32;

/// `PlayUSound` flags, as the game passes them.
pub mod flags {
    pub const LOOP: u32 = 0x2;
    pub const STREAMING: u32 = 0x4;
    pub const NO_CATEGORY: u32 = 0x8;
    pub const MUSIC: u32 = 0x10;
    pub const VOICE: u32 = 0x20;
    pub const SFX: u32 = 0x40;
    pub const NON_POSITIONAL: u32 = 0x100;
    pub const IN_BATTLE: u32 = 0x200;
    pub const NO_OCCLUSION: u32 = 0x800;
    pub const NO_FALLOFF: u32 = 0x1000;
    pub const OWNED: u32 = 0x2000;
}

/// A sound the engine is playing (or has queued this frame).
#[derive(Component, Debug, Clone)]
pub struct EngineSound {
    pub meta: Arc<SoundMeta>,
    pub flags: u32,
    /// The game's source id: bit 0 set for sounds that can't be cut for a
    /// more important one, bits 1 to 3 the kind, the rest the actor or a
    /// unique number.
    pub id: i32,
    /// The rolled volume (`+0x90`).
    pub volume: f32,
    /// The rolled pitch, as played (0.7 to 1.2).
    pub pitch: f32,
    pub priority: f32,
    /// The caller's own volume (from its `PlaybackSettings`).
    pub call_volume: f32,
    /// When it started, engine clock seconds.
    pub started: f64,
    /// EAX occlusion, millibels.
    pub occlusion: i32,
}

impl EngineSound {
    /// Bits 1 to 3 of the id.
    pub fn kind(&self) -> i32 {
        (self.id >> 1) & 7
    }
}

/// How a new sound's volume and pitch come out (`PlayUSound`): `r1` and
/// `r2` are random numbers from 0 to 1. `None`: too quiet to play.
pub fn roll(meta: &SoundMeta, r1: f32, r2: f32) -> Option<(f32, f32)> {
    let v = meta.min_volume + (meta.max_volume - meta.min_volume) * r1;
    if v < 0.0001 {
        return None;
    }
    let v = v.min(1.0);
    let p = (meta.min_pitch + (meta.max_pitch - meta.min_pitch) * r2).clamp(0.5, 2.0);
    // ALAudio's own limit.
    Some((v, p.clamp(0.7, 1.2)))
}

static UNIQUE: AtomicI32 = AtomicI32::new(0);

/// The source id `PlayUSound` gives a sound: the kind from its category
/// (music 0xe, voice 0xa, effects 0), `| 6` for an actor's ambient sound,
/// the actor's number above, bit 0 unless it is a character's own cue.
/// Kinds 0 and 7 get a number of their own instead.
pub fn source_id(category: u8, ambient: bool, actor_index: u32, owned: bool) -> i32 {
    let mut id: i32 = match category {
        MUSIC => 0xe,
        VOICE => 0xa,
        _ => 0,
    };
    if ambient {
        id |= 6;
    }
    id |= (actor_index as i32).wrapping_shl(4);
    if !owned {
        id |= 1;
    }
    let kind = (id >> 1) & 7;
    if kind == 0 || kind == 7 {
        let n = UNIQUE.fetch_sub(1, Ordering::Relaxed) - 1;
        id = n.wrapping_shl(4);
    }
    id
}

/// `FAbstractSource::UpdatePriority`: `distance` from the listener and
/// `radius` (the sound's `Radius`) in Unreal units, `volume` the rolled
/// volume.
pub fn priority(flags: u32, kind: i32, volume: f32, distance: f32, radius: f32) -> f32 {
    let df = if radius == 0.0 || flags & flags::NON_POSITIONAL != 0 {
        1.0
    } else {
        let f = 1.0 - distance / (radius * MAX_RADIUS_MULTIPLIER);
        if f < 0.01 {
            0.01
        } else if f < 1.0 {
            f
        } else {
            1.0
        }
    };
    let vf = if volume < 0.0 || volume == 0.0 {
        0.0
    } else if volume >= 1.0 {
        1.0
    } else {
        0.85 + 0.15 * volume
    };
    base_priority(flags, kind) + vf * df
}

fn base_priority(flags: u32, kind: i32) -> f32 {
    let mut p = 0.0;
    if flags & flags::NON_POSITIONAL != 0 {
        p += 1.0;
    }
    if flags & flags::STREAMING != 0 {
        p += 1.0;
    }
    p += match kind {
        7 => 3.0,
        5 => 2.0,
        0 => 1.0,
        _ => 0.0,
    };
    p
}

/// `UAudioSubsystem::SoundPriority`, which an ambient sound keeps for as
/// long as it plays: as [`priority`] but by the distance squared.
pub fn start_priority(flags: u32, kind: i32, volume: f32, distance: f32, radius: f32) -> f32 {
    let df = if radius == 0.0 || flags & flags::NON_POSITIONAL != 0 {
        1.0
    } else {
        let r = radius * MAX_RADIUS_MULTIPLIER;
        let f = 1.0 - distance * distance / (r * r);
        if f < 0.01 {
            0.01
        } else if f < 1.0 {
            f
        } else {
            1.0
        }
    };
    let v = volume.clamp(0.0, 1.0);
    let vf = if v == 0.0 { 0.0 } else { 0.85 + 0.15 * v };
    base_priority(flags, kind) + vf * df
}

/// Sort indices by priority, most important first, exactly as the game
/// does: the Microsoft C library's `qsort` (small runs of up to 8 by
/// selection, else the middle one as the pivot), so equal priorities end
/// up in the same order as in the original.
pub fn sort(prio: &[f32], order: &mut [usize]) {
    let n = order.len();
    if n < 2 {
        return;
    }
    let p = |i: usize, order: &[usize]| prio[order[i]];
    let mut stack: Vec<(usize, usize)> = vec![(0, n - 1)];
    while let Some((mut lo, mut hi)) = stack.pop() {
        loop {
            let size = hi - lo + 1;
            if size <= 8 {
                // Selection: the least important to the end.
                let mut h = hi;
                while h > lo {
                    let mut min = lo;
                    for q in lo + 1..=h {
                        if p(q, order) < p(min, order) {
                            min = q;
                        }
                    }
                    order.swap(min, h);
                    h -= 1;
                }
                break;
            }
            let mid = lo + size / 2;
            order.swap(mid, lo);
            let pivot = p(lo, order);
            let mut loguy = lo;
            let mut higuy = hi + 1;
            loop {
                loop {
                    loguy += 1;
                    if !(loguy <= hi && p(loguy, order) > pivot) {
                        break;
                    }
                }
                loop {
                    higuy -= 1;
                    if !(higuy > lo && p(higuy, order) <= pivot) {
                        break;
                    }
                }
                if higuy < loguy {
                    break;
                }
                order.swap(loguy, higuy);
            }
            order.swap(lo, higuy);
            let left = (lo + 1 < higuy).then(|| (lo, higuy - 1));
            let right = (loguy < hi).then_some((loguy, hi));
            match (left, right) {
                (Some(l), Some(r)) => {
                    stack.push(l);
                    (lo, hi) = r;
                }
                (Some(l), None) => (lo, hi) = l,
                (None, Some(r)) => (lo, hi) = r,
                (None, None) => break,
            }
        }
    }
}

/// The lowering of the other sounds while a radio line plays
/// (`UAudioSubsystem::UpdateDuckVolume`), with the original's settings
/// (`DuckMusic` 0.5, `DuckVoice` 0.25, `DuckSFX` 0.25, `DuckFadeIn` and
/// `DuckFadeOut` 0.5 s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duck {
    pub ducking: bool,
    /// 1 fading down, -1 fading back up, 0 still.
    direction: f32,
    time_left: f32,
    now: [f32; 3],
    from: [f32; 3],
    to: [f32; 3],
}

pub const DUCK_TO: [f32; 3] = [0.5, 0.25, 0.25];
pub const DUCK_FADE_IN: f32 = 0.5;
pub const DUCK_FADE_OUT: f32 = 0.5;

impl Default for Duck {
    fn default() -> Self {
        Duck {
            ducking: false,
            direction: 0.0,
            time_left: 0.0,
            now: [1.0; 3],
            from: [1.0; 3],
            to: [1.0; 3],
        }
    }
}

impl Duck {
    /// One frame of `dt` seconds; `voices` ducking voices were playing.
    /// The game's own steps: the share done is `1 - time left / DuckFadeOut`
    /// both ways.
    pub fn step(&mut self, dt: f32, voices: usize) {
        let want = voices > 0;
        if want != self.ducking {
            self.ducking = want;
            if want {
                self.direction = 1.0;
                self.time_left = DUCK_FADE_IN;
                self.to = DUCK_TO;
            } else {
                self.direction = -1.0;
                self.time_left = DUCK_FADE_OUT;
                self.to = [1.0; 3];
            }
            self.from = self.now;
        }
        self.time_left = (self.time_left - dt).max(0.0);
        if self.direction != 0.0 {
            let t = 1.0 - self.time_left / DUCK_FADE_OUT;
            for i in 0..3 {
                self.now[i] = self.from[i] + (self.to[i] - self.from[i]) * t;
            }
        }
        if self.time_left < 0.0001 && self.direction != 0.0 {
            self.direction = 0.0;
        }
    }

    /// Music, voice, effects.
    pub fn levels(&self) -> [f32; 3] {
        self.now
    }
}

/// `FAbstractSource::GetDuckMultiplier`: 1 while paused, for a voice
/// line (kind 5, every voice a character or script says) and for a voice
/// that itself disables voice cues; else the category's level.
pub fn duck_multiplier(duck: &Duck, paused: bool, kind: i32, meta: &SoundMeta) -> f32 {
    if paused || kind == 5 {
        return 1.0;
    }
    let [music, voice, sfx] = duck.levels();
    match meta.category {
        MUSIC => music,
        VOICE if meta.voice_cues => 1.0,
        VOICE => voice,
        _ => sfx,
    }
}

/// `UAudioSubsystem::GetMasterVolumeMultiplier`: the slider for the
/// sound's category (and the port's master volume).
pub fn category_volume(mix: &SoundMix, category: u8) -> f32 {
    let c = match category {
        MUSIC => mix.music,
        VOICE => mix.voice,
        _ => mix.sfx,
    };
    c * mix.master
}

/// EAX occlusion when a sound starts (`UALAudioSubsystem::PlaySource`):
/// `2000 × log10(reach now / full reach)`, at least -10000 mB.
pub fn occlusion_at_start(reach: f32, full: f32) -> i32 {
    let r = if full > 0.0 { reach / full } else { 1.0 };
    (2000.0 * r.max(1e-5).log10()).round() as i32
}

/// EAX occlusion each frame (`UALAudioSubsystem::Update`): the game
/// divides the reach by OpenAL's reference distance, which is the reach
/// itself for a sound that fades with distance (the result, +4000 mB, is
/// out of range and the driver refuses it, keeping the value from the
/// start) and the distance for one that doesn't. `None`: refused.
pub fn occlusion_each_frame(reach: f32, distance: f32, falloff: bool) -> Option<i32> {
    let reference = if falloff { reach } else { distance } * 0.01;
    if reference <= 0.0 {
        return None;
    }
    let mb = 2000.0 * (reach / reference).max(1e-5).log10();
    (mb <= 0.0).then(|| mb.round() as i32)
}

/// One entry of the frame's list: a sound queued this frame or playing.
#[derive(Debug, Clone)]
pub struct Slot {
    pub priority: f32,
    pub id: i32,
    pub queued: bool,
    /// May be dropped for lack of a channel when no channel is free:
    /// a queued bullet impact (`imp_*`, `exp_GEN_proxMine_01`), placed.
    pub impact: bool,
    /// Skipped instead of played: a placed effect longer than 2 s whose
    /// reach behind walls has shrunk to nothing.
    pub muffled_long: bool,
}

/// What [`schedule`] decided for each slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Playing on (or newly started).
    Play,
    /// A playing sound stopped, or a queued one dropped.
    Stop,
}

/// The channel handing out of `UAudioSubsystem::Update` for one frame.
/// `channels[k]`: the slot playing on the sound card's channel `k`, or
/// `None` when it is free; updated to the new layout.
pub fn schedule(slots: &[Slot], channels: &mut [Option<usize>]) -> Vec<Verdict> {
    let n = slots.len();
    let count = channels.len();
    let mut verdict: Vec<Option<Verdict>> = vec![None; n];
    let prio: Vec<f32> = slots.iter().map(|s| s.priority).collect();
    let mut order: Vec<usize> = (0..n).collect();
    sort(&prio, &mut order);
    let threshold = if n > count {
        prio[order[count]].trunc()
    } else {
        0.0
    };
    // Playing sounds at or below the cut-off stop, unless bit 0 of their
    // id is set.
    let mut free = count as i64;
    for &i in &order {
        let s = &slots[i];
        if s.queued {
            continue;
        }
        if threshold >= s.priority && s.id & 1 == 0 {
            verdict[i] = Some(Verdict::Stop);
            for c in channels.iter_mut() {
                if *c == Some(i) {
                    *c = None;
                }
            }
        } else {
            verdict[i] = Some(Verdict::Play);
            free -= 1;
        }
    }
    // Impacts make way when no channel is free (`ImpactNumChannelThreshold`
    // is 0: not in the game's ini).
    if free <= 0 {
        order.retain(|&i| {
            let s = &slots[i];
            if s.queued && s.impact && !(free > 0 && s.priority >= 1.88) {
                verdict[i] = Some(Verdict::Stop);
                false
            } else {
                true
            }
        });
    }
    let top = count.min(order.len());
    for &i in order.iter().take(top) {
        let s = &slots[i];
        if !s.queued || s.priority <= threshold || verdict[i].is_some() {
            continue;
        }
        if s.muffled_long {
            verdict[i] = Some(Verdict::Stop);
            continue;
        }
        match find_source(slots, channels, i) {
            Some(k) => {
                if let Some(old) = channels[k] {
                    verdict[old] = Some(Verdict::Stop);
                }
                channels[k] = Some(i);
                verdict[i] = Some(Verdict::Play);
            }
            None => verdict[i] = Some(Verdict::Stop),
        }
    }
    verdict
        .into_iter()
        .map(|v| v.unwrap_or(Verdict::Stop))
        .collect()
}

/// `UAudioSubsystem::FindSource`: the channels in order; the first free
/// one is taken, unless a channel before it plays a sound with the same
/// id (bit 0 aside): that one is taken over when its bit 0 is clear, and
/// the new sound is refused when set. With no free channel and no same
/// id, the least important playing sound that is less important than the
/// new one gives up its channel.
fn find_source(slots: &[Slot], channels: &[Option<usize>], new: usize) -> Option<usize> {
    let s = &slots[new];
    let mut lowest = s.priority;
    let mut pick = None;
    for (k, c) in channels.iter().enumerate() {
        let Some(j) = *c else { return Some(k) };
        let other = &slots[j];
        if (s.id ^ other.id) & !1 == 0 {
            return if other.id & 1 == 0 { Some(k) } else { None };
        }
        if lowest > other.priority {
            lowest = other.priority;
            pick = Some(k);
        }
    }
    pick
}

/// The engine's state between frames.
#[derive(Resource, Debug, Default)]
pub struct SoundEngine {
    pub duck: Duck,
    /// Ducking voices playing at the end of the last frame.
    pub ducking_voices: usize,
    /// The sound card's channels: what plays on each.
    pub channels: Vec<Option<Entity>>,
    /// Counts for the log and the self-test.
    pub started: u64,
    pub dropped: u64,
    pub stopped: u64,
    pub unheard: u64,
}

/// What the engine reads of a new sound.
type NewSound<'a> = (
    Entity,
    &'a AudioPlayer,
    &'a mut PlaybackSettings,
    Option<&'a mut SoundAt>,
    Option<&'a mut Transform>,
    Option<&'a SoundBy>,
    Option<&'a mut Placed>,
);

/// What the engine reads of a sound it is playing.
type Playing<'a> = (
    Entity,
    &'a mut EngineSound,
    &'a PlaybackSettings,
    Option<&'a AudioSink>,
    Option<&'a SpatialAudioSink>,
    Option<&'a Placed>,
    Option<&'a SoundAt>,
);

/// Stop a sound: its sink stops (Bevy then cleans it up as its
/// `PlaybackSettings` say); one that never reached the sound card is
/// despawned (`DESPAWN`) or loses its player.
fn stop_entity(
    commands: &mut Commands,
    e: Entity,
    despawn: bool,
    sink: Option<&AudioSink>,
    spatial: Option<&SpatialAudioSink>,
) {
    match (sink, spatial) {
        (Some(s), _) => s.stop(),
        (_, Some(s)) => s.stop(),
        _ if despawn => {
            commands.entity(e).try_despawn();
            return;
        }
        _ => {
            commands.entity(e).try_remove::<AudioPlayer>();
        }
    }
    commands.entity(e).try_remove::<EngineSound>();
}

/// Start the new sounds and hand out the channels (see the module notes,
/// steps 1 to 4). Runs after the parts have spawned this frame's sounds
/// and before Bevy starts them.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn schedule_sounds(
    mut commands: Commands,
    mut engine: ResMut<SoundEngine>,
    (mix, eax): (Option<Res<SoundMix>>, Option<Res<EaxOn>>),
    play: Option<Res<State<Play>>>,
    mut ducking: ResMut<Ducking>,
    mut new: Query<
        NewSound,
        (
            Added<AudioPlayer>,
            Without<EngineSound>,
            Without<MusicTrack>,
        ),
    >,
    mut playing: Query<Playing, Without<MusicTrack>>,
    music: Query<(Entity, Option<&AudioSink>, Has<AudioPlayer>), With<MusicTrack>>,
    tags: Query<(Entity, &ScriptTags, &GlobalTransform)>,
    dead: Query<(), With<Dead>>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    hearing: Option<Res<LevelHearing>>,
    script: Option<Res<crate::LevelScript>>,
    time: Res<Time>,
    log: Option<Res<rc_game_core::GameLog>>,
    mut last_log: Local<(f64, [u64; 4])>,
) {
    let mix = mix.map(|m| *m).unwrap_or_default();
    let eax = eax.is_none_or(|e| e.0);
    let paused = play.is_some_and(|p| *p.get() == Play::Paused);
    let now = sound_clock(script.as_deref(), &time);
    let dt = time.delta_secs().clamp(0.0001, 1.0);
    let ear = ears.iter().next().map(|g| to_unreal(g.translation()));
    let hearing = hearing.as_ref().map(|h| &*h.0);
    let count = mix.channels.clamp(1, MAX_CHANNELS) as usize;

    // The lowering for radio lines, from last frame's count.
    let voices = engine.ducking_voices;
    engine.duck.step(dt, voices);

    struct Entry {
        entity: Entity,
        slot: Slot,
        new: Option<EngineSound>,
        despawn: bool,
        voice_cues: bool,
        music: bool,
    }
    let mut list: Vec<Entry> = Vec::new();
    let distance_to = |p: Option<UVec3>| -> f32 {
        match (p, ear) {
            (Some(a), Some(e)) => (a - e).length(),
            _ => 0.0,
        }
    };
    // The playing sounds (`UpdateBase`: a new priority, except for an
    // ambient sound).
    for (e, mut s, settings, sink, spatial, placed, at) in &mut playing {
        let empty = sink.map(|s| s.empty()).or(spatial.map(|s| s.empty()));
        let looping = matches!(settings.mode, PlaybackMode::Loop);
        let over = match empty {
            Some(empty) => empty,
            None => {
                !looping
                    && s.meta.seconds > 0.0
                    && now - s.started > (s.meta.seconds / s.pitch) as f64 + 0.25
            }
        };
        if over {
            commands.entity(e).try_remove::<EngineSound>();
            continue;
        }
        if s.kind() != 3 {
            let radius = placed.map_or(s.meta.radius, |p| p.radius);
            let d = distance_to(at.map(|a| to_unreal(a.0)));
            s.priority = priority(s.flags, s.kind(), s.volume, d, radius);
        }
        list.push(Entry {
            entity: e,
            slot: Slot {
                priority: s.priority,
                id: s.id,
                queued: false,
                impact: false,
                muffled_long: false,
            },
            new: None,
            despawn: matches!(settings.mode, PlaybackMode::Despawn),
            voice_cues: s.meta.voice_cues,
            music: false,
        });
    }
    // Music holds a channel too (streamed, heard everywhere, a number of
    // its own).
    for (e, sink, has_player) in &music {
        if !has_player || sink.is_some_and(|s| s.empty()) {
            continue;
        }
        let f = flags::NON_POSITIONAL | flags::STREAMING | flags::MUSIC;
        list.push(Entry {
            entity: e,
            slot: Slot {
                priority: priority(f, 0, 1.0, 0.0, 0.0),
                id: (e.index_u32() as i32).wrapping_shl(4),
                queued: false,
                impact: false,
                muffled_long: false,
            },
            new: None,
            despawn: false,
            voice_cues: false,
            music: true,
        });
    }

    // The new sounds: `PlayUSound` and the hearing check.
    for (e, player, mut settings, mut at, mut transform, by, mut placed) in &mut new {
        let registered = sound_meta::meta(player.0.id());
        let meta = registered
            .clone()
            .unwrap_or_else(|| Arc::new(SoundMeta::default()));
        let by = by.copied().unwrap_or_default();
        let despawn = matches!(settings.mode, PlaybackMode::Despawn);
        let drop = |commands: &mut Commands, counter: &mut u64| {
            *counter += 1;
            if despawn {
                commands.entity(e).try_despawn();
            } else {
                commands.entity(e).try_remove::<AudioPlayer>();
            }
        };
        // Heard from the actor its `ActorTag` names.
        if let Some(tag) = &meta.actor_tag {
            let found = tags.iter().find(|(_, t, _)| {
                t.tag
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(tag))
            });
            if let Some((t, _, g)) = found {
                if let Some(at) = at.as_deref_mut() {
                    at.0 = g.translation();
                }
                if let Some(tr) = transform.as_deref_mut() {
                    tr.translation = g.translation();
                }
                if let Some(p) = placed.as_deref_mut() {
                    p.follow = Some(t);
                }
            }
        }
        // A voice from a dead character isn't heard.
        if meta.category == VOICE && by.actor.is_some_and(|a| dead.contains(a)) {
            drop(&mut commands, &mut engine.unheard);
            continue;
        }
        let Some((volume, pitch)) = roll(&meta, sound_meta::srand(), sound_meta::srand()) else {
            drop(&mut commands, &mut engine.dropped);
            continue;
        };
        let non_positional = meta.non_positional || by.everywhere || at.is_none();
        let mut f = 0u32;
        if non_positional {
            f |= flags::NON_POSITIONAL;
        }
        if meta.streaming {
            f |= flags::STREAMING;
        }
        // The game clears a loop request unless the sound itself loops (a
        // sound it doesn't know keeps what the caller asked).
        if matches!(settings.mode, PlaybackMode::Loop) {
            if registered.is_some() && !meta.looping {
                settings.mode = PlaybackMode::Despawn;
            } else {
                f |= flags::LOOP;
            }
        }
        if meta.no_occlusion {
            f |= flags::NO_OCCLUSION;
        }
        if meta.no_falloff {
            f |= flags::NO_FALLOFF;
        }
        f |= match meta.category {
            MUSIC => flags::MUSIC,
            VOICE => flags::VOICE,
            _ => flags::SFX,
        };
        if by.owned {
            f |= flags::OWNED;
        }
        let place = at.as_deref().map(|a| to_unreal(a.0));
        // `CheckHearSound`: only within `Radius × 100` (times the rooms
        // share when a wall is between).
        if !non_positional && !by.skip_hear_check {
            if let (Some(p), Some(ear)) = (place, ear) {
                let (a, b) = ([p.x, p.y, p.z], [ear.x, ear.y, ear.z]);
                let m = match hearing {
                    Some(h) if !h.line_clear(a, b) => {
                        h.radius_multiplier(h.zone_at(a), h.zone_at(b))
                    }
                    _ => 1.0,
                };
                let reach = meta.radius * MAX_RADIUS_MULTIPLIER * m;
                let dv = p - ear;
                if dv.dot(dv) >= reach * reach {
                    drop(&mut commands, &mut engine.unheard);
                    continue;
                }
            }
        }
        let actor_index = by
            .actor_index
            .or(by.actor.map(|a| a.index_u32()))
            .unwrap_or(0);
        let id = source_id(meta.category, by.ambient, actor_index, by.owned);
        let kind = (id >> 1) & 7;
        // Placed sounds get the distance and walls rule.
        let mut fresh = (!non_positional && placed.is_none())
            .then(|| Placed::new(meta.radius, !meta.no_falloff, by.actor));
        let target: Option<&mut Placed> = match placed.as_deref_mut() {
            Some(p) if !non_positional => Some(p),
            _ => fresh.as_mut(),
        };
        let mut reach_now = 0.0;
        let mut radius = meta.radius;
        if let Some(p) = target {
            if let (Some(pl), Some(ear)) = (place, ear) {
                p.update(hearing, pl, ear, now);
            }
            reach_now = p.reach.current;
            radius = p.radius;
        }
        let d = if non_positional {
            0.0
        } else {
            distance_to(place)
        };
        let prio = if kind == 3 {
            start_priority(f, kind, volume, d, radius)
        } else {
            priority(f, kind, volume, d, radius)
        };
        let occlusion = if eax && !non_positional && f & flags::NO_OCCLUSION == 0 {
            occlusion_at_start(reach_now, radius)
        } else {
            0
        };
        let muffled_long = f & 0xb3e == 0 && meta.seconds > 2.0 && reach_now < 0.001;
        let lower = meta.name.to_ascii_lowercase();
        let impact = !non_positional
            && meta.category == SFX
            && (lower.starts_with("imp_") || lower == "exp_gen_proxmine_01");
        if let Some(p) = fresh {
            commands.entity(e).insert(p);
        }
        settings.speed *= pitch;
        if non_positional {
            settings.spatial = false;
        } else if mix.sound_3d {
            settings.spatial = true;
            settings.spatial_scale = Some(SpatialScale::new(SPATIAL_SHRINK));
        }
        list.push(Entry {
            entity: e,
            slot: Slot {
                priority: prio,
                id,
                queued: true,
                impact,
                muffled_long,
            },
            new: Some(EngineSound {
                meta: meta.clone(),
                flags: f,
                id,
                volume,
                pitch,
                priority: prio,
                call_volume: settings.volume.to_linear(),
                started: now,
                occlusion,
            }),
            despawn: matches!(settings.mode, PlaybackMode::Despawn),
            voice_cues: meta.voice_cues,
            music: false,
        });
    }

    // The sound card's channels as they stand: AUDIO CHANNELS may have
    // changed; a sound that has ended frees its channel; music that the
    // music part started takes the first free one.
    let index: std::collections::HashMap<Entity, usize> = list
        .iter()
        .enumerate()
        .map(|(i, e)| (e.entity, i))
        .collect();
    let mut channels: Vec<Option<usize>> = engine
        .channels
        .iter()
        .map(|c| c.and_then(|e| index.get(&e).copied()))
        .collect();
    channels.resize(count, None);
    let held: std::collections::HashSet<usize> = channels.iter().flatten().copied().collect();
    let mut cut: Vec<usize> = engine
        .channels
        .iter()
        .skip(count)
        .flatten()
        .filter_map(|e| index.get(e).copied())
        .collect();
    for (i, entry) in list.iter().enumerate() {
        if entry.slot.queued || held.contains(&i) {
            continue;
        }
        match channels.iter_mut().find(|c| c.is_none()) {
            Some(c) => *c = Some(i),
            None if !entry.music => cut.push(i),
            None => {}
        }
    }

    // Hand out the channels.
    let slots: Vec<Slot> = list.iter().map(|e| e.slot.clone()).collect();
    let mut verdicts = schedule(&slots, &mut channels);
    for i in cut {
        verdicts[i] = Verdict::Stop;
    }
    let mut voices_now = 0;
    for (entry, v) in list.iter_mut().zip(&verdicts) {
        let e = entry.entity;
        match (v, entry.new.take()) {
            (Verdict::Play, new) => {
                if entry.voice_cues {
                    voices_now += 1;
                }
                if let Some(s) = new {
                    engine.started += 1;
                    let mut c = commands.entity(e);
                    if s.occlusion != 0 {
                        c.insert(EaxOcclusion {
                            millibels: s.occlusion,
                        });
                    }
                    c.insert(s);
                }
            }
            (Verdict::Stop, Some(_)) => {
                engine.dropped += 1;
                stop_entity(&mut commands, e, entry.despawn, None, None);
            }
            (Verdict::Stop, None) => {
                engine.stopped += 1;
                if entry.music {
                    if let Ok((_, Some(sink), _)) = music.get(e) {
                        sink.stop();
                    }
                    continue;
                }
                let (sink, spatial) = match playing.get(e) {
                    Ok((_, _, _, a, b, _, _)) => (a, b),
                    Err(_) => (None, None),
                };
                stop_entity(&mut commands, e, entry.despawn, sink, spatial);
            }
        }
    }
    engine.channels = channels.iter().map(|c| c.map(|i| list[i].entity)).collect();
    engine.ducking_voices = voices_now;
    let [m, v, s] = engine.duck.levels();
    *ducking = Ducking {
        music: m,
        voice: v,
        sfx: s,
        voice_cues_blocked: voices_now > 0,
        paused,
    };
    // A line for the log every 10 s while something changes.
    let counts = [
        engine.started,
        engine.stopped,
        engine.dropped,
        engine.unheard,
    ];
    if counts != last_log.1 && (now - last_log.0).abs() >= 10.0 {
        if let Some(log) = &log {
            let busy = engine.channels.iter().filter(|c| c.is_some()).count();
            log.line(format!(
                "sound engine: {busy} of {} channels busy; so far {} started, {} stopped to make room, {} not started (no channel), {} out of hearing; ducking {}",
                engine.channels.len(),
                counts[0],
                counts[1],
                counts[2],
                counts[3],
                if voices_now > 0 { "on" } else { "off" }
            ));
        }
        *last_log = (now, counts);
    }
}

/// Each sound's loudness this frame (step 5), and the EAX occlusion of a
/// placed sound that doesn't fade with distance (step 6).
#[allow(clippy::type_complexity)]
pub fn set_volumes(
    engine: Res<SoundEngine>,
    mix: Option<Res<SoundMix>>,
    eax: Option<Res<EaxOn>>,
    ducking: Res<Ducking>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    mut sounds: Query<(
        &EngineSound,
        &mut PlaybackSettings,
        Option<&mut AudioSink>,
        Option<&mut SpatialAudioSink>,
        Option<&Placed>,
        Option<&SoundFade>,
        Option<&SoundAt>,
        Option<&mut EaxOcclusion>,
    )>,
) {
    let mix = mix.map(|m| *m).unwrap_or_default();
    let eax = eax.is_none_or(|e| e.0);
    let ear = ears.iter().next().map(|g| to_unreal(g.translation()));
    for (s, mut settings, sink, spatial, placed, fade, at, occlusion) in &mut sounds {
        let fade = fade.map_or(1.0, |f| f.0);
        let g = s.volume
            * duck_multiplier(&engine.duck, ducking.paused, s.kind(), &s.meta)
            * fade
            * category_volume(&mix, s.meta.category);
        let g = g.clamp(0.0001, 0.9999) * s.call_volume * placed.map_or(1.0, |p| p.gain);
        let sink: Option<&mut dyn AudioSinkPlayback> = match (sink, spatial) {
            (Some(x), _) => Some(x.into_inner()),
            (_, Some(x)) => Some(x.into_inner()),
            _ => None,
        };
        match sink {
            Some(sink) => {
                if (sink.volume().to_linear() - g).abs() > 1e-6 {
                    sink.set_volume(Volume::Linear(g));
                }
            }
            None => {
                if (settings.volume.to_linear() - g).abs() > 1e-6 {
                    settings.volume = Volume::Linear(g);
                }
            }
        }
        // EAX: only a sound that doesn't fade with distance can change
        // its occlusion after it starts.
        if let (true, Some(p), Some(at), Some(ear), Some(mut o)) = (eax, placed, at, ear, occlusion)
        {
            if let Some(mb) =
                occlusion_each_frame(p.reach.applied, (to_unreal(at.0) - ear).length(), p.falloff)
            {
                if o.millibels != mb {
                    o.millibels = mb;
                }
            }
        }
    }
}

/// Bevy's 3D sound (rodio's) also makes a sound quieter with distance, by
/// one over the distance squared in Bevy units. The engine's own distance
/// rule does that job, so placed sounds are shrunk by this much before
/// Bevy sees them: its fading only starts beyond 1000 units (a
/// kilometre). Which ear a sound is nearer doesn't change.
pub const SPATIAL_SHRINK: f32 = 1e-3;

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(min_v: f32, max_v: f32, min_p: f32, max_p: f32) -> SoundMeta {
        SoundMeta {
            min_volume: min_v,
            max_volume: max_v,
            min_pitch: min_p,
            max_pitch: max_p,
            ..Default::default()
        }
    }

    /// Volume and pitch between the `AudioParams` limits; too quiet plays
    /// nothing; pitch within OpenAL's 0.7 to 1.2.
    #[test]
    fn rolls() {
        let m = meta(0.5, 1.0, 0.9, 1.1);
        assert_eq!(roll(&m, 0.0, 0.0), Some((0.5, 0.9)));
        let (v, p) = roll(&m, 1.0, 0.5).unwrap();
        assert!((v - 1.0).abs() < 1e-6 && (p - 1.0).abs() < 1e-6);
        assert_eq!(roll(&meta(0.0, 0.0, 1.0, 1.0), 0.5, 0.5), None);
        assert_eq!(roll(&meta(1.0, 2.0, 0.1, 3.0), 1.0, 1.0), Some((1.0, 1.2)));
        assert_eq!(roll(&meta(1.0, 1.0, 0.1, 0.1), 0.0, 0.0), Some((1.0, 0.7)));
    }

    /// A voice line keeps its speaker's number and kind; effects get a
    /// number of their own every time.
    #[test]
    fn source_ids() {
        let a = source_id(VOICE, false, 7, true);
        assert_eq!(a, 0xa | 7 << 4);
        assert_eq!(source_id(VOICE, false, 7, false), a | 1);
        assert_eq!((a >> 1) & 7, 5);
        let b = source_id(SFX, false, 7, true);
        let c = source_id(SFX, false, 7, true);
        assert_ne!(b, c);
        assert_eq!(b & 0xf, 0);
        // An actor's ambient sound: kind 3 (sfx | 6).
        assert_eq!((source_id(SFX, true, 2, true) >> 1) & 7, 3);
        // Music: kind 7, a number of its own.
        assert_eq!(source_id(MUSIC, false, 2, true) & 0xf, 0);
    }

    #[test]
    fn priorities() {
        // Radio voice line, full volume: 1 (heard everywhere) + 2 (voice)
        // + 1.
        assert_eq!(priority(flags::NON_POSITIONAL, 5, 1.0, 5000.0, 10.0), 4.0);
        // An effect half way to its reach at volume 0.5.
        let p = priority(0, 0, 0.5, 500.0, 10.0);
        assert!((p - (1.0 + 0.925 * 0.5)).abs() < 1e-5, "{p}");
        // Beyond its reach: at least 0.01 of the volume part.
        let p = priority(0, 0, 1.0, 1e6, 10.0);
        assert!((p - 1.01).abs() < 1e-5);
        // Silent.
        assert_eq!(priority(flags::STREAMING, 1, 0.0, 0.0, 10.0), 1.0);
        // The ambient start priority goes by the distance squared.
        let p = start_priority(0, 3, 1.0, 500.0, 10.0);
        assert!((p - 0.75).abs() < 1e-5, "{p}");
    }

    /// Most important first; equal ones in the C library's own order.
    #[test]
    fn sorts_like_the_game() {
        let prio = [1.0, 1.0, 2.0];
        let mut o = vec![0, 1, 2];
        sort(&prio, &mut o);
        assert_eq!(o, [2, 1, 0]);
        let prio: Vec<f32> = (0..40).map(|i| ((i * 7) % 11) as f32).collect();
        let mut o: Vec<usize> = (0..40).collect();
        sort(&prio, &mut o);
        assert!(o.windows(2).all(|w| prio[w[0]] >= prio[w[1]]));
        let mut seen = o.clone();
        seen.sort();
        assert_eq!(seen, (0..40).collect::<Vec<_>>());
    }

    fn slot(priority: f32, id: i32, queued: bool) -> Slot {
        Slot {
            priority,
            id,
            queued,
            impact: false,
            muffled_long: false,
        }
    }

    /// Free channels first; a speaker's newer line takes over the older
    /// one; a line that can't be cut makes a newer one wait out; when
    /// full, the least important gives way.
    #[test]
    fn hands_out_channels() {
        // Two channels: one free.
        let slots = [slot(1.5, 0x10, false), slot(1.2, 0x20, true)];
        let mut ch = vec![Some(0), None];
        assert_eq!(schedule(&slots, &mut ch), [Verdict::Play, Verdict::Play]);
        assert_eq!(ch, [Some(0), Some(1)]);
        // Same speaker, owned: the new line replaces the old.
        let slots = [slot(3.0, 0xa | 0x50, false), slot(3.0, 0xa | 0x50, true)];
        let mut ch = vec![Some(0), None];
        assert_eq!(schedule(&slots, &mut ch), [Verdict::Stop, Verdict::Play]);
        assert_eq!(ch, [Some(1), None]);
        // Same speaker, the old line can't be cut: the new one is refused.
        let slots = [slot(3.0, 0xb | 0x50, false), slot(3.0, 0xb | 0x50, true)];
        let mut ch = vec![Some(0), None];
        assert_eq!(schedule(&slots, &mut ch), [Verdict::Play, Verdict::Stop]);
        // Full: the least important playing sound gives way.
        let slots = [
            slot(1.9, 0x10, false),
            slot(1.1, 0x20, false),
            slot(4.0, 0x31, true),
        ];
        let mut ch = vec![Some(0), Some(1)];
        let v = schedule(&slots, &mut ch);
        assert_eq!(v, [Verdict::Play, Verdict::Stop, Verdict::Play]);
        assert_eq!(ch, [Some(0), Some(2)]);
    }

    /// Down to the duck levels over half a second, and back up.
    #[test]
    fn ducks_and_comes_back() {
        let mut d = Duck::default();
        d.step(0.25, 1);
        assert_eq!(d.levels(), [0.75, 0.625, 0.625]);
        d.step(0.25, 1);
        assert_eq!(d.levels(), DUCK_TO);
        d.step(0.5, 0);
        assert_eq!(d.levels(), [1.0; 3]);
        let m = SoundMeta {
            category: MUSIC,
            ..Default::default()
        };
        d.step(0.5, 1);
        assert_eq!(duck_multiplier(&d, false, 7, &m), 0.5);
        assert_eq!(duck_multiplier(&d, true, 7, &m), 1.0);
        assert_eq!(duck_multiplier(&d, false, 5, &m), 1.0);
    }

    #[test]
    fn occlusion() {
        assert_eq!(occlusion_at_start(100.0, 100.0), 0);
        assert_eq!(occlusion_at_start(10.0, 100.0), -2000);
        assert_eq!(occlusion_at_start(0.0, 100.0), -10000);
        assert_eq!(occlusion_each_frame(100.0, 500.0, true), None);
        assert_eq!(occlusion_each_frame(100.0, 10000.0, false), Some(0));
    }
}
