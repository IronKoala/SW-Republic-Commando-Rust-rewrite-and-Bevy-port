//! The room's echo, heard: every sound the game plays feeds the echo of
//! the room the player is in (`rc_game_core::RoomEcho`, set by the level's
//! zone sounds; the echo itself is `rc_audio::room`), except the music.
//!
//! **The game's rules here** (read in its `Engine.dll` and `ALAudio.dll`,
//! 2026-11-12/13; the decompiled code stays on the PC and is never
//! committed):
//! - the game gives the sound card one echo, the listener's, and every
//!   sound is sent to it at full level (its EAX `Room` send is 0 mB)
//!   unless the sound counts as music: its `AudioParams.Category` is
//!   `AG_Music`, or it was started as the menu's background music
//!   (`UAudioSubsystem::PlayUSound` sets flag 0x10 or passes 0x8, and
//!   `PlaySource` then sends -10000 mB). A sound that names no `Params`
//!   gets the class defaults, whose `Category` is `AG_Music`, so it gets
//!   no echo either. A group (`SoundMultiple`) plays its pick with the
//!   picked sound's own `Params`. Which `Params` each sound names is
//!   noted as it is read (`rc_combat::sound_params`);
//! - the echo's level does not fall with distance unless the room's
//!   `RoomRolloffFactor` says so (`rc_audio::room::room_rolloff_gain`);
//!   every room but four sets it to 0. So each sound is sent at its own
//!   loudness (its volume and the SOUND options) without the fading with
//!   distance, times that rolloff;
//! - the room's `AirAbsorptionHF` turns high notes down by distance, on
//!   the sound and its send alike (`rc_audio::room::air_absorption_gain`;
//!   the game sets no per-sound factor, so EAX's 1 applies), with
//!   distances in metres at 0.01 per Unreal unit.
//!
//! **Ours:** how it is wired. Bevy plays each sound on its own and mixes
//! them, so each sound is wrapped as it starts: it is converted to the
//! sound card's rate and two channels, and as it plays it adds itself to
//! a shared send ([`RoomBus`]). One endless sound, started with the game,
//! plays the echo of that send. A looping sound loops inside the wrapper
//! (Bevy's own looping would replay a recording of it, and the replays
//! would not reach the echo). The distance used for rolloff and air
//! absorption is from the sound's place to the camera; a sound with no
//! place is at the listener. A sound not read from the game's files
//! (none at the moment) feeds the echo, and the log says so.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{
    AddAudioSource, AudioSink, AudioSinkPlayback, AudioSource, Decodable, GlobalVolume,
    PlaybackMode, PlaybackSettings, SpatialAudioSink,
};
use bevy::prelude::*;
use rc_audio::room::{
    air_absorption_gain, occlusion_gains, room_rolloff_gain, HighShelf, RoomBus, RoomReverb,
    METRES_PER_UU,
};
use rc_combat::AudioParams;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{EaxOcclusion, EaxOn, GameLog, GamePaths, PlayerCamera, RoomEcho, SoundAt};
use rc_scripts_plugin::sounds::Placed;
use rodio::source::UniformSourceIterator;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::collections::{HashMap, HashSet};

use crate::music::Track;

pub struct RoomSoundPlugin;

