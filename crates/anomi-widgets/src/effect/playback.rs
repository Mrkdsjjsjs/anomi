//! When a profile effect plays and which of its layers show: the clock of
//! `ProfileEffect` and the `shown` of `EffectLayerView`
//! (`ui/controls.slint:860`, `:833`), with the wait of Discord's card,
//! which shows an effect only once every layer's picture has come, and
//! half a second after.
//!
//! Slint ticked a `Timer` 24 times a second while the effect played and
//! added 41 ms each time. Here the play's time is the frame times' own,
//! moved on only between two frames it played in, and the widget asks for
//! a frame only when a layer shows or hides, the play ends, its delay ends
//! or its wait for its pictures runs out; the frames between are its
//! pictures' own, asked for by their players.

use std::time::{Duration, Instant};

use super::EffectLayer;

/// How long a card's effect waits, once every layer's picture has come,
/// before its layers' time 0, as Discord's card does.
pub(super) const DELAY: Duration = Duration::from_millis(500);
/// How long it waits for its layers' pictures before it shows as at rest,
/// until they come.
pub(super) const WAIT: Duration = Duration::from_secs(5);

/// Whether `layer` shows `at` milliseconds into the play: from its start
/// for its duration, and, when it loops, again after each delay, or for
/// good without one. `shown` in `EffectLayerView`.
pub(super) fn shown(layer: &EffectLayer, at: u64) -> bool {
    let Some(since) = at.checked_sub(u64::from(layer.start)) else {
        return false;
    };
    let duration = u64::from(layer.duration);
    if !layer.looped {
        return since <= duration;
    }
    let delay = u64::from(layer.loop_delay);
    delay == 0 || since % (duration + delay) <= duration
}

/// The first millisecond after `at` at which `layer` shows or hides; none
/// once it never changes again.
pub(super) fn next_change(layer: &EffectLayer, at: u64) -> Option<u64> {
    let start = u64::from(layer.start);
    let duration = u64::from(layer.duration);
    let Some(since) = at.checked_sub(start) else {
        return Some(start);
    };
    if !layer.looped {
        return (since <= duration).then_some(start + duration + 1);
    }
    let delay = u64::from(layer.loop_delay);
    if delay == 0 {
        return None;
    }
    let cycle = duration + delay;
    let phase = since % cycle;
    Some(if phase <= duration {
        at + (duration - phase) + 1
    } else {
        at + (cycle - phase)
    })
}

/// What an effect shows at a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Look {
    /// It doesn't play: its still.
    #[default]
    Rest,
    /// It's about to: nothing, while its pictures come and for its delay.
    Waiting,
    /// Its layers.
    Playing,
}

/// What a play runs through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Course {
    /// When its last layer ends, in milliseconds of its layers' time: an
    /// effect that doesn't loop is over then.
    pub end: u64,
    /// Some layer loops, so it plays for as long as it's asked.
    pub loops: bool,
    /// How long it waits, once ready, before its layers' time 0.
    pub delay: Duration,
}

/// Where a play is: Slint's `time`, `finished` and `playing`, and how long
/// it has waited.
#[derive(Debug, Default)]
pub(super) struct Playback {
    /// It was asked to play at the frame before; a change starts the play
    /// over, as `changed play => { root.time = 0; }`.
    asked: bool,
    /// How far into the play it is, its delay included.
    time: Duration,
    /// The frame time `time` last moved at, while it played.
    last: Option<Instant>,
    /// The delay it was last given.
    delay: Duration,
    /// The frame time it was first asked at in this play while its
    /// pictures hadn't all come.
    since: Option<Instant>,
    /// Its time moves: it's asked, may play, is ready and isn't over.
    running: bool,
    /// It's asked and may play, but its pictures haven't all come.
    pending: bool,
    look: Look,
}

impl Playback {
    /// Takes the frame at `now`: starts over when `asked` changed, and moves
    /// on while it's asked, `may` play (the window plays and there's
    /// something to), is `ready` (its pictures came, where it waits for
    /// them) and isn't over. Says whether its layers show.
    ///
    /// Its layers show `course.delay` into the play. Until then it waits
    /// and shows nothing, as it does while asked and not ready, unless it
    /// has waited [`WAIT`] for its pictures: then it shows as at rest. An
    /// effect whose layers don't loop is over `course.end` milliseconds
    /// into their time, and shows as at rest from then on until asked
    /// again.
    pub fn frame(
        &mut self,
        asked: bool,
        may: bool,
        ready: bool,
        course: Course,
        now: Instant,
    ) -> bool {
        if asked != self.asked {
            self.asked = asked;
            self.restart();
        }
        let Course { end, loops, delay } = course;
        self.delay = delay;
        let over = |time: Duration| !loops && millis(time.saturating_sub(delay)) > end;
        let goes = asked && may && ready;
        if goes && !over(self.time) {
            if let Some(last) = self.last {
                self.time += now.saturating_duration_since(last);
            }
            self.last = Some(now);
        }
        self.running = goes && !over(self.time);
        if !self.running {
            // Behind another window or held: the time it isn't in front
            // isn't the play's.
            self.last = None;
        }
        self.pending = asked && may && !ready;
        if self.pending {
            self.since.get_or_insert(now);
        }
        let waited = self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= WAIT);
        self.look = if self.running && self.time >= delay {
            Look::Playing
        } else if (self.running || self.pending) && !waited {
            Look::Waiting
        } else {
            Look::Rest
        };
        self.look == Look::Playing
    }

    /// Starts the play over, as a change of what's asked does.
    pub fn restart(&mut self) {
        self.time = Duration::ZERO;
        self.last = None;
        self.since = None;
    }

    /// Its layers showed at the last frame.
    pub fn playing(&self) -> bool {
        self.look == Look::Playing
    }

    /// What it showed at the last frame.
    pub fn look(&self) -> Look {
        self.look
    }

    /// It was asked and might have played at the last frame, but its
    /// pictures hadn't all come.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// How far into its layers' time it is, in milliseconds: 0 until its
    /// delay is over.
    pub fn at(&self) -> u64 {
        millis(self.time.saturating_sub(self.delay))
    }

    /// When a frame is next due for the play itself: while it plays, the
    /// first layer to show or hide or the end of an effect that doesn't
    /// loop; while in its delay, the delay's end; while it waits for its
    /// pictures, the end of [`WAIT`].
    pub fn next(&self, layers: &[EffectLayer], course: Course, now: Instant) -> Option<Instant> {
        let Course { end, loops, delay } = course;
        if self.running {
            let Some(into) = self.time.checked_sub(delay) else {
                return Some(now + (delay - self.time));
            };
            let at = millis(into);
            let ends = (!loops && at <= end).then_some(end + 1);
            let soonest = layers
                .iter()
                .filter_map(|layer| next_change(layer, at))
                .chain(ends)
                .min()?;
            return Some(now + Duration::from_millis(soonest).saturating_sub(into));
        }
        let waited = self.since.filter(|_| self.pending)? + WAIT;
        (waited > now).then_some(waited)
    }
}

fn millis(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
