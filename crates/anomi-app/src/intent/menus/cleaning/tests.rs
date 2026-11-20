//! Someone's voice cleaned from their menu in a call and the card over it.

use anomi_vm::menu::{self, Choice, Spot};
use anomi_vm::shell::Layer;
use anomi_vm::voice::participant_menu::{self as card, ParticipantMenu};
use yapfast_core::noise::Suppressor;
use yapfast_core::volume;

use crate::message::Message;
use crate::project::menus::subs::suppressor_name;
use crate::project::menus::tests::discord_app;
use crate::tests::Folder;

fn ask(intent: impl Into<anomi_vm::Intent>) -> Message {
    Message::Intent(intent.into())
}

/// Where the menu and the card over it opened.
const AT: Spot = Spot {
    anchor: iced::Rectangle {
        x: 400.0,
        y: 300.0,
        width: 0.0,
        height: 0.0,
    },
    side: anomi_vm::menu::Side::Below,
    align: anomi_vm::menu::Align::Start,
    gap: 0.0,
};

/// A suppressor chosen for their voice, and the box that takes out this
/// account's echo, reach the call's audio and are kept, each alone; the
/// card over the menu grows a slider for each, whose value is heard at once
/// and kept when let go, and the rows move down under it.
#[test]
fn a_persons_voice_is_cleaned_from_their_menu_and_card_in_a_call() {
    let folder = Folder::new("menus-cleaning");
    let mut app = discord_app(&folder);
    // A person of this test alone: the registry is the process's.
    let person = "discord:u-cleaned";
    let target = format!("in-voice:discord:g1:c1/{person}");
    let choice = |id: &str| Choice::person(target.clone(), id);
    let row = |id: &str| menu::Row::new(id, choice(id).into());
    let suppressors = [
        "their-noise:off",
        "their-noise:rnnoise",
        "their-noise:deepfilternet",
    ]
    .map(|id| row(id).radio(id == "their-noise:off"))
    .to_vec();
    app.vm.menu = menu::Menu::new(
        1,
        AT,
        vec![
            menu::Row::opens("suppress-their-noise", suppressors),
            row("remove-my-echo").checkbox(false),
        ],
    );
    // Their card over it, as it opened at the menu's spot.
    app.size = iced::Size::new(1400.0, 860.0);
    app.vm.voice.participant_menu = ParticipantMenu {
        person: person.into(),
        joined: true,
        ..ParticipantMenu::default()
    };
    app.shown.menus.card_at = AT.anchor.position();
    app.vm.shell.open_at(Layer::ParticipantMenu, AT);
    let plain = app.vm.voice.participant_menu.height();

    drop(app.update(ask(choice("their-noise:rnnoise"))));
    let cleaning = volume::cleaning(person);
    assert_eq!(
        (cleaning.noise, cleaning.own_echo),
        (Suppressor::RNNoise, false)
    );
    assert_eq!(app.layout.cleaned_voices[person].noise, Suppressor::RNNoise);
    let marked: Vec<bool> = (app.vm.menu.rows[0].submenu.iter())
        .map(|row| row.checked)
        .collect();
    assert_eq!(marked, [false, true, false], "the mark moves at once");
    assert_eq!(app.vm.menu.rows[0].subtitle, "RNNoise");
    let card = &app.vm.voice.participant_menu;
    assert_eq!(card.noise_strength, Some(100), "all of it, as before");
    assert_eq!(card.echo_depth, None);
    assert!(card.height() > plain, "a slider more");
    let top = AT.anchor.position().y;
    let rows_at = top + card.height() - card::HEAD_OVERLAP;
    assert_eq!(app.vm.menu.at.anchor.y, rows_at, "the rows right under it");

    // Its strength: heard while dragged, kept when let go.
    let strength = |strength, last| card::Intent::NoiseStrength {
        person: person.into(),
        strength,
        last,
    };
    drop(app.update(ask(strength(60, false))));
    assert_eq!(volume::cleaning(person).noise_strength, 60);
    assert_eq!(app.layout.cleaned_voices[person].noise_strength, None);
    drop(app.update(ask(strength(60, true))));
    assert_eq!(app.layout.cleaned_voices[person].noise_strength, Some(60));

    drop(app.update(ask(choice("remove-my-echo"))));
    assert!(app.vm.menu.rows[1].checked, "ticked at once");
    assert!(volume::cleaning(person).own_echo);
    assert!(app.layout.cleaned_voices[person].echo);
    assert_eq!(app.vm.voice.participant_menu.echo_depth, Some(20));
    let depth = |depth, last| card::Intent::EchoDepth {
        person: person.into(),
        depth,
        last,
    };
    drop(app.update(ask(depth(30, true))));
    assert_eq!(volume::cleaning(person).echo_depth, 30);
    assert_eq!(app.layout.cleaned_voices[person].echo_depth, Some(30));

    // Off and unticked: the sliders go, the strengths are kept for next
    // time.
    drop(app.update(ask(choice("their-noise:off"))));
    drop(app.update(ask(choice("remove-my-echo"))));
    let cleaning = volume::cleaning(person);
    assert!(!cleaning.any());
    assert_eq!((cleaning.noise_strength, cleaning.echo_depth), (60, 30));
    let card = &app.vm.voice.participant_menu;
    assert_eq!((card.noise_strength, card.echo_depth), (None, None));
    assert_eq!(card.height(), plain);
    let off = suppressor_name(Suppressor::Off);
    assert_eq!(app.vm.menu.rows[0].subtitle, off);
    let kept = app.layout.cleaned_voices[person];
    assert_eq!((kept.noise, kept.echo), (Suppressor::Off, false));
    // Back to as they come: forgotten.
    drop(app.update(ask(strength(100, true))));
    drop(app.update(ask(depth(20, true))));
    assert!(!app.layout.cleaned_voices.contains_key(person));
    assert_eq!(volume::cleaning(person), volume::Cleaning::NOTHING);
}
