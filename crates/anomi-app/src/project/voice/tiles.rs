//! The call's tiles from the core's call, in Discord's order
//! (`super::focus`): everyone in the call's order, each stream in a tile of
//! its own right before its streamer's, this account's screen right before
//! its own tile (`ParticipantGrid`'s three loops, `ui/voice.slint:988`-
//! `:1116`, over `show_voice`'s rows, `src/ui/refresh.rs:4650`, as Discord
//! lays them out). Someone's own tile shows their camera or their picture
//! with their speaking ring and marks, never their stream; the stream's
//! shows it watched, its preview behind "Watch stream", or the screen icon
//! without one, with the LIVE badge.
//!
//! A tile is built again only when what it reads changed: its participant
//! (compared with the list shown before, which the core replaces whole), or
//! what it reads beside them (the stream watched, a preview, its picture,
//! whether its camera's first frame came), kept as a hash per tile. So
//! someone muting or starting to speak builds their tile alone. A camera's
//! or a stream's frames reach its tile through one `Live` per feed, kept
//! while the feed lasts; the tile holds it once its first frame came. A
//! stream not watched costs what it did before: its preview, or nothing.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use yapfast_core::application::AppState;
use yapfast_core::voice::{CameraPhase, Participant, SharePhase};
use yapfast_lang::tr;

use anomi_vm::kind::Live;
use anomi_vm::voice::tiles::{Chip, Fill, Key, Mark, Outline, Tile, Viewers};

use super::focus::{Slot, Stage};
use super::lives::Lives;
use crate::pictures::Requests;
use crate::project::rows;

/// What the tiles showed: each with the hash of what it read beside its
/// participant, and the participants they were built from.
#[derive(Default)]
pub struct Shown {
    built: HashMap<Key, (u64, Tile)>,
    participants: Arc<[Participant]>,
    /// How many tiles were built, for the tests.
    #[cfg(test)]
    pub builds: usize,
}

/// The tiles of the call, in their slots; built again only where changed.
/// This account's screen shows no picture of itself while its preview is
/// `hidden`.
pub fn tiles(
    state: &AppState,
    stage: Stage<'_>,
    lives: &Lives,
    hidden: bool,
    shown: &mut Shown,
    requests: &mut Requests,
) -> Vec<Tile> {
    let voice = &state.voice;
    let watching = voice.watching.as_deref();
    let mut built = std::mem::take(&mut shown.built);
    let mut builds = 0;
    let mut take =
        |key: Key, context: u64, same: bool, build: &dyn Fn() -> Tile| match built.remove(&key) {
            Some((kept, tile)) if same && kept == context => (key, context, tile),
            _ => {
                builds += 1;
                (key, context, build())
            }
        };
    let camera_starting = voice.camera_phase == CameraPhase::Starting;
    let mut made = Vec::with_capacity(stage.count());
    for slot in stage.slots() {
        made.push(match slot {
            Slot::OwnScreen => {
                let sharing = voice.sharing.as_ref();
                let live = (lives.own.as_ref()).map(|own| &own.1).filter(|_| !hidden);
                let context = hash((
                    sharing.map(|sharing| sharing.phase as u8),
                    live.is_some_and(Live::has_frame),
                    hidden,
                    sharing.map(|sharing| &sharing.viewers),
                    state.picture_changes,
                ));
                take(Key::OwnScreen, context, true, &|| {
                    own_screen(state, stage.participants, live, hidden)
                })
            }
            Slot::Stream(index) => {
                let person = &stage.participants[index];
                let watched = watching.filter(|watching| watching.user == person.id);
                // The preview comes again after watching stops.
                let context = hash((
                    watch_context(state, watched, lives),
                    &name(person),
                    state.stream_previews.get(&person.id).map(Arc::as_ptr),
                ));
                let key = Key::Stream(person.id.clone());
                take(key, context, true, &|| {
                    stream(state, person, watched, lives)
                })
            }
            Slot::Person(index) => {
                let person = &stage.participants[index];
                let before = shown.participants.iter().find(|old| old.id == person.id);
                let same = before == Some(person);
                // Their picture, and its moving version, which the tile
                // plays while they speak: asked for, and built again as
                // either comes.
                let still = person.avatar.as_deref();
                let moving = crate::pictures::moving_url(still);
                requests.want(state, still);
                requests.want(state, moving.as_deref());
                let context = hash((
                    picture_at(state, still),
                    picture_at(state, moving.as_deref()),
                    lives.camera(&person.id).is_some_and(Live::has_frame),
                    person.local && camera_starting,
                ));
                let key = Key::Person(person.id.clone());
                take(key, context, same, &|| participant(state, person, lives))
            }
        });
    }
    #[cfg(test)]
    {
        shown.builds += builds;
    }
    let _ = builds;
    // The core's list itself, shared: no participant is copied.
    shown.participants = if voice.active() {
        voice.participants.clone()
    } else {
        Arc::default()
    };
    let mut tiles = Vec::with_capacity(made.len());
    for (key, context, tile) in made {
        tiles.push(tile.clone());
        shown.built.insert(key, (context, tile));
    }
    tiles
}

