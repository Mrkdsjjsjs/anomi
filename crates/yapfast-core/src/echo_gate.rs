//! Takes our own voice out of someone else's in a Discord call, when their
//! speakers play it back into their microphone: an echo gate, one for each
//! person it's on for, on the call's thread.
//!
//! Each 20 ms tick it measures how loud what this account sent was
//! ([`EchoGate::sent`]) and how loud what came from them is
//! ([`EchoGate::heard`]), in decibels in eight bands from 200 Hz to 8 kHz.
//! The echo's delay, up to 1.5 s, is the lag at which their levels rise and
//! fall with ours: a normalised cross-correlation of the two, kept over about
//! 3 s, over the ticks when something of ours could be coming back. Once one
//! lag stands out of the rest for half a second, after two seconds of that,
//! the gate follows its peak as the delay drifts. From then on it predicts our
//! echo in each band, from what we sent around that lag and how much quieter
//! it comes back, which it learns as each band's running median. While what
//! comes from them is no louder than that, by 6 dB in all but a band or two,
//! it turns them down 20 dB, or as far as [`EchoGate::set_depth`] says, over
//! 60 ms, and keeps them down through the pauses between our words; once
//! they're louder than our echo in several bands, it lets them through again
//! within 5 ms.
//!
//! It works like a speakerphone: while our echo comes back, they come through
//! only by talking louder than it, and the first 20 to 40 ms of what they say
//! over it can be lost. Until it has found the delay it does nothing, and when
//! the echo no longer shows for 30 s of our speech, as after they put on
//! headphones, it stops until it finds it again. It takes out our own voice
//! only: not others', nor the sounds their speakers play.
//!
//! After [`EchoGate::new`] it allocates nothing. A tick costs two 320-point
//! FFTs, one of ours and one of theirs, and sums over 76 lags of 8 bands: a
//! few µs. Once we have said nothing for 1.6 s, what comes from them costs
//! nothing. A gate keeps about 7 KB.
//!
//! What we sent can be measured once ([`Measure`]) into an [`Ours`] of 36
//! bytes for every gate ([`EchoGate::take`]), and carried to a gate in
//! another process: a watched stream's, in its worker, whose sound plays
//! there and doesn't come in step with the call's ticks
//! ([`EchoGate::heard_unpaced`]).

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};
use serde::{Deserialize, Serialize};

/// Samples of a channel in one 20 ms tick at 48 kHz.
const FRAME: usize = 960;
/// A tick is measured at 16 kHz: each three samples averaged into one.
const DECIMATE: usize = 3;
/// Samples measured a tick; their spectrum's bins are 50 Hz apart.
const SIZE: usize = FRAME / DECIMATE;
/// Bands measured.
const BANDS: usize = 8;
/// The bands' edges, in bins: 200, 400, 700, 1100, 1700, 2600, 3800, 5500
/// and 8000 Hz.
const EDGES: [usize; BANDS + 1] = [4, 8, 14, 22, 34, 52, 76, 110, SIZE / 2 + 1];
/// Scales a band's summed power to its share of the sound's mean square:
/// Parseval, over a one-sided spectrum, through a Hann window, whose mean
/// square is 3/8.
const POWER: f32 = 2.0 / (SIZE as f32 * SIZE as f32 * 0.375);
/// Power under any level measured, so that silence has one.
const SILENT: f32 = 1e-12;

/// The latest our echo is looked for: 1.5 s, in ticks.
const LATEST: usize = 75;
/// The lags looked at: 0 to [`LATEST`] ticks.
const LAGS: usize = LATEST + 1;
/// Our echo at a lag is predicted from the ticks we sent around it: up to
/// this many sent after it, for an echo that comes back a little early or is
/// just starting...
const SOONER: usize = 2;
/// ...and this many before it, for the room's tail.
const LATER: usize = 4;
/// Those ticks, by how many before the lag we sent them, and what each adds
/// to the prediction, in dB.
const AROUND: [(isize, f32); SOONER + 1 + LATER] = [
    (-2, -6.0),
    (-1, 0.0),
    (0, 0.0),
    (1, 0.0),
    (2, 0.0),
    (3, -6.0),
    (4, -12.0),
];
/// Ticks of ours kept: up to the latest lag and the room's tail after it.
const RING: usize = LATEST + LATER + 1;

