//! A tile's ground and what fills it: someone's picture on the raised
//! ground, or on black their camera, a watched stream (`StreamPicture`,
//! `ui/voice.slint:840`), a stream offered on its preview (`StreamPreview`,
//! `:849`; `:1034`), this account's screen (`:999`), or the screen icon
//! standing for a stream shown large above.
//!
//! Frames are read as they're drawn, so a new one builds no view, and are
//! drawn square and whole, never copied; `ring` rounds the corners over
//! them.
//!
//! What the middle says is as large as the tile's room lets it be
//! (`super::room`): the icon or the ring with its words, alone, smaller, or
//! nothing, clear of the badge and the viewers above and the chip below.

use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{Space, column, container, responsive, stack, text};
use iced::{Alignment, Background, Border, Color, Fill, Padding, Size};

use anomi_theme::prelude::*;
use anomi_theme::typography;
use anomi_vm::kind::{Avatar, Live};
use anomi_vm::voice::tiles::{Fill as Filling, Intent, Outline, Tile};
use anomi_widgets::avatar::portrait;
use anomi_widgets::icon::{Icon, icon};
use anomi_widgets::picture::{Fit, live, picture};

use super::room::{Middle, Room};
use super::{GREY, waiting};

/// Someone's picture: 45% of the tile's height, at most 80 px, 8 px above
/// its middle (`:1085`, `:1086`), at half strength until they connect
/// (`:1087`).
const AVATAR_SHARE: f32 = 0.45;
const AVATAR_MAX: f32 = 80.0;
const AVATAR_RAISE: f32 = 8.0;
const UNCONNECTED: f32 = 0.5;
/// What dims a preview so what's over it reads (`:852`).
const DIM: Color = anomi_theme::rgba(0x0000_0059);
/// The screen over a stream offered, 30 px, and over this account's screen
/// before its first picture, 34; each 10 px above what follows (`:1001`,
/// `:1003`, `:1035`, `:1036`).
pub(super) const OFFER_ICON: f32 = 30.0;
pub(super) const SHARE_ICON: f32 = 34.0;
pub(super) const GAP: f32 = 10.0;
/// The screen alone, as a large stream's stand-in has it, and in a tile
/// too small for its words.
pub(super) const SMALL_ICON: f32 = 20.0;
/// This account's screen's status, 14 px (`:1004`).
pub(super) const STATUS: f32 = typography::size::SMALL;

pub fn view<'a>(tile: &'a Tile, room: Room, radius: f32, motion: Motion) -> Element<'a, Intent> {
    let middle = room.middle;
    let content: Element<'a, Intent> = match &tile.fill {
        Filling::Avatar { avatar, connected } => face(avatar, *connected, speaking(tile, motion)),
        Filling::Camera(frame)
        | Filling::Watched {
            frame: Some(frame), ..
        }
        | Filling::OwnScreen {
            frame: Some(frame), ..
        } => whole(frame),
        Filling::Watched {
            frame: None,
            status,
            stuck,
        } => match middle {
            Middle::Full(_) => waiting::line(status, motion, *stuck).boxed(),
            Middle::Glyph(side) => waiting::ring(side, room.raised(), motion, *stuck).boxed(),
            Middle::Empty | Middle::InChip => Space::new().boxed(),
        },
        Filling::Offer { preview } => offer(preview.as_ref(), room),
        Filling::OwnScreen {
            frame: None,
            status,
        } => screen_starting(status, room),
        Filling::Large => glyph(Color::WHITE, room),
    };
    let black = tile.fill.on_black();
    container(content)
        .width(Fill)
        .height(Fill)
        .style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(if black {
                Color::BLACK
            } else {
                theme.palette().raised
            })),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .boxed()
}

/// How someone's picture plays on their tile: while they speak, as
/// Discord's does, and on past the 15 s without input, as a call is
/// watched without touching the window; still while they don't. Hidden or
/// with animations off, it holds all the same.
fn speaking(tile: &Tile, motion: Motion) -> Motion {
    if tile.outline == Outline::Speaking {
        motion.awake()
    } else {
        Motion::default()
    }
}

/// Their picture in the middle, sized by the tile, playing with `motion`;
/// a not yet connected one's under a veil of the ground at half strength,
/// which is what half its opacity looks like on that ground.
fn face(avatar: &Avatar, connected: bool, motion: Motion) -> Element<'_, Intent> {
    responsive(move |size: Size| {
        let side = (size.height * AVATAR_SHARE).min(AVATAR_MAX).floor();
        let face = portrait(avatar, side).motion(motion).view();
        let face: Element<'_, Intent> = if connected {
            face.boxed()
        } else {
            stack![face, veil(side)].boxed()
        };
        container(face)
            .center(Fill)
            .padding(Padding::ZERO.bottom(2.0 * AVATAR_RAISE))
    })
    .boxed()
}

