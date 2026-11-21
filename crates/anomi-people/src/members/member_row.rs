//! One member in the list: the avatar with its presence and decoration, the
//! name with its marks, the activity under it, the nameplate behind it, 42 px
//! tall. The Slint side: the row of `MemberPanel` (`ui/people.slint:265`).
//!
//! A row draws twice, as iced's `hover` lays one over the other: at rest
//! (a selected member's avatar and decoration playing, as Discord's do),
//! and as it looks under the pointer (its wash, its nameplate, avatar and
//! name playing). A click opens the member's card beside the row, a right
//! click their menu at the pointer.

use iced::widget::{container, hover, mouse_area, row};
use iced::{Alignment, Fill, Padding, Rectangle};

use anomi_theme::metrics;
use anomi_theme::prelude::*;
use anomi_vm::menu::Spot;
use anomi_vm::people::members::{Intent, Member};
use anomi_widgets::menu::opener;
use anomi_widgets::nameplate::{Place, nameplate};

mod face;
mod lines;

use super::{OFFLINE, SIDE, WASH_HOVER, WASH_SELECTED, wash};

/// How far above the row the card's top lies (`root.profile-y - 16px`,
/// `ui/yapfast.slint:5035`): the card sits beside the panel, not beside the
/// row. The panel's 1 px edge lies over the rows (`ui/people.slint:125`).
const ABOVE: f32 = 16.0;

/// The row's height, corners and inner spacing (`height: 42px`,
/// `border-radius: 8px`, `spacing: 12px`).
const HEIGHT: f32 = metrics::DM_ROW_HEIGHT;
const RADIUS: f32 = metrics::RADIUS_CARD;
const SPACING: f32 = metrics::SPACE_M;
/// Space before the avatar, and after the text: 8, or 12 over a nameplate
/// (`padding-left: 8px`, `padding-right: ... 12px : 8px`).
const LEFT: f32 = metrics::SPACE_S;
const RIGHT: f32 = metrics::SPACE_S;
const RIGHT_PLATED: f32 = metrics::SPACE_M;

pub fn view<'a>(
    member: &'a Member,
    selected: bool,
    gems: bool,
    motion: Motion,
) -> Element<'a, Intent> {
    // At rest the lines and the nameplate hold still, a selected row's
    // plate too; under the pointer everything plays.
    let still = Motion::default();
    let resting = resting_face(selected, motion);
    let rest = body(member, selected, false, gems, resting, still);
    let over = body(member, selected, true, gems, motion, motion);
    // The app swaps in the moving avatar and decoration while the pointer
    // is on the row or it's selected, the moving nameplate while pointed,
    // and the still ones as it leaves.
    let pointed = mouse_area(hover(rest, over))
        .on_enter(Intent::Pointed(Some(member.id.clone())))
        .on_exit(Intent::Pointed(None));
    opener(pointed)
        .on_press(move |bounds| Intent::Open(member.id.clone(), Spot::beside(card_anchor(bounds))))
        .on_right_press(move |at| Intent::Menu(member.id.clone(), Spot::pointer(at)))
        .boxed()
}

/// The row moved to the panel's left edge and up: what the card opens beside.
fn card_anchor(row: Rectangle) -> Rectangle {
    Rectangle {
        x: row.x - SIDE,
        y: row.y - ABOVE,
        ..row
    }
}

/// How the face of the row at rest plays: while the member is selected
/// (their card open), still otherwise.
fn resting_face(selected: bool, motion: Motion) -> Motion {
    if selected { motion } else { Motion::default() }
}

/// The row in its look at rest or under the pointer: its face (avatar and
/// decoration) playing with `face_motion`, its lines and nameplate with
/// `motion`.
fn body<'a>(
    member: &'a Member,
    selected: bool,
    hovered: bool,
    gems: bool,
    face_motion: Motion,
    motion: Motion,
) -> Element<'a, Intent> {
    let plated = member.plate.picture.is_some();
    let content = row![
        face::view(member, face_motion),
        lines::column(member, gems, hovered, motion)
    ]
    .spacing(SPACING)
    .align_y(Alignment::Center);
    let content = container(content)
        .padding(Padding {
            top: 0.0,
            right: if plated { RIGHT_PLATED } else { RIGHT },
            bottom: 0.0,
            left: LEFT,
        })
        .width(Fill)
        .height(Fill)
        .align_y(Alignment::Center);
    // An offline member's plate is halved, with its picture and fade.
    let plate = nameplate(content, Place::Member, motion)
        .plate(&member.plate)
        .selected(selected)
        .radius(RADIUS)
        .opacity(if member.offline() { OFFLINE } else { 1.0 });
    // Chosen or pointed at, the wash lies on the list's ground; the row
    // under the pointer is opaque, so the one at rest doesn't show through.
    let lit = selected || hovered;
    container(plate)
        .width(Fill)
        .height(HEIGHT)
        .style(move |theme: &Theme| container::Style {
            background: lit.then(|| {
                let alpha = if selected { WASH_SELECTED } else { WASH_HOVER };
                iced::Background::Color(wash(theme.palette().chat, alpha))
            }),
            border: iced::Border::default().rounded(RADIUS),
            ..container::Style::default()
        })
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    use anomi_theme::Motion;

    #[test]
    fn a_row_at_rest_plays_its_face_only_while_selected() {
        let front = Motion {
            front: true,
            ..Motion::default()
        };
        assert!(resting_face(true, front).watched());
        assert!(!resting_face(false, front).watched());
    }
}
