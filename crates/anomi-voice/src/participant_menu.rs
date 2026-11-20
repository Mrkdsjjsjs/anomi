//! A participant's menu: `VolumeMenu` (`ui/voice.slint:1594`), the card over
//! the context menu's rows with the person's name and their volumes, the
//! voice's and, while they share their screen, the stream's. Task V11
//! (docs/iced/tasks/voice.md). In a Discord call, Anomi's own sliders follow
//! while each is on: how much of their noise is taken out, and how far
//! they're turned down while only this account's echo comes from them. A
//! watched stream's card, over the menu of its tile, starts as Discord's
//! menu of a stream does: under its name its own rows ("Stop Watching", a
//! line, the box "Mute"), drawn as a menu's are, then the stream's volume
//! alone, "Stream volume", then the same two sliders for its sound. The rows
//! under it are the window's menu (`vm.menu`); the shell draws both, this
//! is the card alone.

use iced::border::Radius;
use iced::widget::text::{Ellipsis, Wrapping};
use iced::widget::{Column, column, container, space, text};
use iced::{Background, Border, Color, Length, Padding, Shadow, Vector};

use anomi_theme::prelude::*;
use anomi_theme::{catalog, metrics, typography};
use anomi_vm::voice::participant_menu::{
    DEPTH, Intent, MAXIMUM, NAME_HEIGHT, PADDING, ParticipantMenu, ROWS_GAP, ROWS_PADDING, RULE,
    SPACING, STRENGTH,
};
use anomi_widgets::menu::item;
use anomi_widgets::slider::{self, Volume};
use yapfast_lang::tr;

/// The card's corners, as a context menu's (`border-radius: 8px`).
const RADIUS: f32 = metrics::RADIUS_CARD;
/// Its outline and shadow (`ui/voice.slint:1608-1610`: `border-width: 1px`,
/// `drop-shadow-blur: 24px; drop-shadow-color: #00000080;
/// drop-shadow-offset-y: 8px`).
const OUTLINE: f32 = 1.0;
const SHADOW: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.5);
const SHADOW_BLUR: f32 = 24.0;
const SHADOW_DROP: f32 = 8.0;

pub fn view(menu: &ParticipantMenu, motion: Motion) -> impl View<Intent> + '_ {
    let name = text(menu.name.as_str())
        .size(typography::size::BODY)
        .font(typography::BOLD)
        .width(Length::Fill)
        .height(NAME_HEIGHT)
        .wrapping(Wrapping::None)
        .ellipsis(Ellipsis::End)
        .style(catalog::text::primary);
    let head = container(name).padding(Padding::new(PADDING).bottom(0.0));
    // A stream's card: its volume alone.
    let mut sliders = column![volume(menu, menu.stream)].spacing(SPACING);
    if menu.live && !menu.stream {
        sliders = sliders.push(volume(menu, true));
    }
    if let Some(strength) = menu.noise_strength {
        sliders = sliders.push(noise_strength(menu, strength));
    }
    if let Some(depth) = menu.echo_depth {
        sliders = sliders.push(echo_depth(menu, depth));
    }
    // The card's own rows, if any, take the room above the first slider.
    let above = if menu.rows.is_empty() { SPACING } else { 0.0 };
    let sliders = container(sliders).padding(Padding::new(PADDING).top(above));
    let mut content: Column<Element<'_, Intent>> = Column::new().push(head.boxed());
    if !menu.rows.is_empty() {
        content = content.push(rows(menu, motion));
    }
    let joined = menu.joined;
    container(content.push(sliders.boxed()))
        .width(menu.width)
        .height(menu.height())
        .style(move |theme: &Theme| card(theme, joined))
}

/// A stream's own rows, as a menu's: each lights under the pointer and
/// sends its intent when clicked, a line before one that starts a group.
fn rows(menu: &ParticipantMenu, motion: Motion) -> Element<'_, Intent> {
    let mut rows = Column::new().spacing(ROWS_GAP);
    for (index, row) in menu.rows.iter().enumerate() {
        if index > 0 && row.separated {
            rows = rows.push(rule());
        }
        rows = rows.push(item(row, motion).boxed());
    }
    container(rows).padding(ROWS_PADDING).boxed()
}

