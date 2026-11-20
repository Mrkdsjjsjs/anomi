//! The card beside someone's row under a voice channel, from the core's
//! preview with Bob in its Lounge playing Factorio.

use std::sync::Arc;

use serde_json::json;
use yapfast_core::application::{Action, Application};
use yapfast_core::chat::{Picture, Pixels, StorePage, VoicePerson as Person};
use yapfast_core::voice::{self, Phase};
use yapfast_core::yapfast_discord::activity::activities;

use anomi_vm::icon::Icon;
use anomi_vm::kind::{Elapsed, Expected};
use anomi_vm::sidebar::channels::{ChannelKind, ChannelRow, Channels, HoverCard, VoicePerson};

use super::{Hover, show};

const FACTORIO: &str = "1400000000000000900";

fn preview(streaming: bool) -> Application {
    let mut core = Application::default();
    drop(core.dispatch(Action::Preview));
    let bob = Person {
        id: "bob".into(),
        name: "Bob".into(),
        streaming,
        activities: activities(&json!([
            {"type": 2, "name": "Spotify"},
            {"type": 0, "name": "Factorio", "application_id": FACTORIO,
             "timestamps": {"start": 1_000}},
        ])),
        ..Person::default()
    };
    let lounge = core.state.rooms.iter_mut().find(|room| room.id == "lounge");
    Arc::make_mut(lounge.expect("the preview's Lounge")).people = vec![bob];
    core
}

fn hovered() -> Hover {
    Hover {
        at: Some(("lounge".into(), "bob".into())),
        ..Hover::default()
    }
}

/// The list as the sidebar shows it: the Lounge with Bob under it, and
/// Carol, who neither plays nor streams.
fn listed() -> Channels {
    let person = |user: &str, game: Option<Expected>| VoicePerson {
        id: user.into(),
        user: user.into(),
        game,
        ..VoicePerson::default()
    };
    Channels {
        rows: vec![ChannelRow {
            id: "lounge".into(),
            kind: ChannelKind::Voice,
            people: vec![
                person("bob", Some(Expected::Waiting)),
                person("carol", None),
            ],
            ..ChannelRow::default()
        }],
        ..Channels::default()
    }
}

/// The card shown into the list, and whether that changed it.
fn shown(core: &Application, hover: &mut Hover) -> (bool, Option<HoverCard>) {
    let mut channels = listed();
    let changed = show(core, hover, &mut channels);
    (changed, channels.hover.map(|card| *card))
}

#[test]
fn a_streamers_card_shows_the_stream_and_the_game_they_play() {
    let core = preview(true);
    let mut hover = hovered();
    let (changed, card) = shown(&core, &mut hover);
    let card = card.expect("Bob's card");
    assert!(changed);
    let stream = card.stream.expect("his stream");
    assert_eq!(stream.person, "bob");
    assert!(stream.preview.is_none() && !stream.watching, "no still yet");
    let game = card.game.expect("his game, not his song");
    assert_eq!(game.name, "Factorio");
    assert!(matches!(game.elapsed, Some(Elapsed::Since(_))));
    assert_eq!(game.store, None, "nowhere known yet");
    // Built again, the time played is the same: nothing changed.
    let mut channels = listed();
    show(&core, &mut hover, &mut channels);
    assert!(!show(&core, &mut hover, &mut channels));
    // Another server's list, or a fold, doesn't show his row: no card.
    let mut elsewhere = Channels::default();
    assert!(!show(&core, &mut hover, &mut elsewhere));
    assert!(elsewhere.hover.is_none());
}

#[test]
fn the_still_the_store_and_a_watch_show_as_they_come() {
    crate::pictures::install();
    let mut core = preview(true);
    core.state.stream_previews.insert(
        "bob".into(),
        Arc::new(Picture {
            width: 1,
            height: 1,
            rgba: Pixels::new(1, 1, vec![1, 2, 3, 255]),
            ..Default::default()
        }),
    );
    core.state.game_store = Some((FACTORIO.into(), StorePage::of("steam", "427520")));
    let (_, card) = shown(&core, &mut hovered());
    let card = card.expect("Bob's card");
    assert!(
        card.stream
            .as_ref()
            .is_some_and(|stream| stream.preview.is_some())
    );
    let store = card.game.and_then(|game| game.store).expect("its store");
    assert_eq!(store.glyph, Icon::Steam);
    assert_eq!(store.url, "https://store.steampowered.com/app/427520/");
    // Watching him in the call: said, and the still let go by the core.
    core.state.voice = voice::State {
        phase: Phase::Connected,
        room: Some("lounge".into()),
        watching: Some(Box::new(voice::Watching {
            user: "bob".into(),
            name: "Bob".into(),
            opening: false,
            frames: None,
            viewers: vec![],
        })),
        ..Default::default()
    };
    let (_, card) = shown(&core, &mut hovered());
    assert!(
        card.and_then(|card| card.stream)
            .is_some_and(|stream| stream.watching)
    );
}

#[test]
fn a_player_has_no_stream_and_nobody_has_nothing_to_show() {
    let core = preview(false);
    let (_, card) = shown(&core, &mut hovered());
    let card = card.expect("Bob playing");
    assert!(card.stream.is_none() && card.game.is_some());
    let mut nobody = Hover {
        at: Some(("lounge".into(), "carol".into())),
        ..Hover::default()
    };
    assert_eq!(shown(&core, &mut nobody), (false, None));
    assert_eq!(shown(&core, &mut Hover::default()), (false, None));
}
