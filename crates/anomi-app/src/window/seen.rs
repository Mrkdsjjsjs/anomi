//! Whether the desktop paints the window, from its frames alone, with no
//! timer of its own. On Wayland a window draws only as the compositor sends
//! it a frame callback, and GNOME's mutter and Weston send none to a window
//! they don't paint: covered, minimised, on another workspace, the screen
//! off; one on another monitor gets them as ever. KWin goes on drawing a
//! covered window, and X11 and Windows draw at once whatever covers it:
//! there this fires only where nothing draws at all, as for a window
//! minimised on Windows, whose size of 0 iced doesn't draw.
//!
//! Every update asks the window for a redraw (iced redraws every window
//! after one), and so does every frame of a video or a camera taken in
//! (`crate::media::frames`): [`asked`] notes the first redraw asked since
//! the last frame drawn. Each update starts by [`judge`]: a redraw asked
//! [`UNPAINTED`] ago and still not drawn makes the window unseen. The
//! frames' task, which takes frames with no update, only looks ([`due`])
//! and wakes one ([`Heard::Late`]), so the probe turns and the window
//! hears it in one update, never after a frame that came since. Nothing
//! runs while nothing asks to draw; a window covered while quiet is judged
//! at the next thing that wants to draw it, a clock's message, a batch of
//! the backend's, a video's frame.
//!
//! Painted again takes two frames drawn at most [`AGAIN`] apart ([`drawn`],
//! from the window's events): mutter hands a window one frame on every
//! configure, even one it doesn't paint, and the update that frame brings
//! asks another, which only a painted window draws. A first frame brings
//! that update only once every [`REPROBE`]: where the GPU draws straight
//! into a hidden window, each frame waits a second, and an update for each
//! would ask the next without end.
//!
//! A frame counts as the window's filter hears of it, after iced drew it
//! and after any update iced ran inside that redraw: iced stamps a frame on
//! the display's refresh grid, rounded up, so two drawn within one refresh
//! share a stamp, and a redraw asked between them would wait for a frame no
//! stamp passes. Nothing is judged before a renderer's first frame: making
//! one (a window, again out of the tray, or another renderer) blocks iced's
//! loop, and the updates it runs as that frame is drawn would find the ask
//! before it late.
//!
//! Times are milliseconds since the first one read, plus 2, so that 0 and 1
//! are free to mark the probe, and a [`DRAWN`] of 0 says no frame yet.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A redraw asked this long ago and not drawn: the window isn't painted.
pub const UNPAINTED: Duration = Duration::from_secs(1);
/// Two frames drawn this close together: it's painted again.
pub const AGAIN: Duration = Duration::from_millis(500);
/// How long after a first frame drawn unseen another first one brings an
/// update again: where the GPU draws straight into the window, a hidden
/// window's frame waits a second for an image the compositor keeps
/// (`vendor/iced_wgpu`'s compositor), and the update each first frame
/// brings would ask the next at once, a frame a second for good.
pub const REPROBE: Duration = Duration::from_secs(2);

const UNPAINTED_MS: u64 = UNPAINTED.as_millis() as u64;
const AGAIN_MS: u64 = AGAIN.as_millis() as u64;
const REPROBE_MS: u64 = REPROBE.as_millis() as u64;

/// The probe while the window is painted: frames say nothing.
const PAINTED: u64 = 0;
/// The probe while it's unseen and no frame came since.
const WAITING: u64 = 1;

/// What the times count from.
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// The last frame drawn, as the window's filter heard of it; 0 before a
/// renderer's first.
static DRAWN: AtomicU64 = AtomicU64::new(0);
/// The first redraw asked after [`DRAWN`]; one waits while it's later.
static ASKED: AtomicU64 = AtomicU64::new(0);
/// [`PAINTED`], [`WAITING`], or the time of a first frame drawn since the
/// window went unseen.
static PROBE: AtomicU64 = AtomicU64::new(PAINTED);

/// What reaches `update` of the window's sight (`Message::Sight`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Whether the desktop stops drawing a window it doesn't paint, so that
    /// the frames tell (`crate::platform::window`).
    Detected(bool),
    /// A redraw asked [`UNPAINTED`] ago wasn't drawn: the desktop doesn't
    /// paint the window.
    Unseen,
    /// The frames' task found a redraw [`UNPAINTED`] old ([`due`]): the
    /// update this brings judges it as it starts.
    Late,
    /// A frame drawn while unseen: maybe a configure's lone one, which the
    /// next tells.
    First,
    /// A second frame close after the first: painted again.
    Seen,
}

/// A redraw is asked now: after every update, and for every frame the
/// frames' task takes in. Only the first after a frame drawn counts.
pub fn asked(now: Instant) {
    let now = ms(now);
    let drawn = DRAWN.load(Ordering::Relaxed);
    let _ = ASKED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |asked| {
        (asked <= drawn).then_some(now)
    });
}

/// Whether [`judge`] would find the window unseen now, without turning the
/// probe: the frames' task asks, off the update.
pub fn due(now: Instant) -> bool {
    PROBE.load(Ordering::Relaxed) == PAINTED
        && late(
            ASKED.load(Ordering::Relaxed),
            DRAWN.load(Ordering::Relaxed),
            ms(now),
        )
}

