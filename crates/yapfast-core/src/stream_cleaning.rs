//! A watched stream's sound cleaned before it plays, as the menu of its tile
//! asks (`volume::stream_cleaning`, Anomi's own): its noise taken out by
//! RNNoise or DeepFilterNet at the strength chosen, and this account's own
//! voice, back from the streamer's speakers in the sound their computer
//! shares, turned down by an echo gate against what the call sent. The
//! same objects clean someone's voice on the call's thread
//! (`discord_audio::Cleaned`); here they run in the watching worker, on the
//! thread that plays the stream's sound (`stream_audio::Player`), one
//! stream a worker.
//!
//! The costs are a voice's. Off, a frame plays as it came, with one atomic
//! load. On, nothing runs while the stream is silent, since no frame comes;
//! RNNoise or DeepFilterNet goes 10 s after the stream's last sound, the
//! echo gate after a minute, either as soon as it's turned off, and all of
//! it with the worker, as the stream ends or isn't watched any more. Its
//! buffers are made as the stream first sounds with something on; after
//! that a frame allocates nothing and takes no lock: what's chosen is one
//! atomic ([`Chosen`]), and what the call sent comes through a bounded
//! channel. DeepFilterNet runs on a thread of its own, as for a voice
//! (`their_noise`), loaded in the worker the first time it's on while the
//! stream sounds; RNNoise stands in until it has, and the sound jumps by
//! the 40 ms between the two once, as it changes over.
//!
//! The sound is cleaned 20 ms at a time in one channel, as a voice is, and
//! plays in mono while anything is on, 10 ms later with RNNoise, 50 with
//! DeepFilterNet. Noise suppression is made for speech: music and a game's
//! sound come out of it damaged, which the menu says.
//!
//! What the call sent reaches the worker as a line of [`Ours`] on its
//! input ([`crate::stream_worker::EchoFeed`]), which the line's reader hands
//! to the player here ([`sent`]), a few milliseconds after the call sent
//! it; the gate takes each as it comes, whether the stream sounds or not,
//! and hears the stream as it comes ([`EchoGate::heard_unpaced`]), its lag
//! measured from there.

use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicU32, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};

use yapfast_discord::voice::FRAME;

use crate::echo_gate::{EchoGate, Ours};
use crate::noise::{BLOCK, Rnnoise, Suppressor};
use crate::their_noise::{Strong, Theirs};
use crate::volume::Cleaning;

/// Silence this long lets go of the denoiser, as the call forgets a
/// speaker; the echo gate keeps what it learned longer, since learning it
/// again takes seconds of our speech.
const AWAY: Duration = Duration::from_secs(10);
const GATE_AWAY: Duration = Duration::from_secs(60);
/// A gap this long between frames starts the stream's sound again, as
/// someone speaking after a pause: what the denoiser held of before goes.
const GAP: Duration = Duration::from_millis(60);
/// The most a frame brings, Opus's longest, 120 ms, by channel, with a
/// 20 ms frame's worth waiting: what the buffers hold, made once.
const MOST: usize = 48 * 120 + FRAME;
/// Ticks of what the call sent that wait for the player: 2 s.
const QUEUE: usize = 100;
/// The stream's side of DeepFilterNet's thread, which names whose 20 ms it
/// cleans: a worker plays one stream.
const STREAM: u64 = 0;
/// Samples are cleaned in 16-bit range; the echo gate hears full scale.
const FULL_SCALE: f32 = 32768.0;

/// Where the worker's input hands what the call sent to the stream's
/// player, while one plays.
static SENT: Mutex<Option<SyncSender<Ours>>> = Mutex::new(None);

/// What the call sent, from a line of the worker's input, on to the
/// player's echo gate; dropped while no stream plays, or a tick it has no
/// room for. On the line's reader's thread, never the player's.
pub(crate) fn sent(ticks: Vec<Ours>) {
    let player = SENT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(player) = player.as_ref() {
        for ours in ticks {
            let _ = player.try_send(ours);
        }
    }
}

/// How the stream's sound is cleaned, which Yapfast changes while it plays
/// (`Control::Cleaning`): one atomic, read once a frame.
#[derive(Debug)]
pub struct Chosen(AtomicU32);

impl Chosen {
    pub fn new(cleaning: Cleaning) -> Self {
        Self(AtomicU32::new(pack(cleaning)))
    }

