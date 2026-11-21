//! What a real-time audio thread needs made anew, made on an ordinary thread
//! and handed over. Loading DeepFilterNet's model takes about 0.2 s of a
//! core, as long as Linux lets a real-time thread run without waiting before
//! it kills the whole program (`audio_io`'s RLIMIT_RTTIME), and WebRTC's
//! echo cancellation sets up its buffers and filters as it's made. So a
//! real-time thread never makes such a thing itself: as another is wanted,
//! [`Rebuild::poll`] starts a thread that makes it, while the one in use
//! goes on working, so the sound has no gap; the next block after it's
//! made, it's handed over, to take the old one's place, and the old one
//! goes back to that thread to be dropped there ([`Rebuild::retire`]), a
//! model's 39 MiB freed off the real-time thread too. The thread then ends:
//! it lives only while something is being made or waits to go back.
//!
//! One is made at a time. Another choice made meanwhile waits for it; what
//! was made then goes back unused, and what's wanted by then is made from
//! the next block, so the newest choice wins and at most two are held, the
//! one in use and the one being made, beside one going back for a moment.

use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
};

/// Makes `T`s for a real-time thread on threads of their own, each for a
/// `K`: what it's made for, such as the suppressor chosen.
pub(crate) struct Rebuild<K, T> {
    /// The name of the threads that make them.
    name: &'static str,
    building: Option<Building<K, T>>,
    /// Where the one the last made replaces goes ([`Rebuild::retire`]).
    back: Option<SyncSender<T>>,
    /// What wasn't made, its thread having failed: not tried again until
    /// something else is wanted.
    failed: Option<K>,
}

/// One being made, on its thread.
struct Building<K, T> {
    made_for: K,
    made: Receiver<T>,
    /// Takes what it replaces, or itself unused, back to its thread.
    back: SyncSender<T>,
}

impl<K: Copy + PartialEq, T: Send + 'static> Rebuild<K, T> {
    /// Its threads take `name`.
    pub(crate) const fn new(name: &'static str) -> Self {
        Self {
            name,
            building: None,
            back: None,
            failed: None,
        }
    }

    /// Follows what's `wanted`, once a block, for an object in use that was
    /// made for `has`: returns one made for `wanted` once it's ready, to take
    /// the old one's place, which then goes to [`Self::retire`]. While they
    /// differ and nothing is being made, `make` starts on a thread of its
    /// own; otherwise it isn't used. Costs a channel's look while something
    /// is being made, a thread's start as it begins, and nothing otherwise.
    pub(crate) fn poll(
        &mut self,
        has: K,
        wanted: K,
        make: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        // The one replaced last time wasn't handed back: its thread ends.
        self.back = None;
        if self.failed.is_some_and(|failed| failed != wanted) {
            self.failed = None;
        }
        let Some(building) = self.building.take() else {
            if has != wanted && self.failed != Some(wanted) {
                self.start(wanted, make);
            }
            return None;
        };
        match building.made.try_recv() {
            Ok(made) if building.made_for == wanted && has != wanted => {
                self.back = Some(building.back);
                Some(made)
            }
            // Made for what's no longer wanted: back it goes, and what is
            // wanted now is made from the next block, once it's gone.
            Ok(unused) => {
                give_back(&building.back, unused);
                None
            }
            Err(TryRecvError::Empty) => {
                self.building = Some(building);
                None
            }
            Err(TryRecvError::Disconnected) => {
                tracing::warn!(
                    thread = self.name,
                    "an audio thread's processing wasn't made anew; what it had goes on"
                );
                self.failed = Some(building.made_for);
                None
            }
        }
    }

    /// Sends `old`, which the one [`Self::poll`] gave last replaced, to the
    /// thread that made that one, to be dropped there: before polling
    /// again, which lets that thread go.
    pub(crate) fn retire(&mut self, old: T) {
        match self.back.take() {
            Some(back) => give_back(&back, old),
            None => drop(old),
        }
    }

    /// Whether something is being made.
    #[cfg(test)]
    pub(crate) fn building(&self) -> bool {
        self.building.is_some()
    }

    fn start(&mut self, wanted: K, make: impl FnOnce() -> T + Send + 'static) {
        let (made, taken) = mpsc::sync_channel(1);
        let (back, returned) = mpsc::sync_channel(1);
        // Started from a thread raised with SCHED_RESET_ON_FORK, it begins
        // at normal priority.
        let spawned = thread::Builder::new()
            .name(self.name.to_owned())
            .spawn(move || {
                // No one waits for it any more: it goes here.
                if made.send(make()).is_err() {
                    return;
                }
                drop(made);
                // What it replaces comes back to be dropped here, or itself
                // unused; or nothing, once the audio thread lets go.
                drop(returned.recv());
            });
        match spawned {
            Ok(_) => {
                tracing::debug!(
                    thread = self.name,
                    "making an audio thread's processing anew"
                );
                self.building = Some(Building {
                    made_for: wanted,
                    made: taken,
                    back,
                });
            }
            Err(error) => {
                tracing::warn!(
                    thread = self.name,
                    %error,
                    "couldn't start making an audio thread's processing anew; what it had goes on"
                );
                self.failed = Some(wanted);
            }
        }
    }
}

