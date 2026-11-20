//! DeepFilterNet on others' voices in a Discord call: the strong choice in
//! the submenu of someone's "Suppress their noise" (`volume::Cleaning`).
//! Its model isn't Send and takes about 0.2 s of a core to load, which the
//! call's thread can't spend without the speakers running dry, so it runs on
//! a thread of its own, `yapfast-their-noise`: one for the call, however
//! many it's on for.
//!
//! The call's thread starts it the first time someone it's on for speaks,
//! and lets it go, model and all, once none of them has spoken for 10 s,
//! once it's on for no one, while this account is deafened, and with the
//! call (`discord_audio::People`). The model loads there once; each person
//! gets a copy of it as they first speak ([`crate::noise::Deep`]), which
//! shares the model's weights and keeps its own state, and loses it after
//! 10 s without them. Until the model has loaded, RNNoise stands in;
//! DeepFilterNet takes over as they next start speaking, so their voice
//! never jumps by the 40 ms between the two in the middle of a word.
//!
//! Each tick the call's thread hands the thread their 20 ms
//! ([`Theirs::hand`]) and plays what it gave back of the tick before: they
//! play a tick later than DeepFilterNet's own 30 ms, 50 ms in all, and a tick
//! the thread is late with plays silent rather than holding up the mix. The
//! 20 ms go there and back in [`FRAMES`] buffers made as the person first
//! speaks, through bounded channels, so the call's thread allocates nothing
//! as it goes. DeepFilterNet takes about 270 µs a 10 ms block (82 µs its
//! encoder, 105 and 82 its two decoders), 2.7% of a core for each person
//! while they speak.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use yapfast_discord::voice::FRAME;

use crate::discord_audio::CLEANED_AT_ONCE;
use crate::noise::{BLOCK, Deep};

/// The buffers each person's 20 ms go to the thread and back in: one filled,
/// one there, one back to play, and one for a tick the thread is late with.
const FRAMES: usize = 4;
/// What each channel holds: every buffer of as many people as a call cleans
/// at once, and word to let some go.
const QUEUE: usize = CLEANED_AT_ONCE * (FRAMES + 1);
/// Someone's copy of the model goes after this long without their voice, as
/// the call forgets a speaker.
const AWAY: Duration = Duration::from_secs(10);
/// How often the thread looks for copies to let go of while nothing comes.
const SWEEP: Duration = Duration::from_secs(1);

/// The thread's state: loading the model, cleaning, or its model didn't load.
const LOADING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

/// DeepFilterNet's thread for others' voices, as the call's thread holds it.
/// Dropped, the thread ends at its next word or second, its model and
/// copies with it; nothing waits for it.
pub(crate) struct Strong {
    jobs: SyncSender<Job>,
    done: Receiver<Done>,
    state: Arc<AtomicU8>,
}

enum Job {
    /// 20 ms of someone's voice, mono at 48 kHz in 16-bit range, to clean,
    /// turning noise down at most `limit` dB.
    Clean {
        user: u64,
        tick: u64,
        limit: f32,
        samples: Vec<f32>,
    },
    /// Their copy of the model goes.
    Forget(u64),
}

impl Job {
    /// The buffer a job took, back.
    fn samples(self) -> Vec<f32> {
        match self {
            Job::Clean { samples, .. } => samples,
            Job::Forget(_) => Vec::new(),
        }
    }
}

/// Someone's 20 ms of tick `tick`, cleaned, 30 ms late as DeepFilterNet is.
pub(crate) struct Done {
    pub(crate) user: u64,
    tick: u64,
    samples: Vec<f32>,
}

impl Strong {
    /// Starts the thread, which loads the model; `None` if it couldn't
    /// start.
    pub(crate) fn start() -> Option<Self> {
        let (jobs, taken) = mpsc::sync_channel(QUEUE);
        let (gave, done) = mpsc::sync_channel(QUEUE);
        let state = Arc::new(AtomicU8::new(LOADING));
        let shared = Arc::clone(&state);
        // Started from the call's thread, it begins at normal priority:
        // `audio_io` raises that one with SCHED_RESET_ON_FORK.
        let started = thread::Builder::new()
            .name("yapfast-their-noise".into())
            .spawn(move || work(&taken, &gave, &shared));
        match started {
            Ok(_) => Some(Self { jobs, done, state }),
            Err(error) => {
                tracing::warn!(%error, "DeepFilterNet's thread for others' voices didn't start");
                None
            }
        }
    }

    /// The model has loaded: what comes is cleaned.
    pub(crate) fn ready(&self) -> bool {
        self.state.load(Ordering::Acquire) == READY
    }

    /// The model didn't load, or the thread is gone.
    pub(crate) fn failed(&self) -> bool {
        self.state.load(Ordering::Acquire) == FAILED
    }

    /// Lets `user`'s copy of the model go. With the channel full, the
    /// thread lets it go itself, [`AWAY`] after their last 20 ms.
    pub(crate) fn forget(&self, user: u64) {
        let _ = self.jobs.try_send(Job::Forget(user));
    }

