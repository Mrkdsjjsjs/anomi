//! Anomi: whether a video frame's bytes stay as they are from when they're
//! lent to the window until a renderer has read them, for a review:
//! `ANOMI_FRAMES_VERIFY` set. A frame lies in memory its writer gets back
//! only once the window read a newer one (`premultiplied::frame_of`); a
//! frame whose bytes changed after they were lent ([`lent`]), or between
//! the hash before a read and the hash after it, was written under the
//! window, and shows a later frame, or torn. Each renderer hashes a frame
//! around the read (`raster.rs` here, `image/cache.rs` in
//! `vendor/iced_wgpu`). Off, it costs a load.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::core::image::{Handle, Id};

/// The words hashed, spread evenly over a frame.
const WORDS: usize = 256;
/// The frames lent lately whose hash is kept, across every source.
const KEPT: usize = 32;

static CHANGED: AtomicUsize = AtomicUsize::new(0);
static LENT: Mutex<VecDeque<(Id, u64)>> = Mutex::new(VecDeque::new());

/// Whether frames are checked: `ANOMI_FRAMES_VERIFY` set, read once.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();

    *ON.get_or_init(|| std::env::var_os("ANOMI_FRAMES_VERIFY").is_some())
}

/// `bytes` hashed at [`WORDS`] eight-byte words spread evenly over them.
pub fn hash(bytes: &[u8]) -> u64 {
    let words = bytes.len() / 8;
    let step = (words / WORDS).max(1);

    (0..words.min(WORDS))
        .map(|index| index * step * 8)
        .filter_map(|at| bytes.get(at..at + 8))
        .fold(0xcbf2_9ce4_8422_2325, |hash: u64, word| {
            let word = u64::from_ne_bytes(word.try_into().unwrap_or_default());

            (hash.rotate_left(5) ^ word).wrapping_mul(0x0100_0000_01b3)
        })
}

/// A frame lent to the window, its bytes as they are now: a read of them
/// later finds them the same, or they were written under the window.
pub fn lent(handle: &Handle) {
    let Handle::Rgba { id, pixels, .. } = handle else {
        return;
    };

    if !on() {
        return;
    }

    let mut lent = LENT.lock().unwrap_or_else(PoisonError::into_inner);

    if lent.len() == KEPT {
        let _ = lent.pop_front();
    }

    lent.push_back((*id, hash(pixels)));
}

/// After the read of `bytes`, which hashed to `before` ahead of it (none
/// while the check is off): a warning, naming `id`, if they changed since
/// they were lent or during the read.
pub fn after(before: Option<u64>, bytes: &[u8], id: &Id) {
    let Some(before) = before else {
        return;
    };

    let lent = LENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .rev()
        .find(|(lent, _)| lent == id)
        .map(|(_, hash)| *hash);

    let when = if hash(bytes) != before {
        "while they were drawn"
    } else if lent.is_some_and(|lent| lent != before) {
        "after they were lent, before they were drawn"
    } else {
        return;
    };

    let changed = CHANGED.fetch_add(1, Ordering::Relaxed) + 1;

    log::warn!("a frame's bytes changed {when}: {id:?}, {changed} so far");
}

/// How many reads of a frame found its bytes changed, so far.
pub fn changed() -> usize {
    CHANGED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_anywhere_a_word_is_read_changes_the_hash() {
        let mut frame = vec![7_u8; 1280 * 720 * 4];
        let before = hash(&frame);
        assert_eq!(hash(&frame), before, "the same bytes");
        // 460,800 words: the 100th read lies 100 steps of 1800 words in.
        frame[100 * 1800 * 8 + 3] = 8;
        assert_ne!(hash(&frame), before);
        assert_eq!(hash(&[1, 2, 3]), hash(&[]), "no whole word");
    }
}
