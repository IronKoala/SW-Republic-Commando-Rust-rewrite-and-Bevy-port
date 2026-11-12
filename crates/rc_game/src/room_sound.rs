//! The room's echo, heard: every sound the game plays (except the music
//! and the menu clicks) also feeds the echo of the room the player is in
//! (`rc_game_core::RoomEcho`, set by the level's zone sounds; the echo
//! itself is `rc_audio::room`).
//!
//! **The game's rules here** (read in its `ALAudio.dll`, 2026-11-12; the
//! decompiled code stays on the PC and is never committed): the game gives
//! the sound card one echo, the listener's, and every sound is sent to it
//! at full level (its EAX `Room` send is 0 mB) unless the sound's own flags
//! turn the send off. The echo's level does not fall with distance unless
//! the room's `RoomRolloffFactor` says so, and every room but four sets it
//! to 0. So each sound is sent at its own loudness (its volume and the
//! SOUND options) but *without* the fading with distance.
//!
//! **Ours:** how it is wired. Bevy plays each sound on its own and mixes
//! them, so each sound is wrapped as it starts: it is converted to the
//! sound card's rate and two channels, and as it plays it adds itself to
//! a shared send ([`RoomBus`]). One endless sound, started with the game,
//! plays the echo of that send. A looping sound loops inside the wrapper
//! (Bevy's own looping would replay a recording of it, and the replays
//! would not reach the echo). The four rooms whose echo does fall with
//! distance (`RoomRolloffFactor` above 0) are played without that fall.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{
    AddAudioSource, AudioSink, AudioSinkPlayback, AudioSource, Decodable, GlobalVolume,
    PlaybackMode, PlaybackSettings, SpatialAudioSink,
};
use bevy::prelude::*;
use rc_audio::room::{RoomBus, RoomReverb};
use rc_game_core::{Dry, GameLog, RoomEcho};
use rc_scripts_plugin::sounds::Placed;
use rodio::source::UniformSourceIterator;
use rodio::{ChannelCount, Sample, SampleRate, Source};

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

/// How strongly a playing sound feeds the echo, as `f32` bits, updated
/// every frame from its volume.
#[derive(Component, Clone)]
pub struct RoomSend(Arc<AtomicU32>);

impl RoomSend {
    fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// A game sound that also feeds the echo.
#[derive(Asset, TypePath)]
pub struct RoomSound {
    source: AudioSource,
    looping: bool,
    send: Arc<AtomicU32>,
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
    send: Arc<AtomicU32>,
    bus: Arc<RoomBus>,
    rate: SampleRate,
    /// Which channel the next sample is for.
    channel: u16,
    frame: f32,
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
        self.channel += 1;
        if self.channel == CHANNELS.get() {
            self.channel = 0;
            let send = f32::from_bits(self.send.load(Ordering::Relaxed));
            self.bus.send(self.frame * 0.5 * send);
            self.frame = 0.0;
        }
        Some(s)
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
/// set its volume and before Bevy starts it.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn wrap_new_sounds(
    mut commands: Commands,
    output: Res<RoomOutput>,
    sources: Res<Assets<AudioSource>>,
    mut rooms: ResMut<Assets<RoomSound>>,
    global: Res<GlobalVolume>,
    mut new: Query<
        (
            Entity,
            &AudioPlayer<AudioSource>,
            &mut PlaybackSettings,
            Option<&Placed>,
        ),
        (
            Added<AudioPlayer<AudioSource>>,
            Without<Track>,
            Without<Dry>,
        ),
    >,
    log: Res<GameLog>,
    mut not_ready: Local<u32>,
) {
    for (e, player, mut p, placed) in &mut new {
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
        let send = RoomSend(Arc::new(AtomicU32::new(0)));
        send.set(send_level(
            p.volume.to_linear() * global.volume.to_linear(),
            placed,
        ));
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
            .insert((AudioPlayer(h), send));
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
/// fades, pauses).
#[allow(clippy::type_complexity)]
fn send_levels(
    sounds: Query<(
        &RoomSend,
        Option<&AudioSink>,
        Option<&SpatialAudioSink>,
        Option<&Placed>,
    )>,
) {
    for (send, sink, spatial, placed) in &sounds {
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
        send.set(send_level(v, placed));
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
            send: Arc::new(AtomicU32::new(1.0f32.to_bits())),
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