impl Plugin for RoomSoundPlugin {
    fn build(&self, app: &mut App) {
        let (rate, found) = output_rate();
        app.add_audio_source::<RoomSound>()
            .add_audio_source::<RoomReturn>()
            .insert_resource(RoomOutput {
                bus: RoomBus::new(),
                rate,
                found,
            })
            .init_resource::<RoomEcho>()
            .init_resource::<SoundCategories>()
            .init_resource::<EaxOn>()
            .add_systems(Startup, start_echo)
            .add_systems(Update, forward_echo)
            .add_systems(
                PostUpdate,
                (wrap_new_sounds, send_levels)
                    .chain()
                    .after(crate::sound_options::SoundOptionsSystems)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// Two channels: the echo is mixed in stereo, as every sound is before
/// Bevy spreads it over the speakers.
const CHANNELS: ChannelCount = ChannelCount::new(2).unwrap();

/// The rate the sound card plays at: what Bevy's audio (rodio) opens it
/// with, its default. `false` when no sound card was found (44.1 kHz).
fn output_rate() -> (SampleRate, bool) {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let rate = rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.default_output_config().ok())
        .and_then(|c| SampleRate::new(c.sample_rate()));
    match rate {
        Some(r) => (r, true),
        None => (SampleRate::new(44100).unwrap(), false),
    }
}

/// The shared send and the sound card's rate.
#[derive(Resource)]
pub struct RoomOutput {
    pub bus: Arc<RoomBus>,
    pub rate: SampleRate,
    found: bool,
}

/// The endless sound that plays the echo.
#[derive(Component)]
pub struct EchoReturn;

/// A playing sound's link to its wrapper, updated every frame (all as
/// `f32` bits): how strongly it feeds the echo and the send's high-note
/// gain; the gain of the sound itself and its high-note gain (occlusion
/// and air absorption, on top of the volume Bevy's sink gives it); and
/// the high-note filter's coefficient (the room's `HFReference`).
#[derive(Default)]
pub struct SendState {
    send: AtomicU32,
    send_hf: AtomicU32,
    direct: AtomicU32,
    direct_hf: AtomicU32,
    coefficient: AtomicU32,
}

impl SendState {
    fn new(send: f32) -> Self {
        let s = SendState::default();
        s.send.store(send.to_bits(), Ordering::Relaxed);
        for a in [&s.send_hf, &s.direct, &s.direct_hf] {
            a.store(1.0f32.to_bits(), Ordering::Relaxed);
        }
        s
    }
    fn get(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Ordering::Relaxed))
    }
    fn put(a: &AtomicU32, v: f32) {
        a.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// See [`SendState`].
#[derive(Component, Clone)]
pub struct RoomSend(Arc<SendState>);

/// What a sound wrapped for the echo was started with: its full-loudness
/// distance and whether it fades with distance (for the room's rolloff).
#[derive(Component, Clone, Copy)]
pub struct SendReach {
    radius: f32,
    falloff: bool,
}

/// The game's `AudioParams`, read once per package as sounds name them.
#[derive(Resource, Default)]
pub struct SoundCategories {
    params: HashMap<String, AudioParams>,
    read: HashSet<String>,
    unknown: u32,
}

impl SoundCategories {
    /// The `AudioParams` a sound plays with: its own `Params`, or the class
    /// defaults when it names none. `None` when the sound wasn't read from
    /// the game's files or its `Params` couldn't be read.
    fn of(
        &mut self,
        sound: &Handle<AudioSource>,
        paths: Option<&GamePaths>,
        log: &GameLog,
    ) -> Option<AudioParams> {
        let named = rc_combat::sound_params(sound.id())?;
        let Some(name) = named else {
            return Some(AudioParams::default());
        };
        let package = name.split_once('.').map(|(p, _)| p.to_string())?;
        if !self.read.contains(&package) {
            self.read.insert(package.clone());
            if let Some(paths) = paths {
                let (found, notes) =
                    rc_combat::read_audio_params(paths, std::slice::from_ref(&package));
                for n in notes {
                    log.line(format!("sound: room echo: {n}"));
                }
                self.params.extend(found);
            }
        }
        let p = self.params.get(&name).cloned();
        if p.is_none() {
            log.line(format!(
                "sound: room echo: Params {name} wasn't read; that sound feeds the echo"
            ));
        }
        p
    }
}

/// A game sound that also feeds the echo.
#[derive(Asset, TypePath)]
pub struct RoomSound {
    source: AudioSource,
    looping: bool,
    send: Arc<SendState>,
    bus: Arc<RoomBus>,
    rate: SampleRate,
}

/// The echo itself, as a sound.
#[derive(Asset, TypePath)]
pub struct RoomReturn {
    bus: Arc<RoomBus>,
    rate: SampleRate,
}

type Converted = UniformSourceIterator<<AudioSource as Decodable>::Decoder>;

/// A sound, converted and feeding the send as it plays.
pub struct Tapped {
    source: AudioSource,
    inner: Converted,
    looping: bool,
    send: Arc<SendState>,
    bus: Arc<RoomBus>,
    rate: SampleRate,
    /// Which channel the next sample is for.
    channel: u16,
    frame: f32,
    /// The high-note filters: the sound's own, per channel, and its send.
    air: [HighShelf; 2],
    send_air: HighShelf,
}

impl Tapped {
    fn convert(source: &AudioSource, rate: SampleRate) -> Converted {
        UniformSourceIterator::new(source.decoder(), CHANNELS, rate)
    }
}

impl Iterator for Tapped {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let s = match self.inner.next() {
            Some(s) => s,
            None if self.looping && self.channel == 0 => {
                self.inner = Self::convert(&self.source, self.rate);
                // An empty sound would loop forever doing nothing.
                self.inner.next()?
            }
            None => return None,
        };
        self.frame += s;
        let c = SendState::get(&self.send.coefficient);
        let hf = SendState::get(&self.send.direct_hf);
        let gain = SendState::get(&self.send.direct);
        let out = if hf < 1.0 {
            self.air[self.channel as usize % 2].run(s, c, hf) * gain
        } else {
            s * gain
        };
        self.channel += 1;
        if self.channel == CHANNELS.get() {
            self.channel = 0;
            let send = SendState::get(&self.send.send);
            let hf = SendState::get(&self.send.send_hf);
            let mono = self.frame * 0.5;
            let mono = if hf < 1.0 {
                self.send_air.run(mono, c, hf)
            } else {
                mono
            };
            self.bus.send(mono * send);
            self.frame = 0.0;
        }
        Some(out)
    }
}

impl Source for Tapped {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        CHANNELS
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            None
        } else {
            self.inner.total_duration()
        }
    }
}

