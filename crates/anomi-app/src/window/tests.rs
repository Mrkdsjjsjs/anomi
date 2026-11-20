//! The window's focus as the core hears it, and its sight: what keeps going
//! in front, on screen unfocused and hidden.

use std::sync::Arc;

use iced::Size;
use yapfast_core::application::{Action, Effect};
use yapfast_core::chat::{self, Attachment, ChatMessage, Command, DiscordCommand, Event};
use yapfast_core::frames::{Frames, Writer};

use anomi_theme::motion::{Background, Sight};

use super::seen::Heard;
use crate::app::App;
use crate::message::Message;
use crate::tests::{SIZE, start};

/// Whether `effects` ask Discord for the Activity panel's feed.
fn asks_for_feed(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Backend(Command::Discord(DiscordCommand::ActivityFeed))
        )
    })
}

/// The feed is due again, as Discord's timer says: whether the core asks.
fn asks_when_due(app: &mut App) -> bool {
    asks_for_feed(&app.core.apply(Event::ActivityFeedDue))
}

/// The feed asked for came: whether the core asks again, as it does when
/// the feed fell due meanwhile. One that wasn't asked for is dropped.
fn feed_came(app: &mut App) -> bool {
    asks_for_feed(&app.core.apply(Event::ActivityFeed(Arc::default())))
}

#[test]
fn the_core_hears_whether_the_window_is_in_front() {
    let folder = std::env::temp_dir().join(format!("anomi-focus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mut app = App::for_tests(&folder);
    // A Discord server's channel, online: the Activity panel's feed is due.
    drop(app.dispatch(Action::Preview));
    let state = &mut app.core.state;
    let mut room = (*state.rooms[0]).clone();
    room.id = yapfast_core::discord::room_id("1", "2");
    room.direct = false;
    room.forum = false;
    state.selected = Some(room.id.clone());
    state.rooms = vec![Arc::new(room)];
    state.preview = false;
    state.discord_online = true;

    assert!(!asks_when_due(&mut app), "behind until the window says so");
    drop(app.update(Message::Focused(true)));
    assert!(app.motion.front);
    // Coming to the front asked for the feed (of no backend here): it's on
    // its way, and as it comes the core asks again for the turn that fell
    // due meanwhile.
    assert!(!asks_when_due(&mut app), "one is on its way");
    assert!(feed_came(&mut app), "in front, it's asked when due");
    assert!(!feed_came(&mut app));

    drop(app.update(Message::Focused(false)));
    assert!(!app.motion.front);
    assert!(!asks_when_due(&mut app), "behind another window");
    drop(app);
    let _ = std::fs::remove_dir_all(&folder);
}

/// Bob's clip at the end of the preview's #general, playing from a
/// worker's mapping, its row in view: the worker's side of it.
fn play(app: &mut App) -> Writer {
    drop(app.dispatch(Action::Open("general".into())));
    let frames = Arc::new(Frames::create(4, 2).expect("frames"));
    let writer = Writer::open(&frames.path().expect("its file")).expect("the worker's side");
    frames.unlink();
    let state = &mut app.core.state;
    state.preview = false;
    let messages = Arc::make_mut(&mut state.messages);
    let last = messages.last().map_or(0, |message| message.timestamp);
    let clip = Attachment {
        name: "clip.mp4".into(),
        size: None,
        image: false,
        dimensions: Some((480, 270)),
        preview: None,
        preview_failed: false,
        preview_percent: 0,
        placement: chat::Placement::Message,
        spoiler: false,
        video: true,
        gif: None,
        url: Some("https://cdn.example/clip.mp4".into()),
        description: None,
    };
    messages.push(Arc::new(ChatMessage {
        id: "clip".into(),
        item: "clip".into(),
        author: "Bob".into(),
        sender: "Bob".into(),
        timestamp: last + 60,
        attachments: vec![clip],
        ..ChatMessage::default()
    }));
    state.video = Some(chat::Video {
        room: "general".into(),
        item: "clip".into(),
        index: 0,
        frames: Some(frames),
        duration_ms: Some(6_000),
        size: Some((480, 270)),
        loading: false,
        playing: true,
        ended: false,
        sound: true,
        muted: false,
        volume: 1.0,
        error: None,
    });
    app.refresh();
    writer
}

/// Whether what matters keeps going: the timeline's day has its clock, the
/// clip's frames are taken (its worker isn't told to pause), and the
/// view-model says so; all three agree.
fn live(app: &App, writer: &Writer) -> bool {
    let followed = !writer.paused();
    assert_eq!(app.clocks.running()[1], followed, "the day's clock");
    assert_eq!(app.vm.chrome.live, followed, "the view-model and the clip");
    followed
}

fn sighted(app: &mut App, heard: Heard) {
    drop(app.update(Message::Sight(heard)));
}

/// The tests' desktop says nothing until told: unfocused, the window rests
/// as one behind a game would. Where the desktop says when nothing of the
/// window shows, unfocused on screen keeps what matters going; unpainted
/// or minimised, nothing goes, focused or not, until two frames close
/// together say it's painted again.
#[test]
fn what_keeps_going_follows_the_focus_and_whether_the_window_is_painted() {
    let mut window = start("window-sight", true);
    let app = &mut window.app;
    let writer = play(app);
    sighted(app, Heard::Detected(false));
    assert_eq!(app.motion.sight(), Sight::Front);
    assert!(live(app, &writer), "in front");
    assert!(app.motion.playing(), "and moving");

    drop(app.update(Message::Focused(false)));
    assert_eq!(app.motion.sight(), Sight::Visible);
    assert_eq!(app.motion.background, Background::Nothing);
    assert!(!live(app, &writer), "where a game may cover it unseen");

    sighted(app, Heard::Detected(true));
    assert_eq!(app.motion.background, Background::Important);
    assert!(live(app, &writer), "on screen, unfocused");
    assert!(!app.motion.playing(), "transitions land at once");

    sighted(app, Heard::Unseen);
    assert_eq!(app.motion.sight(), Sight::Hidden);
    assert!(!live(app, &writer), "covered");
    drop(app.update(Message::Focused(true)));
    assert!(app.motion.front, "the focus is the focus");
    assert!(!live(app, &writer), "focused, but covered");
    sighted(app, Heard::First);
    assert!(!live(app, &writer), "a configure's lone frame");
    sighted(app, Heard::Seen);
    assert_eq!(app.motion.sight(), Sight::Front);
    assert!(live(app, &writer), "painted again");

    // Minimised on Windows: a size of 0, which iced doesn't draw.
    drop(app.update(Message::Resized(Size::ZERO)));
    assert_eq!(app.motion.sight(), Sight::Hidden);
    assert!(!live(app, &writer), "minimised");
    drop(app.update(Message::Resized(SIZE)));
    assert!(live(app, &writer), "shown again");

    // In the tray nothing goes, whatever was heard.
    app.hidden = true;
    drop(super::sight(app));
    assert!(!app.vm.chrome.live && app.motion.unseen, "in the tray");
}
