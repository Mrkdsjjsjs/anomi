//! The picker of Discord's games on the Game activity page, open in place
//! of Add game as Discord's "Add it!" opens in place: a search over
//! Discord's list with the best matches, then, once a game is chosen, the
//! programs running to tie it to, or none to show it until it's turned off.
//! While the list isn't there, it says so: being read, being downloaded, or
//! failed with Retry.

use iced::widget::{column, container, row, space, svg, text};
use iced::{Alignment, Border, Length};

use anomi_theme::prelude::*;
use anomi_theme::{catalog, metrics, typography};
use anomi_vm::icon::Icon;
use anomi_vm::settings::games::{Intent, List, Match, Picker, Program};
use anomi_widgets::button::{Kind, action_button};
use anomi_widgets::field::field;
use anomi_widgets::icon::icon;
use yapfast_lang::tr;

use super::{GROUP, icon_of};
use crate::kit::pressable_on;

/// The search field's id, which the app gives the keys as the picker opens.
pub const SEARCH: &str = "games-search";

/// Inside the picker's ground, and its lift over the page.
const PAD: f32 = 12.0;
const GROUND: f32 = 0.04;
/// A row's icon, the gap after it, its padding, the lines' gap, and the
/// lift under the pointer.
const ROW_ICON: f32 = 32.0;
const BESIDE: f32 = 12.0;
const ROW_PAD: f32 = 6.0;
const LINES: f32 = 2.0;
const HOVER: f32 = 0.06;
/// A program's glyph, in the box an icon would take.
const GLYPH: f32 = 20.0;

pub fn view(picker: &Picker, motion: Motion) -> Element<'_, Intent> {
    let body = match &picker.chosen {
        None => games(picker, motion),
        Some(chosen) => programs(picker, chosen, motion),
    };
    container(body)
        .padding(PAD)
        .width(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.palette().lift(GROUND).into()),
            border: Border::default().rounded(metrics::RADIUS_CARD),
            ..container::Style::default()
        })
        .boxed()
}

/// The search, then what the list says to it.
fn games(picker: &Picker, motion: Motion) -> Element<'_, Intent> {
    let search = field(tr!("Search Discord's games"), picker.query.as_str())
        .id(SEARCH)
        .trailing(Icon::Search, None)
        .motion(motion)
        .on_input(Intent::Search);
    let top = row![
        container(search.view()).width(Length::Fill),
        action_button(tr!("Cancel"), Kind::Ghost, motion).on_press(Intent::ClosePicker),
    ]
    .spacing(GROUP)
    .align_y(Alignment::Center);
    let mut parts = column![top].spacing(GROUP).width(Length::Fill);
    match picker.list {
        List::Reading => parts = parts.push(note(tr!("Opening Discord's list of games..."))),
        List::Downloading => {
            parts = parts.push(note(tr!(
                "Discord's list of games isn't downloaded yet. Downloading it..."
            )));
        }
        List::Failed => {
            parts = parts.push(
                row![
                    container(note(tr!("Couldn't download Discord's list of games.")))
                        .width(Length::Fill),
                    action_button(tr!("Retry"), Kind::Secondary, motion)
                        .small()
                        .on_press(Intent::RetryList),
                ]
                .spacing(GROUP)
                .align_y(Alignment::Center)
                .boxed(),
            );
        }
        List::Ready if picker.matches.is_empty() => {
            parts = parts.push(note(tr!("No game in Discord's list is called that.")));
        }
        List::Ready => {
            for (index, found) in picker.matches.iter().enumerate() {
                parts = parts.push(game(index, found, motion));
            }
            if picker.more {
                parts = parts.push(note(tr!(
                    "These are the best matches. Type more of the name to find others."
                )));
            }
        }
    }
    parts.boxed()
}

