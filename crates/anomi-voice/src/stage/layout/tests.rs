//! Where the grid puts its tiles, held to the goldens' rooms: V04's, and
//! a large stream's stand-in in the rows beneath it.

use super::*;

/// The goldens' room for the grid: the conversation beside a 376 px
/// sidebar, between the call's line (85.5 px down) and the pill's 80.
const GOLDEN: Size = Size::new(1024.0, 694.5);
const LEFT: f32 = 376.0;
const TOP: f32 = 85.5;

fn grid(count: usize, focused: Option<usize>) -> Geometry {
    let arrangement = Arrangement {
        count,
        focused,
        ..Arrangement::default()
    };
    Geometry::new(arrangement, GOLDEN)
}

/// The tile at `slot` in the window, as V03's tests measured the
/// goldens: left, top, width, height.
fn window(geometry: &Geometry, slot: usize) -> (f32, f32, f32, f32) {
    let rect = geometry.rect(slot);
    (rect.x + LEFT, rect.y + TOP, rect.width, rect.height)
}

#[test]
fn four_sit_two_by_two() {
    // `voice/stage.png`.
    let four = grid(4, None);
    assert_eq!(window(&four, 0), (462.0, 190.5, 420.0, 236.25));
    assert_eq!(window(&four, 3), (894.0, 438.75, 420.0, 236.25));
    assert_eq!(four.content, GOLDEN.height, "nothing to scroll");
}

#[test]
fn a_short_last_row_is_centred() {
    // `light/voice_stage.png`: two over one, not three in a row.
    let three = grid(3, None);
    assert_eq!(window(&three, 1).0, 894.0);
    assert_eq!(window(&three, 2), (678.0, 438.75, 420.0, 236.25));
}

#[test]
fn a_large_tile_has_the_others_in_a_row_beneath() {
    // `voice/watching_stream.png`: Bob's tile, the second, large.
    let watching = grid(4, Some(1));
    assert_eq!(window(&watching, 1), (420.0, 109.5, 936.0, 526.5));
    assert_eq!(window(&watching, 0), (588.0, 648.0, 192.0, 108.0));
    assert_eq!(window(&watching, 2).0, 792.0, "the rows skip it");
    assert_eq!(window(&watching, 3).0, 996.0);
    // `voice/with_camera.png`: four beneath.
    let camera = grid(5, Some(0));
    assert_eq!(window(&camera, 1).0, 486.0);
    assert_eq!(window(&camera, 4).0, 1098.0);
}

#[test]
fn a_short_call_fits_its_room() {
    // `small/voice_stage.png`: 624 by 475 keeps two columns.
    let small = Geometry::new(
        Arrangement {
            count: 4,
            ..Arrangement::default()
        },
        Size::new(624.0, 475.0),
    );
    assert_eq!(small.rect(0).width, 282.0);
    // `dmcall/connected_bob_dm.png`: two side by side above the messages.
    let dm = Geometry::new(
        Arrangement {
            count: 2,
            ..Arrangement::default()
        },
        Size::new(704.0, 359.0),
    );
    assert_eq!((dm.rect(0).width, dm.rect(1).y), (322.0, dm.rect(0).y));
    // One alone is as large as fits, at most 420 px.
    assert_eq!(grid(1, None).rect(0).width, 420.0);
}

#[test]
fn many_take_columns_of_at_least_240_and_scroll() {
    // Three of 317 px fit the 976 px between the insets; a fourth of
    // 240 doesn't.
    let ten = grid(10, None);
    let rect = ten.rect(0);
    assert_eq!(ten.rect(2).y, rect.y, "three to a row");
    assert_eq!(ten.rect(3).y, rect.y + rect.height + GAP);
    assert!(rect.width >= CROWDED && rect.width <= WIDEST);
    assert_eq!(window(&ten, 9).0, 376.0 + (1024.0 - rect.width) / 2.0);
    assert!(ten.content > GOLDEN.height, "four rows scroll");
    assert_eq!(rect.y, INSET);
}

#[test]
fn full_screen_shows_the_large_tile_alone_edge_to_edge() {
    let arrangement = Arrangement {
        count: 4,
        focused: Some(1),
        full: true,
        others_hidden: false,
        stand_in: false,
    };
    let full = Geometry::new(arrangement, GOLDEN);
    assert_eq!(full.rect(1), Rectangle::new(Point::ORIGIN, GOLDEN));
    assert!(arrangement.shown(1) && !arrangement.shown(0));
    let placement = full.placement(0.0);
    assert_eq!((placement.columns, placement.focused), (0, Some(1)));
    assert_eq!(placement.shown_at(0, 1.0), None);
}

#[test]
fn hidden_others_give_the_large_tile_their_room() {
    let arrangement = Arrangement {
        count: 4,
        focused: Some(1),
        full: false,
        others_hidden: true,
        stand_in: false,
    };
    let alone = Geometry::new(arrangement, GOLDEN);
    assert!(alone.rect(1).height > grid(4, Some(1)).rect(1).height);
    assert!(!arrangement.shown(0) && arrangement.shown(1));
    assert_eq!(alone.placement(0.0).columns, 0);
}

