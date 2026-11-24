//! A stream on its way: a ring turning once a second over a note that says
//! what it waits for (`StreamWaiting`, `ui/voice.slint:658`). It may wait
//! for ever, so the ring turns only while someone may be looking
//! (`Motion::watched`). A stream's ground is black in every theme, so both
//! are a fixed grey. The stage's tiles draw it, its note on one line cut
//! short so it holds the height their room counts (`super::room`), or the
//! ring alone where the note has no room; picture in picture draws it with
//! the note wrapped.

use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{column, container, text};
use iced::{Alignment, Fill, Padding};

use anomi_theme::prelude::*;
use anomi_theme::typography;

use super::GREY;
use super::spinner::{self, Spinner, spinner};

/// The ring, 28 px, 14 px above the note, which is 14 px and kept 16 px
/// from the sides (`:660`, `:664`, `:669`).
pub(super) const RING: f32 = 28.0;
pub(super) const GAP: f32 = 14.0;
const SIDE: f32 = 16.0;
pub(super) const NOTE: f32 = typography::size::SMALL;

/// The ring over `note`, wrapped, in the middle of what it fills.
pub fn waiting<'a, M: 'a>(note: &'a str, motion: Motion) -> impl View<M> + 'a {
    laid(note, false, mark(RING, motion, false))
}

/// The ring over `note` on one line, cut short where it's too long: a
/// tile's; `stuck`, the still mark of a video that can't be shown.
pub(super) fn line<'a, M: 'a>(note: &'a str, motion: Motion, stuck: bool) -> impl View<M> + 'a {
    laid(note, true, mark(RING, motion, stuck))
}

/// The ring alone, `side` px, in the middle of what it fills, `raised`
/// round it.
pub(super) fn ring<'a, M: 'a>(
    side: f32,
    raised: Padding,
    motion: Motion,
    stuck: bool,
) -> impl View<M> + 'a {
    container(mark(side, motion, stuck))
        .center(Fill)
        .padding(raised)
}

/// The ring, `side` px, or the still mark of a video that can't be shown.
fn mark(side: f32, motion: Motion, stuck: bool) -> Spinner {
    if stuck {
        spinner::stuck(side, GREY)
    } else {
        spinner(side, GREY, motion)
    }
}

fn laid<'a, M: 'a>(note: &'a str, one_line: bool, mark: Spinner) -> impl View<M> + 'a {
    let mut note = text(note)
        .size(NOTE)
        .color(GREY)
        .width(Fill)
        .align_x(Alignment::Center);
    if one_line {
        note = note.wrapping(Wrapping::None).ellipsis(Ellipsis::End);
    }
    container(column![mark, note].spacing(GAP).align_x(Alignment::Center))
        .padding(Padding::ZERO.left(SIDE).right(SIDE))
        .center_y(Fill)
        .width(Fill)
}