/// A game found: its icon, its name, and the other name it was found by.
fn game(index: usize, found: &Match, motion: Motion) -> Element<'_, Intent> {
    let mut words = column![
        text(found.name.as_str())
            .size(typography::size::BODY)
            .font(typography::SEMIBOLD)
            .style(catalog::text::inherit)
    ]
    .spacing(LINES)
    .width(Length::Fill);
    if !found.alias.is_empty() {
        words = words.push(
            text(tr!("Also known as {}", found.alias))
                .size(typography::size::CAPTION)
                .style(catalog::text::muted)
                .boxed(),
        );
    }
    let line = row![icon_of(&found.icon, ROW_ICON), words]
        .spacing(BESIDE)
        .align_y(Alignment::Center)
        .padding(ROW_PAD);
    lit(line.boxed(), &found.name, Intent::PickGame(index), motion)
}

/// The game chosen, which programs are running, and the ways on.
fn programs<'a>(picker: &'a Picker, chosen: &'a Match, motion: Motion) -> Element<'a, Intent> {
    let head = row![
        icon_of(&chosen.icon, ROW_ICON),
        column![
            text(chosen.name.as_str())
                .size(typography::size::TITLE)
                .font(typography::SEMIBOLD)
                .style(catalog::text::primary),
            typography::label(tr!(
                "Which program is this game? While it runs, your status shows the game."
            ))
            .width(Length::Fill),
        ]
        .spacing(LINES)
        .width(Length::Fill),
    ]
    .spacing(BESIDE)
    .align_y(Alignment::Center);
    let mut parts = column![head].spacing(GROUP).width(Length::Fill);
    if picker.listing {
        parts = parts.push(note(tr!("Looking at the programs running...")));
    } else if picker.programs.is_empty() {
        parts = parts.push(note(tr!(
            "None of your programs is running. Start the game, then look again."
        )));
    } else {
        for (index, running) in picker.programs.iter().enumerate() {
            parts = parts.push(program(index, running, motion));
        }
    }
    let buttons = row![
        action_button(tr!("Look again"), Kind::Secondary, motion)
            .small()
            .leading(Icon::RefreshCw)
            .enabled(!picker.listing)
            .on_press(Intent::ListPrograms),
        action_button(tr!("Without a program"), Kind::Secondary, motion)
            .small()
            .on_press(Intent::WithoutProgram),
        space().width(Length::Fill),
        action_button(tr!("Back"), Kind::Ghost, motion)
            .small()
            .on_press(Intent::BackToGames),
        action_button(tr!("Cancel"), Kind::Ghost, motion)
            .small()
            .on_press(Intent::ClosePicker),
    ]
    .spacing(GROUP)
    .align_y(Alignment::Center);
    parts
        .push(buttons.boxed())
        .push(note(tr!(
            "Without a program, the game shows in your status until you turn it off."
        )))
        .boxed()
}

/// A program running: its file name over its path.
fn program(index: usize, running: &Program, motion: Motion) -> Element<'_, Intent> {
    let glyph = container(
        icon(Icon::Gamepad, GLYPH).style(|theme: &Theme, _| svg::Style {
            color: Some(theme.palette().muted),
        }),
    )
    .center(Length::Fixed(ROW_ICON));
    let words = column![
        text(running.name.as_str())
            .size(typography::size::BODY)
            .font(typography::SEMIBOLD)
            .style(catalog::text::inherit),
        text(running.path.as_str())
            .size(typography::size::CAPTION)
            .style(catalog::text::muted)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::Start)
            .width(Length::Fill),
    ]
    .spacing(LINES)
    .width(Length::Fill);
    let line = row![glyph, words]
        .spacing(BESIDE)
        .align_y(Alignment::Center)
        .padding(ROW_PAD);
    lit(
        line.boxed(),
        &running.path,
        Intent::PickProgram(index),
        motion,
    )
}

/// A row pressed as one, its ground lifting under the pointer.
fn lit<'a>(
    content: Element<'a, Intent>,
    label: &'a str,
    on_press: Intent,
    motion: Motion,
) -> Element<'a, Intent> {
    pressable_on(
        container(content).width(Length::Fill).boxed(),
        label,
        on_press,
        metrics::RADIUS_ROW,
        |theme: &Theme, _| theme.palette().text,
        |theme: &Theme, lit| theme.palette().lift(HOVER * lit),
        motion,
    )
}

/// What the picker says, in small grey words.
fn note<'a>(words: &'a str) -> Element<'a, Intent> {
    typography::label(words).width(Length::Fill).boxed()
}