/// A group's line, 1 px in the line colour in the middle of its room.
fn rule<'a>() -> Element<'a, Intent> {
    const LINE: f32 = 1.0;
    let line = container(space())
        .width(Length::Fill)
        .height(LINE)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().line)),
            ..container::Style::default()
        });
    let room = (RULE - LINE) / 2.0;
    container(line)
        .padding(Padding::ZERO.top(room).bottom(room))
        .boxed()
}

/// One slider: the person's voice, or their stream.
fn volume(menu: &ParticipantMenu, stream: bool) -> Element<'_, Intent> {
    let (label, value) = if stream {
        (tr!("Stream volume"), menu.stream_volume)
    } else {
        (tr!("User volume"), menu.voice_volume)
    };
    let say = move |person: &str, volume, last| Intent::Volume {
        person: person.to_owned(),
        stream,
        volume,
        last,
    };
    slider::volume(
        Volume::up_to(label, value, MAXIMUM),
        move |volume| say(&menu.person, volume, false),
        move |volume| say(&menu.person, volume, true),
    )
}

/// How much of their noise the suppressor chosen takes out, from none to
/// all it's let to.
fn noise_strength(menu: &ParticipantMenu, strength: i32) -> Element<'_, Intent> {
    let say = move |strength, last| Intent::NoiseStrength {
        person: menu.person.clone(),
        strength,
        last,
    };
    slider::volume(
        Volume::up_to(tr!("Noise suppression strength"), strength, STRENGTH).step(10),
        move |strength| say(strength, false),
        move |strength| say(strength, true),
    )
}

/// How far they're turned down while only our echo comes from them, in dB.
fn echo_depth(menu: &ParticipantMenu, depth: i32) -> Element<'_, Intent> {
    let say = move |depth, last| Intent::EchoDepth {
        person: menu.person.clone(),
        depth,
        last,
    };
    slider::volume(
        Volume::up_to(tr!("Echo removal strength"), depth, DEPTH)
            .shown(tr!("{} dB", depth))
            .step(5),
        move |depth| say(depth, false),
        move |depth| say(depth, true),
    )
}

/// A context menu's ground, outline and shadow; square at the bottom while
/// the rows join it.
fn card(theme: &Theme, joined: bool) -> container::Style {
    let palette = theme.palette();
    let corner = if joined { 0.0 } else { RADIUS };
    container::Style {
        background: Some(palette.menu().into()),
        border: Border {
            color: palette.menu_line(),
            width: OUTLINE,
            radius: Radius::new(RADIUS).bottom_left(corner).bottom_right(corner),
        },
        shadow: Shadow {
            color: SHADOW,
            offset: Vector::new(0.0, SHADOW_DROP),
            blur_radius: SHADOW_BLUR,
        },
        ..container::Style::default()
    }
}

/// Its shots for `anomi-shots`: the whole window with a participant's menu
/// open over the stage, as `voice/participant_menu.png`.
#[cfg(feature = "shots")]
pub fn shots() -> Vec<crate::shots::Shot> {
    use crate::shots::{GOLDEN, Shot};
    use anomi_vm::fixtures::voice::participant_menu as fixture;
    vec![
        Shot {
            area: "voice",
            name: "participant_menu",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::carol,
        },
        Shot {
            area: "voice",
            name: "participant_menu_stream",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::sharing,
        },
        Shot {
            area: "voice",
            name: "participant_menu_alone",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::alone,
        },
        Shot {
            area: "voice",
            name: "participant_menu_cleaning",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::cleaning,
        },
        Shot {
            area: "voice",
            name: "participant_menu_their_noise",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::their_noise,
        },
        Shot {
            area: "voice",
            name: "stream_menu",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::stream,
        },
        Shot {
            area: "voice",
            name: "stream_menu_muted",
            size: GOLDEN,
            theme: "bonfire",
            vm: fixture::stream_muted,
        },
    ]
}
