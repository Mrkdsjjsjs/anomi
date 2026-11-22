//! Threads, forums, search and pins (`src/ui/callbacks.rs:1637`-`:1795`,
//! `:1123`-`:1137`, `:2738`): what their screens ask, as the core's
//! actions. One module each: [`search`] (the header's field, the results
//! and the pins), [`threads`] (the threads popover and the new thread's
//! form), [`aside`] (the thread beside the channel, its timeline and its
//! composer) and [`forum`] (its posts, the new post's form and the post's
//! bar).

mod aside;
mod forum;
mod search;
#[cfg(test)]
mod tests;
mod threads;

pub(crate) use aside::{after as aside_after, let_go as aside_let_go};
pub(crate) use search::compact as search_compact;

use iced::Task;
use yapfast_core::application::Action;

use anomi_vm::chat::Intent;

use crate::app::App;
use crate::message::Message;

/// `Search`, `Pins`, `Threads`, `ThreadPanel`, `ChannelPanel`, `NewThread`,
/// `Forum` and `NewPost`.
pub fn handle(app: &mut App, intent: Intent) -> Task<Message> {
    match intent {
        Intent::Search(intent) => search::search(app, intent),
        Intent::Pins(intent) => search::pins(app, intent),
        Intent::Threads(intent) => threads::popover(app, intent),
        Intent::NewThread(intent) => threads::new(app, intent),
        Intent::ThreadPanel(intent) => aside::handle(app, intent),
        Intent::ChannelPanel(intent) => channel_panel(app, intent),
        Intent::Forum(intent) => forum::forum(app, intent),
        Intent::NewPost(intent) => forum::new_post(app, intent),
        intent => {
            tracing::debug!(?intent, "not threads', forums', search's or pins'");
            Task::none()
        }
    }
}

/// A voice channel's chat beside its stage: the cross closes it, the stage
/// filling the body again.
fn channel_panel(app: &mut App, intent: anomi_vm::chat::channel_panel::Intent) -> Task<Message> {
    use anomi_vm::chat::channel_panel::Intent as I;
    match intent {
        I::Close => app.dispatch(Action::ShowVoiceChat(false)),
        _ => Task::none(),
    }
}

/// The bar over a forum post open as the conversation: React to post,
/// Follow, Copy link (the window's `shell::Intent::ReactToPost`,
/// `FollowPost`, `CopyPostLink`).
pub fn post_bar(app: &mut App, intent: anomi_vm::shell::Intent) -> Task<Message> {
    forum::post_bar(app, intent)
}
