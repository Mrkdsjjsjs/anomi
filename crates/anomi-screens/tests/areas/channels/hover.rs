//! The card beside someone's row in a voice channel: asked for once the
//! pointer rests on the row of one who plays or streams, let go a moment
//! after the pointer left the row and the card, and its buttons. Bob is the
//! Lounge's first, 176-206 px down; Carol, who does neither, is next.

use std::time::{Duration, Instant};

use iced::{Event, Point, window};

use anomi_vm::fixtures::sidebar::channels::{self, LOUNGE, bob};
use anomi_vm::sidebar::channels::{Channels, Intent};

use super::{CAROL_Y, moved, simulate};
use crate::support::asked;

const BOB_Y: f32 = 191.0;
/// Long after any wait the row sets: the moment a frame then comes.
const LATER: Duration = Duration::from_secs(2);

fn redrawn_later() -> Event {
    Event::Window(window::Event::RedrawRequested(Instant::now() + LATER))
}

/// Rests the pointer at `at`, then lets time pass.
fn rest(list: &Channels, at: Point) -> Vec<Intent> {
    let mut ui = simulate(list);
    ui.point_at(at);
    let _ = ui.simulate([moved(at), redrawn_later()]);
    asked(ui)
}

fn hover() -> Intent {
    Intent::Hover {
        channel: LOUNGE.into(),
        user: bob().user,
    }
}

#[test]
fn resting_on_someone_who_streams_or_plays_asks_for_their_card() {
    let at = Point::new(120.0, BOB_Y);
    assert_eq!(rest(&channels::channels(), at), [hover()], "streaming");
    assert_eq!(rest(&channels::playing(), at), [hover()], "playing");
    let carol = Point::new(120.0, CAROL_Y);
    let asked = rest(&channels::channels(), carol);
    assert!(asked.is_empty(), "Carol does neither: {asked:?}");
}

#[test]
fn the_card_goes_a_moment_after_the_pointer_left_the_row_and_the_card() {
    let list = channels::streaming();
    let mut ui = simulate(&list);
    let bob_at = Point::new(120.0, BOB_Y);
    ui.point_at(bob_at);
    let _ = ui.simulate([moved(bob_at)]);
    let away = Point::new(120.0, 600.0);
    ui.point_at(away);
    let _ = ui.simulate([moved(away), redrawn_later()]);
    assert_eq!(
        asked(ui),
        [Intent::Unhover {
            channel: LOUNGE.into(),
            user: bob().user,
        }]
    );
}

/// The window going behind another lets the card go at once, the pointer
/// still on the row, and it isn't asked for again until the pointer comes
/// back.
#[test]
fn the_card_goes_as_the_window_goes_behind_another() {
    let list = channels::streaming();
    let mut ui = simulate(&list);
    let bob_at = Point::new(120.0, BOB_Y);
    ui.point_at(bob_at);
    let unfocused = Event::Window(window::Event::Unfocused);
    let _ = ui.simulate([moved(bob_at), unfocused, moved(bob_at), redrawn_later()]);
    assert_eq!(
        asked(ui),
        [Intent::Unhover {
            channel: LOUNGE.into(),
            user: bob().user,
        }]
    );
}

#[test]
fn the_card_watches_the_stream_and_opens_the_games_store() {
    let list = channels::streaming();
    let mut ui = simulate(&list);
    ui.find("The stream has started. Don't miss it!")
        .expect("no still yet");
    ui.click("Watch Stream").expect("the card's button");
    assert_eq!(
        asked(ui),
        [Intent::Watch {
            channel: LOUNGE.into(),
            person: bob().id,
        }]
    );
    let mut ui = simulate(&list);
    ui.click("Get on Steam").expect("the store's button");
    assert_eq!(
        asked(ui),
        [Intent::OpenStore(
            "https://store.steampowered.com/app/427520/".into()
        )]
    );
}

#[test]
fn a_stream_watched_already_is_said_and_not_offered() {
    let list = channels::watching();
    let mut ui = simulate(&list);
    let _ = ui.click("Watching Stream");
    let asked = asked(ui);
    assert!(
        !asked
            .iter()
            .any(|intent| matches!(intent, Intent::Watch { .. })),
        "{asked:?}"
    );
}

#[test]
fn a_player_has_no_stream_to_watch() {
    let list = channels::playing();
    let mut ui = simulate(&list);
    ui.find("Playing now").expect("the header");
    ui.find("Factorio").expect("the game");
    assert!(ui.find("Watch Stream").is_err());
}
