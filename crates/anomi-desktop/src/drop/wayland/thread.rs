//! The thread itself: started with a pipe that closes as its [`Watch`]
//! drops, asleep on files until one can be read or that pipe closes, and
//! waited for as the watch drops.

use std::io::{PipeReader, PipeWriter};
use std::os::fd::BorrowedFd;
use std::time::Duration;

/// How long dropping a [`Watch`] waits for its thread, which lets go of the
/// display in microseconds once woken: the quit it's part of goes on after.
pub(super) const STOP_WITHIN: Duration = Duration::from_millis(100);

/// Files dragged over the window, followed by a thread of its own until
/// this drops. Dropping it wakes the thread, which lets go of what it holds
/// on the display and ends, and waits for that, up to 100 ms: drop it while
/// the display is still open.
#[must_use = "dropping it stops following drags"]
pub struct Watch {
    /// The thread's end of this pipe reads as closed once this goes.
    stop: Option<PipeWriter>,
    /// Reads as closed once the thread has ended.
    ended: PipeReader,
}

impl Watch {
    /// Runs `follow` on the thread, handing it the end of the pipe that
    /// closes as the watch drops.
    pub(super) fn spawn(follow: impl FnOnce(PipeReader) + Send + 'static) -> std::io::Result<Self> {
        let (stopping, stop) = std::io::pipe()?;
        let (ended, ending) = std::io::pipe()?;
        std::thread::Builder::new()
            .name("yapfast-drop".into())
            // It keeps little: the device, an offer, and a dropped list.
            .stack_size(128 * 1024)
            .spawn(move || {
                // Closes as the thread ends, after `follow` let go of all it
                // held; a panic closes it too.
                let _ending = ending;
                follow(stopping);
            })?;
        Ok(Self {
            stop: Some(stop),
            ended,
        })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        drop(self.stop.take());
        if !closes_within(&self.ended, STOP_WITHIN) {
            tracing::warn!(
                target: crate::LOG,
                waited_ms = STOP_WITHIN.as_millis() as u64,
                "the thread following dragged files didn't stop in time"
            );
        }
    }
}

/// Waits up to `within` for `ended` to read as closed: whether it did.
fn closes_within(ended: &PipeReader, within: Duration) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let Ok(within) = Timespec::try_from(within) else {
        return false;
    };
    let mut fds = [PollFd::new(ended, PollFlags::IN)];
    loop {
        match poll(&mut fds, Some(&within)) {
            Ok(0) => return false,
            Ok(_) => return true,
            // A signal: the wait starts again, at most once a signal.
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return false,
        }
    }
}

/// What woke the thread.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Woke {
    /// The file can be read, or has an error or a hang-up to find there.
    Ready,
    /// The watch dropped.
    Stopped,
    /// The file is no longer open: its owner closed it, a display with it.
    Closed,
}

/// Sleeps until `fd` can be read or `stop` closes, as long as that takes:
/// the thread waits here, with nothing to wake it while nothing happens.
/// A closed `fd` is told before a stop, so a display gone isn't touched.
pub(super) fn wait(fd: BorrowedFd<'_>, stop: BorrowedFd<'_>) -> std::io::Result<Woke> {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut fds = [
        PollFd::from_borrowed_fd(fd, PollFlags::IN),
        PollFd::from_borrowed_fd(stop, PollFlags::IN),
    ];
    loop {
        match poll(&mut fds, None) {
            Ok(_) => break,
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let [fd, stop] = fds.map(|polled| polled.revents());
    Ok(if fd.contains(PollFlags::NVAL) {
        Woke::Closed
    } else if !stop.is_empty() {
        Woke::Stopped
    } else {
        Woke::Ready
    })
}
