//! The stage's samples: which tile is large, what stands for a large
//! stream beneath it, and the lists beside them as the goldens have them.

use super::*;

#[test]
fn each_sample_says_which_tile_is_large() {
    assert_eq!(lounge().focused(), None);
    assert_eq!(watching().focused(), Some(1));
    assert_eq!(with_camera().focused(), Some(1));
    assert!(full().full() && !others_hidden().full());
    assert!(others_hidden().others_hidden());
    assert_eq!(crowd(10).len(), 10);
    assert_eq!(crowd(3).len(), 3);
    assert_eq!(
        people_in(&with_camera()),
        4,
        "a stream's own tile isn't a person"
    );
    assert_eq!(people_in(&sharing()), 4, "nor this account's screen");
}

#[test]
fn the_lists_show_the_call_as_the_goldens_do() {
    let vm = dm_window(dm_call());
    let Place::Dms(dms) = &vm.sidebar.place else {
        panic!("the DMs listed");
    };
    let chosen: Vec<_> = dms.rows.iter().filter(|row| row.selected).collect();
    assert_eq!(chosen.len(), 1);
    assert_eq!((chosen[0].name.as_str(), chosen[0].call), ("Bob", true));

    let bob = |mut vm: Vm| {
        let mut found = None;
        in_lounge(&mut vm, |people| {
            found = people
                .iter()
                .find(|person| person.name == "Bob")
                .map(|bob| (bob.speaking, bob.video));
        });
        found
    };
    assert_eq!(bob(speaking_window()), Some((true, false)));
    assert_eq!(bob(camera_window()), Some((false, true)));
    assert_eq!(bob(quiet_window()), None);

    let mut names = Vec::new();
    in_lounge(&mut alone_window(), |people| {
        names = people.iter().map(|person| person.name.clone()).collect();
    });
    assert_eq!(names, ["Alice"], "alone in the list too");

    let mut vm = sharing_window();
    let mut alice_live = false;
    in_lounge(&mut vm, |people| {
        alice_live = people
            .iter()
            .any(|person| person.name == "Alice" && person.live);
    });
    assert!(alice_live && vm.voice.dock.sharing && vm.voice.controls.sharing);

    let Header::Open(place) = &dm_window(dm_call()).header else {
        panic!("Bob's header");
    };
    assert!(place.tools.call.is_none(), "no call button while in it");
}

#[test]
fn a_stream_has_its_own_tile_right_before_its_streamers() {
    use crate::voice::tiles::Fill;
    let keys =
        |stage: &Stage| -> Vec<Key> { stage.tiles.iter().map(|tile| tile.key.clone()).collect() };
    let bob = || Key::Person(BOB.into());
    let stream = Key::Stream(BOB.into());
    let lounge = keys(&lounge());
    assert_eq!(lounge[1..3], [stream.clone(), bob()]);
    let streamer = streamer();
    assert!(matches!(streamer.tiles[1].fill, Fill::Offer { .. }));
    assert!(
        matches!(streamer.tiles[2].fill, Fill::Camera(_)),
        "his camera"
    );
    assert!(streamer.tiles[1].badge && !streamer.tiles[2].badge);
    let alice = Key::Person(ALICE.into());
    assert_eq!(keys(&sharing())[..2], [Key::OwnScreen, alice]);
    // Large, a stream keeps a stand-in beneath it; a person doesn't.
    let watching = watching();
    let stand_in = watching.stand_in.as_ref().expect("the stream's stand-in");
    assert_eq!(
        (stand_in.key.clone(), &stand_in.fill),
        (stream, &Fill::Large)
    );
    assert!(with_camera().stand_in.is_some() && sharing_large().stand_in.is_some());
    assert!(full().stand_in.is_none() && others_hidden().stand_in.is_none());
    assert!(lounge_of(bob()).stand_in.is_none());
}

/// The Lounge with `key`'s tile large.
fn lounge_of(key: Key) -> Stage {
    let tiles = lounge().tiles;
    let slot = tiles.iter().position(|tile| tile.key == key);
    Stage::new(tiles, slot, false, false)
}