impl Decodable for RoomSound {
    type Decoder = Tapped;

    fn decoder(&self) -> Tapped {
        Tapped {
            source: self.source.clone(),
            inner: Tapped::convert(&self.source, self.rate),
            looping: self.looping,
            send: self.send.clone(),
            bus: self.bus.clone(),
            rate: self.rate,
            channel: 0,
            frame: 0.0,
            air: Default::default(),
            send_air: HighShelf::default(),
        }
    }
}

/// The echo, playing forever.
pub struct EchoSource {
    reverb: RoomReverb,
    bus: Arc<RoomBus>,
    rate: SampleRate,
    seen: u64,
    right: Option<f32>,
}

impl Iterator for EchoSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        if let Some((v, p)) = self.bus.changed(self.seen) {
            self.seen = v;
            self.reverb.set(p.as_ref());
        }
        let [l, r] = self.reverb.run(self.bus.take());
        self.right = Some(r);
        Some(l)
    }
}

impl Source for EchoSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        CHANNELS
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for RoomReturn {
    type Decoder = EchoSource;

    fn decoder(&self) -> EchoSource {
        EchoSource {
            reverb: RoomReverb::new(self.rate.get()),
            bus: self.bus.clone(),
            rate: self.rate,
            seen: 0,
            right: None,
        }
    }
}

fn start_echo(
    mut commands: Commands,
    output: Res<RoomOutput>,
    mut returns: ResMut<Assets<RoomReturn>>,
    log: Res<GameLog>,
) {
    let h = returns.add(RoomReturn {
        bus: output.bus.clone(),
        rate: output.rate,
    });
    // `ONCE`: it never ends, and Bevy's looping would keep a recording of
    // it forever.
    commands.spawn((EchoReturn, AudioPlayer(h), PlaybackSettings::ONCE));
    log.line(format!(
        "sound: room echo ready at {} Hz{}",
        output.rate,
        if output.found {
            ""
        } else {
            " (no sound card found)"
        }
    ));
}

