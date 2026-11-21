//! Who wrote a message and the line over it: the author's picture, name,
//! style and tag (`message_row`'s author fields and `Layout::restyle`'s
//! cosmetics), the message it replies to or the command it answers.

use iced::Color;
use iced::widget::image;
use yapfast_core::application::AppState;
use yapfast_core::chat::{ChatMessage, NoticeKind};
use yapfast_core::discord;
use yapfast_lang::tr;

use anomi_vm::chat::message::{Author, Command, Header, Notice, Reply};
use anomi_vm::kind::{Avatar, NameStyle, Tag};

use super::{Context, Look};
use crate::pictures;
use crate::project::rows;

/// The palette's neutral grey, a blocked person's disc: it names no one.
fn grey() -> Color {
    Color::from_rgb8(0x64, 0x6b, 0x7a)
}

pub(super) fn author(cx: &Context<'_>, message: &ChatMessage, blocked: bool) -> Author {
    let state = cx.state;
    if blocked {
        return Author {
            name: tr!("Blocked user").to_owned(),
            avatar: Avatar {
                initial: "?".into(),
                tint: grey(),
                picture: None,
            },
            ..Author::default()
        };
    }
    let cosmetics = cx.look.cosmetics;
    let style = cosmetics
        .style(message.style.as_deref())
        .and_then(|style| NameStyle::new(style.font, style.effect, style.colors()))
        // In a server, an author's name shows its style's font alone.
        .map(|style| {
            if cx.look.in_server {
                style.font_only()
            } else {
                style
            }
        });
    let faces = faces(state, cx.look, message, false);
    Author {
        name: message.author.clone(),
        avatar: faces.avatar,
        decoration: faces.decoration,
        color: message.color.map(rows::rgb),
        style,
        tag: cosmetics.tag(message.tag.as_ref()).map(|tag| Tag {
            text: tag.text.to_string(),
            badge: rows::expected(state, tag.badge.as_deref()),
            server: tag.guild.map(|guild| guild.to_string()).unwrap_or_default(),
        }),
        bot: message.bot,
        known: !message.sender.is_empty(),
    }
}

/// An author's picture and decoration as their message's row shows them,
/// with the addresses of their moving versions.
pub(crate) struct Faces {
    pub avatar: Avatar,
    pub decoration: Option<image::Handle>,
    /// The moving versions shown while lit, to ask for and to watch; none
    /// while dark.
    pub moving: Vec<String>,
}

/// The author's picture and decoration: while the message is `lit` (the
/// pointer on it, which plays them, as in Discord's client), their moving
/// versions once they came; else, and until then, the stills. Not for a
/// blocked person's message, which shows neither.
pub(crate) fn faces(state: &AppState, look: &Look, message: &ChatMessage, lit: bool) -> Faces {
    let avatar = message.avatar.as_deref();
    let decoration = look.cosmetics.decoration(message.decoration.as_deref());
    let author = &message.author;
    if !lit {
        return Faces {
            avatar: rows::avatar(state, author, author, avatar),
            decoration: pictures::find(state, decoration),
            moving: Vec::new(),
        };
    }
    let moving_decoration = decoration
        .map(pictures::animated_url)
        .filter(|moving| !moving.is_empty() && Some(moving.as_str()) != decoration);
    Faces {
        avatar: rows::moving_avatar(state, author, author, avatar),
        decoration: rows::moving_collectible(state, decoration),
        moving: pictures::moving_url(avatar)
            .into_iter()
            .chain(moving_decoration)
            .collect(),
    }
}

/// The message it replies to, or the command it answers when it quotes
/// none.
pub(super) fn header(cx: &Context<'_>, message: &ChatMessage, blocked: bool) -> Header {
    let state = cx.state;
    if blocked {
        return Header::None;
    }
    if let Some(reply) = &message.reply {
        return Header::Reply(Box::new(Reply {
            author: reply.author.clone(),
            avatar: rows::avatar(state, &reply.author, &reply.author, reply.avatar.as_deref()),
            color: reply.color.map(rows::rgb),
            body: reply.body.clone(),
            jumps: reply.item.as_deref().is_some_and(|item| !item.is_empty()),
        }));
    }
    let Some(used) = &message.used else {
        return Header::None;
    };
    Header::Command(Box::new(Command {
        user: used.user.clone(),
        avatar: rows::avatar(state, &used.user, &used.user, used.avatar.as_deref()),
        color: used.color.map(rows::rgb),
        known: discord::sender_user(&used.sender).is_some_and(|user| !user.is_empty()),
        name: used.name.clone(),
        usable: used.slash.is_some() && cx.look.writes,
        target: used.target.clone(),
    }))
}

/// The icon a notice shows in place of its author's picture.
pub(super) fn notice(message: &ChatMessage) -> Option<Notice> {
    Some(match &message.notice.as_ref()?.kind {
        NoticeKind::Pinned(_) => Notice::Pin,
        NoticeKind::Joined => Notice::Join,
        NoticeKind::ThreadStarted => Notice::Thread,
        NoticeKind::Call => Notice::Call,
        NoticeKind::PollResult => Notice::Poll,
    })
}
