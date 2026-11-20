//! The card beside someone's row under a voice channel, as Discord's client
//! shows it once the pointer rests on the row (its voice user popout): a
//! header, "Streaming now" with the LIVE tag or "Playing now"; the stream's
//! latest still, or word that it started until one came; the game's icon,
//! name and how long they've played, ticking once a second only while the
//! card shows and the window is live (`anomi_widgets::clock`,
//! `Motion::live`); Watch Stream, or Watching Stream disabled while this
//! account watches it; and the store the game is sold at. Discord's "Ask
//! to Stream" isn't offered: its request isn't documented
//! (`anomi_vm::sidebar::channels::hover`).
//!
//! The row's [`Pointed`](super::pointed) asks for it and places it, hanging
//! from the row's top, 8 px right of it.

use iced::widget::{Column, column, container, row, text};
use iced::{Alignment, Background, Border, Color, Fill, Shadow, Vector};

use anomi_theme::prelude::*;
use anomi_theme::{catalog, metrics, typography};
use anomi_vm::kind::Expected;
use anomi_vm::sidebar::channels::Intent;
use anomi_vm::sidebar::channels::hover::{Game, HoverCard, Store, Stream};
use anomi_widgets::button::{Kind, action_button};
use anomi_widgets::clock::clock;
use anomi_widgets::icon::{Icon, icon};
use anomi_widgets::picture::{Fit, picture};
use yapfast_lang::tr;

use super::live::live;

/// The card: 280 px wide, 16 px in, with 8 px corners, a hairline and a
/// soft shadow (Discord's popout, as the reference shows it).
const WIDTH: f32 = 280.0;
const PAD: f32 = 16.0;
const SHADOW: Shadow = Shadow {
    color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
    offset: Vector::new(0.0, 4.0),
    blur_radius: 16.0,
};
/// Between the header, the stream, the game and the buttons, and between
/// the buttons.
const SPACING: f32 = 12.0;
const BETWEEN_BUTTONS: f32 = 8.0;
/// The stream's still, 16 by 9 across the card's inside.
const PREVIEW_HEIGHT: f32 = (WIDTH - 2.0 * PAD) * 9.0 / 16.0;
/// The game's icon, 32 px with 4 px corners, 12 px before its name.
const ICON: f32 = 32.0;
const ICON_RADIUS: f32 = 4.0;
const ICON_GAP: f32 = 12.0;
/// The gamepad before the time played, and the space after it.
const PAD_GLYPH: f32 = 14.0;
const PAD_GAP: f32 = 4.0;
/// The buttons: 32 px tall, the card's corners, 16 px glyphs.
const BUTTON: f32 = 32.0;
const BUTTON_GLYPH: f32 = 16.0;

/// The card of `card`.
pub fn view(card: &HoverCard, motion: Motion) -> Element<'_, Intent> {
    let mut parts: Column<Element<'_, Intent>> = Column::new().spacing(SPACING).width(Fill);
    parts = parts.push(header(card.stream.is_some()));
    if let Some(stream) = &card.stream {
        parts = parts.push(preview(stream, motion));
    }
    if let Some(game) = &card.game {
        parts = parts.push(playing(game, motion));
    }
    let store = card.game.as_ref().and_then(|game| game.store.as_ref());
    if card.stream.is_some() || store.is_some() {
        let mut buttons: Column<Element<'_, Intent>> =
            Column::new().spacing(BETWEEN_BUTTONS).width(Fill);
        if let Some(stream) = &card.stream {
            buttons = buttons.push(watch(card, stream, motion));
        }
        if let Some(store) = store {
            buttons = buttons.push(store_button(store, motion));
        }
        parts = parts.push(buttons.boxed());
    }
    container(parts)
        .padding(PAD)
        .width(WIDTH)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.tooltip)),
                border: Border::default()
                    .rounded(metrics::RADIUS_CARD)
                    .width(1)
                    .color(palette.line),
                shadow: SHADOW,
                ..container::Style::default()
            }
        })
        .boxed()
}

/// "Streaming now" with the LIVE tag at the right, or "Playing now".
fn header<'a>(streaming: bool) -> Element<'a, Intent> {
    let words = if streaming {
        tr!("Streaming now")
    } else {
        tr!("VoiceCard" => "Playing now")
    };
    let words = text(words)
        .size(typography::size::CAPTION)
        .font(typography::SEMIBOLD)
        .style(catalog::text::secondary)
        .wrapping(text::Wrapping::None)
        .width(Fill);
    let mut line = row![words].align_y(Alignment::Center);
    if streaming {
        line = line.push(live(None::<fn() -> Intent>));
    }
    line.boxed()
}

