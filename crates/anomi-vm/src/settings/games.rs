//! The Game activity settings page: what it shows and what it asks for. Task
//! S08 (docs/iced/tasks/settings.md), after `GameActivitySettings`
//! (`ui/settings.slint:2134`) and `src/ui/games.rs`. The two drafts and the
//! game being edited are the window's, as the Slint page's private
//! properties were: the app keeps what was typed and projects it back.
//!
//! Beside the games named by hand, the page lists those added from Discord's
//! list of games, as Discord's Registered Games does, and opens a picker
//! over that list: a search, then the program running that is the game, or
//! none to show it until it's turned off. The picker holds only the matches
//! it shows, at most [`MATCHES`]; the list stays the app's.

use crate::kind::Avatar;

/// The longest a game's text may be (`src/ui/games.rs`), in characters.
pub const TEXT_LIMIT: usize = 128;

/// The most games the picker shows for a search.
pub const MATCHES: usize = 20;

/// A game named by its executable and what to show after Playing
/// (`GameExecutable`).
#[derive(Clone, Debug, PartialEq)]
pub struct Executable {
    pub executable: String,
    pub text: String,
}

/// A game added from Discord's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub name: String,
    /// The program it's tied to, by its path; empty for a game shown
    /// without one.
    pub program: String,
    pub icon: Avatar,
    /// Its activity shows.
    pub shown: bool,
}

/// A game of Discord's list the search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub name: String,
    /// The other name it was found by; empty when its own name matched.
    pub alias: String,
    pub icon: Avatar,
}

/// A program running now, which a game can be tied to.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Its file name.
    pub name: String,
    pub path: String,
}

/// Where Discord's list is, for the picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum List {
    /// Being read from the disk.
    #[default]
    Reading,
    /// Not downloaded yet, and being downloaded.
    Downloading,
    /// The download failed.
    Failed,
    /// Searchable.
    Ready,
}

/// The picker of Discord's games, while it's open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picker {
    pub list: List,
    /// The search as typed.
    pub query: String,
    /// The best matches, at most [`MATCHES`].
    pub matches: Vec<Match>,
    /// More games matched than show.
    pub more: bool,
    /// The game chosen: the picker then asks for its program.
    pub chosen: Option<Match>,
    /// The programs running, the newest first.
    pub programs: Vec<Program>,
    /// The programs are being listed.
    pub listing: bool,
}

/// The Game activity settings page: what its screen draws. The window holds
/// it as `vm.settings.games`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Games {
    /// The Discord account shares the game it plays.
    pub shown: bool,
    /// A Discord account is linked; the switch is locked without one.
    pub linked: bool,
    /// The games added from Discord's list.
    pub added: Vec<Added>,
    /// The picker, while it's open.
    pub picker: Option<Picker>,
    pub games: Vec<Executable>,
    /// The game the form edits; `None` adds a new one.
    pub editing: Option<usize>,
    pub executable_draft: String,
    pub text_draft: String,
    /// The last save was refused.
    pub invalid: bool,
}

/// The Game activity settings page: what a click or a key on it asks for.
/// Keep `#[non_exhaustive]`: the app matches it with a wildcard until its I
/// task handles every variant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// Share the game, or stop (`toggled`).
    Share(bool),
    /// The fields were edited: the whole text.
    Executable(String),
    Text(String),
    /// Fill the form with a game to change.
    Edit(usize),
    /// Remove a game (`remove`).
    Remove(usize),
    /// Add the game, or save the one being edited (`save`): the app takes
    /// `editing` and the two drafts it holds, and sets `invalid` when the
    /// game is refused.
    Save,
    /// Leave the edit and empty the form.
    Cancel,
    /// Open the picker of Discord's games ("Add a game").
    OpenPicker,
    /// Close it, adding nothing.
    ClosePicker,
    /// The picker's search was edited: the whole text.
    Search(String),
    /// Choose the match at this place in `matches`.
    PickGame(usize),
    /// Tie the game chosen to the program at this place in `programs`.
    PickProgram(usize),
    /// Show the game chosen without a program, until it's turned off.
    WithoutProgram,
    /// List the programs running again.
    ListPrograms,
    /// Back from the programs to the search.
    BackToGames,
    /// Download Discord's list again, after it failed.
    RetryList,
    /// Turn the activity of the added game at this place on or off.
    ShowAdded(usize, bool),
    /// Remove the added game at this place.
    RemoveAdded(usize),
}
