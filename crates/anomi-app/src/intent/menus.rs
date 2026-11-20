//! Menus' intents: the window's menu (a server's, a folder's, a DM's, a
//! channel's, a category's, a person's, the server header's dropdown and
//! its events, the bell's and a thread's More), opened where the Slint
//! window's `open-context-menu` opens it, with its rows built as it opens
//! (`prepare-menu`, `src/ui/callbacks.rs:1195`) and its choices carried
//! out (`choose-menu`, `:1222`); the message menu ([`message`]); and the
//! menu of a watched stream's tile ([`stream`]), Anomi's own.
//!
//! A closed menu costs nothing: its rows go once it played its way out.

mod choose;
pub(crate) mod cleaning;
pub(crate) mod message;
pub(crate) mod stream;
#[cfg(test)]
mod tests;

use iced::Task;
use yapfast_core::application::Action;

use anomi_vm::menu::{self, Menu, Spot};
use anomi_vm::shell::Layer;
use anomi_vm::voice::participant_menu::{self as card, ParticipantMenu};

use crate::app::App;
use crate::message::Message;
use crate::project::menus::subs::IN_VOICE;
use crate::project::menus::{self as rows, header_open};

/// The window's menu's intents.
pub fn handle(app: &mut App, intent: menu::Intent) -> Task<Message> {
    match intent {
        menu::Intent::Close => {
            close(app);
            Task::none()
        }
        // A menu opened since keeps its rows.
        menu::Intent::Gone => {
            if !app.vm.menu.open {
                app.vm.menu = Menu::default();
                app.shown.menus.target.clear();
                app.shown.menus.apps_due = false;
            }
            Task::none()
        }
        menu::Intent::Choose(choice) => choose::choose(app, choice),
        intent => {
            tracing::debug!(?intent, "not handled by the iced interface yet");
            Task::none()
        }
    }
}

/// The message menu's: `chat::Intent::Menu`.
pub fn message(app: &mut App, intent: anomi_vm::chat::Intent) -> Task<Message> {
    match intent {
        anomi_vm::chat::Intent::Menu(intent) => message::handle(app, intent),
        intent => {
            tracing::debug!(?intent, "not the message menu's");
            Task::none()
        }
    }
}

/// Opens the menu about `target` (a server, a folder, a conversation, a
/// person, or one of `project::menus::subs`' prefixed targets) at `spot`;
/// nothing when it offers nothing. A person's asks for the open room's
/// apps, which go in as they come.
pub(crate) fn open(app: &mut App, target: String, person: bool, spot: Spot) -> Task<Message> {
    let ask = if person {
        app.dispatch(Action::AskAppCommands)
    } else {
        Task::none()
    };
    let rows = rows::rows(&app.core, &app.layout, &target, person);
    if rows.is_empty() {
        return ask;
    }
    put(app, target, person, Menu::new(0, spot, rows));
    ask
}

/// Carries out the menu row `id` about `target` without showing the menu,
/// as a button that stands for it does: the forum post's Follow and Copy
/// link (`choose-menu(root.room-id, false, ..)`).
pub(crate) fn choose_on(app: &mut App, target: String, id: &str) -> Task<Message> {
    let choice = menu::Choice {
        target,
        person: false,
        id: id.to_owned(),
    };
    choose::choose(app, choice)
}

/// Shows `menu`, whose rows carry their own intents, as the window's: the
/// composer's plus (I11).
pub(crate) fn show(app: &mut App, menu: Menu) {
    put(app, String::new(), false, menu);
}

/// Shows `menu` as the window's, about `target`.
fn put(app: &mut App, target: String, person: bool, mut menu: Menu) {
    let state = &app.core.state;
    let shown = &mut app.shown.menus;
    shown.serial += 1;
    menu.serial = shown.serial;
    shown.apps_due = person
        && state
            .selected
            .as_deref()
            .is_some_and(|room| state.app_commands.index(room).is_none());
    shown.target = target;
    shown.person = person;
    app.vm.menu = menu;
    header_open(&mut app.vm, &app.shown.menus);
}

/// Someone's menu under a voice channel, at the pointer: their name and
/// volumes on top, unless it's this account, then what their menu offers
/// there (`open-voice-menu`, `ui/yapfast.slint:1611`). In a Discord call
/// the card also has a slider for each way their voice is cleaned.
pub(crate) fn open_voice(app: &mut App, room: String, user: String, at: Spot) -> Task<Message> {
    let target = format!("{IN_VOICE}{room}/{user}");
    let rows = rows::rows(&app.core, &app.layout, &target, true);
    let local = app.core.state.discord_user.as_deref() == Some(user.as_str());
    let found =
        (app.core.voice_person(&room, &user)).map(|person| (person.name.clone(), person.streaming));
    let Some((name, live)) = found.filter(|_| !local) else {
        if !rows.is_empty() {
            put(app, target, true, Menu::new(0, at, rows));
        }
        return Task::none();
    };
    let size = anomi_widgets::menu::size(&rows, card::WIDTH);
    let slider = |gain: f32| yapfast_core::volume::to_slider(gain).round() as i32;
    let voice = yapfast_core::volume::get(yapfast_core::volume::Kind::Voice, &user);
    // Their stream's volume as chosen, muted here or not.
    let stream_gain = stream::kept(&app.layout, &user);
    let mut made = ParticipantMenu {
        name,
        live,
        stream: false,
        voice_volume: slider(voice),
        stream_volume: slider(stream_gain),
        noise_strength: None,
        echo_depth: None,
        joined: !rows.is_empty(),
        width: size.width.max(card::WIDTH),
        person: user,
        rows: Vec::new(),
    };
    let cleans = (app.core.voice_person_choices(&room, &made.person))
        .contains(&yapfast_core::application::MenuChoice::SuppressTheirNoise);
    if cleans {
        let cleaning = yapfast_core::volume::cleaning(&made.person);
        cleaning::sliders(&mut made, cleaning);
    }
    let pointer = at.anchor.position();
    app.shown.menus.card_at = pointer;
    let (top, below) = made.place(pointer, size, app.size);
    app.vm.voice.participant_menu = made;
    app.vm
        .shell
        .open_at(Layer::ParticipantMenu, Spot::pointer(top));
    if !rows.is_empty() {
        let menu = Menu {
            narrowest: Some(card::WIDTH),
            headed: true,
            ..Menu::new(0, Spot::pointer(below), rows)
        };
        put(app, target, true, menu);
    }
    Task::none()
}

/// The window's menu closes, playing its way out, with the card over it.
pub(crate) fn close(app: &mut App) {
    app.vm.menu.open = false;
    if app.vm.shell.is_open(Layer::ParticipantMenu) {
        app.vm.shell.close(Layer::ParticipantMenu);
    }
    header_open(&mut app.vm, &app.shown.menus);
}