/// The stream's latest still, covering its room, or until one came, word
/// that it started on a ground a step lighter than the card's, as
/// Discord's.
fn preview(stream: &Stream, motion: Motion) -> Element<'_, Intent> {
    let ground = |theme: &Theme| container::Style {
        background: Some(Background::Color(theme.palette().button_default)),
        border: Border::default().rounded(metrics::RADIUS_CARD),
        ..container::Style::default()
    };
    let inside = match &stream.preview {
        Some(still) => picture(still.clone())
            .width(Fill)
            .height(PREVIEW_HEIGHT)
            .radius(metrics::RADIUS_CARD)
            .fit(Fit::Cover)
            .motion(motion)
            .boxed(),
        None => text(tr!("The stream has started. Don't miss it!"))
            .size(typography::size::SMALL)
            .style(catalog::text::secondary)
            .align_x(Alignment::Center)
            .boxed(),
    };
    container(inside)
        .center_x(Fill)
        .center_y(PREVIEW_HEIGHT)
        .style(ground)
        .boxed()
}

/// The game's icon, its name, and how long they've played in green after a
/// gamepad, ticking as it draws.
fn playing(game: &Game, motion: Motion) -> Element<'_, Intent> {
    let face: Element<'_, Intent> = match &game.icon {
        Expected::Ready(handle) => picture(handle.clone())
            .side(ICON)
            .radius(ICON_RADIUS)
            .fit(Fit::Cover)
            .motion(motion)
            .boxed(),
        _ => container(icon(Icon::Gamepad, ICON / 2.0).style(muted))
            .center(ICON)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().raised)),
                border: Border::default().rounded(ICON_RADIUS),
                ..container::Style::default()
            })
            .boxed(),
    };
    let name = text(game.name.as_str())
        .size(typography::size::BODY)
        .font(typography::SEMIBOLD)
        .style(catalog::text::primary)
        .wrapping(text::Wrapping::None)
        .ellipsis(text::Ellipsis::End)
        .width(Fill);
    let mut words = column![name].width(Fill);
    if let Some(elapsed) = game.elapsed {
        let time = clock(elapsed, anomi_vm::voice::dock::elapsed, motion.live())
            .size(typography::size::LABEL)
            .style(live_ink);
        let pad =
            icon(Icon::Gamepad, PAD_GLYPH).style(|theme: &Theme, _| iced::widget::svg::Style {
                color: Some(theme.palette().live),
            });
        words = words.push(
            row![pad, time]
                .spacing(PAD_GAP)
                .align_y(Alignment::Center)
                .boxed(),
        );
    }
    row![face, words]
        .spacing(ICON_GAP)
        .align_y(Alignment::Center)
        .boxed()
}

/// Watch Stream, green; Watching Stream, disabled, while this account
/// watches it.
fn watch<'a>(card: &'a HoverCard, stream: &'a Stream, motion: Motion) -> Element<'a, Intent> {
    let (words, kind) = if stream.watching {
        (tr!("Watching Stream"), Kind::Secondary)
    } else {
        (tr!("Watch Stream"), Kind::Success)
    };
    let mut button = action_button(words, kind, motion)
        .leading(Icon::ScreenShare)
        .glyph(BUTTON_GLYPH)
        .height(BUTTON)
        .radius(metrics::RADIUS_CARD)
        .width(Fill)
        .enabled(!stream.watching);
    if !stream.watching {
        button = button.on_press(Intent::Watch {
            channel: card.channel.clone(),
            person: stream.person.clone(),
        });
    }
    button.boxed()
}

/// The store the game is sold at, opening its page.
fn store_button(store: &Store, motion: Motion) -> Element<'_, Intent> {
    action_button(store.label.as_str(), Kind::Secondary, motion)
        .leading(store.glyph)
        .glyph(BUTTON_GLYPH)
        .height(BUTTON)
        .radius(metrics::RADIUS_CARD)
        .width(Fill)
        .on_press(Intent::OpenStore(store.url.clone()))
        .boxed()
}

/// A mark in the muted colour.
fn muted(theme: &Theme, _status: iced::widget::svg::Status) -> iced::widget::svg::Style {
    iced::widget::svg::Style {
        color: Some(theme.palette().muted),
    }
}

/// The time played, in the live colour.
fn live_ink(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().live),
        ..text::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_still_is_sixteen_by_nine_across_the_cards_inside() {
        assert_eq!(PREVIEW_HEIGHT, 248.0 * 9.0 / 16.0);
    }
}
