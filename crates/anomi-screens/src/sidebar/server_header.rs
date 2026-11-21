//! The server's header at the top of its channel list: the name with its
//! badge and chevron, over the server's banner where it has one, the invite
//! button, and under them the boost goal and the events row. The Slint
//! side: `ServerHeader` (`ui/rail.slint:946`) and the header box in
//! `ui/yapfast.slint:2817`; the dropdown it opens is the window's menu
//! (F06), asked for through [`Intent::Menu`].
//!
//! The block ends in Slint's rules, drawn here: each row has its own, a bare
//! header ends in a full-width one, and a banner with no rows in none.

mod below;
mod chevron;
mod lit;
mod name;
#[cfg(feature = "shots")]
mod shots;

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Operation, Tree, tree};
use iced::advanced::{Shell, mouse, overlay, renderer};
use iced::border::Radius;
use iced::widget::{Column, Row, Space, Stack, container, rule};
use iced::{Border, Event, Fill, Gradient, Length, Padding, Rectangle, Size, Vector, gradient};

use anomi_theme::prelude::*;
use anomi_theme::{Renderer, catalog, metrics};
use anomi_vm::sidebar::server_header::{Intent, ServerHeader};
use anomi_widgets::button::{Side, icon_button};
use anomi_widgets::icon::Icon;
use anomi_widgets::picture::{Fit, picture};
use yapfast_lang::tr;

#[cfg(feature = "shots")]
pub use shots::shots;

/// How much taller the header is with a banner behind the name.
const BANNER_EXTRA: f32 = 81.0;
/// The name's box, and the room above it.
const NAME_HEIGHT: f32 = 32.0;
/// The sheet's rounded top-left corner (`ui/yapfast.slint:2818`).
const SHEET_CORNER: f32 = 12.0;
/// The scrim keeping a white name readable over a bright banner.
const SCRIM_ALPHA: f32 = 0.6;

pub fn view(header: &ServerHeader, motion: Motion) -> impl View<Intent> + '_ {
    let below = below::rows(header, motion);
    let banner = header.has_banner();
    // The part above the rows: 49 px, then its full-width rule where no
    // banner is and (a row or nothing) follows. Under a banner nothing draws
    // that rule, so the top is the whole 131.
    let height = if banner {
        metrics::HEADER_HEIGHT + BANNER_EXTRA
    } else {
        metrics::HEADER_HEIGHT - 1.0
    };
    let mut block = Column::new().push(
        container(top(header, motion))
            .width(Fill)
            .height(Length::Fixed(height))
            .boxed(),
    );
    if !banner {
        block = block.push(rule::horizontal(1).style(catalog::rule::outline).boxed());
    }
    block.extend(below).width(Fill)
}

/// The name, the invite button, and the banner behind them.
fn top(header: &ServerHeader, motion: Motion) -> Element<'_, Intent> {
    let mut line = Row::new()
        .push(name::name(header, motion).boxed())
        .spacing(metrics::SPACE_S);
    if header.invite {
        line = line.push(
            icon_button(Icon::UserPlus, tr!("Invite people"), motion)
                .on_press(Intent::Invite)
                .tip_side(Side::Below)
                .view()
                .boxed(),
        );
    }
    let line = container(line)
        .padding(Padding::new(metrics::SPACE_S).bottom(0.0))
        .width(Fill)
        .height(NAME_HEIGHT + metrics::SPACE_S);
    if !header.has_banner() {
        return line.boxed();
    }
    // The scrim shows from the banner's URL on; the picture joins it when
    // it arrives.
    let over = Stack::new()
        .width(Fill)
        .height(Fill)
        .push(scrim().boxed())
        .push(line.boxed());
    let Some(handle) = header.banner_picture() else {
        return over.boxed();
    };
    let banner = |motion: Motion| {
        picture(handle.clone())
            .width(Fill)
            .height(Fill)
            .fit(Fit::Cover)
            .radius(Radius::default().top_left(SHEET_CORNER))
            .motion(motion)
    };
    // A moving banner plays only under the pointer, from its first frame
    // each time; at rest it holds still.
    Banner {
        parts: [
            banner(Motion::default()).boxed(),
            banner(motion).from_start().capped().boxed(),
            over.boxed(),
        ],
    }
    .boxed()
}

/// A shade from the top edge, over the banner behind the name.
fn scrim<'a>() -> impl View<Intent> + 'a {
    container(Space::new())
        .width(Fill)
        .height(metrics::HEADER_HEIGHT)
        .style(|_theme: &Theme| iced::widget::container::Style {
            background: Some(
                Gradient::Linear(
                    gradient::Linear::new(std::f32::consts::PI)
                        .add_stop(0.0, iced::Color::BLACK.scale_alpha(SCRIM_ALPHA))
                        .add_stop(1.0, iced::Color::TRANSPARENT),
                )
                .into(),
            ),
            border: Border {
                radius: Radius::default().top_left(SHEET_CORNER),
                ..Border::default()
            },
            ..Default::default()
        })
}