/// What we sent counts as silent below this, in dB in a band.
const OUR_FLOOR: f32 = -90.0;
/// We speak on a tick louder than this over all bands, in dBFS.
const OUR_SPEECH: f32 = -55.0;
/// A band of ours this loud, in dBFS, tells how loud its echo comes back.
const OUR_BAND: f32 = -75.0;

/// The correlation's sums fade by this a tick: they remember about 3 s.
const FADE: f32 = 1.0 - 1.0 / 150.0;
/// A lag's correlation counts once this many ticks, faded, went into it:
/// about a second. Over its first few, any lag would seem to match.
const ENOUGH: f32 = 50.0;
/// The bands' averages, which the correlation is taken about, follow over
/// about 2 s.
const MEAN_RATE: f32 = 0.01;
/// A lag is our echo's when its correlation is at least this (on made-up
/// voices, our echo's came to 0.9 through Opus both ways, and two voices that
/// had nothing to do with each other reached 0.45 at some lag)...
const STANDS_OUT: f32 = 0.6;
/// ...and this much above the average over all lags...
const ABOVE_THE_REST: f32 = 0.25;
/// ...near the same lag for this many ticks in a row, half a second...
const STEADY: u32 = 25;
/// ...once this many ticks could have had our echo in them: 2 s.
const LEARNING: u32 = 100;
/// Once found, the delay follows the peak this many ticks either way while
/// its correlation holds at least [`HOLDS`].
const FOLLOW: usize = 2;
const HOLDS: f32 = 0.3;
/// Below this our echo seems gone; gone for [`GIVE_UP`] ticks, 30 s of what
/// could have had it, the gate stops until it finds it again.
const GONE: f32 = 0.2;
const GIVE_UP: u32 = 1_500;

/// How far they're turned down while only our echo comes from them, in dB,
/// unless [`EchoGate::set_depth`] says otherwise.
const DEPTH: f32 = 20.0;
/// Turning them down takes 60 ms at 48 kHz, in samples; letting them through
/// again, 5 ms; whatever the depth.
const FALL: f32 = 2_880.0;
const RISE: f32 = 240.0;
/// What comes from them is more than our echo in a band this much louder
/// than the prediction, in dB, and surely more [`SURELY_MORE`].
const MORE: f32 = 6.0;
const SURELY_MORE: f32 = 12.0;
/// Our echo is expected when it should stand [`MORE`] out of their noise in
/// this many bands.
const ECHO_BANDS: usize = 2;
/// They sound like our echo alone when more than it in at most this many
/// bands, and surely more in none.
const ECHO_ALONE_MOST: usize = 1;
/// They sound like themselves when more than our echo in this many bands;
/// surely, when surely more in [`SURELY_THEM_BANDS`].
const THEM_BANDS: usize = 3;
const SURELY_THEM_BANDS: usize = 2;
/// Ticks they stay turned down after our echo was last expected: through the
/// pauses between our words.
const HOLD: u32 = 12;
/// The echo's gain learns this much a tick, in dB: faster until the delay is
/// found and for its first [`LEARN_FAST_TICKS`], and slower on a tick that
/// doesn't sound like our echo alone.
const LEARN_FAST: f32 = 1.0;
const LEARN_FAST_TICKS: u32 = 50;
const LEARN: f32 = 0.3;
const LEARN_SLOW: f32 = 0.05;
/// Their noise floor rises this much a tick, in dB: 1 dB a second.
const FLOOR_RISE: f32 = 0.02;
/// A band of theirs no more than this above their noise floor, in dB, hides
/// whatever echo is under it.
const HIDDEN: f32 = 3.0;

/// What this account sent on one tick, as a gate takes it: its level in
/// each band, in dB, and whether it was speech.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ours {
    pub levels: [f32; BANDS],
    pub spoke: bool,
}

impl Ours {
    /// A tick that sent nothing.
    pub const NOTHING: Self = Self {
        levels: [OUR_FLOOR; BANDS],
        spoke: false,
    };
}

