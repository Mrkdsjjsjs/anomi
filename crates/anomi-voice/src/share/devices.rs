//! "Devices": a camera or a capture card to stream as the stream itself,
//! as Discord's tab (`discord-golive-4-devices-tab.png`). The cameras the
//! call's camera lists, OBS's virtual one among them, as cards three to a
//! row in the middle, the one picked ringed, scrolling under the words when
//! there are more than two rows; without any, Discord's empty state: the
//! camera, "No capture devices found." and what to do about it.

use iced::widget::{Column, Row, column, container, svg, text};
use iced::{Alignment, Fill};
use yapfast_lang::tr;

use anomi_theme::prelude::*;
use anomi_theme::{catalog, typography};
use anomi_vm::icon::Icon;
use anomi_vm::voice::share::{Intent, Share};
use anomi_widgets::icon::icon;
use anomi_widgets::list::scroll_view;

use super::apps::{UNDER_HEADING, heading};
use super::tile::{self, tile};

/// Cards to a row, and the rows shown before they scroll.
const PER_ROW: usize = 3;
const ROWS: usize = 2;
/// The empty state's camera, and the room between its parts.
const EMPTY_ICON: f32 = 28.0;
const EMPTY_GAP: f32 = 12.0;
/// The empty state's first line.
const EMPTY_TITLE: f32 = 18.0;

pub fn view(share: &Share, motion: Motion) -> Element<'_, Intent> {
    if share.cameras.is_empty() {
        return empty();
    }
    let first = share.first_camera();
    let mut lines: Column<Element<'_, Intent>> =
        Column::new().spacing(tile::GAP).align_x(Alignment::Center);
    for (at, chunk) in share.cameras.chunks(PER_ROW).enumerate() {
        let mut line: Row<Element<'_, Intent>> = Row::new().spacing(tile::GAP);
        for (offset, name) in chunk.iter().enumerate() {
            let index = first + at * PER_ROW + offset;
            let picked = share.source == Some(index);
            line = line.push(tile(Icon::Video, name, Intent::Source(index), picked));
        }
        lines = lines.push(line.boxed());
    }
    let words = heading(tr!("Stream a camera or a capture card"));
    if share.cameras.len() <= PER_ROW * ROWS {
        let parts = column![words, lines]
            .spacing(UNDER_HEADING)
            .align_x(Alignment::Center);
        return container(parts).center(Fill).boxed();
    }
    let list = scroll_view(container(lines.width(Fill)).center_x(Fill), motion).height(Fill);
    column![words, list]
        .spacing(UNDER_HEADING)
        .width(Fill)
        .height(Fill)
        .boxed()
}

/// No camera to stream: Discord's empty state, its words this client's.
fn empty<'a>() -> Element<'a, Intent> {
    let parts = column![
        icon(Icon::Video, EMPTY_ICON).style(|theme: &Theme, _| svg::Style {
            color: Some(theme.palette().muted),
        }),
        text(tr!("No capture devices found."))
            .size(EMPTY_TITLE)
            .font(typography::SEMIBOLD)
            .align_x(Alignment::Center)
            .width(Fill),
        text(tr!(
            "Plug in a camera or a capture card, such as OBS's virtual camera, and open this dialog again."
        ))
        .size(typography::size::LABEL)
        .style(catalog::text::muted)
        .align_x(Alignment::Center)
        .width(Fill),
    ]
    .spacing(EMPTY_GAP)
    .align_x(Alignment::Center)
    .width(Fill);
    container(parts).center(Fill).boxed()
}
