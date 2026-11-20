//! The card beside someone's row under a voice channel, as Discord's client
//! shows it once the pointer rests on the row: what they're doing now. One
//! who streams shows "Streaming now" with the LIVE tag, their stream's
//! latest still (or word that it started, until one came) and Watch Stream,
//! disabled while this account watches it; one who plays shows "Playing
//! now". Either shows the game with how long they've played, and where it's
//! sold. The app fills it while it shows and empties it as it goes; the
//! row's view draws it beside the row whose channel and user it names.
//!
//! Discord's "Ask to Stream" isn't here: docs.discord.food describes no
//! request a client sends for it (`docs/discord/voice.md`).

use iced::widget::image;

use crate::icon::Icon;
use crate::kind::{Elapsed, Expected};

/// What the card shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverCard {
    /// The voice channel's room id and the person's user id: the row it
    /// shows beside.
    pub channel: String,
    pub user: String,
    /// Their stream, while they stream for this account to watch.
    pub stream: Option<Stream>,
    pub game: Option<Game>,
}

impl HoverCard {
    /// It shows beside the row of `user` under the voice channel `channel`.
    pub fn beside(&self, channel: &str, user: &str) -> bool {
        self.channel == channel && self.user == user
    }
}

/// Someone's stream on their card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    /// Who they are in the call, which Watch Stream goes by.
    pub person: String,
    /// Its latest still, once one came; until then the card says the
    /// stream started.
    pub preview: Option<image::Handle>,
    /// This account watches it already: the button says so, disabled.
    pub watching: bool,
}

/// The game on someone's card.
#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub name: String,
    pub icon: Expected,
    /// How long they've played, which the card's clock reads as it draws;
    /// none when their presence doesn't say since when.
    pub elapsed: Option<Elapsed>,
    /// Where it's sold: the button that opens its page; none while that's
    /// unknown, or it's sold nowhere a page can be named.
    pub store: Option<Store>,
}

/// The button to where a game is sold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    /// "Get on Steam", translated.
    pub label: String,
    /// The store's mark before it.
    pub glyph: Icon,
    /// Its page, which opens in the browser.
    pub url: String,
}
