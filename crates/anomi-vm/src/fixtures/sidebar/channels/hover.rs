//! The card beside Bob's row in the Lounge, in its three states as the
//! owner's Discord screenshots show them
//! (`discord-voice-hover-{playing,streaming,watching}.png`): Bob playing
//! Factorio with its Steam page, Bob streaming it with no still yet, and
//! the same while Alice watches him in the call; and once his still came.
//! Factorio is sold on Steam as 427520 (`DETECTABLE_GAMES`,
//! `testing/fake_discord.ts:5891`), and Bob has played it 42 minutes as the
//! fake starts.

use std::time::Duration;

use yapfast_lang::tr;

use crate::fixtures::pictures::{picture, sunset};
use crate::icon::Icon;
use crate::kind::{Elapsed, Expected};
use crate::sidebar::channels::Channels;
use crate::sidebar::channels::hover::{Game, HoverCard, Store, Stream};

use super::people::FACTORIO_ICON;
use super::{LOUNGE, bob};

/// Factorio on the card, 42 minutes and 7 seconds in, with its Steam page.
pub fn factorio() -> Game {
    Game {
        name: "Factorio".into(),
        icon: Expected::Ready(picture(FACTORIO_ICON)),
        elapsed: Some(Elapsed::Stopped(Duration::from_secs(42 * 60 + 7))),
        store: Some(Store {
            label: tr!("Get on Steam").to_owned(),
            glyph: Icon::Steam,
            url: "https://store.steampowered.com/app/427520/".into(),
        }),
    }
}

/// Bob's card with `stream`.
fn card(stream: Option<Stream>) -> Box<HoverCard> {
    Box::new(HoverCard {
        channel: LOUNGE.into(),
        user: bob().user,
        stream,
        game: Some(factorio()),
    })
}

/// His stream on the card: its still, and whether Alice watches it.
fn stream(preview: bool, watching: bool) -> Stream {
    Stream {
        person: bob().id,
        preview: preview.then(|| sunset(512, 288)),
        watching,
    }
}

/// Bob no longer streams and plays Factorio: its icon ends his row, and his
/// card says he's playing, with its Steam page.
pub fn playing() -> Channels {
    let mut channels = super::channels();
    let lounge = &mut channels.rows[3];
    lounge.people[0].live = false;
    lounge.summary = "3 people".into();
    channels.hover = Some(card(None));
    channels
}

/// Bob streams, and his client sent no still yet: the card says the stream
/// started, and offers to watch it.
pub fn streaming() -> Channels {
    let mut channels = super::channels();
    channels.hover = Some(card(Some(stream(false, false))));
    channels
}

/// The same once his still came.
pub fn streaming_preview() -> Channels {
    let mut channels = super::channels();
    channels.hover = Some(card(Some(stream(true, false))));
    channels
}

/// Alice in the call watches Bob's stream: Watching Stream, disabled; the
/// call let go of his still, as the core does for a stream watched.
pub fn watching() -> Channels {
    let mut channels = super::joined();
    channels.hover = Some(card(Some(stream(false, true))));
    channels
}
