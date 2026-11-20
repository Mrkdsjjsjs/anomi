//! The card beside someone's row under a voice channel: the row asks for it
//! once the pointer rests there and lets it go a moment after the pointer
//! left the row and the card, at a click, a right click or the wheel on the
//! row, or as the window goes behind another
//! (`anomi_screens::sidebar::channels::pointed`). While it shows, the core
//! counts their stream's preview as on screen and looks up where their
//! game is sold (`Action::HoverVoicePerson`); nothing is asked, built or
//! drawn for it otherwise. Watch Stream (`Intent::Watch`) and the store's
//! button take it away, as Discord's popout goes.

use iced::Task;
use yapfast_core::application::Action;

use anomi_vm::sidebar::Place;

use crate::app::App;
use crate::message::Message;

/// The pointer rested on the row of `user` under the voice channel
/// `channel`: their card shows.
pub(super) fn rested(app: &mut App, channel: String, user: String) -> Task<Message> {
    let at = Some((channel, user));
    if app.shown.sidebar.hover.at == at {
        return Task::none();
    }
    app.shown.sidebar.hover.at.clone_from(&at);
    show(app);
    app.dispatch(Action::HoverVoicePerson(at))
}

/// Their card goes, if it's still theirs: another's may show by now, as
/// the pointer moved from one row to the next.
pub(super) fn left(app: &mut App, channel: &str, user: &str) -> Task<Message> {
    let theirs = (app.shown.sidebar.hover.at.as_ref())
        .is_some_and(|(shown, whose)| shown == channel && whose == user);
    if theirs { gone(app) } else { Task::none() }
}

/// The card that shows goes.
pub(crate) fn gone(app: &mut App) -> Task<Message> {
    if app.shown.sidebar.hover.at.take().is_none() {
        return Task::none();
    }
    show(app);
    app.dispatch(Action::HoverVoicePerson(None))
}

/// After each update: a card the core counts as shown that no longer shows
/// (the list went to another server, a filter or a fold took its row, they
/// left or stopped what it showed), whose row can't say so any more, goes.
pub(crate) fn lost(app: &mut App) -> Task<Message> {
    let Some((channel, user)) = &app.shown.sidebar.hover.at else {
        return Task::none();
    };
    let shows = match &app.vm.sidebar.place {
        Place::Server(server) => {
            (server.channels.hover.as_deref()).is_some_and(|card| card.beside(channel, user))
        }
        Place::Dms(_) => false,
    };
    if shows { Task::none() } else { gone(app) }
}

/// The store's button: the game's page opens in the browser, and the card
/// goes.
pub(super) fn store(app: &mut App, url: &str) -> Task<Message> {
    let gone = gone(app);
    Task::batch([gone, crate::platform::links::open_page(app, url)])
}

/// The card built, or taken away, at once.
fn show(app: &mut App) {
    let (core, sidebar) = (&app.core, &mut app.vm.sidebar);
    crate::project::sidebar::show_hover(core, sidebar, &mut app.shown.sidebar);
}

#[cfg(all(test, feature = "discord"))]
mod tests;
