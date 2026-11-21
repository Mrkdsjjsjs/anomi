//! A watched stream's tile, laid out once, stays as it was laid out: its
//! frames, of any size or shape, are drawn into the room it has, so one
//! that comes, and the frame of the window it asks for, invalidates nothing
//! and moves no part of the tile, at the strip's sizes and the grid's.

use std::time::Instant;

use iced::advanced::layout::{Layout, Limits};
use iced::advanced::renderer::Settings;
use iced::advanced::shell::{Bus, Invalidation, Waker};
use iced::advanced::widget::{Tree, Widget as _};
use iced::advanced::{Shell, mouse};
use iced::{Event, Point, Rectangle, Size, Vector, Widget as _, window};

use anomi_theme::{Element, Motion, Renderer, fonts, renderer::software, typography};
use anomi_vm::fixtures::voice::tiles as samples;
use anomi_vm::voice::tiles::{Fill, Intent};

use super::super::{INSET, chip, view};

fn renderer() -> Renderer {
    fonts::install();
    software(Settings {
        font: typography::UI,
        ..Settings::default()
    })
}

/// Every node's bounds in the window, depth first.
fn bounds(tree: &Tree, at: Vector, all: &mut Vec<Rectangle>) {
    let at = at + tree.translation;
    all.push(Rectangle::new(Point::ORIGIN + at, tree.size));
    for child in &tree.children {
        bounds(child, at, all);
    }
}

/// Bob's stream watched, laid out in `size`, then shown frames of 720p,
/// 1080p and a phone's upright 1080p, each followed by the window's frame:
/// none invalidates anything, and laid out again the tile is where it was.
fn holds_still(size: Size) {
    let tile = samples::watching().swap_remove(1);
    let Fill::Watched {
        frame: Some(live), ..
    } = &tile.fill
    else {
        panic!("a stream watched, with its picture");
    };
    let renderer = renderer();
    let motion = Motion {
        front: true,
        ..Motion::default()
    };
    let mut root: Element<'_, Intent> = view(&tile, motion).boxed();
    let mut tree = Tree::empty();
    tree.diff(&mut root);
    let limits = Limits::new(Size::ZERO, size);
    root.layout(&mut tree, &renderer, &limits);
    let mut laid = Vec::new();
    bounds(&tree, Vector::ZERO, &mut laid);

    for (width, height) in [(1280, 720), (1920, 1080), (1080, 1920)] {
        live.show(samples::frame(width, height));
        let mut bus = Bus::<Intent>::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let layout = Layout::new(tree.size);
        root.update(
            &mut tree,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            layout,
            mouse::Cursor::Unavailable,
            &renderer,
            &mut shell,
            &Rectangle::with_size(size),
        );
        assert_eq!(
            shell.invalidation(),
            Invalidation::None,
            "a {width}x{height} frame at {size:?}"
        );
        drop(shell);
        root.layout(&mut tree, &renderer, &limits);
        let mut again = Vec::new();
        bounds(&tree, Vector::ZERO, &mut again);
        assert_eq!(again, laid, "a {width}x{height} frame at {size:?}");
    }
}

#[test]
fn a_watched_streams_frames_lay_nothing_out() {
    // The grid's large tile, the strip's beneath it, the strip's least, and
    // the heights either side of the one where the chip comes.
    let chip = 2.0 * INSET + chip::HEIGHT;
    for size in [
        Size::new(420.0, 236.25),
        Size::new(152.0, 85.5),
        Size::new(120.0, 67.5),
        Size::new(160.0, chip - 0.01),
        Size::new(160.0, chip + 0.01),
    ] {
        holds_still(size);
    }
}