    /// The next 20 ms the thread gave back, if one came.
    pub(crate) fn take(&self) -> Option<Done> {
        self.done.try_recv().ok()
    }

    /// The next 20 ms the thread gives back, waiting for it a while.
    #[cfg(test)]
    pub(crate) fn wait(&self) -> Option<Done> {
        self.done.recv_timeout(Duration::from_secs(30)).ok()
    }

    /// Hands the thread 20 ms of `user`'s, or gives the buffer back when it
    /// can't take it: full, or gone, which [`Self::failed`] then says.
    fn clean(&self, user: u64, tick: u64, limit: f32, samples: Vec<f32>) -> Result<(), Vec<f32>> {
        let job = Job::Clean {
            user,
            tick,
            limit,
            samples,
        };
        match self.jobs.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(job)) => Err(job.samples()),
            Err(TrySendError::Disconnected(job)) => {
                self.state.store(FAILED, Ordering::Release);
                Err(job.samples())
            }
        }
    }
}

/// The thread: loads the model, then cleans each 20 ms that comes with the
/// copy of its person, until the call's thread lets go of it.
fn work(jobs: &Receiver<Job>, done: &SyncSender<Done>, state: &AtomicU8) {
    let started = Instant::now();
    let model = match Deep::model() {
        Ok(model) => model,
        Err(error) => {
            tracing::warn!(
                ?error,
                "DeepFilterNet didn't load for others' voices; RNNoise stands in"
            );
            state.store(FAILED, Ordering::Release);
            return;
        }
    };
    tracing::debug!(took = ?started.elapsed(), "DeepFilterNet loaded for others' voices");
    state.store(READY, Ordering::Release);
    let mut people: HashMap<u64, (Deep, Instant)> = HashMap::new();
    loop {
        match jobs.recv_timeout(SWEEP) {
            Ok(Job::Clean {
                user,
                tick,
                limit,
                mut samples,
            }) => {
                let (deep, heard) = people
                    .entry(user)
                    .or_insert_with(|| (Deep::of(&model), Instant::now()));
                *heard = Instant::now();
                deep.limit(limit);
                for block in samples.chunks_exact_mut(BLOCK) {
                    deep.block(block);
                }
                // The call's thread takes what comes every tick, and never
                // has more out than the channel holds; gone, so is this.
                if done
                    .send(Done {
                        user,
                        tick,
                        samples,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(Job::Forget(user)) => {
                people.remove(&user);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        people.retain(|_, (_, heard)| heard.elapsed() < AWAY);
    }
}

/// Someone's side of the thread, on the call's thread: their buffers at
/// hand, those back from the thread to play, the oldest first, and how many
/// are out there.
pub(crate) struct Theirs {
    spare: Vec<Vec<f32>>,
    ready: VecDeque<(u64, Vec<f32>)>,
    out: usize,
}

impl Theirs {
    /// Made as they first speak with DeepFilterNet on: about 15 KB of
    /// buffers, made once.
    pub(crate) fn new() -> Self {
        Self {
            spare: (0..FRAMES).map(|_| Vec::with_capacity(FRAME)).collect(),
            ready: VecDeque::with_capacity(FRAMES),
            out: 0,
        }
    }

    /// Hands the thread `user`'s 20 ms of tick `tick`, in `mono` (16-bit
    /// range), to clean at most `limit` dB, and puts in `mono` the oldest
    /// they gave back of what was said since tick `since`, when they started
    /// speaking: silence while none has come. Past one tick waiting, the
    /// oldest goes unplayed, so they're never more than a tick behind the
    /// thread. Whether the thread is there to take it; if not, `mono` is as
    /// it came.
    pub(crate) fn hand(
        &mut self,
        strong: &Strong,
        user: u64,
        tick: u64,
        since: u64,
        limit: f32,
        mono: &mut [f32],
    ) -> bool {
        if strong.failed() {
            return false;
        }
        if let Some(mut samples) = self.spare.pop() {
            samples.clear();
            samples.extend_from_slice(mono);
            match strong.clean(user, tick, limit, samples) {
                Ok(()) => self.out += 1,
                Err(samples) => {
                    self.spare.push(samples);
                    if strong.failed() {
                        return false;
                    }
                }
            }
        }
        loop {
            let stale = match self.ready.front() {
                Some((at, _)) => *at < since || self.ready.len() > 2,
                None => false,
            };
            if !stale {
                break;
            }
            if let Some((_, samples)) = self.ready.pop_front() {
                self.spare.push(samples);
            }
        }
        match self.ready.pop_front() {
            Some((_, samples)) => {
                mono.copy_from_slice(&samples);
                self.spare.push(samples);
            }
            None => mono.fill(0.0),
        }
        true
    }

    /// 20 ms the thread gave back: kept to play, while one of theirs is out
    /// there. One of a person whose side was made anew counts as theirs, so
    /// they hold [`FRAMES`] buffers however they come back.
    pub(crate) fn came(&mut self, done: Done) {
        if self.out > 0 {
            self.out -= 1;
            self.ready.push_back((done.tick, done.samples));
        }
    }

    /// How many of theirs are out there.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.out
    }
}