    pub fn set(&self, cleaning: Cleaning) {
        self.0.store(pack(cleaning), Ordering::Relaxed);
    }

    pub fn get(&self) -> Cleaning {
        unpack(self.0.load(Ordering::Relaxed))
    }
}

/// `cleaning` in 19 bits: the suppressor in two, its strength in eight,
/// the echo's box in one, its depth in eight.
fn pack(cleaning: Cleaning) -> u32 {
    let noise = match cleaning.noise {
        Suppressor::Off => 0,
        Suppressor::RNNoise => 1,
        Suppressor::DeepFilterNet => 2,
    };
    noise
        | (u32::from(cleaning.noise_strength) << 2)
        | (u32::from(cleaning.own_echo) << 10)
        | (u32::from(cleaning.echo_depth) << 11)
}

fn unpack(bits: u32) -> Cleaning {
    Cleaning {
        noise: match bits & 0b11 {
            1 => Suppressor::RNNoise,
            2 => Suppressor::DeepFilterNet,
            _ => Suppressor::Off,
        },
        noise_strength: ((bits >> 2) & 0xff) as u8,
        own_echo: (bits >> 10) & 1 == 1,
        echo_depth: ((bits >> 11) & 0xff) as u8,
    }
}

/// The player's side: what cleans the stream's sound, made as it's needed.
pub(crate) struct Cleaner {
    chosen: Arc<Chosen>,
    /// What the call sent, tick by tick, as the worker's input brought it.
    sent: Receiver<Ours>,
    rnnoise: Option<Rnnoise>,
    strong: Option<Strong>,
    theirs: Option<Theirs>,
    /// DeepFilterNet takes the noise out of what sounds now, rather than
    /// RNNoise, which stands in until its model has loaded; and whether
    /// the model wouldn't load in this worker, RNNoise standing in for
    /// good.
    deep: bool,
    strong_failed: bool,
    gate: Option<EchoGate>,
    /// The sound in one channel, 16-bit range, waiting for a whole 20 ms,
    /// and the 20 ms being cleaned.
    waiting: Vec<f32>,
    mono: Vec<f32>,
    /// What plays, interleaved as it came.
    out: Vec<i16>,
    /// The 20 ms cleaned so far, the one the sound started again on, and
    /// when it last sounded.
    frames: u64,
    since: u64,
    last: Option<Instant>,
}

impl Cleaner {
    /// Cleans as `chosen` says; what the call sent comes as the worker's
    /// input brings it, until this goes.
    pub(crate) fn new(chosen: Arc<Chosen>) -> Self {
        let (player, sent) = mpsc::sync_channel(QUEUE);
        *SENT.lock().unwrap_or_else(PoisonError::into_inner) = Some(player);
        Self {
            chosen,
            sent,
            rnnoise: None,
            strong: None,
            theirs: None,
            deep: false,
            strong_failed: false,
            gate: None,
            waiting: Vec::new(),
            mono: Vec::new(),
            out: Vec::new(),
            frames: 0,
            since: 0,
            last: None,
        }
    }

    /// Between frames, and as each comes: what the call sent keeps the
    /// echo gate's time, and what's held after the sound stopped long
    /// enough, or was turned off, goes.
    pub(crate) fn idle(&mut self) {
        let cleaning = self.chosen.get();
        self.follow(cleaning);
        for ours in self.sent.try_iter() {
            if let Some(gate) = &mut self.gate {
                gate.take(ours);
            }
        }
        let Some(last) = self.last else {
            return;
        };
        let quiet = last.elapsed();
        if quiet > AWAY && (self.rnnoise.is_some() || self.strong.is_some()) {
            tracing::debug!("the stream has been silent for 10 s; its denoiser goes");
            self.rnnoise = None;
            self.strong = None;
            self.theirs = None;
            self.deep = false;
        }
        if quiet > GATE_AWAY {
            self.gate = None;
            self.last = None;
        }
    }

