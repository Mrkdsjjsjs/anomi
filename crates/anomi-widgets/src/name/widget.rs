//! The name as a widget: its paragraph shaped in the name's font, elided to
//! the width it's given (`sizer` and the texts of `StyledName`), and what
//! it draws, kept in its tree state (`drawn`). It reports its text to
//! operations as iced's text does, so a test finds a row by the name in it.
//!
//! A moving name keeps its own clock: from the first frame the window draws
//! while it moves (`StyledName::moving`), each `RedrawRequested` sets the
//! frame it draws and asks for the next one (`keyframes`). One that holds
//! still forgets its clock and asks for nothing, and one drawn again long
//! after the frame it asked for, such as the top of a `hover` while the
//! pointer was away, starts its loop over, as Discord's does.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use iced::advanced::graphics::text::Paragraph;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::paragraph::Plain;
use iced::advanced::text::{
    Alignment, Ellipsis, Fragment, LineHeight, Paragraph as _, Renderer as _, Shaping, Target,
    Text, Wrapping,
};
use iced::advanced::widget::{self, Operation, Tree, operation, tree};
use iced::advanced::{Shell, mouse};
use iced::alignment::{Horizontal, Vertical};
use iced::{Color, Event, Font, Length, Pixels, Point, Rectangle, Size, window};

use anomi_theme::fonts::{self, Version};
use anomi_theme::{Renderer, Theme};

use super::StyledName;
use super::colours::Colours;
use super::drawn::Kept;
use super::keyframes;
use super::paint;
use super::style::{Effect, Style};

/// How long after the frame it asked for a moving name may be drawn and
/// go on where it was; later, it wasn't drawn meanwhile, and starts over.
const LATE: Duration = Duration::from_millis(250);

/// One letter of a gummy name, measured alone, as each was a `Text` of
/// its own in a row: no kerning between them.
pub(super) struct Letter {
    pub text: String,
    /// Where it starts, from the name's left.
    pub x: f32,
    pub space: bool,
}

#[derive(Default)]
pub(super) struct State {
    /// The name shaped at the width it has, eliding.
    pub paragraph: Plain<Paragraph>,
    /// Bumped whenever `paragraph` is shaped or laid out again.
    pub shaped: u64,
    /// The font, size and line height `natural` and `letters` were
    /// measured in, whether letters were, and in which fonts
    /// ([`fonts::version`]): the person's font can draw in DM Sans's place.
    measured: Option<(Font, u32, LineHeight, bool, Version)>,
    /// The width the whole name takes: its preferred width.
    pub natural: f32,
    /// The renderer's line height, which other text in the row has too.
    line_height: LineHeight,
    /// A gummy name's letters; empty for any other.
    pub letters: Vec<Letter>,
    /// The style's colours for the theme's darkness.
    colours: RefCell<Option<(Style, bool, Option<Colours>)>>,
    /// What an effect drawn from outlines drew.
    pub kept: Kept,
    /// When a moving name's loop began; none while it holds still.
    since: Option<Instant>,
    /// When it asked for its next frame.
    due: Option<Instant>,
    /// The frame a moving name draws (`keyframes::frame`): 0 at rest.
    pub frame: u32,
}

impl State {
    /// Runs a name's clock to `now`, when it moves with `moving`'s effect
    /// and count of colours: the frame it draws, and when its next frame is
    /// due. One that holds still forgets its clock and the glyphs it kept,
    /// and asks for nothing.
    fn advance(&mut self, moving: Option<(Effect, usize)>, now: Instant) -> Option<Instant> {
        let Some((effect, count)) = moving else {
            self.since = None;
            self.due = None;
            self.frame = 0;
            self.kept.rest();
            return None;
        };
        if self.due.is_some_and(|due| now > due + LATE) {
            self.since = None;
        }
        let since = *self.since.get_or_insert(now);
        let at = now.saturating_duration_since(since);
        self.frame = keyframes::frame(effect, count, at);
        self.due = keyframes::next(effect, count, at).map(|next| since + next);
        self.due
    }

    /// Measures the name and a gummy name's letters, when the text, the
    /// font or the size changed.
    fn measure(&mut self, name: &StyledName<'_>, font: Font, line_height: LineHeight) {
        let key = (
            font,
            name.size.to_bits(),
            line_height,
            name.gummy(),
            fonts::version(),
        );
        if self.measured == Some(key) && self.paragraph.content() == name.text {
            return;
        }
        self.measured = Some(key);
        self.line_height = line_height;
        let width = |content: &str| {
            let text = line(name, content, (font, line_height), Size::INFINITE);
            Paragraph::with_text(text).min_width()
        };
        self.natural = width(name.text);
        self.letters.clear();
        if name.gummy() {
            let mut x = 0.0;
            for letter in name.text.chars() {
                let text = letter.to_string();
                let advance = width(&text);
                self.letters.push(Letter {
                    text,
                    x,
                    space: letter.is_whitespace(),
                });
                x += advance;
            }
        }
    }

    /// The style's colours on a dark or a light theme, made once for each.
    fn colours(&self, style: Style, dark: bool) -> Option<Colours> {
        let mut kept = self.colours.borrow_mut();
        match *kept {
            Some((kept_style, kept_dark, colours)) if kept_style == style && kept_dark == dark => {
                colours
            }
            _ => {
                let colours = Colours::of(&style, dark);
                *kept = Some((style, dark, colours));
                colours
            }
        }
    }
}

