//! Whether a submenu stays as the pointer moves, the same for every menu:
//! the window's ([`super::ContextMenu`]) and those a screen lays out with
//! [`super::Beside`].
//!
//! A submenu the pointer opened, or came onto since, is the pointer's: once
//! the pointer is on neither the submenu nor the row that opened it,
//! wherever it went (another row, a card over the menu, the window behind,
//! out of the window), it closes [`GRACE`] later, unless the pointer came
//! back first. So the pointer crosses the gap between the row and the
//! submenu, or a corner of another row on its way to the submenu's lower
//! rows; meanwhile the other rows don't take it, and once the wait ends the
//! row under the pointer does. One the keys opened with the pointer
//! elsewhere stays until a key or a press says otherwise, the other rows
//! not taking the pointer either, until the pointer comes onto the submenu
//! or its row and the submenu is the pointer's. A press anywhere else closes
//! either at once. One wake at the end of the wait; nothing ticks.

use std::time::Instant;

use iced::advanced::Shell;

/// How long: a holding tip's wait for the pointer to cross to its card
/// (`crate::tip::hold`), 200 ms.
pub(crate) use crate::tip::hold::GRACE;

/// Where the pointer stands with a submenu that shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hold {
    /// The pointer has been on the submenu or its row since it showed.
    held: bool,
    /// When the pointer left both, while they were its.
    left: Option<Instant>,
}

impl Hold {
    /// A submenu just shown; `pointer` when the pointer is on its row, which
    /// opened it.
    pub fn new(pointer: bool) -> Self {
        Self {
            held: pointer,
            left: None,
        }
    }

    /// The pointer moved, `on` the submenu or its row or not.
    pub fn moved<M>(&mut self, on: bool, shell: &mut Shell<'_, M>) {
        if on {
            self.held = true;
            self.left = None;
        } else if self.held && self.left.is_none() {
            let now = Instant::now();
            self.left = Some(now);
            shell.request_redraw_at(now + GRACE);
        }
    }

    /// A frame at `now`: whether the wait ran out, for the submenu to close.
    pub fn due<M>(&mut self, now: Instant, shell: &mut Shell<'_, M>) -> bool {
        match self.left {
            Some(left) if now >= left + GRACE => {
                *self = Self::default();
                true
            }
            Some(left) => {
                shell.request_redraw_at(left + GRACE);
                false
            }
            None => false,
        }
    }

    /// With the pointer off the submenu and its row, it stays for now:
    /// within the wait, or while the keys hold it. The other rows don't take
    /// the pointer meanwhile.
    pub fn waiting(self) -> bool {
        self.left.is_some() || !self.held
    }
}
