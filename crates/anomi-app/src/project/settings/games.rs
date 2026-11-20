//! Game activity (`src/ui/games.rs`): the switch, the games added from
//! Discord's list with their icons, the picker over that list while it's
//! open, and the games named by hand; the form's drafts are the window's
//! own and stay. The picker's state is the intents' ([`Picking`], kept
//! beside the page in `Kept`): what it found is made into rows here, the
//! icons of those rows alone asked for.

use yapfast_core::application::AppState;

use anomi_vm::kind::Avatar;
use anomi_vm::settings::games::{self, Added, Games, List, Match, Picker};

use super::people::linked;
use crate::intent::settings::games::source::{self, Directory, Found, Program};
use crate::layout::Layout;
use crate::pictures::Requests;
use crate::project::rows;

/// The picker of Discord's games while it's open, held beside the page:
/// Discord's list as the picker searches it, dropped as it closes, and the
/// few games and programs it shows.
#[derive(Default)]
pub(crate) struct Picking {
    /// Which opening of the picker this is: work off the window's thread
    /// for an earlier one comes back to nothing.
    pub(crate) opening: u64,
    pub(crate) open: bool,
    pub(crate) list: List,
    pub(crate) directory: Option<Directory>,
    /// The list was downloaded during this opening: one still missing then
    /// failed. One found in an old shape is downloaded once more, quietly.
    pub(crate) downloaded: bool,
    pub(crate) query: String,
    /// The best matches of the search, at most `games::MATCHES`, and how
    /// many matched.
    pub(crate) found: Vec<Found>,
    pub(crate) matched: usize,
    /// The game chosen, and the programs running once listed.
    pub(crate) chosen: Option<Found>,
    pub(crate) programs: Option<Vec<Program>>,
}

impl Picking {
    /// Searches the list as the query is now.
    pub(crate) fn search(&mut self) {
        if let Some(directory) = &self.directory {
            let search = directory.search(&self.query, games::MATCHES);
            (self.found, self.matched) = (search.games, search.matched);
        }
    }

    /// Closes the picker: the list and all it found go; late work for this
    /// opening comes back to nothing.
    pub(crate) fn close(&mut self) {
        *self = Self {
            opening: self.opening,
            ..Self::default()
        };
    }
}

/// The page from the core's state, the layout and the picker.
pub(super) fn show(
    state: &AppState,
    layout: &Layout,
    picking: &Picking,
    page: &mut Games,
    requests: &mut Requests,
) {
    page.shown = state.discord_show_game;
    page.linked = linked(state);
    named(layout, page);
    let mut icons: Vec<String> = Vec::new();
    let mut icon = |name: &str, id: Option<u64>| -> Avatar {
        let address = id.map(source::icon).filter(|address| !address.is_empty());
        let avatar = rows::avatar(
            state,
            name,
            &id.unwrap_or_default().to_string(),
            address.as_deref(),
        );
        icons.extend(address);
        avatar
    };
    page.added = (layout.discord_games.iter())
        .map(|game| Added {
            name: game.name.clone(),
            program: game.executable.clone(),
            icon: icon(&game.name, game.id.parse().ok()),
            shown: !game.hidden,
        })
        .collect();
    let mut found = |game: &Found| Match {
        name: game.name.clone(),
        alias: game.alias.clone().unwrap_or_default(),
        icon: icon(&game.name, Some(game.id)),
    };
    page.picker = picking.open.then(|| Picker {
        list: picking.list,
        query: picking.query.clone(),
        matches: picking.found.iter().map(&mut found).collect(),
        more: picking.matched > picking.found.len(),
        chosen: picking.chosen.as_ref().map(&mut found),
        programs: (picking.programs.iter().flatten())
            .map(|program| games::Program {
                name: program.name.clone(),
                path: program.path.clone(),
            })
            .collect(),
        listing: picking.programs.is_none(),
    });
    requests.want(state, icons.iter().map(String::as_str));
}

/// The games named by hand, built again only when they changed.
fn named(layout: &Layout, page: &mut Games) {
    let same = page.games.len() == layout.game_executables.len()
        && (page.games.iter().zip(&layout.game_executables)).all(|(shown, (executable, text))| {
            shown.executable == *executable && shown.text == *text
        });
    if !same {
        page.games = (layout.game_executables.iter())
            .map(|(executable, text)| games::Executable {
                executable: executable.clone(),
                text: text.clone(),
            })
            .collect();
    }
}