/// Measures 20 ms of sound in the gate's bands: an FFT's buffers, made once.
pub struct Measure {
    fft: Arc<dyn RealToComplex<f32>>,
    window: &'static [f32; SIZE],
    /// A tick as measured, its spectrum, and the FFT's scratch.
    measured: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl Default for Measure {
    fn default() -> Self {
        Self::new()
    }
}

impl Measure {
    pub fn new() -> Self {
        let fft = transform();
        Self {
            measured: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window: window(),
        }
    }

    /// What this account sent this tick, as the call hands it to its
    /// listener's `sent`: 20 ms at 48 kHz, mono or interleaved stereo, at
    /// full scale 1.0, or `None` on a tick that sent nothing.
    pub fn ours(&mut self, frame: Option<&[f32]>) -> Ours {
        let Some(frame) = frame else {
            return Ours::NOTHING;
        };
        let (bands, total) = self.levels(frame);
        Ours {
            levels: bands.map(|level| level.max(OUR_FLOOR)),
            spoke: total > OUR_SPEECH,
        }
    }

    /// The level of 20 ms of sound, mono or interleaved stereo at 48 kHz, in
    /// dB in each band and over all of them.
    fn levels(&mut self, sound: &[f32]) -> ([f32; BANDS], f32) {
        let channels = (sound.len() / FRAME).clamp(1, 2);
        let step = DECIMATE * channels;
        for (index, (value, weight)) in self.measured.iter_mut().zip(self.window).enumerate() {
            let start = index * step;
            let samples = sound
                .get(start..(start + step).min(sound.len()))
                .unwrap_or_default();
            *value = samples.iter().sum::<f32>() / step as f32 * weight;
        }
        if self
            .fft
            .process_with_scratch(&mut self.measured, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return ([decibels(0.0); BANDS], decibels(0.0));
        }
        let (mut bands, mut total) = ([0.0; BANDS], 0.0);
        for (band, edges) in bands.iter_mut().zip(EDGES.windows(2)) {
            let bins = &self.spectrum[edges[0]..edges[1]];
            let power = POWER * bins.iter().map(|bin| bin.norm_sqr()).sum::<f32>();
            total += power;
            *band = decibels(power);
        }
        (bands, decibels(total))
    }
}

/// Turns one person down while what comes from them is only our own voice
/// coming back; see the module. Fed on the call's thread every tick:
/// [`Self::sent`], then, when they spoke, [`Self::heard`].
pub struct EchoGate {
    /// Measures what comes from them, and what we sent when it's handed
    /// over unmeasured.
    measure: Measure,
    /// Our levels by band over the last [`RING`] ticks, the newest at
    /// `newest`, and whether we spoke on each.
    ours: Box<[[f32; BANDS]; RING]>,
    spoke: [bool; RING],
    newest: usize,
    /// Ticks since we last spoke, up to [`RING`].
    quiet_for: usize,
    /// Whether this tick's [`Self::sent`] came, and the ticks since they
    /// were last heard.
    fed: bool,
    unheard: u32,
    /// Our bands' average levels and theirs, and the ticks that went into
    /// each.
    our_mean: [f32; BANDS],
    our_ticks: u32,
    their_mean: [f32; BANDS],
    their_ticks: u32,
    /// For each lag, fading sums: of our levels then about their average
    /// times theirs now about theirs, and of each squared.
    cross: [f32; LAGS],
    our_power: [f32; LAGS],
    their_power: [f32; LAGS],
    /// For each lag, the ticks that went into its sums, faded alike.
    samples: [f32; LAGS],
    /// Their noise floor by band, once they've been heard.
    floor: Option<[f32; BANDS]>,
    /// How much louder each band of our echo comes back than we sent it, in
    /// dB, and how many ticks taught it.
    gain: [f32; BANDS],
    taught: [u32; BANDS],
    /// Ticks heard that could have had our echo in them.
    listened: u32,
    /// The lag that stood out last, and for how many ticks in a row near it.
    candidate: usize,
    steady: u32,
    /// The echo's delay in ticks, once found, and the ticks it hasn't shown
    /// since.
    lag: Option<usize>,
    weak: u32,
    /// Whether they're turned down, the ticks since our echo was last
    /// expected, and whether the last tick heard sounded like them.
    ducking: bool,
    since_echo: u32,
    them_before: bool,
    /// The gain on their sound now and the one it moves to, how far down it
    /// goes, in dB and as a gain, and its steps each sample down and up.
    level: f32,
    target: f32,
    depth: f32,
    ducked: f32,
    fall: f32,
    rise: f32,
}

impl Default for EchoGate {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoGate {
    pub fn new() -> Self {
        Self {
            measure: Measure::new(),
            ours: Box::new([[OUR_FLOOR; BANDS]; RING]),
            spoke: [false; RING],
            newest: 0,
            quiet_for: RING,
            fed: false,
            unheard: 0,
            our_mean: [OUR_FLOOR; BANDS],
            our_ticks: 0,
            their_mean: [0.0; BANDS],
            their_ticks: 0,
            cross: [0.0; LAGS],
            our_power: [0.0; LAGS],
            their_power: [0.0; LAGS],
            samples: [0.0; LAGS],
            floor: None,
            gain: [0.0; BANDS],
            taught: [0; BANDS],
            listened: 0,
            candidate: 0,
            steady: 0,
            lag: None,
            weak: 0,
            ducking: false,
            since_echo: 0,
            them_before: false,
            level: 1.0,
            target: 1.0,
            depth: DEPTH,
            ducked: amplitude(-DEPTH),
            fall: amplitude(-DEPTH / FALL),
            rise: amplitude(DEPTH / RISE),
        }
    }

