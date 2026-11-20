//! "Applications": a screen or a window to share. Where the system's dialog
//! picks (Wayland's portal) it's Discord's tab (`discord-golive-1-apps.png`):
//! "Share your screen, a window or an application" over the two cards, the
//! whole screen and a window, each asking that dialog for its kind alone.
//! Elsewhere (X11, Windows) nothing else would pick, so the tab lists them
//! under the same words: the screens as cards with their pictures, two to
//! a row, then the windows, scrolling as one where they don't fit.

use iced::widget::{Column, column, container, row, space, text};
use iced::{Alignment, Fill, Length};
use yapfast_lang::tr;

use anomi_theme::prelude::*;
use anomi_theme::{catalog, typography};
use anomi_vm::icon::Icon;
use anomi_vm::voice::share::{Intent, Offer, Share};
use anomi_widgets::list::scroll_view;
use anomi_widgets::menu::item;

use super::card;
use super::tile::{self, tile};

/// Between the words over the choices and the choices.
pub(super) const UNDER_HEADING: f32 = 40.0;
/// The list's width where Anomi lists, and the room between its parts.
const LIST: f32 = 560.0;
const CARD_GAP: f32 = 4.0;
const SECTION: f32 = 16.0;

pub fn view(share: &Share, motion: Motion) -> Element<'_, Intent> {
    if share.dialog {
        let cards = row![
            tile(
                Icon::Monitor,
                tr!("Share Entire Screen"),
                Intent::Ask(Offer::Screen),
                false,
            ),
            tile(
                Icon::AppWindow,
                tr!("Share a Window"),
                Intent::Ask(Offer::Window),
                false,
            ),
        ]
        .spacing(tile::GAP);
        let parts = column![
            heading(tr!("Share your screen, a window or an application")),
            cards
        ]
        .spacing(UNDER_HEADING)
        .align_x(Alignment::Center);
        return container(parts).center(Fill).boxed();
    }
    let mut list: Column<Element<'_, Intent>> = Column::new().spacing(CARD_GAP).width(LIST);
    if !share.screens.is_empty() {
        list = list.push(screens(share));
    }
    if !share.windows.is_empty() {
        if !share.screens.is_empty() {
            list = list.push(space().height(SECTION).boxed());
        }
        for row in &share.rows.windows {
            list = list.push(item(row, motion).boxed());
        }
    }
    if share.screens.is_empty() && share.windows.is_empty() {
        list = list.push(
            text(tr!("There is no screen or window to share."))
                .size(typography::size::SMALL)
                .style(catalog::text::muted)
                .align_x(Alignment::Center)
                .width(Fill)
                .boxed(),
        );
    }
    column![
        heading(tr!("Share your screen, a window or an application")),
        container(scroll_view(list, motion).height(Fill)).center_x(Fill),
    ]
    .spacing(SECTION)
    .align_x(Alignment::Center)
    .width(Fill)
    .height(Fill)
    .boxed()
}

/// The words over the choices: 20 px, semibold, in the middle.
pub(super) fn heading<'a>(words: &'a str) -> Element<'a, Intent> {
    text(words)
        .size(typography::size::DIALOG_TITLE)
        .font(typography::SEMIBOLD)
        .align_x(Alignment::Center)
        .width(Fill)
        .boxed()
}

/// The screens, two to a row; one alone sits in the middle.
fn screens(share: &Share) -> Element<'_, Intent> {
    let width = (LIST - CARD_GAP) / 2.0;
    let make = |index: usize| {
        card::card(
            &share.screens[index],
            index,
            share.source == Some(index),
            width,
        )
        .boxed()
    };
    let mut all = column![].spacing(CARD_GAP).width(Fill);
    let count = share.screens.len();
    for first in (0..count).step_by(2) {
        let line = if count == 1 {
            row![
                space().width(Length::Fill),
                make(first),
                space().width(Length::Fill)
            ]
        } else {
            let mut line = row![make(first)].spacing(CARD_GAP);
            if first + 1 < count {
                line = line.push(make(first + 1));
            }
            line
        };
        all = all.push(line);
    }
    all.boxed()
}
