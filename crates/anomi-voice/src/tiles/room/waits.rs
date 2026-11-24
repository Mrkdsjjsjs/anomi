//! A stream's tile while its picture is on its way draws one ring, in the
//! middle of the tile above what it waits for, at the stage's size after
//! the strip's and after the watch started over, its tile offered between.
//! The ring turns while the stream's picture would play, drawn turned in
//! its place, and is the still clock where nothing moves. Before, the
//! software renderer drew a turned picture unturned, smaller and nearer
//! the window's top left by the turn's cosine: the ring stood still and,
//! as the damage showed it, a second one strayed towards the top left.

use std::time::{Duration, Instant};

use iced::advanced::layout::{Layout, Limits};
use iced::advanced::renderer::{self, Headless as _, Settings};
use iced::advanced::shell::{Bus, Waker};
use iced::advanced::widget::{Tree, Widget as _, tree::Tag};
use iced::advanced::{Shell, mouse};
use iced::{Color, Event, Point, Rectangle, Size, Vector, window};

use anomi_theme::prelude::*;
use anomi_theme::{Renderer, fonts, renderer::software, typography};
use anomi_vm::fixtures::voice::tiles as samples;
use anomi_vm::voice::tiles::{Fill, Intent, Tile};

use super::super::{spinner, view};

/// The strip's tile beneath a large one, and the stage's large tile.
const SMALL: Size = Size::new(185.6, 104.4);
const LARGE: Size = Size::new(942.4, 530.1);

fn waiting(status: &str) -> Tile {
    Tile {
        fill: Fill::Watched {
            frame: None,
            status: status.into(),
            stuck: false,
        },
        ..samples::waiting()
    }
}

/// Each ring's bounds in the tile, depth first: a leaf of the ring's tag,
/// as a container takes its content's.
fn rings(tree: &Tree, at: Vector, all: &mut Vec<Rectangle>) {
    let at = at + tree.translation;
    if tree.tag == Tag::of::<spinner::State>() && tree.children.is_empty() {
        all.push(Rectangle::new(Point::ORIGIN + at, tree.size));
    }
    for child in &tree.children {
        rings(child, at, all);
    }
}

/// `tile` laid out at `size` on `tree`, kept from the frames before as the
/// stage keeps a tile's, and drawn at `now` with `motion`: its pixels,
/// RGBA at 1x, and its rings.
fn frame(
    tree: &mut Tree,
    tile: &Tile,
    size: Size,
    motion: Motion,
    now: Instant,
) -> (Vec<u8>, Vec<Rectangle>) {
    fonts::install();
    let mut renderer: Renderer = software(Settings {
        font: typography::UI,
        ..Settings::default()
    });
    let mut root: Element<'_, Intent> = view(tile, motion).boxed();
    tree.diff(&mut root);
    root.layout(tree, &renderer, &Limits::new(Size::ZERO, size));
    let window = Rectangle::with_size(size);
    let mut bus = Bus::<Intent>::new();
    let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
    root.update(
        tree,
        &Event::Window(window::Event::RedrawRequested(now)),
        Layout::new(tree.size),
        mouse::Cursor::Unavailable,
        &renderer,
        &mut shell,
        &window,
    );
    drop(shell);
    root.layout(tree, &renderer, &Limits::new(Size::ZERO, size));
    root.draw(
        tree,
        &mut renderer,
        &Theme::built_in(None),
        &renderer::Style {
            text_color: Color::WHITE,
        },
        Layout::new(tree.size),
        mouse::Cursor::Unavailable,
        &window,
    );
    let physical = Size::new(size.width.ceil() as u32, size.height.ceil() as u32);
    let pixels = renderer.screenshot(physical, 1.0, Color::BLACK);
    let mut found = Vec::new();
    rings(tree, Vector::ZERO, &mut found);
    (pixels, found)
}

/// The lit pixels of a band across the tile from below its top row to
/// just under `ring`, where nothing but the ring is: their places.
fn lit_about(pixels: &[u8], width: u32, ring: Rectangle) -> Vec<(f32, f32)> {
    let (top, bottom) = (60, (ring.y + ring.height + 3.0) as usize);
    pixels
        .chunks(4)
        .enumerate()
        .map(|(at, pixel)| ((at % width as usize), (at / width as usize), pixel))
        .filter(|&(_, y, pixel)| {
            (top..bottom).contains(&y) && pixel[..3].iter().any(|&channel| channel > 64)
        })
        .map(|(x, y, _)| (x as f32, y as f32))
        .collect()
}

/// The tile's one ring, centred across it and above its middle.
fn one_ring(rings: &[Rectangle]) -> Rectangle {
    assert_eq!(rings.len(), 1, "one ring: {rings:?}");
    let ring = rings[0];
    assert!(
        (ring.center_x() - LARGE.width / 2.0).abs() < 0.5,
        "centred: {ring:?}"
    );
    assert!(
        ring.y + ring.height < LARGE.height / 2.0,
        "above the middle: {ring:?}"
    );
    assert_eq!(ring.size(), Size::new(28.0, 28.0));
    ring
}

#[test]
fn a_tile_on_its_way_draws_one_ring_in_its_place_turning_while_live() {
    let live = Motion {
        front: true,
        ..Motion::default()
    };
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut tree = Tree::empty();
    // In the strip, then offered as the watch starts over, then large.
    let waits = waiting("Waiting for Bob's picture...");
    let offered = Tile {
        fill: Fill::Offer { preview: None },
        ..waits.clone()
    };
    let (_, small) = frame(&mut tree, &waits, SMALL, live, at(0));
    assert_eq!(small.len(), 1, "one ring in the strip: {small:?}");
    frame(&mut tree, &offered, SMALL, live, at(10));
    frame(
        &mut tree,
        &waiting("Connecting to Bob's stream..."),
        LARGE,
        live,
        at(20),
    );
    let width = LARGE.width.ceil() as u32;
    // An eighth and three eighths of a turn on.
    let mut shots = vec![];
    for ms in [125, 375] {
        let (pixels, rings) = frame(&mut tree, &waits, LARGE, live, at(ms));
        let ring = one_ring(&rings);
        let lit = lit_about(&pixels, width, ring);
        assert!(
            lit.len() > 40,
            "the ring drawn at {ms} ms: {} pixels",
            lit.len()
        );
        let room = ring.expand(2.0);
        assert!(
            lit.iter().all(|&(x, y)| room.contains(Point::new(x, y))),
            "nothing lit but the ring, in its place, at {ms} ms: {:?}",
            lit.iter().find(|&&(x, y)| !room.contains(Point::new(x, y)))
        );
        shots.push(lit);
    }
    assert_ne!(shots[0], shots[1], "it turns");

    // Where nothing moves (on screen unfocused, resting), the still clock:
    // the same at both times, and not the loader.
    let still = Motion::default();
    let mut clocks = vec![];
    for ms in [125, 375] {
        let (pixels, rings) = frame(&mut tree, &waits, LARGE, still, at(ms));
        let ring = one_ring(&rings);
        let lit = lit_about(&pixels, width, ring);
        assert!(!lit.is_empty(), "the clock drawn");
        clocks.push(lit);
    }
    assert_eq!(clocks[0], clocks[1], "still");
    assert!(
        shots.iter().all(|shot| *shot != clocks[0]),
        "the clock, not the loader"
    );
}