    /// Cleans `pcm`, a frame of the stream's sound as decoded, interleaved,
    /// in `channels`: what to play, which is `pcm` itself while nothing is
    /// on.
    pub(crate) fn clean<'a>(&'a mut self, pcm: &'a mut [i16], channels: usize) -> &'a mut [i16] {
        let cleaning = self.chosen.get();
        self.follow(cleaning);
        if !cleaning.any() || channels == 0 {
            self.waiting.clear();
            return pcm;
        }
        let now = Instant::now();
        if self.last.is_none_or(|last| now.duration_since(last) > GAP) {
            self.since = self.frames + 1;
        }
        self.last = Some(now);
        // Made once, the first time; after that each holds what it needs.
        self.out.clear();
        self.mono.clear();
        self.waiting
            .reserve(MOST.saturating_sub(self.waiting.len()));
        self.mono.reserve(FRAME);
        self.out.reserve(MOST * channels);
        for sample in pcm.chunks_exact(channels) {
            let sum: f32 = sample.iter().map(|value| f32::from(*value)).sum();
            self.waiting.push(sum / channels as f32);
        }
        while self.waiting.len() >= FRAME {
            self.mono.clear();
            self.mono.extend(self.waiting.drain(..FRAME));
            self.frames += 1;
            self.twenty(cleaning);
            for value in &self.mono {
                let sample = value
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    as i16;
                for _ in 0..channels {
                    self.out.push(sample);
                }
            }
        }
        self.out.as_mut_slice()
    }

    /// Lets go of what `cleaning` no longer asks for.
    fn follow(&mut self, cleaning: Cleaning) {
        if !cleaning.denoises() {
            self.rnnoise = None;
        }
        if !(cleaning.denoises() && cleaning.noise == Suppressor::DeepFilterNet) {
            self.strong = None;
            self.theirs = None;
            self.deep = false;
        }
        if !cleaning.removes_echo() {
            self.gate = None;
        }
    }

    /// Cleans the 20 ms in [`Self::mono`]: its noise taken out, then the
    /// echo gate at its depth.
    fn twenty(&mut self, cleaning: Cleaning) {
        if cleaning.denoises() {
            self.denoise(cleaning);
        }
        if cleaning.removes_echo() {
            for ours in self.sent.try_iter() {
                if let Some(gate) = &mut self.gate {
                    gate.take(ours);
                }
            }
            for value in &mut self.mono {
                *value /= FULL_SCALE;
            }
            let gate = self.gate.get_or_insert_with(EchoGate::new);
            gate.set_depth(f32::from(cleaning.echo_depth));
            gate.heard_unpaced(&mut self.mono);
            for value in &mut self.mono {
                *value *= FULL_SCALE;
            }
        }
    }

    /// Takes the noise out of [`Self::mono`] at the strength chosen:
    /// DeepFilterNet on its thread when it's chosen and ready, 20 ms later;
    /// else RNNoise, which stands in while DeepFilterNet loads.
    fn denoise(&mut self, cleaning: Cleaning) {
        let back = self.frames == self.since;
        let deep = cleaning.noise == Suppressor::DeepFilterNet;
        if deep {
            self.want_strong();
        }
        // A voice changes over as it starts again, so it doesn't jump by
        // the 40 ms between the two mid-word; a stream may never pause, so
        // it changes over once the model is there, jumping once.
        let ready = self.strong.as_ref().is_some_and(Strong::ready);
        if back || (deep && ready && !self.deep) {
            self.deep = deep && ready;
        }
        if self.deep
            && let Some(strong) = &self.strong
        {
            let theirs = self.theirs.get_or_insert_with(Theirs::new);
            while let Some(done) = strong.take() {
                theirs.came(done);
            }
            let limit = cleaning.noise_limit(Suppressor::DeepFilterNet);
            let (frames, since) = (self.frames, self.since);
            if theirs.hand(strong, STREAM, frames, since, limit, &mut self.mono) {
                self.rnnoise = None;
                return;
            }
            // Its thread is gone: RNNoise from here on.
            self.deep = false;
            self.theirs = None;
        }
        let limit = cleaning.noise_limit(Suppressor::RNNoise);
        let fresh = self.rnnoise.is_none();
        let rnnoise = self.rnnoise.get_or_insert_with(|| Rnnoise::new(limit));
        rnnoise.limit(limit);
        // What it held of the sound before the gap would otherwise come
        // first.
        if back && !fresh {
            rnnoise.clear();
        }
        for block in self.mono.as_chunks_mut::<BLOCK>().0 {
            rnnoise.block(block);
        }
    }

    /// DeepFilterNet is chosen while the stream sounds: its thread starts,
    /// unless it's there or its model wouldn't load in this worker.
    fn want_strong(&mut self) {
        if self.strong.as_ref().is_some_and(Strong::failed) {
            tracing::warn!("DeepFilterNet isn't there for the stream's sound; RNNoise stands in");
            self.strong = None;
            self.theirs = None;
            self.strong_failed = true;
        }
        if self.strong.is_none() && !self.strong_failed {
            self.strong = Strong::start();
            self.strong_failed = self.strong.is_none();
        }
    }
}

