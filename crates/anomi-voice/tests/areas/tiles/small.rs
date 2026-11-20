//! Each state of a stream's tile at the smallest size the strip beneath a
//! large tile lays one out at, 120 by 67.5, drawn in the simulator, at rest
//! and under the pointer: its name's chip, its LIVE badge and who watches
//! never cover one another nor leave the tile, and nothing is said in its
//! middle, a stream on its way turning its ring in the chip.

use iced::{Event, Point, Rectangle, Size, mouse};
use iced_test::Simulator;
use iced_test::selector::Candidate;

use anomi_theme::{Renderer, Theme};
use anomi_vm::fixtures::voice::tiles as samples;
use anomi_vm::voice::tiles::{Fill, Intent, Tile};
use anomi_voice::tiles;

use super::super::support::{front, settings};

const SMALLEST: Size = Size::new(120.0, 67.5);

/// What a stream's tile may say in its middle, none of which has room here.
const WORDS: [&str; 6] = [
    "Connecting to Bob's stream...",
    "Waiting for Bob's picture...",
    "Your stream preview is hidden",
    "Starting your stream...",
    "Live",
    "Watch stream",
];

type Ui<'a> = Simulator<'a, Intent, Theme, Renderer>;

/// Each state, named, with the name its chip says.
fn states() -> Vec<(&'static str, Tile, &'static str)> {
    let watched = samples::watching().swap_remove(1);
    let mut picture = samples::waiting();
    picture.fill = Fill::Watched {
        frame: None,
        status: "Waiting for Bob's picture...".into(),
    };
    let hidden = Tile {
        fill: Fill::OwnScreen {
            frame: None,
            status: "Your stream preview is hidden".into(),
        },
        ..samples::sharing()
    };
    let unfocused = Tile {
        focused: false,
        ..watched.clone()
    };
    vec![
        ("connecting", samples::waiting(), "Bob"),
        ("waiting for its picture", picture, "Bob"),
        ("watched", unfocused, "Bob"),
        ("preview", samples::offered(true), "Bob"),
        ("not watched", samples::offered(false), "Bob"),
        ("own, live", samples::sharing(), "Your screen"),
        ("own, preview hidden", hidden, "Your screen"),
        ("own, starting", samples::sharing_starting(), "Your screen"),
        ("stand-in", watched.stand_in(), "Bob"),
    ]
}

fn found(ui: &mut Ui<'_>, text: &str) -> Option<Rectangle> {
    ui.find(text).ok().map(|target| target.bounds())
}

/// The container `height` tall that holds `inside`'s middle: a chip, a
/// badge, the viewers' stack.
fn holding(ui: &mut Ui<'_>, inside: Rectangle, height: f32) -> Rectangle {
    let middle = inside.center();
    ui.find(move |candidate: Candidate<'_>| match candidate {
        Candidate::Container { bounds, .. }
            if bounds.contains(middle) && (bounds.height - height).abs() < 0.5 =>
        {
            Some(bounds)
        }
        _ => None,
    })
    .unwrap_or(inside)
}

/// The parts shown: the chip, the badge, the viewers' stack, and any words.
fn parts(ui: &mut Ui<'_>, name: &str) -> Vec<(&'static str, Rectangle)> {
    let mut parts = Vec::new();
    let chip = found(ui, name).unwrap_or_else(|| panic!("the name {name}"));
    parts.push(("chip", holding(ui, chip, 26.0)));
    if let Some(live) = found(ui, "LIVE") {
        parts.push(("badge", holding(ui, live, 16.0)));
    }
    for count in ["1", "5", "+2"] {
        if let Some(count) = found(ui, count) {
            parts.push(("viewers", holding(ui, count, 30.0)));
        }
    }
    for words in WORDS {
        if let Some(said) = found(ui, words) {
            parts.push(("words", said));
        }
    }
    parts
}

#[test]
fn no_part_of_a_small_stream_tile_covers_another() {
    for (state, tile, name) in states() {
        for hovered in [false, true] {
            let mut ui = Simulator::with_size(settings(), SMALLEST, tiles::view(&tile, front()));
            if hovered {
                let at = Point::new(60.0, 30.0);
                ui.point_at(at);
                ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position: at })]);
            }
            let parts = parts(&mut ui, name);
            assert!(
                parts.iter().all(|(part, _)| *part != "words"),
                "{state}: nothing said in the middle: {parts:?}"
            );
            for (index, (one, a)) in parts.iter().enumerate() {
                let inside = a.x >= -0.5
                    && a.y >= -0.5
                    && a.x + a.width <= SMALLEST.width + 0.5
                    && a.y + a.height <= SMALLEST.height + 0.5;
                assert!(inside, "{state}: the {one} leaves the tile: {a:?}");
                for (other, b) in &parts[index + 1..] {
                    assert!(
                        !a.intersects(b),
                        "{state}, hovered {hovered}: the {one} covers the {other}: {a:?} {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_small_stream_tile_keeps_its_name_and_its_badge() {
    for (state, tile, name) in states() {
        let mut ui = Simulator::with_size(settings(), SMALLEST, tiles::view(&tile, front()));
        assert!(found(&mut ui, name).is_some(), "{state}: its name");
        assert_eq!(
            found(&mut ui, "LIVE").is_some(),
            tile.badge,
            "{state}: its badge"
        );
    }
}

/// The first container laid out past the edges of a tile of `size`.
fn outside(ui: &mut Ui<'_>, size: Size) -> Option<Rectangle> {
    ui.find(move |candidate: Candidate<'_>| match candidate {
        Candidate::Container { bounds, .. }
            if bounds.x < -0.5
                || bounds.y < -0.5
                || bounds.x + bounds.width > size.width + 0.5
                || bounds.y + bounds.height > size.height + 0.5 =>
        {
            Some(bounds)
        }
        _ => None,
    })
    .ok()
}

#[test]
fn a_preview_coming_fills_its_tile_whatever_it_showed_before() {
    // As when this account stops watching: the stream's tile shows its
    // screen on black, then its preview once that's fetched.
    let offered = samples::offered(true);
    for size in [Size::new(152.0, 85.5), Size::new(420.0, 236.25)] {
        for (state, before) in [
            ("no preview yet", samples::offered(false)),
            ("connecting", samples::waiting()),
        ] {
            let ui = Simulator::with_size(settings(), size, tiles::view(&before, front()));
            let mut ui = ui.rebuild(tiles::view(&offered, front()));
            assert_eq!(
                outside(&mut ui, size),
                None,
                "{state} at {size:?}: the preview leaves its tile"
            );
        }
    }
}