/// Forgets every tile, as the call ends or the window goes.
pub fn forget(shown: &mut Shown) {
    *shown = Shown::default();
}

/// What a tile showing `watched` reads of it: whether it's watched, its
/// first frame came, who watches it.
fn watch_context(
    state: &AppState,
    watched: Option<&yapfast_core::voice::Watching>,
    lives: &Lives,
) -> u64 {
    let Some(watched) = watched else {
        return 0;
    };
    hash((
        &watched.user,
        watched.frames.is_some(),
        lives
            .watched
            .as_ref()
            .is_some_and(|live| live.1.has_frame()),
        &watched.viewers,
        state.picture_changes,
    ))
}

fn hash(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// The core's picture at `address`, as a number that changes when it comes.
fn picture_at(state: &AppState, address: Option<&str>) -> usize {
    address
        .and_then(|address| state.pictures.get(address))
        .map_or(0, |picture| Arc::as_ptr(picture).addr())
}

/// A participant's name as their row shows it.
fn name(person: &Participant) -> &str {
    crate::project::account::display_part(&person.user)
}

/// This account's shared screen (`:988`), right before its own tile, the
/// LIVE badge at its top right once live; without its picture while its
/// preview is `hidden`.
fn own_screen(
    state: &AppState,
    participants: &[Participant],
    live: Option<&Live>,
    hidden: bool,
) -> Tile {
    let sharing = state.voice.sharing.as_ref();
    let phase = sharing.map(|sharing| sharing.phase);
    let is_live = phase == Some(SharePhase::Live);
    let frame = live.filter(|live| live.has_frame()).cloned();
    let status = own_status(phase, hidden).to_owned();
    let note = if !is_live && frame.is_some() {
        status.clone()
    } else {
        String::new()
    };
    let ids = sharing.map_or(&[][..], |sharing| &sharing.viewers);
    Tile {
        key: Key::OwnScreen,
        fill: Fill::OwnScreen { frame, status },
        chip: Chip {
            name: tr!("Your screen").into(),
            note,
            ..Chip::default()
        },
        badge: is_live,
        viewers: viewers(state, participants, ids, None),
        ..Tile::default()
    }
}

/// How this account's stream goes, as the dock and its tile say it.
pub fn share_status(phase: Option<SharePhase>) -> &'static str {
    match phase {
        Some(SharePhase::Choosing) => tr!("Choose what to share"),
        Some(SharePhase::Starting) => tr!("Starting your stream..."),
        Some(SharePhase::Live) => tr!("Live"),
        None => "",
    }
}

/// What this account's screen says where its picture would be: how its
/// stream goes, or, once live with its preview hidden, that it is.
pub fn own_status(phase: Option<SharePhase>, hidden: bool) -> &'static str {
    match phase {
        Some(SharePhase::Live) if hidden => tr!("Your stream preview is hidden"),
        phase => share_status(phase),
    }
}

