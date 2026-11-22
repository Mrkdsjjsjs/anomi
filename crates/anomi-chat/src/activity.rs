//! A message's activity card: a Discord request to stream, as Discord's
//! client shows it — the application's icon and name, who asked, and the
//! buttons to take it or the bar that says it expired.

use iced::widget::{column, container, image, row, text};
use iced::{Alignment, Background, Border, Fill, Length};

use anomi_theme::prelude::*;
use anomi_theme::{catalog, metrics, typography};
use anomi_vm::chat::activity::{Activity, Intent};
use anomi_widgets::button::{Kind, action_button};
use yapfast_lang::tr;

/// The card's widest (`max-width: 480px`), its inside padding, and the
/// room above it and between its parts.
const WIDTH: f32 = 480.0;
const PAD: f32 = 16.0;
const GAP: f32 = 8.0;
const ABOVE: f32 = 4.0;
/// The application icon's tile.
const TILE: f32 = 40.0;
const HEADING: f32 = 13.0;
const NAME: f32 = 15.0;
const DETAIL: f32 = typography::size::SMALL;
const BAR: f32 = 34.0;

pub fn view(activity: &Activity, key: u64, motion: Motion) -> impl View<Intent> + '_ {
    let icon: Element<'_, Intent> = match activity.icon.ready() {
        Some(handle) => image(handle.clone()).width(Fill).height(Fill).boxed(),
        None => iced::widget::space().boxed(),
    };
    let head = row![
        container(icon)
            .width(TILE)
            .height(TILE)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().field)),
                border: Border::default().rounded(metrics::RADIUS_CARD),
                ..container::Style::default()
            }),
        column![
            text(tr!("Stream request"))
                .size(HEADING)
                .font(typography::BOLD)
                .style(catalog::text::danger),
            text(activity.name.as_str())
                .size(NAME)
                .font(typography::BOLD)
                .wrapping(text::Wrapping::Word)
                .style(catalog::text::primary),
            text(tr!(
                "User {} wants you to stream",
                activity.requester.as_str()
            ))
            .size(DETAIL)
            .wrapping(text::Wrapping::Word)
            .style(catalog::text::muted),
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start);
    let foot: Element<'_, Intent> = if activity.taken {
        iced::widget::space().boxed()
    } else if activity.expired {
        container(
            container(
                text(tr!("The stream request expired."))
                    .size(DETAIL)
                    .font(typography::BOLD)
                    .style(catalog::text::danger),
            )
            .center_x(Fill)
            .center_y(Fill),
        )
        .height(BAR)
        .width(Fill)
        .style(|theme: &Theme| {
            let mut soft = theme.palette().danger;
            soft.a = 0.12;
            container::Style {
                background: Some(Background::Color(soft)),
                border: Border::default().rounded(metrics::RADIUS_ROW),
                ..container::Style::default()
            }
        })
        .boxed()
    } else {
        row![
            action_button(tr!("Accept"), Kind::Success, motion)
                .on_press(Intent::Accept(key))
                .boxed(),
            action_button(tr!("Decline"), Kind::Danger, motion)
                .on_press(Intent::Decline(key))
                .boxed(),
        ]
        .spacing(GAP)
        .boxed()
    };
    let card = container(column![head, foot].spacing(12).padding(PAD))
        .width(Fill)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.sidebar)),
                border: Border::default()
                    .rounded(metrics::RADIUS_CARD)
                    .width(1)
                    .color(palette.line),
                ..container::Style::default()
            }
        });
    container(container(card).width(Length::Fill.max(WIDTH)))
        .padding(iced::Padding::ZERO.top(ABOVE))
        .boxed()
}
