//! The cap on the cosmetic pictures that play at once. Avatars,
//! decorations, the rail's faces and banners are capped
//! ([`Picture::capped`](super::Picture::capped), [`Player::capped`](super::Player::capped)):
//! each takes a slot while it plays, and at most [`MOVING`] distinct ones
//! play; the others hold their first frame until a slot frees. A call of
//! ten speaking, or a list pointed along quickly, decodes and draws no more
//! than that many. Emoji, GIFs, stickers and an effect's layers aren't
//! capped.
//!
//! A slot is a picture's, not a place's: one avatar shown in a message and
//! in its profile at once takes one.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use iced::advanced::image::Id;

/// The most distinct capped pictures that play at once.
pub const MOVING: usize = 8;

/// How long a slot stays taken after the frame it was last claimed at: a
/// picture between two of its frames keeps it, one no longer playing frees
/// it this long after.
const HOLD: Duration = Duration::from_secs(2);

thread_local! {
    /// The pictures holding a slot, each with the frame time it last
    /// claimed it at.
    static SLOTS: RefCell<Vec<(Id, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// Whether the picture of `id` may play at `now`, a frame's time: it holds
/// a slot, which it keeps, or fewer than [`MOVING`] pictures claimed one
/// within [`HOLD`], and it takes one.
pub(crate) fn claim(id: Id, now: Instant) -> bool {
    SLOTS.with_borrow_mut(|slots| {
        slots.retain(|&(_, last)| now.saturating_duration_since(last) < HOLD);
        if let Some(slot) = slots.iter_mut().find(|(held, _)| *held == id) {
            slot.1 = slot.1.max(now);
            return true;
        }
        let free = slots.len() < MOVING;
        if free {
            slots.push((id, now));
        }
        free
    })
}

#[cfg(test)]
mod tests {
    use iced::widget::image;

    use super::*;

    fn ids(count: usize) -> Vec<Id> {
        (0..count)
            .map(|_| image::Handle::from_rgba(1, 1, vec![0u8; 4]).id())
            .collect()
    }

    #[test]
    fn eight_play_and_the_ninth_is_held() {
        let now = Instant::now();
        let pictures = ids(MOVING + 1);
        for &id in &pictures[..MOVING] {
            assert!(claim(id, now));
        }
        assert!(!claim(pictures[MOVING], now), "held");
        // The eight keep theirs at their next frames.
        let next = now + Duration::from_millis(100);
        for &id in &pictures[..MOVING] {
            assert!(claim(id, next));
        }
        assert!(!claim(pictures[MOVING], next));
    }

    #[test]
    fn a_slot_frees_its_hold_after_its_last_claim() {
        let now = Instant::now();
        let pictures = ids(MOVING + 1);
        for &id in &pictures[..MOVING] {
            assert!(claim(id, now));
        }
        // All but the first go on playing.
        let later = now + HOLD - Duration::from_millis(1);
        for &id in &pictures[1..MOVING] {
            assert!(claim(id, later));
        }
        assert!(!claim(pictures[MOVING], later), "not yet free");
        assert!(claim(pictures[MOVING], now + HOLD), "the first's slot");
        assert!(!claim(pictures[0], now + HOLD), "taken from it");
    }

    #[test]
    fn one_picture_in_two_places_takes_one_slot() {
        let now = Instant::now();
        let pictures = ids(MOVING);
        assert!(claim(pictures[0], now));
        assert!(claim(pictures[0], now), "its second place");
        for &id in &pictures[1..] {
            assert!(claim(id, now));
        }
    }
}
