//! A dropped offer's files: the source writes their list to a pipe, which
//! the thread reads as it comes, unless the watch drops first.

use std::io::{ErrorKind, PipeReader, Read};
use std::os::fd::{AsFd, BorrowedFd};

use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_data_offer::WlDataOffer;

use super::thread::{Woke, wait};
use super::{Drag, State, URI_LIST};

/// The most of a dropped list that's read: thousands of paths. A source
/// sending more is refused rather than kept in memory.
const LIST_MAX: usize = 1 << 20;

impl State {
    /// The list of files a dropped offer gives: empty if it can't be read,
    /// and `None` if the watch drops while the source still writes it.
    pub(super) fn receive(&mut self, offer: &WlDataOffer) -> Option<String> {
        let Ok((reader, writer)) = std::io::pipe() else {
            return Some(String::new());
        };
        offer.receive(URI_LIST.into(), writer.as_fd());
        drop(writer);
        match self.connection.flush() {
            // The request goes at the next flush: winit's, before it sleeps.
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => {
                tracing::warn!(target: crate::LOG, %error, "could not ask for the dropped files");
                return Some(String::new());
            }
            Ok(()) => {}
        }
        match read_list(reader, self.stop.as_fd()) {
            Listed::Whole(list) => Some(list),
            Listed::Stopped => {
                self.stopping = true;
                None
            }
            Listed::Failed(error) => {
                tracing::warn!(target: crate::LOG, %error, "could not read the dropped files");
                Some(String::new())
            }
        }
    }

    /// Reports the files a drop listed, from a thread of its own: each is
    /// looked at, which a network mount that stalls can hold up for long,
    /// and nothing stops a look meanwhile, while this thread must let go of
    /// the display at once as the watch drops.
    pub(super) fn dropped(&self, list: String) {
        let report = self.report.clone();
        let looked = std::thread::Builder::new()
            .name("yapfast-dropped".into())
            .stack_size(128 * 1024)
            .spawn(move || report(Drag::Dropped(crate::paste::files(&list))));
        if let Err(error) = looked {
            tracing::warn!(target: crate::LOG, %error, "could not take the dropped files");
        }
    }
}

/// What was read of a dropped list.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Listed {
    /// All of it, as the source closed its end.
    Whole(String),
    /// The watch dropped first.
    Stopped,
    /// It can't be read: why.
    Failed(String),
}

/// Reads the list the source writes to `reader`, as it writes it, to its
/// end: unless `stop` closes first, so a source that never finishes can't
/// hold up a quit, or the list grows past [`LIST_MAX`].
pub(super) fn read_list(mut reader: PipeReader, stop: BorrowedFd<'_>) -> Listed {
    let mut list = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        match wait(reader.as_fd(), stop) {
            Ok(Woke::Ready) => {}
            Ok(Woke::Stopped) => return Listed::Stopped,
            Ok(Woke::Closed) => return Listed::Failed("the pipe was closed".into()),
            Err(error) => return Listed::Failed(error.to_string()),
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) if list.len() + read > LIST_MAX => {
                return Listed::Failed(format!("the list is longer than {LIST_MAX} bytes"));
            }
            Ok(read) => list.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Listed::Failed(error.to_string()),
        }
    }
    String::from_utf8(list).map_or_else(
        |_| Listed::Failed("the list isn't UTF-8".into()),
        Listed::Whole,
    )
}