fn veil<'a>(side: f32) -> impl View<Intent> + 'a {
    container(Space::new())
        .width(side)
        .height(side)
        .style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.palette().raised.scale_alpha(1.0 - UNCONNECTED),
            )),
            border: Border::default().rounded(side / 2.0),
            ..container::Style::default()
        })
}

/// The newest frame, whole and as large as fits.
fn whole(frames: &Live) -> Element<'_, Intent> {
    live(frames)
        .width(Fill)
        .height(Fill)
        .fit(Fit::Contain)
        .boxed()
}

/// What dims a stream's preview filling the tile (`StreamPreview`).
fn dim<'a>() -> Element<'a, Intent> {
    container(Space::new())
        .width(Fill)
        .height(Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(DIM)),
            ..container::Style::default()
        })
        .boxed()
}

/// A stream not watched, in a tile of its own: the screen over the pill
/// that offers it, white on its preview, dimmed, grey on black. Among the
/// small tiles beneath a large one, too low for both between its badge and
/// its chip, the screen alone, smaller, or nothing: a click still watches
/// it.
///
/// With or without its preview it's one stack, the picture first: a tile's
/// tree is kept as its fill changes and a stack places nothing, so a part
/// another fill had placed there (this one's centred screen, before its
/// preview came) would keep its place under the preview.
fn offer(preview: Option<&iced::widget::image::Handle>, room: Room) -> Element<'_, Intent> {
    let ink = if preview.is_some() {
        Color::WHITE
    } else {
        GREY
    };
    let middle = match room.middle {
        Middle::Full(_) => {
            let ink = anomi_theme::catalog::svg::colored(ink);
            let both = column![
                icon(Icon::ScreenShare, OFFER_ICON).style(ink),
                super::marks::watch_pill(),
            ]
            .spacing(GAP)
            .align_x(Alignment::Center);
            container(both).center(Fill).boxed()
        }
        _ => glyph(ink, room),
    };
    let (ground, veil) = match preview {
        Some(preview) => {
            let ground = picture(preview.clone())
                .width(Fill)
                .height(Fill)
                .fit(Fit::Cover);
            (ground.boxed(), dim())
        }
        None => (Space::new().boxed(), Space::new().boxed()),
    };
    stack![ground, veil, middle]
        .width(Fill)
        .height(Fill)
        .boxed()
}

/// The screen alone in `ink`, as large as the `room`'s middle says, in the
/// middle or above it; a stream shown large above, in its place beneath,
/// has it white on black, as Discord's strip shows it. Nothing where
/// there's no room.
fn glyph<'a>(ink: Color, room: Room) -> Element<'a, Intent> {
    let side = match room.middle {
        Middle::Glyph(side) => side,
        Middle::Full(_) => SMALL_ICON,
        Middle::Empty | Middle::InChip => return Space::new().boxed(),
    };
    let ink = anomi_theme::catalog::svg::colored(ink);
    container(icon(Icon::ScreenShare, side).style(ink))
        .center(Fill)
        .padding(room.raised())
        .boxed()
}

/// This account's screen before its first picture: the screen over how
/// it's going, on one line cut short; the screen alone in a tile too small
/// for both.
fn screen_starting(status: &str, room: Room) -> Element<'_, Intent> {
    if !matches!(room.middle, Middle::Full(_)) {
        return glyph(GREY, room);
    }
    let status = text(status)
        .size(STATUS)
        .color(GREY)
        .width(Fill)
        .wrapping(Wrapping::None)
        .ellipsis(Ellipsis::End)
        .align_x(Alignment::Center);
    let icon = icon(Icon::ScreenShare, SHARE_ICON).style(anomi_theme::catalog::svg::colored(GREY));
    container(
        column![icon, status]
            .spacing(GAP)
            .align_x(Alignment::Center),
    )
    .center_y(Fill)
    .width(Fill)
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    use anomi_theme::Motion;

    #[test]
    fn a_face_plays_while_they_speak_idle_or_not() {
        let idle = Motion {
            front: true,
            idle: true,
            ..Motion::default()
        };
        let talking = Tile {
            outline: Outline::Speaking,
            ..Tile::default()
        };
        assert!(speaking(&talking, idle).watched());
        let quiet = Tile {
            outline: Outline::Line,
            ..Tile::default()
        };
        assert!(!speaking(&quiet, idle).watched());
        // Hidden or with animations off, nothing moves.
        let hidden = Motion {
            unseen: true,
            ..idle
        };
        assert!(!speaking(&talking, hidden).watched());
        let never = Motion {
            never: true,
            ..idle
        };
        assert!(!speaking(&talking, never).watched());
    }
}
