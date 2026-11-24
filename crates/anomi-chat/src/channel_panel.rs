//! A voice channel's chat, open beside its stage, in place of its member
//! list: Discord opens a voice channel's chat this way. A header (the
//! chat's icon and the channel's name, the cross), the channel's own
//! timeline (C01's, over the selected room) and its own composer, on the
//! chat's ground with a 1 px edge at its left. The rows and the composer
//! are the channel's own view-models (`vm.chat.timeline`, `vm.chat.composer`),
//! which the stage leaves to this panel.

use iced::widget::{column, container, row, rule, stack, svg, text};
use iced::{Alignment, Fill, Padding, alignment};
use yapfast_lang::tr;

use anomi_theme::prelude::*;
use anomi_theme::{catalog, typography};
use anomi_vm::Vm;
use anomi_vm::chat::{self, channel_panel::Intent};
use anomi_vm::icon::Icon;
use anomi_widgets::button::icon_button;
use anomi_widgets::field::Id;
use anomi_widgets::icon::icon;
use anomi_widgets::tip::Side;

use crate::{composer, timeline};

/// The cross's id, which tests press it by.
pub const CLOSE_ID: &str = "channel-panel-close";

/// `height: 48px`, `padding-left: 16px; padding-right: 10px; spacing: 8px`;
/// the icon is 18 px and muted, the name 15 px bold. The thread panel's
/// header (`ui/thread.slint:244`-`:259`).
const HEIGHT: f32 = 48.0;
const LEFT: f32 = 16.0;
const RIGHT: f32 = 10.0;
const GAP: f32 = 8.0;
const GLYPH: f32 = 18.0;
const TITLE: f32 = 15.0;
const BUTTON: f32 = 28.0;

/// The panel, from the conversation's view-models.
pub fn view(vm: &Vm, motion: Motion) -> impl View<chat::Intent> + '_ {
    let name = vm
        .chat
        .timeline
        .room
        .as_ref()
        .map_or("", |room| room.name.as_str());
    let close = tr!("Close the channel's chat");
    // The header's line is its own last pixel row, as in Slint, so the
    // messages start 48 px down.
    let head = stack![
        container(
            row![
                icon(Icon::MessageSquareText, GLYPH).style(|theme: &Theme, _| svg::Style {
                    color: Some(theme.palette().muted),
                }),
                text(name)
                    .size(TITLE)
                    .font(typography::BOLD)
                    .wrapping(text::Wrapping::None)
                    .ellipsis(text::Ellipsis::End)
                    .width(Fill)
                    .style(catalog::text::primary),
                icon_button(Icon::X, close, motion)
                    .size(BUTTON)
                    .glyph(GLYPH)
                    .face()
                    .id(Id::new(CLOSE_ID))
                    .on_press(chat::Intent::ChannelPanel(Intent::Close))
                    .tipped(close, Side::Below),
            ]
            .spacing(GAP)
            .align_y(Alignment::Center),
        )
        .padding(Padding {
            top: 0.0,
            right: RIGHT,
            bottom: 0.0,
            left: LEFT,
        })
        .height(HEIGHT)
        .width(Fill)
        .align_y(alignment::Vertical::Center),
        container(rule::horizontal(1).style(catalog::rule::line)).align_bottom(Fill),
    ];
    let body = column![
        head,
        container(timeline::view(&vm.chat.timeline, motion))
            .width(Fill)
            .height(Fill),
        composer::view(&vm.chat.composer, motion),
    ]
    .width(Fill)
    .height(Fill);
    container(row![
        rule::vertical(1).style(|theme: &Theme| rule::Style {
            color: theme.palette().edge,
            ..catalog::rule::line(theme)
        }),
        body
    ])
    .width(Fill)
    .height(Fill)
    .style(catalog::container::chat)
}
