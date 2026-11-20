//! The menu of a stream's tile, Anomi's own: a right click on someone's
//! stream while this account watches it opens the card with the stream's
//! volume, Discord's "Stream volume", over the rows that clean its sound
//! (`project::menus::stream`), as someone's menu in a call cleans their
//! voice; the card grows a slider for each that's on. On a stream not
//! watched, whose sound doesn't play, it's its streamer's menu, as before.
//! What's chosen reaches the watching worker through the core
//! (`volume::set_stream_cleaning`) and `layout.json` keeps it by the
//! streamer (`cleaned_streams`), as `super::cleaning` does for voices.

use iced::Task;
use yapfast_core::volume::{self, Kind};
use yapfast_lang::tr;

use anomi_vm::menu::{Menu, Spot};
use anomi_vm::shell::Layer;
use anomi_vm::voice::participant_menu::{self as card, ParticipantMenu};

use super::cleaning::{self, Sound};
use crate::app::App;
use crate::message::Message;
use crate::project::menus::stream as made;
use crate::project::menus::subs::their_noise_of;

#[cfg(test)]
mod tests;

/// A right click on `user`'s stream's tile in the call of `room`, at `at`:
/// the menu of its sound while it's watched, else theirs.
pub(crate) fn open(app: &mut App, room: String, user: String, at: Spot) -> Task<Message> {
    let watched = app.core.state.voice.watching.as_deref();
    let Some(name) = watched
        .filter(|watched| watched.user == user)
        .map(|watched| watched.name.clone())
    else {
        return super::open_voice(app, room, user, at);
    };
    let target = made::target(&room, &user);
    let chosen = volume::stream_cleaning(&user);
    let rows = made::rows(&target, chosen);
    let size = anomi_widgets::menu::size(&rows, card::WIDTH);
    let gain = volume::get(Kind::Stream, &user);
    let mut shown = ParticipantMenu {
        name: tr!("{}'s screen", name),
        live: true,
        stream: true,
        stream_volume: volume::to_slider(gain).round() as i32,
        joined: true,
        width: size.width.max(card::WIDTH),
        person: user,
        ..ParticipantMenu::default()
    };
    cleaning::sliders(&mut shown, chosen);
    let pointer = at.anchor.position();
    app.shown.menus.card_at = pointer;
    let (top, below) = shown.place(pointer, size, app.size);
    app.vm.voice.participant_menu = shown;
    app.vm
        .shell
        .open_at(Layer::ParticipantMenu, Spot::pointer(top));
    let menu = Menu {
        narrowest: Some(card::WIDTH),
        headed: true,
        ..Menu::new(0, Spot::pointer(below), rows)
    };
    super::put(app, target, true, menu);
    Task::none()
}

/// Carries out a row of a stream's menu about `target`, choosing `id`, its
/// box now `ticked`; whether it was one.
pub(super) fn choose(app: &mut App, target: &str, id: &str, ticked: Option<bool>) -> bool {
    let Some((_, user)) = made::stream_target(target) else {
        return false;
    };
    let user = user.to_owned();
    if let Some(suppressor) = their_noise_of(id) {
        cleaning::suppress(app, Sound::Stream, &user, suppressor);
    } else if let Some(on) = ticked {
        cleaning::remove_echo(app, Sound::Stream, &user, on);
    }
    true
}
