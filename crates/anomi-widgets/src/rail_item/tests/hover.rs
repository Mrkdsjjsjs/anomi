//! An animated server icon plays only while the pointer is on it, from its
//! first frame, as Discord's rail plays one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::layout::Layout;
use iced::advanced::renderer::Settings;
use iced::advanced::shell::{Bus, Waker};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Shell, mouse};
use iced::{Event, Point, Rectangle, Size, window};

use anomi_theme::{Renderer, Theme, metrics};

use super::super::widget::State;
use super::super::{Face, RailItem, rail_item};
use super::front;
use crate::picture::{Frames, forget, register};

/// A picture of three frames of 100 ms, round and round.
fn moving() -> iced::widget::image::Handle {
    let still = iced::widget::image::Handle::from_rgba(1, 1, vec![0u8; 4]);
    let frames = vec![vec![0; 4], vec![1; 4], vec![2; 4]];
    register(
        &still,
        Arc::new(Frames::new(1, 1, frames, vec![100; 3], 0)),
    );
    still
}

struct Rig {
    tree: Tree,
    renderer: Renderer,
    cursor: mouse::Cursor,
}

const ROW: Size = Size::new(metrics::RAIL_WIDTH, metrics::RAIL_TILE);

impl Rig {
    fn new(item: &RailItem<'_, u8>) -> Self {
        let mut tree = Tree::empty();
        tree.tag = Widget::<u8, Theme, Renderer>::tag(item);
        tree.state = Widget::<u8, Theme, Renderer>::state(item);
        Self {
            tree,
            renderer: anomi_theme::renderer::software(Settings::default()),
            cursor: mouse::Cursor::Unavailable,
        }
    }

    fn send(&mut self, item: &mut RailItem<'_, u8>, event: Event) {
        let mut bus = Bus::<u8>::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        item.update(
            &mut self.tree,
            &event,
            Layout::new(ROW),
            self.cursor,
            &self.renderer,
            &mut shell,
            &Rectangle::with_size(ROW),
        );
    }

    fn point_at(&mut self, item: &mut RailItem<'_, u8>, position: Point) {
        self.cursor = mouse::Cursor::Available(position);
        self.send(item, Event::Mouse(mouse::Event::CursorMoved { position }));
    }

    /// The frame its face shows after a frame at `now`.
    fn frame(&mut self, item: &mut RailItem<'_, u8>, now: Instant) -> usize {
        self.send(item, Event::Window(window::Event::RedrawRequested(now)));
        self.tree.state.downcast_ref::<State>().players[0].frame()
    }
}

#[test]
fn a_moving_face_plays_only_while_hovered() {
    let face = moving();
    let mut item = rail_item::<u8>(Face::Picture(face.clone()), front());
    let mut rig = Rig::new(&item);
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    assert_eq!(rig.frame(&mut item, start), 0);
    assert_eq!(rig.frame(&mut item, at(150)), 0, "away, it holds");
    rig.point_at(&mut item, Point::new(36.0, 20.0));
    assert_eq!(rig.frame(&mut item, at(200)), 0, "from its first frame");
    assert_eq!(rig.frame(&mut item, at(350)), 1, "under the pointer, it plays");
    rig.point_at(&mut item, Point::new(150.0, 150.0));
    assert_eq!(rig.frame(&mut item, at(360)), 0, "gone, its first frame");
    forget(&face);
}