impl Drop for Cleaner {
    /// What the call sends has nowhere to go any more.
    fn drop(&mut self) {
        *SENT.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held while a test's cleaner is the one the worker's input reaches.
    static ONE: Mutex<()> = Mutex::new(());

    fn cleaner(cleaning: Cleaning) -> (Cleaner, Arc<Chosen>) {
        let chosen = Arc::new(Chosen::new(cleaning));
        (Cleaner::new(chosen.clone()), chosen)
    }

    /// 20 ms of stereo: a 440 Hz tone over hiss, the same in both channels.
    fn frame(seed: &mut u32) -> Vec<i16> {
        (0..FRAME)
            .flat_map(|at| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let hiss = f32::from((*seed >> 16) as u16 as i16) / 8.0;
                let tone = 6000.0 * (at as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
                let sample = (tone + hiss) as i16;
                [sample, sample]
            })
            .collect()
    }

    #[test]
    fn what_is_chosen_travels_in_one_atomic() {
        let both = Cleaning {
            noise: Suppressor::DeepFilterNet,
            noise_strength: 70,
            own_echo: true,
            echo_depth: 35,
        };
        let chosen = Chosen::new(both);
        assert_eq!(chosen.get(), both);
        chosen.set(Cleaning::NOTHING);
        assert_eq!(chosen.get(), Cleaning::NOTHING);
        let rnnoise = Cleaning {
            noise: Suppressor::RNNoise,
            ..Cleaning::NOTHING
        };
        assert_eq!(unpack(pack(rnnoise)), rnnoise);
    }

    #[test]
    fn off_a_frame_plays_as_it_came_and_nothing_is_made() {
        let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut cleaner, _) = cleaner(Cleaning::NOTHING);
        let mut pcm = frame(&mut 1);
        let came = pcm.clone();
        let played = cleaner.clean(&mut pcm, 2).to_vec();
        assert_eq!(played, came);
        assert!(cleaner.rnnoise.is_none() && cleaner.gate.is_none());
        assert_eq!(cleaner.out.capacity(), 0, "no buffer made");
    }

    #[test]
    fn rnnoise_cleans_whole_twenty_ms_in_mono_and_goes_when_turned_off() {
        let rnnoise = Cleaning {
            noise: Suppressor::RNNoise,
            ..Cleaning::NOTHING
        };
        let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut cleaner, chosen) = cleaner(rnnoise);
        let mut seed = 7;
        // A frame of 10 ms waits for the next to make 20.
        let mut half = frame(&mut seed)[..FRAME].to_vec();
        assert!(cleaner.clean(&mut half, 2).is_empty());
        let mut pcm = frame(&mut seed);
        let played = cleaner.clean(&mut pcm, 2).to_vec();
        assert_eq!(played.len(), 2 * FRAME, "20 ms, both channels");
        assert!(
            played
                .as_chunks::<2>()
                .0
                .iter()
                .all(|pair| pair[0] == pair[1]),
            "mono"
        );
        assert!(cleaner.rnnoise.is_some());
        chosen.set(Cleaning::NOTHING);
        cleaner.idle();
        assert!(cleaner.rnnoise.is_none(), "it goes once turned off");
    }

    #[test]
    fn the_echo_gate_takes_what_the_call_sent_and_goes_when_turned_off() {
        let echo = Cleaning {
            own_echo: true,
            ..Cleaning::NOTHING
        };
        let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut cleaner, chosen) = cleaner(echo);
        let mut pcm = frame(&mut 3);
        let played = cleaner.clean(&mut pcm, 2).len();
        assert_eq!(played, 2 * FRAME);
        assert!(cleaner.gate.is_some());
        // What the call sent reaches it through the worker's input, and
        // the gate takes it between frames.
        sent(vec![Ours::NOTHING; 3]);
        assert_eq!(cleaner.sent.try_iter().count(), 3);
        sent(vec![Ours::NOTHING; 2]);
        cleaner.idle();
        assert_eq!(cleaner.sent.try_iter().count(), 0, "taken");
        chosen.set(Cleaning::NOTHING);
        cleaner.idle();
        assert!(cleaner.gate.is_none());
    }
}
