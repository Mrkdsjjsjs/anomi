//! A large choice of the dialog, as Discord's "Share Entire Screen" and
//! "Share a Window" (`discord-golive-1-apps.png`): 262 by 165 on the sunken
//! ground under a thin edge, an icon over its words in the middle, lit
//! under the pointer and ringed in the accent when picked. The cameras of
//! "Devices" are the same.

use iced::widget::{button, column, container, svg, text};
use iced::{Alignment, Background, Border, Fill};

use anomi_theme::prelude::*;
use anomi_theme::{metrics, typography};
use anomi_vm::icon::Icon;
use anomi_vm::voice::share::Intent;
use anomi_widgets::icon::icon;

/// The card, and the room between two (`discord-golive-1-apps.png`).
pub const WIDTH: f32 = 262.0;
pub const HEIGHT: f32 = 165.0;
pub const GAP: f32 = 16.0;
/// The icon, and the room between it and the words.
const ICON: f32 = 32.0;
const ICON_GAP: f32 = 12.0;
/// The words' room each side, so "Share Entire Screen" wraps as Discord's.
const SIDE: f32 = 40.0;
/// The edge, and the picked card's ring.
const EDGE: f32 = 1.0;
const RING: f32 = 2.0;

/// A card saying `words` under `shape`, sending `intent`; ringed while
/// `picked`.
pub fn tile<'a>(shape: Icon, words: &'a str, intent: Intent, picked: bool) -> Element<'a, Intent> {
    let content = column![
        icon(shape, ICON).style(|theme: &Theme, _| svg::Style {
            color: Some(theme.palette().secondary),
        }),
        text(words)
            .size(typography::size::SMALL)
            .align_x(Alignment::Center)
            .width(Fill),
    ]
    .spacing(ICON_GAP)
    .align_x(Alignment::Center);
    let content = container(content).padding([0.0, SIDE]).center(Fill);
    button(content)
        .padding(0)
        .width(WIDTH)
        .height(HEIGHT)
        .on_press(intent)
        .style(move |theme: &Theme, status| {
            let palette = theme.palette();
            let lit = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if lit {
                    palette.hover
                } else {
                    palette.sunken
                })),
                text_color: if lit || picked {
                    palette.text
                } else {
                    palette.secondary
                },
                border: Border::default()
                    .rounded(metrics::RADIUS_CARD)
                    .width(if picked { RING } else { EDGE })
                    .color(if picked { palette.accent } else { palette.line }),
                ..button::Style::default()
            }
        })
        .boxed()
}
