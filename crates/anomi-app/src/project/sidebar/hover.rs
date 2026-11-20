//! The card beside someone's row under a voice channel while it shows
//! (`intent::sidebar::hover` says when): their stream with its preview from
//! `AppState::stream_previews` and whether this account watches it, the
//! game they play with since when, and where it's sold from
//! `AppState::game_store`. Built only while it shows, and again as the
//! sidebar shows anything new (a preview that came, a watch, the game's
//! store), the sidebar's version moving only when it changed. The time
//! played runs on its own as the card draws (`kind::Elapsed`).

use std::time::{Duration, Instant};

use yapfast_core::application::{AppState, Application};
use yapfast_core::chat::{Store as Shop, VoicePerson as Person};
use yapfast_lang::tr;

use anomi_vm::icon::Icon;
use anomi_vm::kind::Elapsed;
use anomi_vm::sidebar::channels::Channels;
use anomi_vm::sidebar::channels::hover::{Game, HoverCard, Store, Stream};

use crate::pictures;
use crate::project::rows;

/// Whose card shows, and since when the game on it is played.
#[derive(Default)]
pub struct Hover {
    /// The voice channel's room id and the person's user id, while their
    /// card shows.
    pub(crate) at: Option<(String, String)>,
    /// The start of the game on the card, in Unix milliseconds, and the
    /// moment it was on the window's clock: kept, so the card built again
    /// keeps the same time and compares the same.
    started: Option<(u64, Instant)>,
}

/// Builds the card that shows into `channels`, or takes it away; whether
/// it changed. It shows only beside a row the list shows: another server,
/// a filter or a folded category takes it away, and so do their leaving
/// and their stopping what it showed (`intent::sidebar::hover::lost` then
/// tells the core).
pub(super) fn show(core: &Application, hover: &mut Hover, channels: &mut Channels) -> bool {
    let card = match &hover.at {
        Some((channel, user)) if listed(channels, channel, user) => {
            card(&core.state, &mut hover.started, channel, user)
        }
        _ => None,
    };
    if channels.hover.as_deref() == card.as_ref() {
        return false;
    }
    channels.hover = card.map(Box::new);
    true
}

/// `user`'s row shows under the voice channel `channel`, with something
/// for a card.
fn listed(channels: &Channels, channel: &str, user: &str) -> bool {
    (channels.rows.iter())
        .filter(|row| row.id == channel && !row.hidden)
        .flat_map(|row| &row.people)
        .any(|person| person.user == user && person.has_card())
}

/// The card of `user` under the voice channel `channel`: none once they
/// neither stream for this account to watch nor play, or left.
fn card(
    state: &AppState,
    started: &mut Option<(u64, Instant)>,
    channel: &str,
    user: &str,
) -> Option<HoverCard> {
    let room = (state.rooms.iter()).find(|room| room.id == channel && room.voice)?;
    let person = room.people.iter().find(|person| person.id == user)?;
    let me = state.discord_user.as_deref() == Some(user);
    let voice = &state.voice;
    let stream = (person.streaming && !me).then(|| Stream {
        person: person.id.clone(),
        preview: (state.stream_previews.get(user)).and_then(pictures::shown),
        watching: voice.room.as_deref() == Some(channel)
            && (voice.watching.as_ref()).is_some_and(|watching| watching.user == user),
    });
    let game = game(state, started, person);
    (stream.is_some() || game.is_some()).then(|| HoverCard {
        channel: channel.to_owned(),
        user: user.to_owned(),
        stream,
        game,
    })
}

/// The game they play, as the card shows it.
fn game(state: &AppState, started: &mut Option<(u64, Instant)>, person: &Person) -> Option<Game> {
    let playing = person.playing()?;
    let elapsed = playing.start.map(|start| {
        let at = match *started {
            Some((kept, at)) if kept == start => at,
            _ => {
                let gone = yapfast_core::clock::now_ms().saturating_sub(start);
                let at = Instant::now()
                    .checked_sub(Duration::from_millis(gone))
                    .unwrap_or_else(Instant::now);
                *started = Some((start, at));
                at
            }
        };
        Elapsed::Since(at)
    });
    let store = (state.game_store.as_ref())
        .filter(|(application, _)| Some(application.as_str()) == playing.application)
        .and_then(|(_, page)| page.as_ref())
        .map(|page| {
            let (label, glyph) = match page.store {
                Shop::Steam => (tr!("Get on Steam"), Icon::Steam),
                Shop::Xbox => (tr!("Get on Xbox"), Icon::Shop),
                Shop::Microsoft => (tr!("Get on Microsoft Store"), Icon::Shop),
            };
            Store {
                label: label.to_owned(),
                glyph,
                url: page.url.clone(),
            }
        });
    Some(Game {
        name: playing.name.to_owned(),
        icon: rows::expected(state, playing.icon),
        elapsed,
        store,
    })
}

#[cfg(all(test, feature = "discord"))]
mod tests;