/// The name as one line of text in `font` at `line_height`, within
/// `bounds`.
fn line<'a>(
    name: &StyledName<'_>,
    content: &'a str,
    (font, line_height): (Font, LineHeight),
    bounds: Size,
) -> Text<&'a str> {
    Text {
        content,
        bounds,
        size: Pixels(name.size),
        line_height,
        font,
        align_x: Alignment::Left,
        align_y: Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
        ellipsis: Ellipsis::End,
        hint_factor: None,
    }
}

/// How the name is drawn, once its colours and width are known.
pub(super) enum Look {
    /// Text in one colour.
    Plain(Color),
    /// Each letter in the next colour.
    Letters(Colours),
    /// From outlines.
    Outlined(Effect, Colours),
}

impl StyledName<'_> {
    pub(super) fn look(&self, state: &State, dark: bool, ink: Color, width: f32) -> Look {
        let Some(style) = self.style else {
            return Look::Plain(ink);
        };
        let Some(colours) = state.colours(style, dark) else {
            return Look::Plain(ink);
        };
        let effect = style.effect();
        // A gummy name too long to show whole is drawn in its first
        // colour, as letters of their own don't elide.
        let fits = state.natural <= width + 0.5 && !state.letters.is_empty();
        if effect == Effect::Gummy && fits {
            Look::Letters(colours)
        } else if paint::outlined(effect, colours.count) {
            Look::Outlined(effect, colours)
        } else {
            Look::Plain(colours.colors[0])
        }
    }

    /// How tall the name's line is, in logical pixels.
    pub(super) fn line(&self, state: &State) -> f32 {
        state.line_height.to_absolute(Pixels(self.size)).0
    }

    /// The effect, and how many colours it has, of a name that moves now:
    /// its motion is watched, its effect moves (prism, neon, toon and pop,
    /// `keyframes::moves`), it isn't faded (a faded one is drawn whole,
    /// once), and it's in sight of `viewport`, edges and all.
    pub(super) fn moving(
        &self,
        bounds: Rectangle,
        viewport: &Rectangle,
    ) -> Option<(Effect, usize)> {
        let style = self.style?;
        let (effect, count) = (style.effect(), style.colors().len());
        let moves = self.motion.watched()
            && self.opacity >= 1.0
            && !self.text.is_empty()
            && keyframes::moves(effect, count)
            && bounds.expand(paint::reach(self.size)).intersects(viewport);
        moves.then_some((effect, count))
    }
}

/// The name as operations read text; it can't be selected.
struct Named<'a>(&'a str);

impl operation::Text for Named<'_> {
    fn text(&self) -> Fragment<'_> {
        self.0.into()
    }

    fn select(&mut self, _start: Point, _end: Point, _target: Target) {}

    fn select_all(&mut self) {}

    fn deselect(&mut self) {}

    fn copy(&mut self) -> Option<String> {
        None
    }
}

impl widget::Meta for StyledName<'_> {}

impl<Message> widget::Widget<Message, Theme, Renderer> for StyledName<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let state = tree.state.downcast_mut::<State>();
        let font = (self.font(), renderer.line_height());
        state.measure(self, font.0, font.1);
        let intrinsic = Size::new(state.natural.ceil(), self.line(state));
        tree.size = limits.resolve(self.width, self.height, intrinsic);
        let bounds = Size::new(tree.size.width, f32::INFINITY);
        if state.paragraph.update(line(self, self.text, font, bounds)) {
            state.shaped += 1;
        }
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.text(None, layout.bounds(), &mut Named(self.text));
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let moving = self.moving(layout.bounds(), viewport);
        let state = tree.state.downcast_mut::<State>();
        if let Some(due) = state.advance(moving, *now) {
            shell.request_redraw_at(due);
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let reach = paint::reach(self.size);
        if self.text.is_empty() || self.opacity <= 0.0 || !bounds.expand(reach).intersects(viewport)
        {
            return;
        }
        let palette = theme.palette();
        let ink = self.ink.resolve(palette);
        let paragraph = state.paragraph.raw();
        let anchor = bounds.anchor(paragraph.min_bounds(), Horizontal::Left, Vertical::Center);
        match self.look(state, palette.dark, ink, bounds.width) {
            Look::Plain(color) => {
                renderer.fill_paragraph(
                    paragraph,
                    anchor,
                    color.scale_alpha(self.opacity),
                    *viewport,
                );
            }
            Look::Letters(colours) => {
                let font = (self.font(), state.line_height);
                let mut turn = 0;
                for letter in &state.letters {
                    let color = colours.letter(turn).scale_alpha(self.opacity);
                    if !letter.space {
                        turn += 1;
                    }
                    let mut text = line(self, &letter.text, font, Size::INFINITE);
                    text.ellipsis = Ellipsis::None;
                    renderer.fill_text(
                        text.with_content(letter.text.clone()),
                        Point::new(anchor.x + letter.x, anchor.y),
                        color,
                        *viewport,
                    );
                }
            }
            Look::Outlined(effect, colours) => {
                let moving = self.moving(bounds, viewport).map(|_| state.frame);
                self.outlined(
                    state,
                    renderer,
                    (palette.dark, viewport),
                    (bounds, anchor),
                    (effect, &colours, moving),
                );
            }
        }
    }
}