/// The banner at rest, the banner playing, and what lies over them, laid
/// out as that is. The playing one shows while the pointer is anywhere on
/// the header, its name and button included, which iced's `hover` under
/// them in a stack wouldn't see; the one at rest otherwise. Only the one
/// shown draws and advances, so at rest it asks for nothing.
struct Banner<'a> {
    parts: [Element<'a, Intent>; 3],
}

const REST: usize = 0;
const PLAYING: usize = 1;
const OVER: usize = 2;

/// Whether the pointer is on the header.
#[derive(Default)]
struct Pointer {
    over: bool,
}

impl Pointer {
    /// The banner on show.
    fn shown(&self) -> usize {
        if self.over { PLAYING } else { REST }
    }
}

impl widget::Meta for Banner<'_> {}

impl widget::Widget<Intent, Theme, Renderer> for Banner<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Pointer>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Pointer::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.parts);
    }

    fn size(&self) -> Size<Length> {
        self.parts[OVER].size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.parts[OVER].layout(&mut tree.children[OVER], renderer, limits);
        let size = tree.children[OVER].size;
        let room = layout::Limits::new(Size::ZERO, size);
        for index in [REST, PLAYING] {
            self.parts[index].layout(&mut tree.children[index], renderer, &room);
        }
        tree.size = size;
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Intent>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<Pointer>();
        let over = cursor.is_over(layout.bounds());
        if over != state.over {
            state.over = over;
            shell.request_redraw();
        }
        let shown = state.shown();
        for (index, (layout, tree)) in layout.iter_mut(&mut tree.children).enumerate() {
            if index == shown || index == OVER {
                self.parts[index].update(tree, event, layout, cursor, renderer, shell, viewport);
            }
        }
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
        use iced::advanced::Renderer as _;
        if !layout.bounds().intersects(viewport) {
            return;
        }
        let shown = tree.state.downcast_ref::<Pointer>().shown();
        for (index, (layout, tree)) in layout.iter(&tree.children).enumerate() {
            if index == shown {
                self.parts[index].draw(tree, renderer, theme, style, layout, cursor, viewport);
            } else if index == OVER {
                // A layer of its own, as a stack's upper layers have, so the
                // scrim lies over the picture.
                renderer.with_layer(*viewport, |renderer| {
                    self.parts[index].draw(tree, renderer, theme, style, layout, cursor, viewport);
                });
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        match layout.iter(&tree.children).nth(OVER) {
            Some((layout, tree)) => {
                self.parts[OVER].mouse_interaction(tree, layout, cursor, viewport, renderer)
            }
            None => mouse::Interaction::None,
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        if let Some((layout, tree)) = layout.iter_mut(&mut tree.children).nth(OVER) {
            self.parts[OVER].operate(tree, layout, viewport, renderer, operation);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Intent, Theme, Renderer>> {
        match layout.iter_mut(&mut tree.children).nth(OVER) {
            Some((layout, tree)) => {
                self.parts[OVER].overlay(tree, layout, renderer, viewport, translation, window)
            }
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::image;
    use iced::{Settings, Size, Widget as _};
    use iced_test::Simulator;

    use anomi_theme::{Element, Motion, Renderer, Theme, fonts, typography};
    use anomi_vm::fixtures::sidebar::server_header as samples;
    use anomi_vm::kind::Expected;
    use anomi_vm::sidebar::server_header::{Intent, ServerHeader};

    use super::{PLAYING, Pointer, REST, view};

    #[test]
    fn the_banner_plays_only_under_the_pointer() {
        assert_eq!(Pointer { over: false }.shown(), REST);
        assert_eq!(Pointer { over: true }.shown(), PLAYING);
    }

    #[test]
    fn over_a_banner_the_name_still_asks_for_the_menu() {
        fonts::install();
        let header = ServerHeader {
            banner: Expected::Ready(image::Handle::from_rgba(1, 1, vec![0x80; 4])),
            ..samples::server_header()
        };
        let front = Motion {
            front: true,
            ..Motion::default()
        };
        let settings = Settings {
            font: typography::UI,
            ..Settings::default()
        };
        let element: Element<'_, Intent> = view(&header, front).boxed();
        let mut ui: Simulator<'_, Intent, Theme, Renderer> =
            Simulator::with_size(settings, Size::new(304.0, 400.0), element);
        ui.click("Server").expect("the name");
        let asked: Vec<_> = ui.into_messages().collect();
        assert!(matches!(asked.as_slice(), [Intent::Menu(_)]), "{asked:?}");
    }
}
