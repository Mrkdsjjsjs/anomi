//! Game activity's picker on the preview: Discord's list handed in as read,
//! searched, a game tied to a program or shown without one, kept in the
//! layout under `discord_games`, switched off and removed; work that comes
//! back after the picker closed changes nothing. No test downloads the
//! list or waits on the tasks `update` returns: what they bring back is
//! handed to `due` here.

use serde_json::json;

use anomi_vm::settings::Intent;
use anomi_vm::settings::frame::Page;
use anomi_vm::settings::games::{Intent as Games, List, MATCHES};

use super::Due;
use super::source::{Directory, Program};
use crate::app::App;
use crate::layout::DiscordGame;
use crate::message::Once;
use crate::tests::{Window, start};

fn send(app: &mut App, intent: Games) {
    drop(crate::intent::settings::handle(app, Intent::Games(intent)));
}

fn due(app: &mut App, due: Due) {
    drop(super::due(app, due));
}

/// The preview with Settings open on Game activity.
fn opened(name: &str) -> Window {
    let mut window = start(name, true);
    drop(crate::intent::settings::open(&mut window.app, Page::Games));
    window
}

/// Discord's list, in its shape: a game with another name, and many more
/// than the picker shows.
fn directory() -> Directory {
    let mut games = vec![
        json!({"id": "1400000000000000900", "name": "Factorio",
            "aliases": ["Factorio: Space Age"]}),
        json!({"id": "1400000000000000902", "name": "Deep Dwarves",
            "aliases": ["Dwarves: The Deep"]}),
    ];
    games.extend(
        (1..=40).map(|n| json!({"id": (2000 + n).to_string(), "name": format!("Filler {n}")})),
    );
    let bytes = serde_json::to_vec(&games).unwrap();
    Directory::from_reader(bytes.as_slice()).unwrap()
}

/// Opens the picker and hands it the list as read; the opening's number.
fn picking(app: &mut App) -> u64 {
    send(app, Games::OpenPicker);
    let opening = app.shown.settings.kept.games.opening;
    let picker = app.vm.settings.games.picker.as_ref().expect("open");
    assert_eq!(picker.list, List::Reading, "the list is read first");
    due(app, Due::Read(opening, Once::new(Some(directory()))));
    opening
}

fn written(app: &mut App) -> serde_json::Value {
    app.layout.flush();
    let file = app.root.join(yapfast_core::layout_file::LAYOUT);
    serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
}

#[test]
fn a_game_from_discords_list_is_tied_to_a_program() {
    let mut window = opened("games-program");
    let app = &mut window.app;
    let opening = picking(app);
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert_eq!(picker.list, List::Ready);
    assert_eq!(picker.matches.len(), MATCHES, "the first games, no more");
    assert!(picker.more);
    send(app, Games::Search("THE DEEP".into()));
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert_eq!(picker.query, "THE DEEP");
    assert_eq!(picker.matches.len(), 1);
    assert_eq!(picker.matches[0].name, "Deep Dwarves");
    assert_eq!(picker.matches[0].alias, "Dwarves: The Deep");
    assert!(!picker.more);
    send(app, Games::PickGame(0));
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert_eq!(
        picker.chosen.as_ref().map(|game| game.name.as_str()),
        Some("Deep Dwarves")
    );
    assert!(picker.listing, "the programs are being listed");
    let program = Program {
        name: "Game.exe".into(),
        path: "C:\\Games\\Deep Dwarves\\Game.exe".into(),
    };
    due(app, Due::Programs(opening, vec![program]));
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert!(!picker.listing && picker.programs[0].name == "Game.exe");
    send(app, Games::PickProgram(0));
    assert!(app.vm.settings.games.picker.is_none(), "the picker closes");
    assert!(
        app.shown.settings.kept.games.directory.is_none(),
        "and the list goes"
    );
    let added = &app.vm.settings.games.added;
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].program, "C:\\Games\\Deep Dwarves\\Game.exe");
    assert!(added[0].shown);
    assert_eq!(
        written(app)["discord_games"],
        json!([{"id": "1400000000000000902", "name": "Deep Dwarves",
            "executable": "C:\\Games\\Deep Dwarves\\Game.exe"}])
    );
    // The same program tied to another game is that game now.
    let opening = picking(app);
    send(app, Games::PickGame(0));
    due(
        app,
        Due::Programs(
            opening,
            vec![Program {
                name: "Game.exe".into(),
                path: "C:\\Games\\Deep Dwarves\\Game.exe".into(),
            }],
        ),
    );
    send(app, Games::PickProgram(0));
    let kept = &app.layout.discord_games;
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "Factorio");
}