/// What a stream watched says until its first picture.
pub fn watch_status(watched: &yapfast_core::voice::Watching) -> String {
    if watched.frames.is_none() {
        tr!("Connecting to {}'s stream...", watched.name)
    } else {
        tr!("Waiting for {}'s picture...", watched.name)
    }
}

/// Someone's stream in its own tile, right before theirs (`:1008`), the
/// LIVE badge at its top right: watched, its picture; else its preview, or
/// the screen icon without one, behind "Watch stream", which a click
/// watches.
fn stream(
    state: &AppState,
    person: &Participant,
    watched: Option<&yapfast_core::voice::Watching>,
    lives: &Lives,
) -> Tile {
    let fill = match watched {
        Some(watched) => {
            let frame = lives.watched.as_ref().map(|live| &live.1);
            Fill::Watched {
                frame: frame.filter(|live| live.has_frame()).cloned(),
                status: watch_status(watched),
            }
        }
        None => Fill::Offer {
            preview: (state.stream_previews.get(&person.id)).and_then(crate::pictures::shown),
        },
    };
    Tile {
        key: Key::Stream(person.id.clone()),
        watch: watched.is_none(),
        viewers: watched.map_or_else(Viewers::default, |watched| {
            viewers(
                state,
                &state.voice.participants,
                &watched.viewers,
                Some(&person.id),
            )
        }),
        fill,
        chip: Chip {
            name: name(person).to_owned(),
            mark: Some(Mark::Screen),
            ..Chip::default()
        },
        badge: true,
        ..Tile::default()
    }
}

/// Someone's own tile (`:1051`): their camera, or their picture, with
/// their speaking ring and marks. Their stream, if they share one, has its
/// own tile right before.
fn participant(state: &AppState, person: &Participant, lives: &Lives) -> Tile {
    let camera = lives
        .camera(&person.id)
        .filter(|live| person.video.is_some() && live.has_frame());
    let fill = if let Some(live) = camera {
        Fill::Camera(live.clone())
    } else {
        let key = if person.local {
            state.account.as_deref().unwrap_or("Preview")
        } else {
            name(person)
        };
        // Its moving version once it came: the tile plays it while they
        // speak (stage, grid and picture in picture alike).
        Fill::Avatar {
            avatar: rows::moving_avatar(state, name(person), key, person.avatar.as_deref()),
            connected: person.connected,
        }
    };
    let speaking = person.speaking && !person.muted && person.connected;
    let mark = if person.deafened {
        Some(Mark::Deafened)
    } else if person.muted {
        Some(Mark::Muted)
    } else {
        None
    };
    let note = match &person.problem {
        Some(problem) => problem.clone(),
        None if !person.connected => tr!("Connection lost").into(),
        None => String::new(),
    };
    let starting = person.local && state.voice.camera_phase == CameraPhase::Starting;
    Tile {
        key: Key::Person(person.id.clone()),
        fill,
        outline: if speaking {
            Outline::Speaking
        } else {
            Outline::Line
        },
        chip: Chip {
            name: name(person).to_owned(),
            mark,
            live: false,
            note,
        },
        starting,
        ..Tile::default()
    }
}

/// Who watches a stream, in the call's order, its streamer left out: the
/// first three's pictures, how many, and their names for the tooltip.
pub fn viewers(
    state: &AppState,
    participants: &[Participant],
    ids: &[String],
    streamer: Option<&str>,
) -> Viewers {
    let watching: Vec<&Participant> = participants
        .iter()
        .filter(|person| Some(person.id.as_str()) != streamer && ids.contains(&person.id))
        .collect();
    if watching.is_empty() {
        return Viewers::default();
    }
    let names: Vec<&str> = watching.iter().map(|person| name(person)).collect();
    Viewers {
        faces: watching
            .iter()
            .take(Viewers::FACES)
            .map(|person| rows::avatar(state, name(person), name(person), person.avatar.as_deref()))
            .collect(),
        count: watching.len(),
        names: tr!("Watching: {}", names.join(", ")),
    }
}
