//! Game activity's intents: the switch, which is the account's; the games
//! named by hand (`src/ui/games.rs`'s `on_save_game_executable` and
//! `remove`); and the games added from Discord's list through the picker
//! (`picker`), each with its switch and Remove. Every change is kept in the
//! layout and handed to detection at once (`source::detect`).

mod picker;
pub(crate) mod source;
#[cfg(all(test, feature = "discord"))]
mod tests;

use iced::Task;
use yapfast_core::application::Action;

use anomi_vm::settings::games;

use crate::app::App;
use crate::layout::DiscordGame;
use crate::message::Message;
use crate::project::Reach;

pub use picker::Due;
pub(super) use picker::due;

pub(super) fn handle(app: &mut App, intent: games::Intent) -> Task<Message> {
    use games::Intent as I;
    let page = &mut app.vm.settings.games;
    match intent {
        I::Share(on) => return app.dispatch(Action::ShowGame(on)),
        I::Executable(text) => page.executable_draft = text,
        I::Text(text) => page.text_draft = text,
        I::Edit(index) => {
            if let Some(game) = page.games.get(index) {
                page.executable_draft.clone_from(&game.executable);
                page.text_draft.clone_from(&game.text);
                (page.editing, page.invalid) = (Some(index), false);
            }
        }
        I::Cancel => clear(page),
        I::Remove(index) => {
            clear(page);
            if index < app.layout.game_executables.len() {
                app.layout.edit(|saved| {
                    saved.game_executables.remove(index);
                });
                games_changed(app);
            }
        }
        I::Save => save_game(app),
        I::ShowAdded(index, on) => show_added(app, index, on),
        I::RemoveAdded(index) => {
            if index < app.layout.discord_games.len() {
                app.layout.edit(|saved| {
                    saved.discord_games.remove(index);
                });
                games_changed(app);
            }
        }
        intent => return picker::handle(app, intent),
    }
    Task::none()
}

/// The page is left, or Settings closes: the picker closes with it, and
/// Discord's list goes from memory.
pub(super) fn left(app: &mut App) {
    let kept = &mut app.shown.settings.kept;
    if kept.games.open {
        kept.games.close();
        kept.changed();
    }
}

/// The form empties and edits nothing.
fn clear(page: &mut games::Games) {
    page.editing = None;
    page.executable_draft.clear();
    page.text_draft.clear();
}

/// Adds the game the form holds, or saves the one it edits; another entry
/// for the same executable gives way to it (`on_save_game_executable`).
fn save_game(app: &mut App) {
    let page = &mut app.vm.settings.games;
    let (executable, text) = (page.executable_draft.trim(), page.text_draft.trim());
    if executable.is_empty()
        || text.is_empty()
        || executable.len() > 4096
        || text.chars().count() > games::TEXT_LIMIT
        || executable.chars().any(char::is_control)
        || text.chars().any(char::is_control)
    {
        page.invalid = true;
        return;
    }
    let entry = (executable.to_owned(), text.to_owned());
    let games = &app.layout.game_executables;
    let editing = page.editing.filter(|&at| at < games.len());
    let same = (games.iter().position(|(other, _)| *other == entry.0))
        .filter(|&same| Some(same) != editing);
    page.invalid = false;
    clear(page);
    app.layout.edit(|saved| {
        let games = &mut saved.game_executables;
        match (editing, same) {
            (Some(at), same) => {
                games[at] = entry;
                if let Some(same) = same {
                    games.remove(same);
                }
            }
            (None, Some(same)) => games[same] = entry,
            (None, None) => games.push(entry),
        }
    });
    games_changed(app);
}

/// Adds a game from Discord's list: in place of the one tied to the same
/// program, as a program is one game, or, without a program, of the same
/// game shown without one, which shows again from now.
fn add(app: &mut App, game: DiscordGame) {
    app.layout.edit(|saved| {
        let games = &mut saved.discord_games;
        let same = games.iter().position(|kept| {
            if game.executable.is_empty() {
                kept.executable.is_empty() && kept.id == game.id
            } else {
                kept.executable == game.executable
            }
        });
        match same {
            Some(at) => games[at] = game,
            None => games.push(game),
        }
    });
    games_changed(app);
}

/// Turns an added game's activity on or off; one without a program shows
/// from the moment it's turned on.
fn show_added(app: &mut App, index: usize, on: bool) {
    if index >= app.layout.discord_games.len() {
        return;
    }
    let now = picker::now();
    app.layout.edit(|saved| {
        let game = &mut saved.discord_games[index];
        game.hidden = !on;
        if on && game.executable.is_empty() {
            game.since = Some(now);
        }
    });
    games_changed(app);
}

/// The games of this device changed: detection takes them, the page shows
/// them.
fn games_changed(app: &mut App) {
    source::detect(&app.layout);
    app.show(Reach::SETTINGS);
}
