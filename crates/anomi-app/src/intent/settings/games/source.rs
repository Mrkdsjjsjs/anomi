//! What Game activity asks of Discord's side (`yapfast_discord::
//! game_detection`): its list of games and downloading it, the programs
//! running, a game's icon, and the games of this device handed to
//! detection. A build without Discord has no list, lists no program and
//! hands nothing on.

use crate::layout::Saved;

#[cfg(feature = "discord")]
pub(crate) use yapfast_core::yapfast_discord::game_detection::{Directory, Found, Program};

#[cfg(not(feature = "discord"))]
pub(crate) use without::{Directory, Found, Program};

/// The most programs listed, the newest first.
const PROGRAMS: usize = 40;

/// Discord's list as last downloaded; none until it was. It reads the
/// disk: off the window's thread.
pub(crate) fn read() -> Option<Directory> {
    #[cfg(feature = "discord")]
    return yapfast_core::yapfast_discord::game_detection::read_directory();
    #[cfg(not(feature = "discord"))]
    None
}

/// Downloads Discord's list now, on the core's network runtime.
pub(crate) async fn download() -> Result<(), String> {
    #[cfg(feature = "discord")]
    return (yapfast_core::yapfast_discord::game_detection::download_list().await)
        .map_err(|error| format!("{error:#}"));
    #[cfg(not(feature = "discord"))]
    Err(String::from("no Discord in this build"))
}

/// The programs running, the newest first. It asks the system for every
/// process: off the window's thread.
pub(crate) fn programs() -> Vec<Program> {
    #[cfg(feature = "discord")]
    return yapfast_core::yapfast_discord::game_detection::programs::running(PROGRAMS);
    #[cfg(not(feature = "discord"))]
    {
        let _ = PROGRAMS;
        Vec::new()
    }
}

/// The address of a game's icon, by its application's id, which the core's
/// pictures ask Discord for; empty without Discord.
pub(crate) fn icon(id: u64) -> String {
    #[cfg(feature = "discord")]
    return yapfast_core::yapfast_discord::game_detection::icon_address(id);
    #[cfg(not(feature = "discord"))]
    {
        let _ = id;
        String::new()
    }
}

/// Hands detection the games this device names, those named by hand first,
/// as the Slint app hands it the first alone (`Layout::apply`).
pub(crate) fn detect(saved: &Saved) {
    #[cfg(feature = "discord")]
    {
        use yapfast_core::yapfast_discord::game_detection::custom_games::{self, CustomGame};
        let named = (saved.game_executables.iter()).map(|(executable, name)| CustomGame {
            executable: executable.clone(),
            name: name.clone(),
            ..CustomGame::default()
        });
        let listed = saved.discord_games.iter().map(|game| CustomGame {
            executable: game.executable.clone(),
            name: game.name.clone(),
            id: game.id.parse().ok(),
            hidden: game.hidden,
            since: game.since,
        });
        custom_games::set(named.chain(listed).collect());
    }
    #[cfg(not(feature = "discord"))]
    let _ = saved;
}

/// What stands for Discord's side in a build without it, where no list is
/// ever read.
#[cfg(not(feature = "discord"))]
#[allow(dead_code)]
mod without {
    pub(crate) struct Directory;

    impl Directory {
        pub(crate) fn search(&self, _query: &str, _limit: usize) -> Search {
            Search::default()
        }

        pub(crate) fn old_shape(&self) -> bool {
            false
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct Found {
        pub(crate) id: u64,
        pub(crate) name: String,
        pub(crate) alias: Option<String>,
    }

    #[derive(Default)]
    pub(crate) struct Search {
        pub(crate) games: Vec<Found>,
        pub(crate) matched: usize,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct Program {
        pub(crate) name: String,
        pub(crate) path: String,
    }
}
