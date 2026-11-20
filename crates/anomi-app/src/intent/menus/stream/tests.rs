//! A watched stream's sound cleaned from the menu of its tile and the card
//! over it, apart from its streamer's voice, and kept by the streamer under
//! a key of its own.

use anomi_vm::menu::{Choice, Spot};
use anomi_vm::shell::Layer;
use anomi_vm::voice::participant_menu as card;
use yapfast_core::noise::Suppressor;
use yapfast_core::voice::Watching;
use yapfast_core::volume;

use crate::message::Message;
use crate::project::menus::tests::discord_app;
use crate::tests::Folder;

fn ask(intent: impl Into<anomi_vm::Intent>) -> Message {
    Message::Intent(intent.into())
}

/// A streamer of this test alone: the registry is the process's.
const STREAMER: &str = "discord:u-streams";
const ROOM: &str = "discord:g1:c1";

#[test]
fn a_watched_streams_sound_is_cleaned_from_the_menu_of_its_tile() {
    let folder = Folder::new("menus-stream");
    let mut app = discord_app(&folder);
    app.size = iced::Size::new(1400.0, 860.0);
    app.core.state.voice.room = Some(ROOM.into());
    app.core.state.voice.watching = Some(Box::new(Watching {
        user: STREAMER.into(),
        name: "Bob".into(),
        opening: false,
        frames: None,
        viewers: Vec::new(),
    }));
    let at = Spot::pointer(iced::Point::new(400.0, 300.0));
    drop(super::open(&mut app, ROOM.into(), STREAMER.into(), at));
    assert!(app.vm.shell.is_open(Layer::ParticipantMenu));
    let shown = &app.vm.voice.participant_menu;
    assert!(shown.stream && shown.joined);
    assert_eq!(shown.name, "Bob's screen");
    assert_eq!(
        (shown.noise_strength, shown.echo_depth),
        (None, None),
        "off"
    );
    let plain = shown.height();
    assert_eq!(app.vm.menu.rows.len(), 2);
    let note = app.vm.menu.rows[0].subtitle.clone();
    assert!(note.contains("speech"), "{note}");

    let target = format!("in-stream:{ROOM}/{STREAMER}");
    let choice = |id: &str| Choice::person(target.clone(), id);
    drop(app.update(ask(choice("their-noise:rnnoise"))));
    assert_eq!(volume::stream_cleaning(STREAMER).noise, Suppressor::RNNoise);
    assert_eq!(
        volume::cleaning(STREAMER),
        volume::Cleaning::NOTHING,
        "not their voice"
    );
    assert_eq!(
        app.layout.cleaned_streams[STREAMER].noise,
        Suppressor::RNNoise
    );
    assert!(!app.layout.cleaned_voices.contains_key(STREAMER));
    let marked: Vec<bool> = (app.vm.menu.rows[0].submenu.iter())
        .map(|row| row.checked)
        .collect();
    assert_eq!(marked, [false, true, false]);
    assert_eq!(app.vm.menu.rows[0].subtitle, note, "the note stays");
    let shown = &app.vm.voice.participant_menu;
    assert_eq!(shown.noise_strength, Some(100));
    assert!(shown.height() > plain, "a slider more");

    // The card's slider is the stream's.
    let strength = |strength, last| card::Intent::NoiseStrength {
        person: STREAMER.into(),
        strength,
        last,
    };
    drop(app.update(ask(strength(40, true))));
    assert_eq!(volume::stream_cleaning(STREAMER).noise_strength, 40);
    assert_eq!(
        app.layout.cleaned_streams[STREAMER].noise_strength,
        Some(40)
    );
    assert_eq!(volume::cleaning(STREAMER).noise_strength, 100);

    drop(app.update(ask(choice("remove-my-echo"))));
    assert!(app.vm.menu.rows[1].checked);
    assert!(volume::stream_cleaning(STREAMER).own_echo);
    assert_eq!(app.vm.voice.participant_menu.echo_depth, Some(20));
    let depth = card::Intent::EchoDepth {
        person: STREAMER.into(),
        depth: 30,
        last: true,
    };
    drop(app.update(ask(depth)));
    assert_eq!(app.layout.cleaned_streams[STREAMER].echo_depth, Some(30));

    // Off and unticked, both strengths back as they were: forgotten.
    drop(app.update(ask(choice("their-noise:off"))));
    drop(app.update(ask(choice("remove-my-echo"))));
    drop(app.update(ask(strength(100, true))));
    let back = card::Intent::EchoDepth {
        person: STREAMER.into(),
        depth: 20,
        last: true,
    };
    drop(app.update(ask(back)));
    assert!(!app.layout.cleaned_streams.contains_key(STREAMER));
    assert_eq!(volume::stream_cleaning(STREAMER), volume::Cleaning::NOTHING);
}

#[test]
fn a_stream_not_watched_opens_its_streamers_menu() {
    let folder = Folder::new("menus-stream-unwatched");
    let mut app = discord_app(&folder);
    app.size = iced::Size::new(1400.0, 860.0);
    let at = Spot::pointer(iced::Point::new(400.0, 300.0));
    drop(super::open(&mut app, ROOM.into(), STREAMER.into(), at));
    assert!(
        !app.vm.voice.participant_menu.stream,
        "no card of its sound"
    );
    let ours = app
        .vm
        .menu
        .rows
        .iter()
        .any(|row| row.text == "Suppress noise");
    assert!(!ours, "nor its rows");
}
