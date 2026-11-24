//! Every state of a stream's tile at every size the strip and the grid lay
//! one out at: where each part goes by its room, none covering another nor
//! leaving the tile; at the smallest, the waiting ring in the chip.

use iced::{Point, Rectangle, Size};

use anomi_vm::fixtures::voice::tiles as samples;
use anomi_vm::voice::tiles::{Chip, Fill, Key, Tile};

use super::super::{INSET, PILL, chip, controls, live, viewers};
use super::{CLEAR, Middle, Room};

/// The smallest tile: the strip's least width, 16:9.
const SMALLEST: Size = Size::new(120.0, 67.5);

/// The widths a tile takes: the strip's, 120 to 192, the grid's, 160 to
/// 420, and a large one's.
const WIDTHS: [f32; 13] = [
    120.0, 132.0, 146.0, 160.0, 176.0, 192.0, 210.0, 240.0, 280.0, 320.0, 360.0, 420.0, 700.0,
];

/// Each state of a stream's tile, named.
fn states() -> Vec<(&'static str, Tile)> {
    let watched = samples::watching().swap_remove(1);
    let mut picture = samples::waiting();
    picture.fill = Fill::Watched {
        frame: None,
        status: "Waiting for MrTokin's picture...".into(),
        stuck: false,
    };
    let hidden = Tile {
        fill: Fill::OwnScreen {
            frame: None,
            status: "Your stream preview is hidden".into(),
        },
        ..samples::sharing()
    };
    let noted = Tile {
        badge: false,
        chip: Chip {
            note: "Starting your stream...".into(),
            ..samples::sharing().chip
        },
        ..samples::sharing()
    };
    vec![
        ("connecting", samples::waiting()),
        ("waiting for its picture", picture),
        (
            "watched",
            Tile {
                focused: false,
                ..watched.clone()
            },
        ),
        ("watched, large", watched.clone()),
        ("preview", samples::offered(true)),
        ("not watched", samples::offered(false)),
        ("own, live", samples::sharing()),
        ("own, preview hidden", hidden),
        ("own, starting", samples::sharing_starting()),
        ("own, picture before live", noted),
        ("stand-in", watched.stand_in()),
    ]
}

/// Where `room` puts each part of `tile` in a tile of `size`, at rest or
/// `hovered`: the chip as wide as it may be, the top row's parts from the
/// right as wide as they may be, the middle centred.
fn parts(tile: &Tile, size: Size, hovered: bool) -> Vec<(&'static str, Rectangle)> {
    let room = Room::of(tile, size);
    let inner = size.width - 2.0 * INSET;
    let at = |x, y, width, height| Rectangle::new(Point::new(x, y), Size::new(width, height));
    let mut parts = Vec::new();
    if room.chip {
        let top = size.height - INSET - chip::HEIGHT;
        parts.push(("chip", at(INSET, top, inner, chip::HEIGHT)));
    }
    let mut right = size.width - INSET;
    let mut row = |name, width: f32, height: f32, parts: &mut Vec<_>| {
        parts.push((name, at(right - width, INSET, width, height)));
        right -= width + controls::GAP;
    };
    if room.badge {
        row("badge", live::width(), live::HEIGHT, &mut parts);
    }
    let watched = matches!(tile.fill, Fill::Watched { .. });
    let button = (watched && (hovered || tile.focused)) || (hovered && tile.key == Key::OwnScreen);
    if room.button && button {
        row("button", controls::BUTTON, controls::BUTTON, &mut parts);
    }
    if room.viewers[usize::from(hovered)] {
        let width = viewers::width(&tile.viewers);
        row("viewers", width, controls::BUTTON, &mut parts);
    }
    if let Some(height) = room.middle.height() {
        let width = match room.middle {
            Middle::Glyph(side) => side,
            _ => inner,
        };
        let top = (size.height - height) / 2.0 - room.lift;
        parts.push(("middle", at((size.width - width) / 2.0, top, width, height)));
    }
    if room.starting {
        parts.push(("starting", at(INSET, INSET, inner / 2.0, PILL)));
    }
    parts
}

fn assert_apart(name: &str, tile: &Tile, size: Size) {
    for hovered in [false, true] {
        let parts = parts(tile, size, hovered);
        for (index, (one, a)) in parts.iter().enumerate() {
            let inside = a.x >= 0.0
                && a.y >= 0.0
                && a.x + a.width <= size.width
                && a.y + a.height <= size.height;
            assert!(
                inside,
                "{name} at {size:?}: the {one} leaves the tile: {a:?}"
            );
            for (other, b) in &parts[index + 1..] {
                assert!(
                    !a.intersects(b),
                    "{name} at {size:?}, hovered {hovered}: the {one} covers the {other}: {a:?} {b:?}"
                );
            }
        }
    }
}

