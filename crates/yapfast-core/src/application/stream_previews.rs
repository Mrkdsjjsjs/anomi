//! The previews of others' streams, which their tiles in this account's call
//! show until the stream is watched, and the card beside a streamer's row in
//! the channel list: when Discord is asked for them
//! (`DiscordCommand::StreamPreview`), and keeping what comes.
//!
//! Each is asked for as it starts to show, then again every
//! [`STREAM_PREVIEW_EVERY_MS`] while its tile or its card is on screen, as
//! the interface says ([`Action::VisibleStreams`],
//! [`Action::HoverVoicePerson`]): docs.discord.food names no interval for a
//! viewer, so a preview is asked for as often as Discord's client uploads
//! one (`THUMBNAIL_EVERY` in `stream_worker.rs`). Nothing is asked for a tile
//! out of sight, behind another window or in the tray, and a tile that comes
//! back after its preview fell due asks once, at once. The interface waits
//! for the moment the next one falls due ([`Application::stream_previews_due`])
//! and says when it came ([`Action::StreamPreviewsDue`]). An answer that
//! didn't come keeps the picture there was until the next time. A stream
//! outside the call keeps its preview only while its card shows.

use super::*;

/// How often a preview on screen is asked for again, in milliseconds: as
/// often as the streamer's client uploads one.
pub const STREAM_PREVIEW_EVERY_MS: u64 = 5 * 60 * 1000;

/// When each stream's preview was asked for, whose tiles show, and whose
/// card.
#[derive(Default)]
pub(super) struct PreviewAsks {
    /// When each stream's preview was last asked for, by its streamer's user
    /// id, in Unix milliseconds.
    asked: HashMap<String, u64>,
    /// The streams whose tiles show, as the interface last said.
    visible: Vec<String>,
    /// Whose card shows beside their row in the channel list, by the voice
    /// channel's room id and their user id, as the interface last said.
    pub(super) hovered: Option<(String, String)>,
}

/// Whose streams show a preview in their tiles: everyone else sharing their
/// screen in the Discord call this account is in, but the one it watches,
/// whose tile shows the stream itself.
fn previewed(state: &voice::State) -> impl Iterator<Item = &str> {
    let here = matches!(state.phase, Phase::Connected | Phase::Reconnecting)
        && state
            .room
            .as_deref()
            .is_some_and(|room| discord::parse_room(room).is_some());
    let watched = state
        .watching
        .as_ref()
        .map(|watching| watching.user.as_str());
    state
        .participants
        .iter()
        .filter(move |person| here && person.streaming && !person.local)
        .map(|person| person.id.as_str())
        .filter(move |id| Some(*id) != watched)
}

/// The stream whose card shows, by room and user id, while the card shows
/// its preview: someone else's Discord stream, going on as the channel list
/// has it, that this account doesn't watch.
fn carded<'a>(
    state: &'a AppState,
    hovered: &'a Option<(String, String)>,
) -> Option<(&'a str, &'a str)> {
    let (room, user) = hovered.as_ref()?;
    discord::parse_room(room)?;
    let voice = &state.voice;
    let watched = voice.room.as_ref() == Some(room)
        && (voice.watching.as_ref()).is_some_and(|watching| watching.user == *user);
    let streams = (state.rooms.iter())
        .find(|listed| listed.id == *room)
        .and_then(|listed| listed.people.iter().find(|person| person.id == *user))
        .is_some_and(|person| person.streaming);
    (streams && !watched && state.discord_user.as_ref() != Some(user))
        .then_some((room.as_str(), user.as_str()))
}

/// The streams whose previews show now, by room and user id: the call's
/// whose tiles are in sight, and the one whose card shows, once.
fn on_screen<'a>(
    state: &'a AppState,
    asks: &'a PreviewAsks,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let call = state.voice.room.as_deref().unwrap_or_default();
    let in_sight = move |user: &str| asks.visible.iter().any(|shown| shown == user);
    let tiles = previewed(&state.voice)
        .filter(move |user| in_sight(user))
        .map(move |user| (call, user));
    let card =
        carded(state, &asks.hovered).filter(move |(room, user)| *room != call || !in_sight(user));
    tiles.chain(card)
}

/// Asks for the preview of `user`'s stream in `room`, noting it was asked
/// for at `now`.
fn ask(asked: &mut HashMap<String, u64>, room: &str, user: &str, now: u64) -> Effect {
    asked.insert(user.to_owned(), now);
    Effect::Backend(Command::Discord(DiscordCommand::StreamPreview {
        room: room.to_owned(),
        user: user.to_owned(),
    }))
}

