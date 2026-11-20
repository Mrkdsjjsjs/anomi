//! How someone's voice is cleaned in a Discord call, Anomi's own: the
//! suppressor their menu's "Suppress their noise" chooses
//! (`project::menus::subs::their_noise`), the "Remove my echo" box, and the
//! card's sliders for how much of their noise goes and how far our echo
//! from them is turned down. Each reaches the call's audio at once
//! (`yapfast_core::volume::set_cleaning`), and `layout.json` keeps it
//! (`cleaned_voices`), a slider's as it's let go. The card shows a slider
//! for each that's on; as one comes or goes, the card and the rows under it
//! are placed again from where they opened.
//!
//! A watched stream's sound is cleaned the same way from the menu of its
//! tile ([`Sound::Stream`], `super::stream`): kept apart from the
//! streamer's voice (`volume::set_stream_cleaning`, `cleaned_streams`),
//! its "Suppress noise" row keeping its note rather than naming the
//! suppressor chosen.

use std::collections::BTreeMap;

use yapfast_core::noise::Suppressor;
use yapfast_core::volume::{self, Cleaning};

use anomi_vm::menu::{Intent, Menu, Row, Spot};
use anomi_vm::shell::Layer;
use anomi_vm::voice::participant_menu::{self as card, ParticipantMenu};

use crate::app::App;
use crate::layout::{CleanedVoice, Saved};
use crate::project::menus::subs::{suppressor_name, their_noise_id};

#[cfg(test)]
mod tests;

/// Whose sound is cleaned: someone's voice in a call, or the sound of
/// their stream watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sound {
    Voice,
    Stream,
}

impl Sound {
    /// What the card open over a menu cleans.
    pub(crate) fn of(card: &ParticipantMenu) -> Self {
        if card.stream {
            Self::Stream
        } else {
            Self::Voice
        }
    }

    /// How `person`'s is cleaned now.
    fn cleaning(self, person: &str) -> Cleaning {
        match self {
            Self::Voice => volume::cleaning(person),
            Self::Stream => volume::stream_cleaning(person),
        }
    }

    /// Hands the call's audio, or the watching worker, how `person`'s is
    /// cleaned.
    fn set(self, person: &str, cleaning: Cleaning) {
        match self {
            Self::Voice => volume::set_cleaning(person, cleaning),
            Self::Stream => volume::set_stream_cleaning(person, cleaning),
        }
    }

    /// Where `layout.json` keeps it.
    fn kept(self, saved: &mut Saved) -> &mut BTreeMap<String, CleanedVoice> {
        match self {
            Self::Voice => &mut saved.cleaned_voices,
            Self::Stream => &mut saved.cleaned_streams,
        }
    }
}

/// A suppressor chosen in someone's submenu: it takes their noise out from
/// the call's next tick, at the strength they have, or their stream's from
/// its next frame, and the submenu's mark moves to it while the menu stays.
pub(super) fn suppress(app: &mut App, sound: Sound, person: &str, suppressor: Suppressor) {
    let cleaning = Cleaning {
        noise: suppressor,
        ..sound.cleaning(person)
    };
    keep(app, sound, person, cleaning, true);
    // A stream's row keeps its note of what this does to music.
    let named = (sound == Sound::Voice).then(|| suppressor_name(suppressor));
    mark(&mut app.vm.menu, &their_noise_id(suppressor), named);
    show(app, sound, person, cleaning);
}

/// "Remove my echo" ticked or unticked.
pub(super) fn remove_echo(app: &mut App, sound: Sound, person: &str, on: bool) {
    let cleaning = Cleaning {
        own_echo: on,
        ..sound.cleaning(person)
    };
    keep(app, sound, person, cleaning, true);
    show(app, sound, person, cleaning);
}