#[test]
fn no_part_covers_another_at_the_smallest_size() {
    for (name, tile) in states() {
        assert_apart(name, &tile, SMALLEST);
        let room = Room::of(&tile, SMALLEST);
        assert!(room.chip, "{name}: its name stays");
        assert!(
            !matches!(room.middle, Middle::Full(_)),
            "{name}: no words in the middle of the smallest tile"
        );
    }
}

#[test]
fn no_part_covers_another_at_any_size() {
    for width in WIDTHS {
        let size = Size::new(width, width * 0.5625);
        for (name, tile) in states() {
            assert_apart(name, &tile, size);
        }
    }
}

#[test]
fn a_stream_on_its_way_shows_its_ring_alone_where_its_words_have_no_room() {
    let waiting = samples::waiting();
    let middle = |width: f32| Room::of(&waiting, Size::new(width, width * 0.5625)).middle;
    assert_eq!(middle(120.0), Middle::InChip, "in the chip, the smallest");
    assert!(
        matches!(middle(192.0), Middle::Glyph(_)),
        "{:?}",
        middle(192.0)
    );
    assert!(
        matches!(middle(420.0), Middle::Full(_)),
        "with its note, large"
    );
    // Large, the badge, the viewers, the ring and its note all show.
    let room = Room::of(&waiting, Size::new(420.0, 236.25));
    assert!(room.badge && room.viewers == [true, true] && room.button);
}

#[test]
fn the_badge_and_the_chip_stay_where_the_viewers_go() {
    let sharing = samples::sharing();
    let small = Room::of(&sharing, SMALLEST);
    assert!(small.badge && small.chip);
    assert_eq!(small.viewers, [false, false], "no room above the chip");
    assert!(!small.button);
    let large = Room::of(&sharing, Size::new(420.0, 236.25));
    assert_eq!(large.viewers, [true, true]);
    assert!(large.button, "the gear under the pointer");
}

/// What a tile has room for depends on its size alone: the same at the
/// same size, from the strip's least width to a monitor's, at 16:9 and at
/// the strip's height, and a hundredth of a pixel either side of each
/// height where a part comes or goes; a camera's tile too. A tile laid out
/// again at its size is the same tile, so a frame that redraws it lays
/// nothing out.
#[test]
fn a_tiles_room_is_the_same_at_the_same_size() {
    let mut tiles = states();
    tiles.push(("camera", samples::with_camera().swap_remove(2)));
    let strip = 85.5;
    let thresholds = [
        2.0 * INSET + chip::HEIGHT,
        2.0 * INSET + live::HEIGHT + CLEAR + chip::HEIGHT,
        2.0 * INSET + controls::BUTTON + CLEAR + chip::HEIGHT,
    ];
    let steady = |name: &str, tile: &Tile, size: Size| {
        let (laid, again) = (Room::of(tile, size), Room::of(tile, size));
        assert_eq!(laid, again, "{name} at {size:?}");
    };
    for &(name, ref tile) in &tiles {
        for quarter in 480..=7680 {
            let width = quarter as f32 / 4.0;
            steady(name, tile, Size::new(width, width * 0.5625));
            steady(name, tile, Size::new(width, strip));
        }
        for height in thresholds {
            for width in (120..=1920).step_by(8) {
                for offset in [-0.01, 0.0, 0.01] {
                    steady(name, tile, Size::new(width as f32, height + offset));
                }
            }
        }
    }
}

#[test]
fn the_strips_tiles_keep_their_screen_and_ring_between_the_badge_and_the_chip() {
    // Six tiles beneath a large one in a window 1400 px wide: too low for
    // the screen centred clear of the chip, which Discord's strip shows.
    let strip = Size::new(152.0, 85.5);
    for (name, tile) in states() {
        let room = Room::of(&tile, strip);
        match &tile.fill {
            Fill::Offer { .. } | Fill::Large | Fill::OwnScreen { frame: None, .. } => {
                assert!(matches!(room.middle, Middle::Glyph(_)), "{name}");
                assert!(room.lift > 0.0, "{name}: raised clear of the chip");
            }
            Fill::Watched { frame: None, .. } => {
                assert_eq!(room.middle, Middle::Glyph(20.0), "{name}: its ring");
            }
            _ => assert_eq!(room.middle, Middle::Empty, "{name}"),
        }
        assert_apart(name, &tile, strip);
    }
}
