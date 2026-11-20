//! Game activity's picker of Discord's games, as Discord's "Add it!" opens
//! in place: opening it reads Discord's list off the window's thread, and
//! downloads it first when it isn't there (or is in an old shape); typing
//! searches the list in memory, at most `MATCHES` games found; choosing a
//! game lists the programs running, and choosing one of them, or none,
//! adds the game. Closing it, leaving the page or Settings drops the list.
//! Nothing runs while it waits for the person: no timer, no scan.

use std::time::{SystemTime, UNIX_EPOCH};

use iced::Task;

use anomi_vm::settings::games::{self, List};

use super::source::{self, Directory, Program};
use crate::app::App;
use crate::layout::DiscordGame;
use crate::message::{Message, Once};
use crate::project::settings::games::Picking;

/// What the picker's work off the window's thread brought back, for its
/// opening numbered so.
#[derive(Clone, Debug)]
pub enum Due {
    /// Discord's list as read from the disk; none until it was downloaded.
    Read(u64, Once<Option<Directory>>),
    /// Downloading it ended.
    Downloaded(u64, Result<(), String>),
    /// The programs running, the newest first.
    Programs(u64, Vec<Program>),
}

fn picking(app: &mut App) -> &mut Picking {
    &mut app.shown.settings.kept.games
}

fn back(due: Due) -> Message {
    Message::Settings(crate::intent::settings::Due::Games(due))
}

/// The picker's intents; the page shows what they changed.
pub(super) fn handle(app: &mut App, intent: games::Intent) -> Task<Message> {
    use games::Intent as I;
    let task = match intent {
        I::OpenPicker => open(app),
        I::ClosePicker => {
            picking(app).close();
            Task::none()
        }
        I::Search(query) => {
            let picking = picking(app);
            picking.query = query;
            picking.search();
            Task::none()
        }
        I::PickGame(index) => {
            let picking = picking(app);
            picking.chosen = picking.found.get(index).cloned();
            if picking.chosen.is_none() {
                return Task::none();
            }
            list_programs(app)
        }
        I::ListPrograms => {
            if picking(app).chosen.is_none() {
                return Task::none();
            }
            list_programs(app)
        }
        I::BackToGames => {
            let picking = picking(app);
            (picking.chosen, picking.programs) = (None, None);
            Task::none()
        }
        I::PickProgram(index) => {
            pick_program(app, index);
            Task::none()
        }
        I::WithoutProgram => {
            without_program(app);
            Task::none()
        }
        I::RetryList => {
            let picking = picking(app);
            if !picking.open {
                return Task::none();
            }
            picking.list = List::Downloading;
            download(app)
        }
        intent => {
            tracing::debug!(?intent, "nothing for the picker to do");
            return Task::none();
        }
    };
    shown(app);
    task
}

/// What the picker holds changed: the page shows it.
fn shown(app: &mut App) {
    app.shown.settings.kept.changed();
    app.show(crate::project::Reach::SETTINGS);
}

/// Opens the picker, empty, reads Discord's list, and gives the search the
/// keys.
fn open(app: &mut App) -> Task<Message> {
    let picking = picking(app);
    let opening = picking.opening.wrapping_add(1);
    *picking = Picking {
        opening,
        open: true,
        ..Picking::default()
    };
    Task::batch([
        read(app, opening),
        anomi_widgets::field::focus(anomi_settings::games::SEARCH),
    ])
}

/// Reads Discord's list on the core's blocking pool.
fn read(app: &App, opening: u64) -> Task<Message> {
    let reading = app.runtime.spawn_blocking(source::read);
    Task::perform(reading, move |read| {
        back(Due::Read(opening, Once::new(read.ok().flatten())))
    })
}

/// Downloads Discord's list on the core's network runtime.
fn download(app: &mut App) -> Task<Message> {
    let opening = picking(app).opening;
    let downloading = app.runtime.spawn(source::download());
    Task::perform(downloading, move |done| {
        let done = done.unwrap_or_else(|error| Err(error.to_string()));
        back(Due::Downloaded(opening, done))
    })
}

/// Lists the programs running on the core's blocking pool.
fn list_programs(app: &mut App) -> Task<Message> {
    let picking = picking(app);
    picking.programs = None;
    let opening = picking.opening;
    let listing = app.runtime.spawn_blocking(source::programs);
    Task::perform(listing, move |listed| {
        back(Due::Programs(opening, listed.unwrap_or_default()))
    })
}

/// What came back, for the picker open now; for one closed since, nothing.
pub(crate) fn due(app: &mut App, due: Due) -> Task<Message> {
    let picking = picking(app);
    let opening = match &due {
        Due::Read(opening, _) | Due::Downloaded(opening, _) | Due::Programs(opening, _) => *opening,
    };
    if !picking.open || opening != picking.opening {
        return Task::none();
    }
    let task = match due {
        Due::Read(_, read) => match read.take().flatten() {
            Some(directory) => {
                let old = directory.old_shape();
                picking.directory = Some(directory);
                picking.list = List::Ready;
                picking.search();
                // A list in an old shape searches by names alone until
                // the new one is read in its place.
                if old && !picking.downloaded {
                    picking.downloaded = true;
                    download(app)
                } else {
                    Task::none()
                }
            }
            None if picking.downloaded => {
                picking.list = List::Failed;
                Task::none()
            }
            None => {
                (picking.downloaded, picking.list) = (true, List::Downloading);
                download(app)
            }
        },
        Due::Downloaded(_, Ok(())) => read(app, opening),
        Due::Downloaded(_, Err(error)) => {
            tracing::info!(%error, "could not download Discord's list of games");
            if picking.directory.is_none() {
                picking.list = List::Failed;
            }
            Task::none()
        }
        Due::Programs(_, programs) => {
            if picking.chosen.is_some() {
                picking.programs = Some(programs);
            }
            Task::none()
        }
    };
    shown(app);
    task
}

/// Ties the game chosen to the program at `index`.
fn pick_program(app: &mut App, index: usize) {
    let picking = picking(app);
    let program = (picking.programs.iter().flatten()).nth(index).cloned();
    let (Some(game), Some(program)) = (picking.chosen.clone(), program) else {
        return;
    };
    picking.close();
    super::add(
        app,
        DiscordGame {
            id: game.id.to_string(),
            name: game.name,
            executable: program.path,
            ..DiscordGame::default()
        },
    );
}

/// Shows the game chosen without a program, from now until it's turned
/// off.
fn without_program(app: &mut App) {
    let picking = picking(app);
    let Some(game) = picking.chosen.clone() else {
        return;
    };
    picking.close();
    super::add(
        app,
        DiscordGame {
            id: game.id.to_string(),
            name: game.name,
            since: Some(now()),
            ..DiscordGame::default()
        },
    );
}

/// Now, in Unix seconds, as a game without a program counts from.
pub(super) fn now() -> u64 {
    (SystemTime::now().duration_since(UNIX_EPOCH)).map_or(0, |since| since.as_secs())
}