/// Whether a redraw asked [`UNPAINTED`] ago isn't drawn yet, while the
/// window counts as painted: true once, until it's painted again. Only an
/// update judges, which hears it at once (`crate::window::judge`).
pub fn judge(now: Instant) -> bool {
    let (asked, drawn) = (ASKED.load(Ordering::Relaxed), DRAWN.load(Ordering::Relaxed));
    late(asked, drawn, ms(now))
        && PROBE
            .compare_exchange(PAINTED, WAITING, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

/// A frame was drawn, as the window's filter hears of it `at`
/// (`RedrawRequested`): what it says while the window is unseen; nothing
/// while it's painted.
pub fn drawn(at: Instant) -> Option<Heard> {
    let at = ms(at);
    DRAWN.fetch_max(at, Ordering::Relaxed);
    let probe = PROBE.load(Ordering::Relaxed);
    if probe == PAINTED {
        return None;
    }
    let (next, heard) = again(probe, at);
    PROBE
        .compare_exchange(probe, next, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
        .then_some(heard)
        .flatten()
}

/// A window was made again, out of the tray: no redraw asked of the one
/// before counts, and none is probed. Its sight starts painted with it
/// (`crate::window::made`).
pub fn fresh() {
    anew();
    PROBE.store(PAINTED, Ordering::Relaxed);
}

/// A renderer is made, which blocks iced's loop until it draws: nothing is
/// judged before its first frame.
pub fn anew() {
    DRAWN.store(0, Ordering::Relaxed);
    ASKED.store(0, Ordering::Relaxed);
}

/// Whether a redraw asked at `asked`, after the last frame `drawn`, is
/// [`UNPAINTED`] old at `now`; never before a renderer's first frame.
fn late(asked: u64, drawn: u64, now: u64) -> bool {
    drawn != 0 && asked > drawn && now.saturating_sub(asked) >= UNPAINTED_MS
}

/// What a frame drawn `at` says while unseen, the probe at `probe`: the
/// probe next, and what's heard. Within [`AGAIN`] of a first frame it's
/// painted; else it's a first frame itself, which brings an update only if
/// it's the first since the window went unseen, or [`REPROBE`] after the
/// one before.
fn again(probe: u64, at: u64) -> (u64, Option<Heard>) {
    let since = at.saturating_sub(probe);
    if probe > WAITING && since <= AGAIN_MS {
        (PAINTED, Some(Heard::Seen))
    } else if probe == WAITING || since >= REPROBE_MS {
        (at, Some(Heard::First))
    } else {
        (at, None)
    }
}

/// `at` in milliseconds since the first time read, plus 2.
fn ms(at: Instant) -> u64 {
    let epoch = *EPOCH.get_or_init(Instant::now);
    let since = at.saturating_duration_since(epoch).as_millis();
    u64::try_from(since).unwrap_or(u64::MAX).saturating_add(2)
}

#[cfg(test)]
mod tests {
    use super::{AGAIN_MS, Heard, PAINTED, REPROBE_MS, UNPAINTED_MS, WAITING, again, late};

    #[test]
    fn a_redraw_asked_a_second_ago_and_not_drawn_is_late() {
        assert!(!late(1_000, 900, 1_999), "not a second yet");
        assert!(late(1_000, 900, 1_000 + UNPAINTED_MS));
        assert!(late(1_000, 900, 60_000), "nothing drawn since");
        assert!(!late(1_000, 1_000, 60_000), "drawn as it was asked");
        assert!(!late(1_000, 1_016, 60_000), "drawn since");
        assert!(!late(0, 0, 60_000), "nothing asked");
        assert!(
            !late(1_000, 0, 60_000),
            "no frame yet: the renderer is made"
        );
    }

    #[test]
    fn two_frames_close_together_paint_the_window_again() {
        assert_eq!(again(WAITING, 5_000), (5_000, Some(Heard::First)));
        let seen = (PAINTED, Some(Heard::Seen));
        assert_eq!(again(5_000, 5_016), seen, "a refresh on");
        assert_eq!(again(5_000, 5_000 + AGAIN_MS), seen);
        // mutter's one frame for a configure, and another's three seconds on:
        // each only a first.
        assert_eq!(again(5_000, 8_000), (8_000, Some(Heard::First)));
        assert_eq!(
            again(8_000, 8_000 + REPROBE_MS),
            (10_000, Some(Heard::First))
        );
    }

    /// A hidden window drawn straight by the GPU waits a second for each
    /// frame: the next first frame, a second on, brings no update, so asks
    /// for no frame more, and the one after is painted only if it comes
    /// close.
    #[test]
    fn a_first_frame_soon_after_another_brings_no_update() {
        assert_eq!(again(5_000, 6_010), (6_010, None));
        assert_eq!(again(6_010, 6_026), (PAINTED, Some(Heard::Seen)));
        assert_eq!(again(5_000, 5_000 + AGAIN_MS + 1), (5_501, None));
    }
}