impl<K, T> Drop for Rebuild<K, T> {
    /// What was made and not taken goes back to be dropped on its thread.
    fn drop(&mut self) {
        if let Some(building) = self.building.take()
            && let Ok(made) = building.made.try_recv()
        {
            give_back(&building.back, made);
        }
    }
}

/// Hands `object` back to the thread that made what it goes with. Its
/// channel holds one, and only this goes through it; a thread gone, as one
/// that panicked, leaves it to be dropped here.
fn give_back<T>(back: &SyncSender<T>, object: T) {
    let _ = back.try_send(object);
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex, PoisonError,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::*;

    /// Each probe dropped, by its number, with the name of the thread it
    /// was dropped on.
    type Drops = Arc<Mutex<Vec<(u32, Option<String>)>>>;

    struct Probe {
        number: u32,
        drops: Drops,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            let thread = thread::current().name().map(str::to_owned);
            self.drops
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((self.number, thread));
        }
    }

    fn probe(number: u32, drops: &Drops) -> Probe {
        Probe {
            number,
            drops: drops.clone(),
        }
    }

    /// Makes probe `number` once `go` says so, or goes away.
    fn made_on(
        go: mpsc::Receiver<()>,
        number: u32,
        drops: &Drops,
    ) -> impl FnOnce() -> Probe + Send + 'static {
        let drops = drops.clone();
        move || {
            let _ = go.recv();
            probe(number, &drops)
        }
    }

    /// Makes probe `number` at once, counting in `makers` that it started.
    fn made_now(
        number: u32,
        drops: &Drops,
        makers: &Arc<AtomicUsize>,
    ) -> impl FnOnce() -> Probe + Send + 'static {
        let (drops, makers) = (drops.clone(), makers.clone());
        move || {
            makers.fetch_add(1, Ordering::Relaxed);
            probe(number, &drops)
        }
    }

    /// Polls once a millisecond, as a real-time thread does once a block,
    /// until a probe comes, which it returns.
    fn until_made(
        rebuild: &mut Rebuild<u32, Probe>,
        has: u32,
        wanted: u32,
        drops: &Drops,
        makers: &Arc<AtomicUsize>,
    ) -> Probe {
        let started = Instant::now();
        loop {
            if let Some(made) = rebuild.poll(has, wanted, made_now(wanted, drops, makers)) {
                return made;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "made in time");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// The name of the thread probe `number` was dropped on, once it is.
    fn dropped_on(drops: &Drops, number: u32) -> Option<String> {
        let started = Instant::now();
        loop {
            let found = (drops.lock().unwrap_or_else(PoisonError::into_inner).iter())
                .find(|(dropped, _)| *dropped == number)
                .map(|(_, thread)| thread.clone());
            if let Some(thread) = found {
                return thread;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "probe {number} dropped in time"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn was_dropped(drops: &Drops, number: u32) -> bool {
        (drops.lock().unwrap_or_else(PoisonError::into_inner).iter())
            .any(|(dropped, _)| *dropped == number)
    }

    /// The threads of this process named `name`, once none is left.
    #[cfg(target_os = "linux")]
    fn until_gone(name: &str) {
        let started = Instant::now();
        let running = || {
            std::fs::read_dir("/proc/self/task")
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|task| std::fs::read_to_string(task.path().join("comm")).ok())
                .filter(|comm| comm.trim_end() == name)
                .count()
        };
        while running() > 0 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the thread that made it ends"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn until_gone(_: &str) {}

    #[test]
    fn the_old_one_works_until_the_new_one_is_made_and_goes_where_that_was() {
        const NAME: &str = "test-rebuild-a";
        let drops = Drops::default();
        let makers = Arc::new(AtomicUsize::new(0));
        let mut rebuild = Rebuild::new(NAME);
        let mut current = probe(1, &drops);
        assert!(rebuild.poll(1, 1, made_now(1, &drops, &makers)).is_none());
        assert!(!rebuild.building(), "nothing to make");
        let (go, wait) = mpsc::channel();
        assert!(rebuild.poll(1, 2, made_on(wait, 2, &drops)).is_none());
        assert!(rebuild.building());
        // While it's made, the old one stays, and no other starts.
        for _ in 0..5 {
            assert!(rebuild.poll(1, 2, made_now(9, &drops, &makers)).is_none());
            assert_eq!(current.number, 1);
        }
        assert_eq!(makers.load(Ordering::Relaxed), 0, "one at a time");
        let _ = go.send(());
        let made = until_made(&mut rebuild, 1, 2, &drops, &makers);
        assert_eq!(made.number, 2);
        let old = std::mem::replace(&mut current, made);
        rebuild.retire(old);
        assert_eq!(
            dropped_on(&drops, 1).as_deref(),
            Some(NAME),
            "the old one went to be dropped where the new one was made"
        );
        until_gone(NAME);
        assert!(rebuild.poll(2, 2, made_now(9, &drops, &makers)).is_none());
        assert!(!rebuild.building());
        assert_eq!(makers.load(Ordering::Relaxed), 0);
        assert!(!was_dropped(&drops, 2), "the new one is in use");
    }

    #[test]
    fn what_is_chosen_while_another_is_made_wins_and_that_one_goes_unused() {
        const NAME: &str = "test-rebuild-b";
        let drops = Drops::default();
        let makers = Arc::new(AtomicUsize::new(0));
        let mut rebuild = Rebuild::new(NAME);
        let mut current = probe(1, &drops);
        let (go, wait) = mpsc::channel();
        assert!(rebuild.poll(1, 2, made_on(wait, 2, &drops)).is_none());
        // 3 is chosen while 2 is made: it waits for 2.
        assert!(rebuild.poll(1, 3, made_now(3, &drops, &makers)).is_none());
        assert_eq!(makers.load(Ordering::Relaxed), 0, "one at a time");
        let _ = go.send(());
        let made = until_made(&mut rebuild, 1, 3, &drops, &makers);
        assert_eq!(made.number, 3, "the newest choice");
        assert_eq!(
            dropped_on(&drops, 2).as_deref(),
            Some(NAME),
            "the one made for nothing went back unused"
        );
        let old = std::mem::replace(&mut current, made);
        rebuild.retire(old);
        assert_eq!(dropped_on(&drops, 1).as_deref(), Some(NAME));
        assert_eq!(makers.load(Ordering::Relaxed), 1, "3 made once");
        until_gone(NAME);
        // Chosen back while another is made, the one in use stays, and the
        // other goes back unused once it's made.
        let (go, wait) = mpsc::channel();
        assert!(rebuild.poll(3, 4, made_on(wait, 4, &drops)).is_none());
        assert!(rebuild.poll(3, 3, made_now(9, &drops, &makers)).is_none());
        let _ = go.send(());
        let started = Instant::now();
        while !was_dropped(&drops, 4) {
            assert!(rebuild.poll(3, 3, made_now(9, &drops, &makers)).is_none());
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "4 goes in time"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(dropped_on(&drops, 4).as_deref(), Some(NAME));
        assert_eq!(current.number, 3);
        assert_eq!(makers.load(Ordering::Relaxed), 1, "nothing more made");
        // Made and not taken as the audio thread lets go, it's dropped
        // where it was made too.
        let (go, wait) = mpsc::channel();
        assert!(rebuild.poll(3, 5, made_on(wait, 5, &drops)).is_none());
        drop(go);
        thread::sleep(Duration::from_millis(50));
        drop(rebuild);
        assert_eq!(dropped_on(&drops, 5).as_deref(), Some(NAME));
    }

    #[test]
    fn a_maker_that_fails_leaves_what_was_there_and_isnt_tried_again_for_the_same() {
        let mut rebuild = Rebuild::<u32, u32>::new("test-rebuild-c");
        let tries = Arc::new(AtomicUsize::new(0));
        let failing = |tries: &Arc<AtomicUsize>| {
            let tries = tries.clone();
            move || -> u32 {
                tries.fetch_add(1, Ordering::Relaxed);
                panic!("the model didn't load")
            }
        };
        let started = Instant::now();
        assert!(rebuild.poll(1, 2, failing(&tries)).is_none());
        while rebuild.building() {
            assert!(rebuild.poll(1, 2, failing(&tries)).is_none());
            assert!(started.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..5 {
            assert!(rebuild.poll(1, 2, failing(&tries)).is_none());
        }
        assert_eq!(tries.load(Ordering::Relaxed), 1, "not tried again");
        // Something else chosen is made.
        let started = Instant::now();
        loop {
            if let Some(made) = rebuild.poll(1, 3, || 3) {
                assert_eq!(made, 3);
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(1));
        }
    }
}