impl Application {
    /// Asks Discord for the preview of each stream in the call that began
    /// to show one since `before`: someone went live, this account joined a
    /// call where they already were, or it stopped watching them, by when
    /// their client has likely uploaded a newer one. Lets go of the previews
    /// of streams that ended or are watched now, and of a call that was left,
    /// but the one whose card shows. It runs on every change in the call,
    /// such as who speaks, and without a stream in the call it only looks
    /// through who's there.
    pub(super) fn preview_streams(&mut self, before: &voice::State) -> Vec<Effect> {
        let carded =
            carded(&self.state, &self.preview_asks.hovered).map(|(_, user)| user.to_owned());
        let after = &self.state.voice;
        let same_call = before.room == after.room;
        let previews = &mut self.state.stream_previews;
        let asked = &mut self.preview_asks.asked;
        if !previews.is_empty() || !asked.is_empty() {
            let shown = |user: &String| {
                carded.as_ref() == Some(user)
                    || (same_call && previewed(after).any(|shown| shown == user))
            };
            previews.retain(|user, _| shown(user));
            asked.retain(|user, _| shown(user));
        }
        let Some(room) = &after.room else {
            return vec![];
        };
        let now = crate::clock::now_ms();
        previewed(after)
            .filter(|user| !same_call || previewed(before).all(|shown| shown != *user))
            .map(|user| ask(asked, room, user, now))
            .collect()
    }

    /// Keeps a stream's preview that came, unless the stream ended, is
    /// watched, or its call was left and its card went meanwhile. Without
    /// one, an older preview stays.
    pub(super) fn keep_stream_preview(
        &mut self,
        room: &str,
        user: String,
        picture: Option<Arc<Picture>>,
    ) {
        let voice = &self.state.voice;
        let in_call =
            voice.room.as_deref() == Some(room) && previewed(voice).any(|shown| shown == user);
        let carded = carded(&self.state, &self.preview_asks.hovered) == Some((room, user.as_str()));
        if let Some(picture) = picture
            && (in_call || carded)
        {
            self.state.stream_previews.insert(user, picture);
        }
    }

    /// The streams whose tiles show at `now`, by their streamers' user ids:
    /// asks at once for those whose previews fell due while they didn't.
    pub(super) fn show_streams(&mut self, users: Vec<String>, now: u64) -> Vec<Effect> {
        self.preview_asks.visible = users;
        self.ask_due_previews(now)
    }

    /// The card beside someone's row in the channel list shows, by the
    /// voice channel's room id and their user id, or it went: a stream of
    /// theirs counts as on screen, asked for at once unless it was lately,
    /// and one outside the call lets go of its preview with its card.
    pub(super) fn hover_stream(
        &mut self,
        hovered: Option<(String, String)>,
        now: u64,
    ) -> Vec<Effect> {
        let before =
            carded(&self.state, &self.preview_asks.hovered).map(|(_, user)| user.to_owned());
        self.preview_asks.hovered = hovered;
        let after =
            carded(&self.state, &self.preview_asks.hovered).map(|(_, user)| user.to_owned());
        if let Some(user) = before
            && after.as_ref() != Some(&user)
            && previewed(&self.state.voice).all(|shown| shown != user)
        {
            self.state.stream_previews.remove(&user);
            self.preview_asks.asked.remove(&user);
        }
        self.ask_due_previews(now)
    }

    /// Asks again for each preview on screen that fell due by `now`, in Unix
    /// milliseconds.
    pub(super) fn ask_due_previews(&mut self, now: u64) -> Vec<Effect> {
        let asks = &self.preview_asks;
        let due: Vec<(String, String)> = on_screen(&self.state, asks)
            .filter(|(_, user)| {
                (asks.asked.get(*user)).is_none_or(|at| at + STREAM_PREVIEW_EVERY_MS <= now)
            })
            .map(|(room, user)| (room.to_owned(), user.to_owned()))
            .collect();
        let asked = &mut self.preview_asks.asked;
        due.iter()
            .map(|(room, user)| ask(asked, room, user, now))
            .collect()
    }

    /// When the next preview on screen falls due, in Unix milliseconds; none
    /// while no stream's tile or card shows. The interface waits until then,
    /// and then sends [`Action::StreamPreviewsDue`].
    pub fn stream_previews_due(&self) -> Option<u64> {
        let asks = &self.preview_asks;
        on_screen(&self.state, asks)
            .filter_map(|(_, user)| asks.asked.get(user))
            .map(|at| at + STREAM_PREVIEW_EVERY_MS)
            .min()
    }
}