    /// Turns them down `decibels` while only our echo comes from them, from
    /// the next sample on, over the same 60 ms; 0 turns them down not at
    /// all, while the gate goes on learning. What it learned stays.
    pub fn set_depth(&mut self, decibels: f32) {
        let depth = decibels.max(0.0);
        if depth == self.depth {
            return;
        }
        self.depth = depth;
        self.ducked = amplitude(-depth);
        // At 0 dB the steps still bring back a gain the last depth lowered.
        let ramp = depth.max(1.0);
        self.fall = amplitude(-ramp / FALL);
        self.rise = amplitude(ramp / RISE);
        if self.ducking {
            self.target = self.ducked;
        }
    }

    /// What this account sent this tick, as the call hands it to its
    /// listener's `sent`: 20 ms at 48 kHz, mono or interleaved stereo, at
    /// full scale 1.0; or `None` on a tick that sent nothing, as while muted
    /// or between words. It comes every tick, before [`Self::heard`], whether
    /// they spoke or not: the gate keeps time by it.
    pub fn sent(&mut self, frame: Option<&[f32]>) {
        let ours = self.measure.ours(frame);
        self.take(ours);
    }

    /// What we sent this tick, measured once for every gate
    /// ([`Measure::ours`]): as [`Self::sent`].
    pub fn take(&mut self, ours: Ours) {
        let Ours { levels, spoke } = ours;
        self.newest = (self.newest + 1) % RING;
        self.ours[self.newest] = levels;
        self.spoke[self.newest] = spoke;
        self.quiet_for = if spoke {
            0
        } else {
            (self.quiet_for + 1).min(RING)
        };
        if self.quiet_for < RING {
            self.our_ticks = self.our_ticks.saturating_add(1);
            follow_mean(&mut self.our_mean, &levels, self.our_ticks);
        }
        self.fed = true;
        self.unheard = self.unheard.saturating_add(1);
    }

    /// Their 20 ms, mono or interleaved stereo at 48 kHz, in any scale,
    /// after this tick's [`Self::sent`]: turned down in place while it
    /// carries only our echo.
    pub fn heard(&mut self, sound: &mut [f32]) {
        if !self.fed {
            // The tick's own word of what we sent never came: nothing, then.
            self.sent(None);
        }
        self.heard_unpaced(sound);
    }

    /// Their 20 ms, as [`Self::heard`], for sound that doesn't come in step
    /// with what we sent, as a watched stream's in its worker, where what
    /// the call sent comes through a pipe: the ticks of ours taken since
    /// the last count as passed, and none taken adds no silent tick of
    /// ours, which would run our timeline ahead of the call's.
    pub fn heard_unpaced(&mut self, sound: &mut [f32]) {
        self.fed = false;
        let passed = std::mem::take(&mut self.unheard).max(1);
        if self.quiet_for < RING {
            let (levels, _) = self.measure.levels(sound);
            self.listen(&levels, passed);
        } else {
            // Nothing we sent could be in it.
            self.ducking = false;
            self.them_before = false;
            self.target = 1.0;
        }
        self.apply(sound);
    }

