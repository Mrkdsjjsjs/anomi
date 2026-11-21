//! A moving name as the window has it: the frames it asks for, or none,
//! for its motion, opacity and place, and what it draws as it goes.

use std::time::{Duration, Instant};

use iced::advanced::layout::Layout;
use iced::advanced::renderer::{Headless as _, Renderer as _, Style as Defaults};
use iced::advanced::shell::{Bus, Waker};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Shell, mouse};
use iced::theme::Base as _;
use iced::window::{self, RedrawRequest};
use iced::{Event, Point, Rectangle, Size};

use anomi_theme::{Motion, Renderer, Theme};

use super::super::{Style, StyledName, styled_name};
use super::support::{inked, laid, renderer};

/// The window, in logical pixels.
const WINDOW: Size = Size::new(400.0, 100.0);

fn window_rect() -> Rectangle {
    Rectangle::with_size(WINDOW)
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// A window in front and in use.
fn front() -> Motion {
    Motion {
        front: true,
        ..Motion::default()
    }
}

/// The owner's prism, purple.
fn prism() -> Option<Style> {
    Style::new(11, 7, &[0x7a5cff, 0xc77dff, 0x5a189a])
}

/// A name laid out alone at the window's top left, sent frames and drawn.
struct Shown<'a> {
    name: StyledName<'a>,
    tree: Tree,
    renderer: Renderer,
}

impl<'a> Shown<'a> {
    fn new(mut name: StyledName<'a>) -> Self {
        let tree = laid(&mut name, WINDOW);
        Self {
            name,
            tree,
            renderer: renderer(),
        }
    }

    /// A frame at `now`, the window showing `viewport`: what the name
    /// asked of the window.
    fn frame_in(&mut self, now: Instant, viewport: Rectangle) -> RedrawRequest {
        let mut bus = Bus::<()>::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let layout = Layout::new(self.tree.size);
        Widget::<(), Theme, Renderer>::update(
            &mut self.name,
            &mut self.tree,
            &Event::Window(window::Event::RedrawRequested(now)),
            layout,
            mouse::Cursor::Unavailable,
            &self.renderer,
            &mut shell,
            &viewport,
        );
        shell.redraw_request()
    }

    fn frame(&mut self, now: Instant) -> RedrawRequest {
        self.frame_in(now, window_rect())
    }

    /// Frames from `start`, each when the one before asked, until one at
    /// or past `until`, which is drawn.
    fn play(&mut self, start: Instant, until: Instant) {
        let mut now = start;
        while now < until {
            let RedrawRequest::At(due) = self.frame(now) else {
                panic!("a moving name asks for its frames");
            };
            now = due;
        }
        self.frame(now);
    }

    /// What it draws, at twice the pixels.
    fn pixels(&mut self) -> image::RgbaImage {
        let theme = Theme::built_in(None);
        let base = theme.base();
        let viewport = window_rect();
        self.renderer.reset(viewport);
        Widget::<(), Theme, Renderer>::draw(
            &self.name,
            &self.tree,
            &mut self.renderer,
            &theme,
            &Defaults {
                text_color: base.text_color,
            },
            Layout::new(self.tree.size),
            mouse::Cursor::Unavailable,
            &viewport,
        );
        let physical = Size::new(WINDOW.width as u32 * 2, WINDOW.height as u32 * 2);
        let rgba = self
            .renderer
            .screenshot(physical, 2.0, base.background_color);
        image::RgbaImage::from_raw(physical.width, physical.height, rgba)
            .expect("a pixel for each of the window's")
    }
}

#[test]
fn a_name_that_may_not_move_asks_for_no_frame() {
    let now = Instant::now();
    let held = [
        Motion::default(),
        Motion {
            idle: true,
            ..front()
        },
        Motion {
            never: true,
            ..front()
        },
        Motion {
            unseen: true,
            ..front()
        },
    ];
    for motion in held {
        let mut shown = Shown::new(styled_name("Alice", prism()).motion(motion));
        assert_eq!(shown.frame(now), RedrawRequest::Wait, "{motion:?}");
    }
    // Styles that never move, in front: Discord's gradient and glow, solid,
    // gummy, a one-colour prism, a face alone, none.
    let still = [
        Style::new(11, 2, &[0x7a5cff, 0xc77dff]),
        Style::new(11, 6, &[0x7a5cff, 0xc77dff]),
        Style::new(11, 1, &[0x7a5cff]),
        Style::new(11, 8, &[0x7a5cff, 0xc77dff]),
        Style::new(11, 7, &[0x7a5cff]),
        Style::new(14, 3, &[0x00ff99]).map(Style::font_only),
        None,
    ];
    for style in still {
        let mut shown = Shown::new(styled_name("Alice", style).motion(front()));
        assert_eq!(shown.frame(now), RedrawRequest::Wait, "{style:?}");
    }
}

#[test]
fn a_moving_prism_asks_for_its_next_frame_at_most_33_ms_ahead() {
    let mut shown = Shown::new(styled_name("Alice", prism()).motion(front()));
    let mut now = Instant::now();
    // Three seconds of frames: past the end of its loop and on.
    for _ in 0..92 {
        let RedrawRequest::At(due) = shown.frame(now) else {
            panic!("a prism asks for frames");
        };
        assert!(due > now && due - now <= ms(33), "{:?}", due - now);
        now = due;
    }
}

