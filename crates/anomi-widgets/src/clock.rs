//! A time that runs on its own, as text: a call's length in the voice dock
//! (`call_timer.rs` in the Slint app). It reads the time as it draws, from
//! an [`Elapsed`] the view-model holds once, so a new second builds no view
//! and lays nothing out: the text is shaped again when the frame's
//! `RedrawRequested` comes, and the next frame is asked for at the next
//! whole second, only while the window is live (`Motion::live`). Hidden,
//! or resting unfocused, nothing ticks; coming back draws the time it is
//! then. It's laid out again only when the text grows past the room it was
//! given, as at ten minutes and at an hour, and that layout shapes the time
//! of the frame that asked for it: iced stamps a frame on the refresh it
//! shows on, up to a refresh ahead of the clock, and a layout at the
//! clock's time could shape the second before, which the next pass of the
//! frame would find grown again, and so on.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::text::{self as core_text, paragraph};
use iced::advanced::widget::text::{self as text_widget, Format};
use iced::advanced::widget::{self, Tree, tree};
use iced::advanced::{Shell, mouse, renderer};
use iced::widget::text;
use iced::{Event, Font, Length, Pixels, Rectangle, Size, window};

use anomi_theme::{Renderer, Theme};
use anomi_vm::kind::Elapsed;

type Paragraph = paragraph::Plain<<Renderer as core_text::Renderer>::Paragraph>;

/// `elapsed` as `shown` writes it, ticking while `live`.
pub fn clock(elapsed: Elapsed, shown: fn(Duration) -> String, live: bool) -> Clock {
    Clock {
        elapsed,
        shown,
        live,
        size: None,
        font: None,
        line_height: None,
        style: |_| text::Style::default(),
    }
}

pub struct Clock {
    elapsed: Elapsed,
    shown: fn(Duration) -> String,
    live: bool,
    size: Option<Pixels>,
    font: Option<Font>,
    line_height: Option<text::LineHeight>,
    style: fn(&Theme) -> text::Style,
}

impl Clock {
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    pub fn line_height(mut self, line_height: impl Into<text::LineHeight>) -> Self {
        self.line_height = Some(line_height.into());
        self
    }

    pub fn style(mut self, style: fn(&Theme) -> text::Style) -> Self {
        self.style = style;
        self
    }

    fn format(&self) -> Format {
        Format {
            width: Length::Shrink,
            height: Length::Shrink,
            size: self.size,
            font: self.font,
            line_height: self.line_height,
            wrapping: text::Wrapping::None,
            ..Format::default()
        }
    }

    fn text(&self, now: Instant) -> String {
        (self.shown)(self.elapsed.at(now))
    }
}

/// The text shaped as it was last drawn, the room it was laid out in, and
/// the last frame's time, which a layout shapes while it's still ahead.
#[derive(Default)]
struct State {
    paragraph: Paragraph,
    limits: Option<layout::Limits>,
    stamp: Option<Instant>,
}

impl widget::Meta for Clock {}

impl<Message> widget::Widget<Message, Theme, Renderer> for Clock {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let state = tree.state.downcast_mut::<State>();
        state.limits = Some(*limits);
        let now = Instant::now();
        let text = self.text(state.stamp.map_or(now, |stamp| stamp.max(now)));
        let size =
            text_widget::layout(&mut state.paragraph, renderer, limits, &text, self.format());
        tree.size = size;
    }

    /// Its text as it was last drawn, for what reads the window's text.
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        operation.text(
            None,
            layout.bounds(),
            &mut text_widget::Operand {
                paragraph: &mut state.paragraph,
                layout,
                selectable: false,
            },
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        _cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let state = tree.state.downcast_mut::<State>();
        state.stamp = Some(*now);
        let text = self.text(*now);
        if text != state.paragraph.content()
            && let Some(limits) = state.limits
        {
            let size = text_widget::layout(
                &mut state.paragraph,
                renderer,
                &limits,
                &text,
                self.format(),
            );
            if size.width > layout.bounds().width + 0.5 {
                shell.invalidate_layout();
            }
        }
        if self.live
            && let Some(next) = self.elapsed.next_second(*now)
        {
            shell.request_redraw_at(next);
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let appearance = (self.style)(theme);
        text_widget::draw(
            renderer,
            style,
            layout.bounds(),
            state.paragraph.raw(),
            appearance,
            iced::Color::TRANSPARENT,
            viewport,
        );
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::shell::{Bus, Invalidation, Waker};

    use super::*;
    use crate::name::tests::support::{laid, renderer};

    /// A call's time a second before it widens: "10:00" from ten seconds
    /// on, "9:59" before, as at ten minutes.
    fn shown(elapsed: Duration) -> String {
        let text = if elapsed >= Duration::from_secs(10) {
            "10:00"
        } else {
            "9:59"
        };
        text.to_owned()
    }

    /// What a frame stamped `now` asks of the window.
    fn frame(
        timer: &mut Clock,
        tree: &mut Tree,
        renderer: &Renderer,
        now: Instant,
    ) -> Invalidation {
        let mut bus = Bus::<()>::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let layout = Layout::new(tree.size);
        let viewport = Rectangle::with_size(tree.size);
        widget::Widget::<(), Theme, Renderer>::update(
            timer,
            tree,
            &Event::Window(window::Event::RedrawRequested(now)),
            layout,
            mouse::Cursor::Unavailable,
            renderer,
            &mut shell,
            &viewport,
        );
        shell.invalidation()
    }

    /// Lays it out again, as iced does within a frame that invalidated it.
    fn lay_out(timer: &mut Clock, tree: &mut Tree, renderer: &Renderer) {
        let limits = layout::Limits::new(Size::ZERO, Size::new(400.0, 40.0));
        widget::Widget::<(), Theme, Renderer>::layout(timer, tree, renderer, &limits);
    }

    fn content(tree: &Tree) -> String {
        tree.state
            .downcast_ref::<State>()
            .paragraph
            .content()
            .to_owned()
    }

    /// A frame stamped on the refresh after the second turns, the clock
    /// still before it (iced rounds a frame's stamp up), widens the text
    /// once: the layout it asks for shapes the frame's time, so the next
    /// pass of the frame finds the text as wide as it was laid out.
    #[test]
    fn a_frame_ahead_of_the_clock_lays_it_out_once() {
        let renderer = renderer();
        let start = Instant::now() - Duration::from_secs(9);
        let mut timer = clock(Elapsed::Since(start), shown, true);
        let mut tree = laid(&mut timer, Size::new(400.0, 40.0));
        assert_eq!(content(&tree), "9:59");
        let stamp = start + Duration::from_millis(10_005);
        let widened = frame(&mut timer, &mut tree, &renderer, stamp);
        assert!(matches!(widened, Invalidation::Layout(_)), "{widened:?}");
        lay_out(&mut timer, &mut tree, &renderer);
        assert_eq!(content(&tree), "10:00", "the frame's time");
        let again = frame(&mut timer, &mut tree, &renderer, stamp);
        assert_eq!(again, Invalidation::None, "laid out once");
    }
}