    /// Our echo's delay, once found.
    pub fn delay(&self) -> Option<Duration> {
        self.lag.map(|lag| Duration::from_millis(lag as u64 * 20))
    }

    /// Whether they were turned down on the last tick heard.
    pub fn turned_down(&self) -> bool {
        self.ducking
    }

    /// Learns from their levels this tick, `passed` ticks after the last
    /// they were heard, and decides whether to turn them down.
    fn listen(&mut self, heard: &[f32; BANDS], passed: u32) {
        let floor = self.follow_floor(heard);
        let clamped: [f32; BANDS] = std::array::from_fn(|band| heard[band].max(floor[band]));
        self.their_ticks = self.their_ticks.saturating_add(1);
        follow_mean(&mut self.their_mean, &clamped, self.their_ticks);
        let theirs: [f32; BANDS] =
            std::array::from_fn(|band| clamped[band] - self.their_mean[band]);
        self.correlate(&theirs);
        self.listened = self.listened.saturating_add(1);
        self.find_delay();
        let lag = self.lag.unwrap_or(self.candidate);
        let predicted = self.predict(lag);
        let now = Comparison::of(heard, &predicted, &self.gain, &floor);
        self.learn_gain(heard, &self.at_lag(lag), &floor, now.echo_alone());
        if self.lag.is_some() {
            self.decide(&now, passed);
        } else {
            self.ducking = false;
            self.target = 1.0;
        }
    }

    /// Follows their noise floor, down halfway to a quieter tick and up
    /// [`FLOOR_RISE`] a tick, and returns it.
    fn follow_floor(&mut self, heard: &[f32; BANDS]) -> [f32; BANDS] {
        let known = self.floor.get_or_insert(*heard);
        for (floor, level) in known.iter_mut().zip(heard) {
            if *level < *floor {
                *floor += 0.5 * (*level - *floor);
            } else {
                *floor = (*floor + FLOOR_RISE).min(*level);
            }
        }
        *known
    }

    /// Where in the ring the tick of ours `age` ticks old is.
    fn at(&self, age: usize) -> usize {
        (self.newest + RING - age % RING) % RING
    }

    /// Adds this tick, `theirs` about their average, to the correlation at
    /// each lag that could have our echo in it now: one we spoke near.
    fn correlate(&mut self, theirs: &[f32; BANDS]) {
        let their_power: f32 = theirs.iter().map(|level| level * level).sum();
        // How many ticks we spoke on, up to each age.
        let mut spoke_by = [0u16; RING + 1];
        let mut spoke = 0;
        for (age, count) in spoke_by.iter_mut().skip(1).enumerate() {
            spoke += u16::from(self.spoke[self.at(age)]);
            *count = spoke;
        }
        for lag in 0..LAGS {
            let (first, last) = (lag.saturating_sub(SOONER), lag + LATER);
            if spoke_by[last + 1] == spoke_by[first] {
                continue;
            }
            let ours = &self.ours[self.at(lag)];
            let (mut cross, mut power) = (0.0, 0.0);
            for ((our, mean), their) in ours.iter().zip(&self.our_mean).zip(theirs) {
                let our = our - mean;
                cross += our * their;
                power += our * our;
            }
            self.cross[lag] = self.cross[lag] * FADE + cross;
            self.our_power[lag] = self.our_power[lag] * FADE + power;
            self.their_power[lag] = self.their_power[lag] * FADE + their_power;
            self.samples[lag] = self.samples[lag] * FADE + 1.0;
        }
    }

    /// The correlation of our levels at `lag` with theirs, once enough
    /// ticks went into it.
    fn correlation(&self, lag: usize) -> f32 {
        let power = self.our_power[lag] * self.their_power[lag];
        if power > 0.0 && self.samples[lag] >= ENOUGH {
            self.cross[lag] / power.sqrt()
        } else {
            0.0
        }
    }

