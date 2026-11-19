//! Optimization's "Interface rendering": Automatic, the processor and each
//! graphics adapter, a row each with a radio mark, its name and what it is
//! on this computer, the chosen one lifted, drawn as Safety Setup's choices
//! are (`server/safety/choices.rs`, whose measures these are). An adapter
//! that can't draw has its name and mark half faded, says why in words
//! left readable, and answers no click. Under the rows, a line saying what
//! draws the interface now.

use iced::widget::{column, container, row, text};
use iced::{Alignment, Border, Length};

use anomi_theme::prelude::*;
use anomi_theme::{metrics, typography};
use anomi_vm::settings::advanced::{Advanced, DrawOn, Intent};
use yapfast_lang::tr;

use super::{GROUP, name, note};
use crate::kit::pressable_on;

const PAD: f32 = 10.0;
const BESIDE: f32 = 12.0;
const WORDS_GAP: f32 = 2.0;
const GAP: f32 = 4.0;
const RING: f32 = 20.0;
const RING_WIDTH: f32 = 2.0;
const DOT: f32 = 10.0;
/// Where the chosen row lifts, and a usable one under the pointer, faded in.
const CHOSEN: f32 = 0.08;
const HOVER: f32 = 0.05;
const FADED: f32 = 0.5;
/// DM Sans's own line, as Slint lays its text.
const LINE: f32 = 1.3;

/// The rows under their name, and the line saying what draws now.
pub(super) fn view(page: &Advanced, motion: Motion) -> Element<'_, Intent> {
    let rows = page
        .renderers
        .iter()
        .enumerate()
        .map(|(at, choice)| one(choice, at == page.renderer, motion));
    let mut parts = vec![
        name(tr!("Interface rendering")),
        column(rows).spacing(GAP).width(Length::Fill).boxed(),
    ];
    if !page.renderer_line.is_empty() {
        parts.push(note(&page.renderer_line));
    }
    column(parts).spacing(GROUP).width(Length::Fill).boxed()
}

/// The radio mark: a ring, a dot inside when `on`.
fn mark<'a>(on: bool, dim: f32) -> Element<'a, Intent> {
    let dot: Element<'a, Intent> = if on {
        container(iced::widget::space())
            .width(DOT)
            .height(DOT)
            .style(move |theme: &Theme| container::Style {
                background: Some(theme.palette().accent.scale_alpha(dim).into()),
                border: Border::default().rounded(DOT / 2.0),
                ..container::Style::default()
            })
            .boxed()
    } else {
        iced::widget::space().boxed()
    };
    container(dot)
        .center(RING)
        .width(RING)
        .height(RING)
        .style(move |theme: &Theme| {
            let palette = theme.palette();
            let ring = if on {
                palette.accent
            } else {
                palette.secondary
            };
            container::Style {
                border: Border::default()
                    .rounded(RING / 2.0)
                    .width(RING_WIDTH)
                    .color(ring.scale_alpha(dim)),
                ..container::Style::default()
            }
        })
        .boxed()
}

/// A row: pressed, it sends its key; one that can't draw only shows.
fn one(choice: &DrawOn, on: bool, motion: Motion) -> Element<'_, Intent> {
    let dim = if choice.usable { 1.0 } else { FADED };
    let words = column![
        text(&choice.title)
            .size(typography::size::BODY)
            .line_height(LINE)
            .font(typography::SEMIBOLD)
            .style(move |theme: &Theme| text::Style {
                color: Some(theme.palette().text.scale_alpha(dim)),
                ..text::Style::default()
            }),
        text(&choice.description)
            .size(typography::size::LABEL)
            .line_height(LINE)
            .width(Length::Fill)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.palette().secondary),
                ..text::Style::default()
            }),
    ]
    .spacing(WORDS_GAP)
    .width(Length::Fill);
    let line = row![mark(on, dim), words]
        .spacing(BESIDE)
        .align_y(Alignment::Center)
        .padding(PAD);
    if choice.usable {
        let lift = move |theme: &Theme, lit: f32| {
            theme.palette().lift(if on { CHOSEN } else { HOVER * lit })
        };
        pressable_on(
            container(line).width(Length::Fill).boxed(),
            &choice.title,
            Intent::Renderer(choice.key.clone()),
            metrics::RADIUS_ROW,
            |theme: &Theme, _| theme.palette().text,
            lift,
            motion,
        )
    } else {
        let ground = move |theme: &Theme| container::Style {
            background: on.then(|| theme.palette().lift(CHOSEN).into()),
            border: Border::default().rounded(metrics::RADIUS_ROW),
            ..container::Style::default()
        };
        container(line).width(Length::Fill).style(ground).boxed()
    }
}
