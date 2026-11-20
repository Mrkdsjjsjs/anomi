//! A card that takes the pointer ([`Pointed::holding`](super::Pointed::holding)),
//! as Discord's card beside someone in a voice channel: asked for once the
//! pointer has rested on the row a moment with the window in front, shown
//! while the app gives it, kept while the pointer is on the row or on the
//! card and [`GRACE`] after it left both, so the pointer crosses the gap
//! between them as `tip::hold`'s card lets it. A click, a right click or the
//! wheel on the row, or the window going behind another, lets it go at
//! once, and it isn't asked for again until the pointer leaves the row and
//! comes back. One wake at the end of each wait; nothing ticks.

use std::time::{Duration, Instant};

use iced::advanced::{Shell, mouse};
use iced::{Event, window};

use anomi_theme::Motion;
use anomi_theme::motion::{Pace, Popup};

use super::state::State;

/// How long the pointer rests on the row before the card is asked for.
pub(super) const REST: Duration = Duration::from_millis(300);
/// How long the card stays once the pointer left the row and the card.
pub(super) const GRACE: Duration = Duration::from_millis(200);

/// What the row sends for its card.
pub(super) struct Hold<'a, M> {
    /// The pointer rested on the row: the app gives the card.
    pub rested: Box<dyn Fn() -> M + 'a>,
    /// The card goes: the app takes it away.
    pub left: Box<dyn Fn() -> M + 'a>,
}

impl<M> Hold<'_, M> {
    /// Follows the pointer and the window for the card: `over` is the
    /// pointer on the row, `carded` the card given, `front` the window in
    /// front, `taken` the event taken by a part of the row.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        state: &mut State,
        event: &Event,
        over: bool,
        carded: bool,
        front: bool,
        taken: bool,
        shell: &mut Shell<'_, M>,
    ) {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let held = over || (carded && state.on_card);
                if held {
                    state.left = None;
                }
                if !over {
                    state.rest = None;
                } else if !state.told && state.rest.is_none() && !state.dismissed && front {
                    let now = Instant::now();
                    state.rest = Some(now);
                    shell.request_redraw_at(now + REST);
                }
                if !held && (state.told || carded) && state.left.is_none() {
                    let now = Instant::now();
                    state.left = Some(now);
                    shell.request_redraw_at(now + GRACE);
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. })
                if over && !taken =>
            {
                self.leave(state, carded, shell);
            }
            Event::Window(window::Event::Unfocused) => {
                state.dismissed |= over;
                self.leave(state, carded, shell);
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(rest) = state.rest {
                    if *now >= rest + REST {
                        state.rest = None;
                        state.told = true;
                        shell.publish((self.rested)());
                    } else {
                        shell.request_redraw_at(rest + REST);
                    }
                }
                if let Some(left) = state.left {
                    if *now >= left + GRACE {
                        self.leave(state, carded, shell);
                    } else {
                        shell.request_redraw_at(left + GRACE);
                    }
                }
            }
            _ => {}
        }
    }

    /// The card goes now: the app hears it if it asked for the card or
    /// gives it.
    fn leave(&self, state: &mut State, carded: bool, shell: &mut Shell<'_, M>) {
        state.rest = None;
        state.left = None;
        state.on_card = false;
        if std::mem::take(&mut state.told) || carded {
            shell.publish((self.left)());
        }
    }
}

/// The row was built again: a card given now grows in, one taken away
/// goes at once.
pub(super) fn given(state: &mut State, carded: bool, motion: Motion) {
    if carded == state.carded {
        return;
    }
    state.carded = carded;
    state.card = if carded {
        Popup::entering(Pace::Popover, motion, Instant::now())
    } else {
        state.on_card = false;
        Popup::closed(Pace::Popover)
    };
}