    /// Finds the lag whose correlation stands out, takes it for our echo's
    /// once it has for a while, follows it as it moves, and lets it go when
    /// our echo stays gone.
    fn find_delay(&mut self) {
        let (mut best, mut highest, mut sum) = (0, f32::MIN, 0.0);
        for lag in 0..LAGS {
            let correlation = self.correlation(lag);
            sum += correlation;
            if correlation > highest {
                (best, highest) = (lag, correlation);
            }
        }
        let stands_out = highest >= STANDS_OUT && highest - sum / LAGS as f32 >= ABOVE_THE_REST;
        self.steady = if stands_out && best.abs_diff(self.candidate) <= FOLLOW {
            self.steady.saturating_add(1)
        } else {
            u32::from(stands_out)
        };
        self.candidate = best;
        let Some(lag) = self.lag else {
            if self.listened >= LEARNING && self.steady >= STEADY {
                self.lag = Some(best);
                self.weak = 0;
                tracing::debug!(
                    delay_ms = best * 20,
                    correlation = highest,
                    "the echo gate found our echo's delay"
                );
            }
            return;
        };
        if best.abs_diff(lag) <= FOLLOW && highest >= HOLDS {
            self.lag = Some(best);
        } else if self.steady >= STEADY {
            tracing::debug!(
                from_ms = lag * 20,
                to_ms = best * 20,
                "our echo's delay moved"
            );
            self.lag = Some(best);
        }
        let holding = self.correlation(self.lag.unwrap_or(best));
        if holding < GONE {
            self.weak += 1;
        } else if holding >= HOLDS {
            self.weak = 0;
        }
        if self.weak >= GIVE_UP {
            tracing::debug!("the echo gate lost our echo");
            self.lag = None;
            self.weak = 0;
            self.steady = 0;
        }
    }

    /// Our echo at `lag` as we sent it, in dB by band: the loudest of the
    /// ticks [`AROUND`] it.
    fn predict(&self, lag: usize) -> [f32; BANDS] {
        let mut predicted = [OUR_FLOOR; BANDS];
        for (offset, change) in AROUND {
            let Some(age) = lag.checked_add_signed(offset).filter(|age| *age < RING) else {
                continue;
            };
            for (prediction, level) in predicted.iter_mut().zip(&self.ours[self.at(age)]) {
                *prediction = prediction.max(level + change);
            }
        }
        predicted
    }

    /// What we sent that comes back now if our echo is `lag` ticks late, by
    /// band: the louder of the two ticks of ours that a delay between `lag`
    /// and one tick more spans.
    fn at_lag(&self, lag: usize) -> [f32; BANDS] {
        let (one, other) = (&self.ours[self.at(lag)], &self.ours[self.at(lag + 1)]);
        std::array::from_fn(|band| one[band].max(other[band]))
    }

    /// Learns how much louder each band of our echo comes back than we sent
    /// it, from what we sent right at the lag (`sent`): a running median, so
    /// that their own voice now and then doesn't sway it. A band at their
    /// noise `floor` hid the echo: it says only that it came back no louder
    /// than that, which teaches the gain down when the gain says louder, and
    /// nothing otherwise. Once the delay is found and the gain settled, it
    /// learns slower on a tick that doesn't sound like our echo alone, which
    /// their voice over it would otherwise raise.
    fn learn_gain(
        &mut self,
        heard: &[f32; BANDS],
        sent: &[f32; BANDS],
        floor: &[f32; BANDS],
        echo_alone: bool,
    ) {
        let found = self.lag.is_some();
        for band in 0..BANDS {
            let (gain, taught) = (&mut self.gain[band], &mut self.taught[band]);
            if sent[band] <= OUR_BAND {
                continue;
            }
            let hidden = heard[band] <= floor[band] + HIDDEN;
            let came_back = heard[band].max(floor[band] + HIDDEN) - sent[band];
            if hidden && (*taught == 0 || came_back >= *gain) {
                continue;
            }
            *gain = if *taught == 0 {
                came_back
            } else {
                let step = if !found || *taught < LEARN_FAST_TICKS {
                    LEARN_FAST
                } else if echo_alone {
                    LEARN
                } else {
                    LEARN_SLOW
                };
                *gain + step * (came_back - *gain).signum()
            };
            *taught = taught.saturating_add(1);
        }
    }