/// Hand the room's echo to the sound that plays it, when it changes.
fn forward_echo(echo: Res<RoomEcho>, output: Res<RoomOutput>) {
    if echo.is_changed() {
        output.bus.set(echo.values.clone());
    }
}

/// Wrap each new sound so it feeds the echo. Runs after the SOUND options
/// set its volume and before Bevy starts it. Music (see the module notes)
/// is left alone.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn wrap_new_sounds(
    mut commands: Commands,
    output: Res<RoomOutput>,
    sources: Res<Assets<AudioSource>>,
    mut rooms: ResMut<Assets<RoomSound>>,
    mut categories: ResMut<SoundCategories>,
    paths: Option<Res<GamePaths>>,
    global: Res<GlobalVolume>,
    mut new: Query<
        (
            Entity,
            &AudioPlayer<AudioSource>,
            &mut PlaybackSettings,
            Option<&Placed>,
        ),
        (Added<AudioPlayer<AudioSource>>, Without<Track>),
    >,
    log: Res<GameLog>,
    mut not_ready: Local<u32>,
) {
    for (e, player, mut p, placed) in &mut new {
        let params = categories.of(&player.0, paths.as_deref(), &log);
        if params
            .as_ref()
            .is_some_and(|a| a.category == rc_package::sound::AG_MUSIC)
        {
            // The game's music category: no echo.
            continue;
        }
        if params.is_none() {
            categories.unknown += 1;
            let n = categories.unknown;
            if n == 1 || n.is_multiple_of(100) {
                log.line(format!(
                    "sound: {n} sounds not read from the game's files, fed to the room's echo"
                ));
            }
        }
        let Some(source) = sources.get(&player.0) else {
            // Still loading from disk: plays without echo.
            *not_ready += 1;
            if *not_ready == 1 || not_ready.is_multiple_of(100) {
                log.line(format!(
                    "sound: {} sounds started before they were read, heard without the room's echo",
                    *not_ready
                ));
            }
            continue;
        };
        let looping = matches!(p.mode, PlaybackMode::Loop);
        if looping {
            p.mode = PlaybackMode::Once;
        }
        let reach = match (placed, &params) {
            (Some(pl), _) => SendReach {
                radius: pl.radius,
                falloff: pl.falloff,
            },
            // The game's radius: 0 counts as 10, unset is the class's 2000.
            (None, Some(a)) => SendReach {
                radius: match a.radius {
                    Some(r) if r > 0.0 => r,
                    Some(_) => 10.0,
                    None => 2000.0,
                },
                falloff: !a.no_falloff,
            },
            (None, None) => SendReach {
                radius: 2000.0,
                falloff: true,
            },
        };
        let send = RoomSend(Arc::new(SendState::new(send_level(
            p.volume.to_linear() * global.volume.to_linear(),
            placed,
        ))));
        let h = rooms.add(RoomSound {
            source: source.clone(),
            looping,
            send: send.0.clone(),
            bus: output.bus.clone(),
            rate: output.rate,
        });
        commands
            .entity(e)
            .remove::<AudioPlayer<AudioSource>>()
            .insert((AudioPlayer(h), send, reach));
    }
}

/// A sound's send: its volume without the fading with distance.
fn send_level(volume: f32, placed: Option<&Placed>) -> f32 {
    match placed {
        Some(p) => volume / p.gain.max(1e-3),
        None => volume,
    }
}

