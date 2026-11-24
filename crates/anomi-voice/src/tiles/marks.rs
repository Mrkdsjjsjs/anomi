//! What a tile says over its picture, 10 px in from its edges: at its top
//! left "Starting camera..." while this account's camera is on its way
//! (`ui/voice.slint:1094`), and at its bottom left its chip (`TileChip`,
//! `:819`). The pill offering a stream that a click watches (`:1100`) is in
//! the middle of the stream's own tile (`fill`), whose top right has its
//! LIVE badge (`controls`). Each shows only where the tile has room for it
//! (`super::room`).

use iced::widget::{Row, Space, column, container, text};
use iced::{Background, Border, Color, Fill, Padding};
use yapfast_lang::{text_in, tr};

use anomi_theme::prelude::*;
use anomi_theme::typography;
use anomi_vm::voice::tiles::{Intent, Tile};

use super::pulse::pulse;
use super::room::{Middle, Room};
use super::{INSET, PILL, PILL_SIDE, PILL_TEXT, SCRIM, chip};

/// More than a letter of a pill's words takes, on average, at its size in
/// bold.
const LETTER: f32 = 9.0;

pub fn view(tile: &Tile, room: Room, motion: Motion) -> Element<'_, Intent> {
    let mut top: Row<Element<'_, Intent>> = Row::new();
    if room.starting {
        top = top.push(starting_pill(motion));
    }
    top = top.push(Space::new().width(Fill).boxed());
    let mut marks = column![top, Space::new().height(Fill)];
    if room.chip {
        let stuck = matches!(
            tile.fill,
            anomi_vm::voice::tiles::Fill::Watched { stuck: true, .. }
        );
        let spinning = (room.middle == Middle::InChip).then_some((motion, stuck));
        marks = marks.push(chip::view(&tile.chip, spinning));
    }
    marks.padding(INSET).width(Fill).height(Fill).boxed()
}

/// The words of "Watch stream", as `ParticipantGrid` words it.
fn watch_words() -> &'static str {
    text_in("ParticipantGrid", "Watch stream")
}

/// At most how wide "Watch stream" is, from its words' letters
/// (`super::room`).
pub(super) fn watch_width() -> f32 {
    2.0 * PILL_SIDE + watch_words().chars().count() as f32 * LETTER
}

/// "Watch stream" on the danger colour, as `ParticipantGrid` words it
/// (`:1042`, `:1105`).
pub fn watch_pill<'a>() -> Element<'a, Intent> {
    words(watch_words(), |theme: &Theme| container::Style {
        background: Some(Background::Color(theme.palette().danger)),
        text_color: Some(theme.palette().on_danger),
        border: Border::default().rounded(PILL / 2.0),
        ..container::Style::default()
    })
}

/// "Starting camera...", white on the scrim, beating while `motion` plays.
fn starting_pill<'a>(motion: Motion) -> Element<'a, Intent> {
    let bare = |_: &Theme| container::Style::default();
    pulse(
        words(tr!("Starting camera..."), bare),
        SCRIM,
        Color::WHITE,
        PILL / 2.0,
        motion,
    )
    .boxed()
}

/// A pill's words on its `ground`, 10 px from its ends, in the colour of
/// what holds them.
fn words<'a>(label: &'a str, ground: fn(&Theme) -> container::Style) -> Element<'a, Intent> {
    container(text(label).size(PILL_TEXT).font(typography::BOLD))
        .padding(Padding::ZERO.left(PILL_SIDE).right(PILL_SIDE))
        .center_y(PILL)
        .style(ground)
        .boxed()
}
