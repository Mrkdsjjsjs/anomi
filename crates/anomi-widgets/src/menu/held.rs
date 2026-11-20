//! A submenu a screen lays out itself in one card with its menu, as the
//! message menu's Apps beside its rows, closing as the window's submenus and
//! [`super::Beside`]'s do (`super::hold`). The screen says where the
//! submenu's row and the submenu are in the card; once a submenu the
//! pointer opened has had the pointer on neither for
//! [`super::hold::GRACE`], wherever it went, `on_leave` asks the view-model
//! to close it, and meanwhile the card's other rows don't see the pointer,
//! so it crosses a corner of another row on its way to the submenu. A press
//! anywhere but on the two asks at once, the press going on to what's under
//! the pointer. One the keys opened, the pointer elsewhere, stays until a
//! key or a press. One wake at the end of the wait; nothing ticks.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Operation, Tree, tree};
use iced::advanced::{Shell, mouse};
use iced::{Event, Length, Rectangle, Size, Vector, touch, window};

use anomi_theme::{Element, Renderer, Theme, View};

use super::hold::Hold;

/// A card with a submenu inside it; see the module's doc.
pub struct Held<'a, M> {
    content: Element<'a, M>,
    open: bool,
    regions: [Rectangle; 2],
    on_leave: M,
}

/// `content`, whose submenu shows while `open`: `regions` are its row and
/// the submenu, from `content`'s top left; `on_leave` asks to close it.
pub fn held<'a, M>(
    content: impl View<M> + 'a,
    open: bool,
    regions: [Rectangle; 2],
    on_leave: M,
) -> Held<'a, M> {
    Held {
        content: content.boxed(),
        open,
        regions,
        on_leave,
    }
}

/// Whether the submenu showing is the pointer's, and where the pointer was
/// at its last move.
#[derive(Default)]
struct State {
    shown: bool,
    hold: Hold,
    /// `on_leave` went for this showing.
    told: bool,
    on: bool,
}

impl<M> Held<'_, M> {
    /// The pointer is on the submenu or its row.
    fn on(&self, layout: Layout, cursor: mouse::Cursor) -> bool {
        let origin = layout.position();
        cursor.position().is_some_and(|point| {
            let point = point - Vector::new(origin.x, origin.y);
            self.regions.iter().any(|region| region.contains(point))
        })
    }

    /// The pointer as the card's rows see it: nowhere while the submenu
    /// waits for it off the two.
    fn seen(&self, state: &State, on: bool, cursor: mouse::Cursor) -> mouse::Cursor {
        if self.open && !state.told && state.hold.waiting() && !on {
            cursor.levitate()
        } else {
            cursor
        }
    }
}

impl<M: Clone> Held<'_, M> {
    /// Asks the view-model to close the submenu, once a showing.
    fn leave(&self, state: &mut State, shell: &mut Shell<'_, M>) {
        if !state.told {
            state.told = true;
            state.hold = Hold::default();
            shell.publish(self.on_leave.clone());
            shell.request_redraw();
        }
    }
}

impl<M> widget::Meta for Held<'_, M> {}

impl<M: Clone> widget::Widget<M, Theme, Renderer> for Held<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let on = self.on(layout, cursor);
        let Tree {
            state, children, ..
        } = tree;
        let state = state.downcast_mut::<State>();
        if !self.open {
            *state = State {
                on: state.on,
                ..State::default()
            };
        } else if !state.shown {
            // Shown with the pointer on its row, the pointer opened it.
            state.shown = true;
            state.hold = Hold::new(state.on);
        }
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft)
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                state.on = on;
                if self.open && !state.told {
                    state.hold.moved(on, shell);
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(_))
            | Event::Touch(touch::Event::FingerPressed { .. })
                if self.open && !on =>
            {
                self.leave(state, shell);
            }
            Event::Window(window::Event::RedrawRequested(now)) if self.open && !state.told => {
                // The pointer resting on it at a frame is on it too.
                state.hold.moved(on, shell);
                if state.hold.due(*now, shell) {
                    self.leave(state, shell);
                }
            }
            _ => {}
        }
        let cursor = self.seen(state, on, cursor);
        let (child, child_tree) = layout.iter_mut(children).next().unwrap();
        self.content
            .update(child_tree, event, child, cursor, renderer, shell, viewport);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let cursor = self.seen(tree.state.downcast_ref(), self.on(layout, cursor), cursor);
        let (layout, tree) = layout.iter(&tree.children).next().unwrap();
        self.content
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let cursor = self.seen(tree.state.downcast_ref(), self.on(layout, cursor), cursor);
        let (layout, tree) = layout.iter(&tree.children).next().unwrap();
        self.content
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let (layout, tree) = layout.iter_mut(&mut tree.children).next().unwrap();
        self.content
            .operate(tree, layout, viewport, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, M, Theme, Renderer>> {
        let (layout, tree) = layout.iter_mut(&mut tree.children).next().unwrap();
        self.content
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
