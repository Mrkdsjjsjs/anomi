//! A stream is a tile of its own, as in Discord's call: right before its
//! streamer's, who keeps their picture or camera; one's own screen right
//! before one's own tile; each shows large what it is, and a stream large
//! keeps its stand-in beneath it.

use std::sync::Arc;

use yapfast_core::frames::Frames;
use yapfast_core::voice::{SharePhase, Sharing, Watching};

use anomi_vm::voice::tiles::{self, Fill, Key, Mark};
use anomi_vm::voice::{Intent, stage};

use super::lives::Lives;
use super::tests::in_call;
use crate::app::App;
use crate::project::Reach;

fn keys(app: &App) -> Vec<Key> {
    (app.vm.voice.stage.tiles.iter())
        .map(|tile| tile.key.clone())
        .collect()
}

fn stream(id: &str) -> Key {
    Key::Stream(id.into())
}

fn person(id: &str) -> Key {
    Key::Person(id.into())
}

/// Bob shares his screen and films; Carol shares hers alone.
fn streaming(app: &mut App) {
    let mut people = app.core.state.voice.participants.to_vec();
    people[1].streaming = true;
    people[1].video = Some(yapfast_core::picture::Feed::default());
    people[2].streaming = true;
    app.core.state.voice.participants = people.into();
    app.show(Reach::VOICE);
}