#[test]
fn the_placement_is_what_the_app_reads() {
    let four = grid(4, None).placement(12.0);
    assert_eq!((four.columns, four.width, four.gap), (2, 420.0, GAP));
    assert_eq!((four.top, four.scrolled, four.shown), (105.0, 12.0, 694.5));
    assert_eq!((four.focused, four.focus_width), (None, 0.0));
    let large = grid(4, Some(1)).placement(0.0);
    assert_eq!((large.columns, large.focus_height), (3, 526.5));
    assert_eq!(large.shown_at(1, 2.0), Some((1872, 1053)));
    assert_eq!(large.shown_at(3, 1.0), Some((192, 108)));
    assert_eq!(grid(0, None).placement(0.0).focused, None);
}

/// Whether two tiles' places share any area.
fn overlap(one: Rectangle, other: Rectangle) -> bool {
    one.x < other.x + other.width
        && other.x < one.x + one.width
        && one.y < other.y + other.height
        && other.y < one.y + one.height
}

/// The grid of `count` with `focused` large, a stream with its stand-in
/// beneath it while `stand_in`, in a room of `size`.
fn streaming(count: usize, focused: Option<usize>, stand_in: bool, size: Size) -> Geometry {
    let arrangement = Arrangement {
        count,
        focused,
        stand_in,
        ..Arrangement::default()
    };
    Geometry::new(arrangement, size)
}

#[test]
fn a_large_streams_stand_in_keeps_its_place_in_the_rows() {
    // Alice, Bob's stream large, Bob, Carol, Dave: five beneath it, the
    // stand-in second, all in one row of 185.6 px.
    let watching = streaming(5, Some(1), true, GOLDEN);
    let (alice, stand_in, bob) = (watching.rect(0), watching.rect(STAND_IN), watching.rect(2));
    assert!((alice.width - 185.6).abs() < 0.01, "{alice:?}");
    assert!(alice.x < stand_in.x && stand_in.x < bob.x);
    assert!((stand_in.x - alice.x - (185.6 + GAP)).abs() < 0.01);
    assert_eq!((stand_in.y, bob.y), (alice.y, alice.y), "one row");
    assert_eq!(watching.rect(1), watching.focus);
    assert!(watching.rect(4).x + watching.rect(4).width <= GOLDEN.width - INSET + 0.01);
    let placement = watching.placement(0.0);
    assert!(placement.stand_in);
    assert_eq!(
        placement.shown_at(2, 1.0),
        Some((186, 104)),
        "Bob in its row"
    );
    // Without the stand-in the rows close up after the large tile.
    let person = streaming(5, Some(1), false, GOLDEN);
    assert_eq!(
        person.rect(2).x,
        person.rect(0).x + person.rect(0).width + GAP
    );
    assert_eq!(person.rect(STAND_IN), Rectangle::default());
    // Full screen, or the others hidden, leave it out.
    let full = Arrangement {
        full: true,
        ..Arrangement {
            count: 5,
            focused: Some(1),
            stand_in: true,
            ..Arrangement::default()
        }
    };
    assert_eq!(
        Geometry::new(full, GOLDEN).rect(STAND_IN),
        Rectangle::default()
    );
}

/// Six people, three of them sharing a screen while two film: nine tiles.
/// Over a window 1000 by 640, full screen's room, they sit three to a row
/// with nothing to scroll; in its conversation, about 640 by 460 beside
/// the sidebar, two to a row, scrolling. Carol's stream large, the eight
/// others and its stand-in lie in rows beneath it. None lies over another,
/// or past the room's sides.
#[test]
fn several_streams_and_cameras_fit_a_small_window() {
    for room in [Size::new(1000.0, 640.0), Size::new(640.0, 460.0)] {
        let grid = streaming(9, None, false, room);
        let all: Vec<Rectangle> = (0..9).map(|slot| grid.rect(slot)).collect();
        for (index, rect) in all.iter().enumerate() {
            assert!(rect.x >= INSET - 0.01 && rect.x + rect.width <= room.width - INSET + 0.01);
            assert!(rect.width >= CROWDED, "{rect:?} in {room:?}");
            for other in &all[index + 1..] {
                assert!(!overlap(*rect, *other), "{rect:?} {other:?}");
            }
        }
        let scrolls = grid.content > room.height + 0.01;
        assert_eq!(scrolls, room.width < 1000.0, "{room:?}");

        let large = streaming(9, Some(4), true, room);
        let mut beneath: Vec<Rectangle> = (0..9)
            .filter(|slot| *slot != 4)
            .map(|slot| large.rect(slot))
            .collect();
        beneath.push(large.rect(STAND_IN));
        for (index, rect) in beneath.iter().enumerate() {
            assert!(
                !overlap(*rect, large.focus),
                "{rect:?} under {:?}",
                large.focus
            );
            assert!(rect.y >= large.focus.y + large.focus.height);
            assert!(rect.x >= 0.0 && rect.x + rect.width <= room.width + 0.01);
            assert!(rect.width >= STRIP_LEAST - 0.01);
            for other in &beneath[index + 1..] {
                assert!(!overlap(*rect, *other), "{rect:?} {other:?}");
            }
        }
        assert!(large.focus.height >= NARROWEST * RATIO);
        assert!(
            large.content <= room.height + 0.01,
            "the rows fit beneath it"
        );
    }
}