/// Keep each sound's send in step with its volume (the SOUND options,
/// fades, pauses), and apply the room's rolloff and air absorption for
/// its distance, and the sound's EAX occlusion. With the AUDIO menu's
/// EAX off the game uses none of it: no echo, no muffling.
#[allow(clippy::type_complexity)]
fn send_levels(
    echo: Res<RoomEcho>,
    eax: Res<EaxOn>,
    output: Res<RoomOutput>,
    ears: Query<&GlobalTransform, With<PlayerCamera>>,
    sounds: Query<(
        &RoomSend,
        &SendReach,
        Option<&AudioSink>,
        Option<&SpatialAudioSink>,
        Option<&Placed>,
        Option<(&SoundAt, &Transform)>,
        Option<&EaxOcclusion>,
    )>,
) {
    let ear = ears.iter().next().map(|g| g.translation());
    let room = echo.values.as_ref().filter(|_| eax.0);
    // EAX's default HFReference when the room has no echo.
    let hf_reference = room.map_or(5000.0, |r| r.hf_reference);
    let coefficient = HighShelf::coefficient(hf_reference, output.rate.get() as f32);
    for (send, reach, sink, spatial, placed, at, occlusion) in &sounds {
        let sink: &dyn AudioSinkPlayback = match (sink, spatial) {
            (Some(s), _) => s,
            (_, Some(s)) => s,
            _ => continue,
        };
        let v = if sink.is_muted() {
            0.0
        } else {
            sink.volume().to_linear()
        };
        // Unreal units from the camera; a sound with no place is at it.
        let distance = match (at, ear) {
            // The transform follows a sound that moves with its source.
            (Some((_, t)), Some(ear)) => t.translation.distance(ear) * UU_PER_UNIT,
            _ => 0.0,
        };
        // With no falloff the game puts the sound card's full-loudness
        // distance at the sound's own distance: no rolloff, no absorption.
        let reference = if reach.falloff {
            reach.radius
        } else {
            distance
        };
        let (rolloff, air) = match room {
            Some(r) => (
                room_rolloff_gain(r.room_rolloff_factor, reference, distance),
                air_absorption_gain(
                    r.air_absorption_hf,
                    (distance - reference).max(0.0) * METRES_PER_UU,
                ),
            ),
            None => (1.0, 1.0),
        };
        let [direct, direct_hf, room_gain, room_hf] = match occlusion {
            Some(o) if eax.0 => occlusion_gains(o.millibels as f32),
            _ => [1.0; 4],
        };
        let level = if eax.0 { send_level(v, placed) } else { 0.0 };
        SendState::put(&send.0.send, level * rolloff * room_gain);
        SendState::put(&send.0.send_hf, air * room_hf);
        SendState::put(&send.0.direct, direct);
        SendState::put(&send.0.direct_hf, air * direct_hf);
        SendState::put(&send.0.coefficient, coefficient);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sound wrapped for the echo plays exactly as before (same
    /// samples, converted to two channels), and feeds the send.
    #[test]
    fn wrapped_sound_plays_and_feeds_the_send() {
        // 4 samples of mono 16-bit at 22050 Hz.
        let pcm: [i16; 4] = [1000, -2000, 3000, -4000];
        let w = rc_audio::Wav {
            format_tag: 1,
            channels: 1,
            sample_rate: 22050,
            bits_per_sample: 16,
            samples: pcm.to_vec(),
            trailing_bytes: 0,
            other_chunks: vec![],
        };
        let bytes = w.to_bytes();
        let source = AudioSource {
            bytes: bytes.into(),
        };
        let bus = RoomBus::new();
        let rate = SampleRate::new(22050).unwrap();
        let room = RoomSound {
            source: source.clone(),
            looping: false,
            send: Arc::new(SendState::new(1.0)),
            bus: bus.clone(),
            rate,
        };
        let out: Vec<f32> = room.decoder().collect();
        assert_eq!(out.len(), 8, "two channels of four samples");
        assert!((out[0] - 1000.0 / 32768.0).abs() < 1e-4, "{out:?}");
        assert_eq!(out[0], out[1]);
        // Every frame was sent: the sum of the four samples.
        let sum = (1000.0 - 2000.0 + 3000.0 - 4000.0) / 32768.0;
        assert!((bus.take() - sum).abs() < 1e-4);

        // Looping keeps going past the end.
        let looped = RoomSound {
            looping: true,
            ..room
        };
        assert_eq!(looped.decoder().take(40).count(), 40);
    }
}
