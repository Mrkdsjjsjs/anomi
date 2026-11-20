//! The card beside someone in a voice channel, on an app without a window
//! over the core's preview, Bob in its Lounge streaming Factorio.

use std::sync::Arc;

use serde_json::json;
use yapfast_core::application::Action;
use yapfast_core::chat::VoicePerson as Person;
use yapfast_core::yapfast_discord::activity::activities;

use anomi_vm::sidebar::channels::{HoverCard, Intent};
use anomi_vm::sidebar::{Intent as Sidebar, Place};

use crate::app::App;

fn app(name: &str) -> (App, std::path::PathBuf) {
    let folder = std::env::temp_dir().join(format!("anomi-card-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mut app = App::for_tests(&folder);
    drop(app.dispatch(Action::Preview));
    let bob = Person {
        id: "bob".into(),
        name: "Bob".into(),
        streaming: true,
        activities: activities(&json!([{"type": 0, "name": "Factorio",
            "application_id": "1400000000000000900", "timestamps": {"start": 1_000}}])),
        ..Person::default()
    };
    let state = &mut app.core.state;
    let lounge = state.rooms.iter_mut().find(|room| room.id == "lounge");
    Arc::make_mut(lounge.expect("the preview's Lounge")).people = vec![bob];
    state.list_changes += 1;
    app.refresh();
    (app, folder)
}

fn intent(app: &mut App, intent: Intent) {
    drop(super::super::handle(app, Sidebar::Channels(intent)));
}

fn card(app: &App) -> Option<&HoverCard> {
    match &app.vm.sidebar.place {
        Place::Server(server) => server.channels.hover.as_deref(),
        Place::Dms(_) => None,
    }
}

fn hover(user: &str) -> Intent {
    Intent::Hover {
        channel: "lounge".into(),
        user: user.into(),
    }
}

fn unhover(user: &str) -> Intent {
    Intent::Unhover {
        channel: "lounge".into(),
        user: user.into(),
    }
}

#[test]
fn a_card_shows_while_its_row_holds_the_pointer_and_asks_the_core_once() {
    let (mut app, folder) = app("shows");
    let row = |app: &App| {
        let Place::Server(server) = &app.vm.sidebar.place else {
            panic!("the preview's server");
        };
        let lounge = server.channels.rows.iter().find(|row| row.id == "lounge");
        lounge.and_then(|row| row.people.first().cloned())
    };
    let bob = row(&app).expect("Bob under the Lounge");
    assert!(
        bob.has_card() && bob.game.is_some(),
        "his game ends his row"
    );
    assert!(card(&app).is_none(), "no card until the pointer rests");
    let version = app.vm.sidebar.version;
    intent(&mut app, hover("bob"));
    let shown = card(&app).expect("Bob's card");
    assert!(shown.beside("lounge", "bob") && shown.stream.is_some());
    assert!(app.vm.sidebar.version > version, "the list shows it");
    // The core looks up where Factorio is sold, once.
    let asked = app.core.state.game_store.clone();
    assert_eq!(asked, Some(("1400000000000000900".into(), None)));
    // Another's row let go late changes nothing; his own takes it away.
    intent(&mut app, unhover("carol"));
    assert!(card(&app).is_some());
    intent(&mut app, unhover("bob"));
    assert!(card(&app).is_none());
    assert!(app.shown.sidebar.hover.at.is_none());
    assert_eq!(app.core.stream_previews_due(), None, "nothing falls due");
    drop(app);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn watching_or_going_to_the_store_takes_the_card_away() {
    let (mut app, folder) = app("acts");
    intent(&mut app, hover("bob"));
    intent(
        &mut app,
        Intent::Watch {
            channel: "lounge".into(),
            person: "bob".into(),
        },
    );
    assert!(card(&app).is_none(), "Watch Stream");
    intent(&mut app, hover("bob"));
    let url = "https://store.steampowered.com/app/427520/";
    intent(&mut app, Intent::OpenStore(url.into()));
    assert!(card(&app).is_none(), "the store's button");
    assert_eq!(app.asked.pages, [url]);
    drop(app);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_window_going_behind_another_takes_the_card_away() {
    let (mut app, folder) = app("behind");
    drop(crate::window::focused(&mut app, true));
    intent(&mut app, hover("bob"));
    assert!(card(&app).is_some());
    drop(crate::window::focused(&mut app, false));
    assert!(card(&app).is_none(), "gone with the focus");
    assert!(app.shown.sidebar.hover.at.is_none());
    assert_eq!(app.core.stream_previews_due(), None, "nothing falls due");
    drop(app);
    let _ = std::fs::remove_dir_all(&folder);
}
