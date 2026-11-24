//! What a watched stream shows while its picture is on its way: the loader
//! turning once a second, as `StreamWaiting`'s does (`transform-rotation:
//! animation-tick() / 1ms * 0.36deg`, `ui/voice.slint:665`). A loader that
//! stands still reads as a hang, so it goes with the stream's video rather
//! than with what only decorates: it turns whenever the stream's picture
//! would play (`Motion::live`: in front, or on screen under "Everything" or
//! "What matters"), however long the window has gone untouched, unless
//! animations are off. Where nothing moves (hidden, resting, animations
//! off) it isn't a frozen loader but a still clock, which asks for no
//! frame. A stream whose video can't be shown has nothing to wait for: the
//! screen crossed out, still ([`stuck`]).

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::svg::{self as vector, Renderer as _};
use iced::advanced::widget::{self, Tree, tree};
use iced::advanced::{Shell, mouse};
use iced::{Color, Event, Length, Radians, Rectangle, Size, window};

use anomi_theme::{Motion, Renderer, Theme};
use anomi_widgets::icon::Icon;

use super::tick;

/// A turn a second.
const TURN: Duration = Duration::from_secs(1);

pub struct Spinner {
    side: f32,
    color: Color,
    motion: Motion,
    stuck: bool,
}

/// The loader, `side` px square, in `color`, turning while the stream's
/// picture would play with `motion`, or else the still clock.
pub fn spinner(side: f32, color: Color, motion: Motion) -> Spinner {
    Spinner {
        side,
        color,
        motion,
        stuck: false,
    }
}

/// The screen crossed out, `side` px square, in `color`: a stream whose
/// video can't be shown. It never moves.
pub fn stuck(side: f32, color: Color) -> Spinner {
    Spinner {
        side,
        color,
        motion: Motion::default(),
        stuck: true,
    }
}

/// Whether the loader turns with `motion`: whenever a stream's picture
/// would play, idle or not, unless animations are off.
pub(super) fn turns(motion: Motion) -> bool {
    motion.live() && !motion.never
}

/// How far round it is after `elapsed`: upright unless it turns.
pub(super) fn angle(motion: Motion, elapsed: Duration) -> Radians {
    if !turns(motion) {
        return Radians(0.0);
    }
    let turn = (elapsed.as_secs_f32() / TURN.as_secs_f32()).fract();
    Radians(turn * std::f32::consts::TAU)
}

impl Spinner {
    /// What it draws: the loader turning, the still clock, or the screen
    /// crossed out.
    fn icon(&self) -> Icon {
        if self.stuck {
            Icon::ScreenShareOff
        } else if turns(self.motion) {
            Icon::Loader
        } else {
            Icon::Clock
        }
    }

    fn turning(&self) -> bool {
        !self.stuck && turns(self.motion)
    }
}

/// The frame it's drawn at.
#[derive(Default)]
pub(super) struct State {
    now: Option<Instant>,
}

impl widget::Meta for Spinner {}

impl<Message> widget::Widget<Message, Theme, Renderer> for Spinner {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.side), Length::Fixed(self.side))
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) {
        let side = Size::new(self.side, self.side);
        tree.size = limits.resolve(self.side, self.side, side);
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
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            tree.state.downcast_mut::<State>().now = Some(*now);
            if self.turning() && layout.bounds().intersects(viewport) {
                shell.request_redraw();
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let elapsed = tree
            .state
            .downcast_ref::<State>()
            .now
            .map(tick)
            .unwrap_or_default();
        let rotation = if self.turning() {
            angle(self.motion, elapsed)
        } else {
            Radians(0.0)
        };
        let svg = vector::Svg {
            handle: self.icon().handle(),
            color: Some(self.color),
            rotation,
            opacity: 1.0,
        };
        renderer.draw_svg(svg, layout.bounds(), *viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use anomi_theme::motion::Background;

    /// It turns as a stream's picture plays: in front, idle too, and on
    /// screen unfocused under "Everything" or "What matters"; elsewhere,
    /// and with animations off, it's the still clock.
    #[test]
    fn it_turns_as_the_streams_picture_would_play() {
        let front = Motion {
            front: true,
            ..Motion::default()
        };
        let quarter = Duration::from_millis(250);
        assert!((angle(front, quarter).0 - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        let turning = |motion: Motion| {
            let spinner = spinner(28.0, Color::WHITE, motion);
            (spinner.turning(), spinner.icon())
        };
        let on = (true, Icon::Loader);
        let off = (false, Icon::Clock);
        assert_eq!(turning(front), on);
        assert_eq!(
            turning(Motion {
                idle: true,
                ..front
            }),
            on,
            "idle"
        );
        for (background, turns) in [
            (Background::Usual, on),
            (Background::Important, on),
            (Background::Nothing, off),
        ] {
            assert_eq!(
                turning(Motion {
                    background,
                    ..Motion::default()
                }),
                turns,
                "{background:?}"
            );
        }
        assert_eq!(
            turning(Motion {
                unseen: true,
                ..front
            }),
            off,
            "hidden"
        );
        assert_eq!(
            turning(Motion {
                never: true,
                ..front
            }),
            off,
            "animations off"
        );
        assert_eq!(
            angle(
                Motion {
                    never: true,
                    ..front
                },
                quarter
            ),
            Radians(0.0)
        );
        let stuck = stuck(28.0, Color::WHITE);
        assert_eq!(
            (stuck.turning(), stuck.icon()),
            (false, Icon::ScreenShareOff)
        );
    }
}