/// The card's noise suppression strength moved, in percent: heard at once,
/// kept once `last`.
pub(crate) fn noise_strength(app: &mut App, sound: Sound, person: &str, strength: i32, last: bool) {
    let cleaning = Cleaning {
        noise_strength: within(strength, card::STRENGTH),
        ..sound.cleaning(person)
    };
    keep(app, sound, person, cleaning, last);
    show(app, sound, person, cleaning);
}

/// How far our echo from them is turned down moved, in dB: heard at once,
/// kept once `last`.
pub(crate) fn echo_depth(app: &mut App, sound: Sound, person: &str, depth: i32, last: bool) {
    let cleaning = Cleaning {
        echo_depth: within(depth, card::DEPTH),
        ..sound.cleaning(person)
    };
    keep(app, sound, person, cleaning, last);
    show(app, sound, person, cleaning);
}

/// A slider's value, from 0 to `most`.
fn within(value: i32, most: i32) -> u8 {
    u8::try_from(value.clamp(0, most)).unwrap_or(u8::MAX)
}

/// Hands the call's audio how `person`'s voice is cleaned, or the watching
/// worker their stream's, and, when `kept`, `layout.json`: each person's
/// choices, none of them once nothing is chosen.
fn keep(app: &mut App, sound: Sound, person: &str, cleaning: Cleaning, kept: bool) {
    sound.set(person, cleaning);
    if !kept {
        return;
    }
    let voice = CleanedVoice::of(cleaning);
    app.layout.edit(|saved| {
        let people = sound.kept(saved);
        if voice == CleanedVoice::default() {
            people.remove(person);
        } else {
            people.insert(person.to_owned(), voice);
        }
    });
}

/// The card's sliders for how `cleaning` cleans their voice: one for their
/// noise while a suppressor is chosen, one for our echo while it's taken
/// out.
pub(super) fn sliders(card: &mut ParticipantMenu, cleaning: Cleaning) {
    card.noise_strength =
        (cleaning.noise != Suppressor::Off).then_some(i32::from(cleaning.noise_strength));
    card.echo_depth = cleaning.own_echo.then_some(i32::from(cleaning.echo_depth));
}

/// Shows `cleaning` on the card open over `person`'s menu, or their
/// stream's, if one is.
fn show(app: &mut App, sound: Sound, person: &str, cleaning: Cleaning) {
    let open = app.vm.shell.is_open(Layer::ParticipantMenu);
    let card = &mut app.vm.voice.participant_menu;
    if !open || card.person != person || Sound::of(card) != sound {
        return;
    }
    let height = card.height();
    sliders(card, cleaning);
    if card.height() != height {
        place(app);
    }
}

/// Places the card and the rows under it again, at the pointer they opened
/// at, as `super::open_voice` did: the rows stay right under the card,
/// both moving up when they no longer fit below.
pub(super) fn place(app: &mut App) {
    let card = &app.vm.voice.participant_menu;
    let rows = anomi_widgets::menu::size(&app.vm.menu.rows, card::WIDTH);
    let (top, below) = card.place(app.shown.menus.card_at, rows, app.size);
    let joined = card.joined;
    app.vm
        .shell
        .open_at(Layer::ParticipantMenu, Spot::pointer(top));
    if joined && app.vm.menu.open {
        app.vm.menu.at = Spot::pointer(below);
    }
}

/// Marks the row choosing `id` in a submenu of the menu showing as the one
/// chosen, its others not, and names it under the row that opens it when
/// `named`.
fn mark(menu: &mut Menu, id: &str, named: Option<&str>) {
    let chooses = |row: &Row| {
        matches!(&row.intent,
            Some(anomi_vm::Intent::Menu(Intent::Choose(choice))) if choice.id == id)
    };
    for row in &mut menu.rows {
        if !row.submenu.iter().any(chooses) {
            continue;
        }
        for sub in &mut row.submenu {
            if sub.radio {
                let picked = chooses(&*sub);
                sub.checked = picked;
            }
        }
        if let Some(chosen) = named {
            row.subtitle = chosen.to_owned();
        }
    }
}
