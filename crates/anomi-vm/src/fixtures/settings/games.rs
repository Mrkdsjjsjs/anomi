//! Samples of the Game activity settings page for tests and shots, after the
//! fake Discord's data: task S08's, and the games added from Discord's list
//! with the picker over it, after the fake's detectable games
//! (`DETECTABLE_GAMES`) and their icons.

use crate::Vm;
use crate::fixtures::avatar;
use crate::fixtures::pictures::picture;
use crate::fixtures::settings::frame::settings;
use crate::kind::Avatar;
use crate::settings::frame::Page;
use crate::settings::games::{Added, Executable, Games, List, Match, Picker, Program};

/// The page as the golden screenshot has it: sharing on, a Discord account
/// linked, no custom game.
pub fn games() -> Games {
    Games {
        shown: true,
        linked: true,
        ..Games::default()
    }
}

/// The window with the settings open on Game activity.
pub fn page() -> Vm {
    with(games())
}

/// The window with Game activity as `page` has it.
pub fn with(page: Games) -> Vm {
    let mut vm = settings(Page::Games);
    vm.settings.games = page;
    vm
}

/// Two custom games, the second being edited.
pub fn custom() -> Games {
    Games {
        games: vec![
            Executable {
                executable: "/opt/games/blockhead/blockhead.x86_64".into(),
                text: "Blockhead".into(),
            },
            Executable {
                executable: "dwarves.exe".into(),
                text: "Deep Dwarves".into(),
            },
        ],
        editing: Some(1),
        executable_draft: "dwarves.exe".into(),
        text_draft: "Deep Dwarves".into(),
        ..games()
    }
}

/// With no account linked the switch is locked.
pub fn unlinked() -> Games {
    Games {
        linked: false,
        ..games()
    }
}

/// A refused save.
pub fn refused() -> Games {
    Games {
        executable_draft: "game".into(),
        text_draft: "x".into(),
        invalid: true,
        ..games()
    }
}

/// A game's icon as the fake serves it, by its application and icon.
fn icon(name: &str, id: &str, hash: &str) -> Avatar {
    Avatar {
        picture: Some(picture(&format!("/app-icons/{id}/{hash}.png"))),
        ..avatar(&name[..1], 0x5865f2)
    }
}

fn factorio() -> Avatar {
    icon("Factorio", "1400000000000000900", "factorioicon")
}

fn dwarves() -> Avatar {
    icon("Deep Dwarves", "1400000000000000902", "dwarvesicon")
}

/// Two games added from Discord's list: Factorio tied to its program, and
/// Deep Dwarves shown without one, turned off.
pub fn added() -> Games {
    Games {
        added: vec![
            Added {
                name: "Factorio".into(),
                program: "/home/alice/.factorio/bin/x64/factorio".into(),
                icon: factorio(),
                shown: true,
            },
            Added {
                name: "Deep Dwarves".into(),
                program: String::new(),
                icon: dwarves(),
                shown: false,
            },
        ],
        ..games()
    }
}

/// The picker open on a search, two games found, one by another name.
pub fn picker() -> Games {
    Games {
        picker: Some(Picker {
            list: List::Ready,
            query: "fac".into(),
            matches: vec![
                Match {
                    name: "Factorio".into(),
                    alias: String::new(),
                    icon: factorio(),
                },
                Match {
                    name: "Deep Dwarves".into(),
                    alias: "Dwarves: The Deep Factory".into(),
                    icon: dwarves(),
                },
            ],
            more: true,
            ..Picker::default()
        }),
        ..added()
    }
}

/// Factorio chosen: the programs running, to tie it to one.
pub fn programs() -> Games {
    let mut page = picker();
    if let Some(picker) = &mut page.picker {
        picker.chosen = picker.matches.first().cloned();
        picker.programs = vec![
            Program {
                name: "factorio".into(),
                path: "/home/alice/.factorio/bin/x64/factorio".into(),
            },
            Program {
                name: "Game.exe".into(),
                path: "C:\\Games\\Deep Dwarves\\Game.exe".into(),
            },
        ];
    }
    page
}

/// The picker before Discord's list is there, downloading it.
pub fn downloading() -> Games {
    Games {
        picker: Some(Picker {
            list: List::Downloading,
            ..Picker::default()
        }),
        ..games()
    }
}

/// The picker after the list's download failed.
pub fn failed() -> Games {
    Games {
        picker: Some(Picker {
            list: List::Failed,
            ..Picker::default()
        }),
        ..games()
    }
}