#[test]
fn someone_streaming_has_two_tiles_their_streams_first() {
    let (mut app, folder) = in_call("two-tiles");
    streaming(&mut app);
    assert_eq!(
        keys(&app),
        [
            person("alice"),
            stream("bob"),
            person("bob"),
            stream("carol"),
            person("carol"),
        ]
    );
    let tiles = &app.vm.voice.stage.tiles;
    // The stream's tile: offered, the screen on its chip, LIVE at its top
    // right.
    let bobs = &tiles[1];
    assert!(bobs.watch && bobs.badge && !bobs.chip.live);
    assert_eq!(
        (bobs.chip.name.as_str(), bobs.chip.mark),
        ("Bob", Some(Mark::Screen))
    );
    assert!(matches!(bobs.fill, Fill::Offer { preview: None }));
    // His own: his picture until his camera's first frame, no badge, no
    // offer; so is Carol's, who shares without a camera.
    for own in [&tiles[2], &tiles[4]] {
        assert!(!own.badge && !own.watch && !own.chip.live, "{own:?}");
        assert!(matches!(own.fill, Fill::Avatar { .. }));
    }
    drop(app);
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn each_tile_shows_large_what_it_is_and_a_stream_keeps_its_stand_in() {
    let (mut app, folder) = in_call("focus-tiles");
    streaming(&mut app);
    let ask = |app: &mut App, key: Key| {
        let intent = Intent::Tiles(tiles::Intent::Focus(key));
        drop(crate::intent::voice::handle(app, intent));
    };
    ask(&mut app, person("bob"));
    let stage = &app.vm.voice.stage;
    assert_eq!(stage.focused(), Some(2), "his own tile, not his stream");
    assert!(stage.stand_in.is_none(), "a person leaves the row");
    ask(&mut app, stream("carol"));
    let stage = &app.vm.voice.stage;
    assert_eq!(stage.focused(), Some(3));
    let stand_in = stage.stand_in.as_ref().expect("its stand-in beneath");
    assert_eq!(
        (&stand_in.key, &stand_in.fill),
        (&stream("carol"), &Fill::Large)
    );
    // Watching Bob's stream shows it large in its own tile, his camera's
    // tile staying beside it.
    app.core.state.voice.watching = Some(Box::new(Watching {
        user: "bob".into(),
        name: "Bob".into(),
        opening: true,
        frames: None,
        viewers: vec!["alice".into()],
    }));
    app.show(Reach::VOICE);
    let stage = &app.vm.voice.stage;
    assert_eq!(stage.focused(), Some(1));
    assert!(stage.tiles[1].watched() && !stage.tiles[2].watched());
    assert_eq!(stage.tiles[1].viewers.count, 1);
    assert_eq!(stage.tiles[2].key, person("bob"));
    // The corner, off screen, shows the stream watched; a click there shows
    // its tile large.
    let corner = app.shown.voice.pip.expect("the corner");
    assert_eq!(corner, (super::Pip::Watched, Some(1)));
    // Where the grid says the large one is, its stand-in holds its place.
    let placed = stage::Placement {
        columns: 5,
        width: 150.0,
        height: 84.0,
        shown: 600.0,
        top: 400.0,
        focused: Some(1),
        focus_width: 640.0,
        focus_height: 360.0,
        stand_in: true,
        ..Default::default()
    };
    drop(crate::intent::voice::handle(
        &mut app,
        Intent::Stage(stage::Intent::Placed(placed)),
    ));
    assert_eq!(app.shown.voice.placement, Some(placed));
    drop(app);
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn this_accounts_screen_is_a_tile_right_before_its_own() {
    let (mut app, folder) = in_call("own-screen");
    let frames = yapfast_core::frames::Frames::create(4, 2).expect("frames");
    app.core.state.voice.sharing = Some(Sharing {
        phase: SharePhase::Starting,
        frames: Arc::new(frames),
        viewers: Vec::new(),
    });
    let mut people = app.core.state.voice.participants.to_vec();
    people[0].streaming = true;
    app.core.state.voice.participants = people.into();
    app.show(Reach::VOICE);
    assert_eq!(
        keys(&app),
        [
            Key::OwnScreen,
            person("alice"),
            person("bob"),
            person("carol")
        ]
    );
    let tiles = &app.vm.voice.stage.tiles;
    assert!(!tiles[0].badge, "not live yet");
    assert!(
        !tiles[1].chip.live && !tiles[1].badge,
        "her own tile says nothing of it"
    );
    if let Some(sharing) = app.core.state.voice.sharing.as_mut() {
        sharing.phase = SharePhase::Live;
    }
    app.show(Reach::VOICE);
    let own = &app.vm.voice.stage.tiles[0];
    assert!(own.badge && !own.chip.live, "LIVE at its top right");
    assert_eq!(own.chip.name, "Your screen");
    drop(app);
    let _ = std::fs::remove_dir_all(folder);
}

/// A stream Discord moves to another server comes in new frames, from a
/// new worker: its `Live`, and the picture in it, stay until that worker's
/// first frame, rather than "Connecting…". Someone else's stream gets a
/// `Live` of its own.
#[test]
fn a_moved_stream_keeps_its_picture() {
    let (mut app, folder) = in_call("moved-stream");
    let watch = |user: &str, frames: &Arc<Frames>| {
        Some(Box::new(Watching {
            user: user.into(),
            name: user.into(),
            opening: false,
            frames: Some(frames.clone()),
            viewers: vec!["alice".into()],
        }))
    };
    let frames = || Arc::new(Frames::create(4, 2).expect("frames"));
    let mut lives = Lives::default();
    app.core.state.voice.watching = watch("bob", &frames());
    lives.follow(&app.core.state.voice);
    let live = lives.watched.as_ref().expect("bob's stream").1.clone();
    live.show(iced::widget::image::Handle::from_rgba(
        1,
        1,
        vec![1, 2, 3, 255],
    ));

    let moved = frames();
    app.core.state.voice.watching = watch("bob", &moved);
    lives.follow(&app.core.state.voice);
    let (shown, kept, _) = lives.watched.as_ref().expect("still bob's");
    assert!(Arc::ptr_eq(shown, &moved), "the new worker's frames");
    assert!(
        *kept == live && kept.has_frame(),
        "the same Live, its picture kept"
    );

    app.core.state.voice.watching = watch("carol", &frames());
    lives.follow(&app.core.state.voice);
    let carols = &lives.watched.as_ref().expect("carol's stream").1;
    assert!(*carols != live && !carols.has_frame(), "a Live of its own");
    drop(app);
    let _ = std::fs::remove_dir_all(folder);
}