#[test]
fn neon_past_its_flicker_asks_for_the_next_loops_15_percent() {
    let neon = Style::new(11, 3, &[0xb388ff]);
    let mut shown = Shown::new(styled_name("Bob", neon).motion(front()));
    let start = Instant::now();
    // At rest until 15%: its first frame asks for 15%.
    assert_eq!(shown.frame(start), RedrawRequest::At(start + ms(600)));
    // Frame by frame through the flicker, to 50%.
    shown.play(start + ms(600), start + ms(2000));
    // A frame at 60%, as something else is drawn, asks for the next 15%.
    assert_eq!(
        shown.frame(start + ms(2400)),
        RedrawRequest::At(start + ms(4600))
    );
}

#[test]
fn a_faded_name_and_one_out_of_sight_ask_for_no_frame() {
    let now = Instant::now();
    let neon = Style::new(14, 3, &[0x00ff99]);
    let faded = styled_name("Erin", neon).motion(front()).opacity(0.5);
    assert_eq!(Shown::new(faded).frame(now), RedrawRequest::Wait);
    let mut away = Shown::new(styled_name("Alice", prism()).motion(front()));
    let below = Rectangle::new(Point::new(0.0, 500.0), WINDOW);
    assert_eq!(away.frame_in(now, below), RedrawRequest::Wait);
    // Scrolled back into sight, it asks.
    assert!(matches!(away.frame(now), RedrawRequest::At(_)));
}

#[test]
fn a_name_drawn_again_long_after_the_frame_it_asked_for_starts_over() {
    // The top of a `hover` isn't drawn while the pointer is away.
    let mut shown = Shown::new(styled_name("Alice", prism()).motion(front()));
    let start = Instant::now();
    assert_eq!(shown.frame(start), RedrawRequest::At(start + ms(33)));
    // A second on, as from its start: the frame after its first.
    let back = start + ms(1000);
    assert_eq!(shown.frame(back), RedrawRequest::At(back + ms(33)));
}

#[test]
fn a_prism_starts_at_rest_scrolls_left_and_rests_again_when_it_stops() {
    let start = Instant::now();
    let style = Style::new(11, 7, &[0xff0000, 0x0000ff]);
    let name = || styled_name("mmmmmmmm", style).size(32.0);
    let rest = Shown::new(name()).pixels();
    let mut shown = Shown::new(name().motion(front()));
    shown.frame(start);
    assert!(shown.pixels() == rest, "its loop starts as it rests");
    // Half its loop on, half a cycle scrolled: blue at the left, red half
    // way along, where at rest it's red, then blue.
    shown.play(start, start + ms(1000));
    let [left, middle, _] = thirds(&shown.pixels());
    assert!(left[2] > left[0], "blue at the left: {left:?}");
    assert!(middle[0] > middle[2], "red half way: {middle:?}");
    // In a still motion again, it draws at rest and asks for nothing.
    shown.name = name();
    assert_eq!(shown.frame(start + ms(1033)), RedrawRequest::Wait);
    assert!(shown.pixels() == rest, "at rest again");
}

#[test]
fn neon_flickers_to_its_light_shade_and_starts_white() {
    let start = Instant::now();
    let style = Style::new(14, 3, &[0x00ff99]);
    let name = || styled_name("neon", style).size(32.0);
    let rest = Shown::new(name()).pixels();
    let mut shown = Shown::new(name().motion(front()));
    shown.frame(start);
    assert!(shown.pixels() == rest, "white until 15%");
    // At 16% the letters are the shade: hsl(156, 100%, 85%), less red.
    shown.frame(start + ms(640));
    let lit = shown.pixels();
    let white = |x: u32, y: u32| rest.get_pixel(x, y).0[..3].iter().all(|c| *c > 230);
    let (width, height) = rest.dimensions();
    let inside: Vec<_> = (2..width - 2)
        .flat_map(|x| (2..height - 2).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            white(x, y) && white(x - 2, y) && white(x + 2, y) && white(x, y - 2) && white(x, y + 2)
        })
        .map(|(x, y)| lit.get_pixel(x, y).0)
        .collect();
    assert!(inside.len() > 50, "{} pixels inside", inside.len());
    let mean = |channel: usize| {
        let sum: f32 = inside.iter().map(|pixel| f32::from(pixel[channel])).sum();
        sum / inside.len() as f32
    };
    assert!(mean(0) < 215.0, "less red: {}", mean(0));
    assert!(mean(1) > 235.0, "still green: {}", mean(1));
}

/// The mean colours of the left, middle and right thirds of what `image`
/// draws off its ground.
fn thirds(image: &image::RgbaImage) -> [[f32; 3]; 3] {
    let pixels = inked(image);
    let (min, max) = pixels.iter().fold((u32::MAX, 0), |(min, max), (x, _, _)| {
        (min.min(*x), max.max(*x))
    });
    let third = (max.saturating_sub(min) / 3).max(1);
    let part = |part: u32| {
        let from = min.saturating_add(third * part);
        let within = |x: &u32| (from..from.saturating_add(third)).contains(x);
        let mut sum = [0.0_f32; 3];
        let mut count = 0.0_f32;
        for (_, _, pixel) in pixels.iter().filter(|(x, ..)| within(x)) {
            for (total, channel) in sum.iter_mut().zip(pixel) {
                *total += f32::from(*channel);
            }
            count += 1.0;
        }
        sum.map(|total| total / count.max(1.0))
    };
    [part(0), part(1), part(2)]
}