#[test]
fn a_game_without_a_program_shows_until_its_turned_off() {
    let mut window = opened("games-without");
    let app = &mut window.app;
    picking(app);
    send(app, Games::Search("factorio".into()));
    send(app, Games::PickGame(0));
    send(app, Games::WithoutProgram);
    let kept = app.layout.discord_games.clone();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].executable.is_empty() && !kept[0].hidden);
    assert!(kept[0].since.is_some(), "shown from now");
    assert_eq!(app.vm.settings.games.added[0].program, "");
    send(app, Games::ShowAdded(0, false));
    assert!(app.layout.discord_games[0].hidden);
    assert!(!app.vm.settings.games.added[0].shown);
    let file = written(app);
    assert_eq!(file["discord_games"][0]["hidden"], true);
    send(app, Games::ShowAdded(0, true));
    assert!(!app.layout.discord_games[0].hidden);
    assert!(written(app)["discord_games"][0].get("hidden").is_none());
    // Out of range, nothing changes.
    send(app, Games::ShowAdded(5, false));
    send(app, Games::RemoveAdded(5));
    assert_eq!(app.layout.discord_games.len(), 1);
    send(app, Games::RemoveAdded(0));
    assert!(app.layout.discord_games.is_empty() && app.vm.settings.games.added.is_empty());
    assert!(
        written(app).get("discord_games").is_none(),
        "written only while there's one"
    );
}

#[test]
fn what_comes_back_after_the_picker_closed_changes_nothing() {
    let mut window = opened("games-late");
    let app = &mut window.app;
    send(app, Games::OpenPicker);
    let opening = app.shown.settings.kept.games.opening;
    send(app, Games::ClosePicker);
    due(app, Due::Read(opening, Once::new(Some(directory()))));
    assert!(app.vm.settings.games.picker.is_none());
    assert!(app.shown.settings.kept.games.directory.is_none());
    // A later opening ignores the earlier one's work too.
    send(app, Games::OpenPicker);
    due(app, Due::Read(opening, Once::new(Some(directory()))));
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert_eq!(picker.list, List::Reading);
    // Leaving the page closes it and lets the list go.
    let opening = app.shown.settings.kept.games.opening;
    due(app, Due::Read(opening, Once::new(Some(directory()))));
    assert!(app.shown.settings.kept.games.directory.is_some());
    drop(crate::intent::settings::handle(
        app,
        Intent::Frame(anomi_vm::settings::frame::Intent::Choose(Page::About)),
    ));
    assert!(app.shown.settings.kept.games.directory.is_none());
    assert!(!app.shown.settings.kept.games.open);
}

#[test]
fn a_list_still_missing_after_its_download_says_so() {
    let mut window = opened("games-missing");
    let app = &mut window.app;
    send(app, Games::OpenPicker);
    let opening = app.shown.settings.kept.games.opening;
    // As if it had been downloaded already: none is asked for here.
    app.shown.settings.kept.games.downloaded = true;
    due(app, Due::Read(opening, Once::new(None)));
    let picker = app.vm.settings.games.picker.as_ref().unwrap();
    assert_eq!(picker.list, List::Failed);
    due(app, Due::Downloaded(opening, Err("offline".into())));
    assert_eq!(
        app.vm.settings.games.picker.as_ref().unwrap().list,
        List::Failed
    );
}

#[test]
fn games_added_reach_the_page_from_the_layout() {
    let mut window = opened("games-layout");
    let app = &mut window.app;
    app.layout.edit(|saved| {
        saved.discord_games = vec![DiscordGame {
            id: "1400000000000000900".into(),
            name: "Factorio".into(),
            executable: "/opt/factorio/bin/x64/factorio".into(),
            hidden: true,
            ..DiscordGame::default()
        }];
    });
    app.show(crate::project::Reach::SETTINGS);
    let added = &app.vm.settings.games.added;
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].name, "Factorio");
    assert!(!added[0].shown);
    assert_eq!(
        added[0].icon.initial, "F",
        "its initial until the icon comes"
    );
}