    /// Turns them down while our echo is expected and they sound like it
    /// alone, and keeps them down through our pauses, until they sound like
    /// themselves: at once when surely, on the second tick when likely.
    fn decide(&mut self, now: &Comparison, passed: u32) {
        let expected = now.expected >= ECHO_BANDS;
        let them_before = self.them_before && passed == 1;
        let them = now.surely >= SURELY_THEM_BANDS || (now.more >= THEM_BANDS && them_before);
        self.them_before = now.more >= THEM_BANDS;
        if self.ducking {
            if them {
                self.ducking = false;
            } else if expected {
                self.since_echo = 0;
            } else {
                self.since_echo = self.since_echo.saturating_add(passed);
                self.ducking = self.since_echo <= HOLD;
            }
        } else if expected && now.echo_alone() {
            self.ducking = true;
            self.since_echo = 0;
        }
        self.target = if self.ducking { self.ducked } else { 1.0 };
    }

    /// Moves the gain on `sound` toward the target sample by sample, and
    /// applies it; nothing at all while they pass in full.
    fn apply(&mut self, sound: &mut [f32]) {
        if self.level == 1.0 && self.target == 1.0 {
            return;
        }
        let channels = (sound.len() / FRAME).clamp(1, 2);
        for samples in sound.chunks_mut(channels) {
            self.level = if self.level > self.target {
                (self.level * self.fall).max(self.target)
            } else {
                (self.level * self.rise).min(self.target)
            };
            for sample in samples {
                *sample *= self.level;
            }
        }
    }
}

/// How what came from them this tick compares with our echo as predicted.
struct Comparison {
    /// Bands in which our echo should stand out of their noise.
    expected: usize,
    /// Bands in which they're louder than our echo, and surely louder.
    more: usize,
    surely: usize,
}

impl Comparison {
    /// Their levels against ours `predicted` at the echo's `gain`, over their
    /// noise `floor`.
    fn of(
        heard: &[f32; BANDS],
        predicted: &[f32; BANDS],
        gain: &[f32; BANDS],
        floor: &[f32; BANDS],
    ) -> Self {
        let mut comparison = Self {
            expected: 0,
            more: 0,
            surely: 0,
        };
        for band in 0..BANDS {
            let echo = predicted[band] + gain[band];
            comparison.expected += usize::from(echo > floor[band] + MORE);
            let above = heard[band] - echo.max(floor[band]);
            comparison.more += usize::from(above > MORE);
            comparison.surely += usize::from(above > SURELY_MORE);
        }
        comparison
    }

    /// Whether it sounds like our echo alone.
    fn echo_alone(&self) -> bool {
        self.more <= ECHO_ALONE_MOST && self.surely == 0
    }
}

/// Moves each band's average toward `levels`: the plain mean of the first
/// ticks, then over about 2 s.
fn follow_mean(mean: &mut [f32; BANDS], levels: &[f32; BANDS], ticks: u32) {
    let rate = (1.0 / ticks.max(1) as f32).max(MEAN_RATE);
    for (mean, level) in mean.iter_mut().zip(levels) {
        *mean += rate * (level - *mean);
    }
}

fn decibels(power: f32) -> f32 {
    10.0 * (power + SILENT).log10()
}

fn amplitude(decibels: f32) -> f32 {
    10f32.powf(decibels / 20.0)
}

/// The FFT every gate measures with, planned once.
fn transform() -> Arc<dyn RealToComplex<f32>> {
    static PLAN: OnceLock<Arc<dyn RealToComplex<f32>>> = OnceLock::new();
    PLAN.get_or_init(|| RealFftPlanner::<f32>::new().plan_fft_forward(SIZE))
        .clone()
}

/// The Hann window every tick is measured through, shared by all gates.
fn window() -> &'static [f32; SIZE] {
    static WINDOW: OnceLock<[f32; SIZE]> = OnceLock::new();
    WINDOW.get_or_init(|| {
        std::array::from_fn(|index| {
            let phase = std::f32::consts::PI * (index as f32 + 0.5) / SIZE as f32;
            phase.sin() * phase.sin()
        })
    })
}

#[cfg(test)]
mod tests;
