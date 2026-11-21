//! What a name drawn from outlines keeps in its tree state until what it's
//! drawn for changes: the effect's geometry, or for a faded name whose
//! effect stacks one thing on another, a picture of it.
//!
//! A moving name's frame (`keyframes::frame`) is part of what it's drawn
//! for, so its geometry is made again when the frame changes and kept
//! while it holds. While it moves it also keeps its glyphs' outlines, made
//! once for each shaping, so a new frame only paints them; it lets them go
//! once it holds still, and a still name keeps none.
//!
//! Slint fades a `StyledName` as one layer (`ui/people.slint:308`, an
//! offline member), so neon's, toon's and pop's edges, drawn under the
//! letters, never show through them. iced fades each fill on its own, which
//! would let the edge through every letter: such a name is drawn once at
//! full opacity into a picture at the window's scale, by a processor's
//! renderer of its own whatever draws the window, and the picture drawn at
//! the name's opacity.
//!
//! A geometry is drawn only by the kind of renderer that made it: iced's
//! fallback renderer can't draw one the other kind made, nor keep it in a
//! cache made for the other (`unreachable!()`). So the faded picture's
//! geometry is made by its own renderer's frame, and the outlines kept for
//! the window's renderer are made anew, from an empty cache, when the
//! window changes between a graphics adapter and the processor.

use std::cell::RefCell;

use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::image::{self, Renderer as _};
use iced::advanced::renderer::{Headless as _, Renderer as _, Settings};
use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size, Vector};

use anomi_theme::Renderer;
use anomi_theme::renderer::{is_gpu, software};

use crate::picture;

use super::StyledName;
use super::colours::Colours;
use super::keyframes;
use super::outline::{self, Glyphs};
use super::paint::{self, Paint};
use super::style::{Effect, Style};
use super::widget::State;

/// What the outlines were drawn for: when any of it changes they're drawn
/// again.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    shaped: u64,
    style: Style,
    dark: bool,
    opacity: u32,
    size: (u32, u32),
    scale: u32,
    /// Whether the window's renderer draws on a graphics adapter: its
    /// geometry is of another kind than the processor's.
    gpu: bool,
    /// The moving effect's frame, 33 ms of its loop each; 0 at rest.
    frame: u32,
}

/// A faded name's effect drawn whole: its pixels, and its size in logical
/// pixels.
struct Faded {
    handle: image::Handle,
    size: Size,
}

/// The effect as last drawn, and what for.
pub(super) struct Kept {
    drawn: RefCell<Option<Drawn>>,
    /// For the window's renderer, of the kind `drawn.gpu` says.
    outlines: RefCell<canvas::Cache<Renderer>>,
    faded: RefCell<Option<Faded>>,
    /// A moving name's glyphs, and the shaping and the height in the box
    /// they were traced for.
    glyphs: RefCell<Option<((u64, u32), Glyphs)>>,
}

impl Default for Kept {
    fn default() -> Self {
        Self {
            drawn: RefCell::default(),
            outlines: RefCell::new(canvas::Cache::new()),
            faded: RefCell::default(),
            glyphs: RefCell::default(),
        }
    }
}

impl Kept {
    /// Lets the glyphs a moving name kept go, once it holds still.
    pub(super) fn rest(&mut self) {
        *self.glyphs.get_mut() = None;
    }

    /// Paints with the glyphs a moving name keeps: those of `paragraph`
    /// with its top left at `origin`, traced again only when it's shaped
    /// again (`shaped`) or lies elsewhere in its box.
    fn with_glyphs(
        &self,
        (paragraph, shaped): (&Paragraph, u64),
        origin: Point,
        paint: impl FnOnce(&Glyphs),
    ) {
        let traced = (shaped, origin.y.to_bits());
        let mut kept = self.glyphs.borrow_mut();
        if !matches!(&*kept, Some((made, _)) if *made == traced) {
            *kept = Some((traced, outline::glyphs(paragraph, origin)));
        }
        if let Some((_, glyphs)) = kept.as_ref() {
            paint(glyphs);
        }
    }

    /// Forgets what was drawn when it was drawn for something else. For
    /// another kind of renderer the outlines' cache is replaced, not
    /// cleared: a cleared one keeps its last geometry to reuse, which the
    /// other kind can't.
    fn keep(&self, drawn: Drawn) {
        let mut kept = self.drawn.borrow_mut();
        if *kept == Some(drawn) {
            return;
        }
        if matches!(*kept, Some(old) if old.gpu != drawn.gpu) {
            *self.outlines.borrow_mut() = canvas::Cache::new();
        } else {
            self.outlines.borrow().clear();
        }
        *self.faded.borrow_mut() = None;
        *kept = Some(drawn);
    }
}

impl StyledName<'_> {
    /// Draws an effect from the name's outlines, made again only when what
    /// they're drawn for changed. `moving` is the frame of a name that
    /// moves now (`State::frame`), and none for one that holds still.
    pub(super) fn outlined(
        &self,
        state: &State,
        renderer: &mut Renderer,
        (dark, viewport): (bool, &Rectangle),
        (bounds, anchor): (Rectangle, Point),
        (effect, colours, moving): (Effect, &Colours, Option<u32>),
    ) {
        let Some(style) = self.style else {
            return;
        };
        let scale = picture::scale();
        let kept = &state.kept;
        let step = moving.unwrap_or(0);
        kept.keep(Drawn {
            shaped: state.shaped,
            style,
            dark,
            opacity: self.opacity.to_bits(),
            size: (bounds.width.to_bits(), bounds.height.to_bits()),
            scale: scale.to_bits(),
            gpu: is_gpu(renderer),
            frame: step,
        });
        let reach = paint::reach(self.size);
        let clip = Rectangle::new(
            Point::new(-reach, -reach),
            Size::new(bounds.width + 2.0 * reach, bounds.height + 2.0 * reach),
        );
        let faded = self.opacity < 1.0 && paint::stacked(effect);
        let moment = keyframes::moment(effect, colours.count, step);
        let draw = |frame: &mut canvas::Frame<Renderer>| {
            let origin = Point::new(0.0, anchor.y - bounds.y);
            let mut fill = |glyphs: &Glyphs| {
                let paint = Paint {
                    glyphs,
                    colours,
                    size: bounds.size(),
                    em: self.size,
                    line_height: self.line(state),
                    // A faded picture is drawn whole, and faded as one.
                    opacity: if faded { 1.0 } else { self.opacity },
                    at: moment,
                };
                paint::effect(frame, effect, &paint);
            };
            if moving.is_some() {
                let paragraph = (state.paragraph.raw(), state.shaped);
                kept.with_glyphs(paragraph, origin, fill);
            } else {
                fill(&outline::glyphs(state.paragraph.raw(), origin));
            }
        };
        if !faded {
            let geometry = kept
                .outlines
                .borrow()
                .draw_with_bounds(renderer, clip, draw);
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                renderer.draw_geometry(geometry);
            });
            return;
        }
        let mut made = kept.faded.borrow_mut();
        if made.is_none() {
            *made = whole(renderer.settings(), clip, scale, draw);
        }
        let Some(faded) = made.as_ref() else {
            return;
        };
        // On the window's pixel grid, so the picture's pixels are the
        // window's and the letters stay sharp.
        let origin = bounds.position() + Vector::new(clip.x, clip.y);
        let snap = |value: f32| (value * scale).round() / scale;
        let at = Rectangle::new(Point::new(snap(origin.x), snap(origin.y)), faded.size);
        if let Some(shown) = at.intersection(viewport) {
            let picture = image::Image {
                opacity: self.opacity,
                ..image::Image::new(faded.handle.clone())
            };
            renderer.draw_image(picture, at, shown);
        }
    }
}

/// What `draw` paints within `clip`, as a picture at `scale` physical
/// pixels a logical one: none when it has no pixels. Drawn on the processor
/// by a renderer of its own, from that renderer's own frame, whatever draws
/// the window: the same pixels either way, and a geometry drawn by the kind
/// of renderer that made it.
fn whole(
    settings: Settings,
    clip: Rectangle,
    scale: f32,
    draw: impl FnOnce(&mut canvas::Frame<Renderer>),
) -> Option<Faded> {
    let width = (clip.width * scale).ceil();
    let height = (clip.height * scale).ceil();
    if !(width >= 1.0 && height >= 1.0) {
        return None;
    }
    let (width, height) = (width as u32, height as u32);
    let size = Size::new(width as f32 / scale, height as f32 / scale);
    let mut own = software(settings);
    let mut frame = canvas::Frame::with_bounds(&own, clip);
    draw(&mut frame);
    let geometry = frame.into_geometry();
    own.reset(Rectangle::with_size(size));
    own.with_translation(Vector::new(-clip.x, -clip.y), |own| {
        own.draw_geometry(geometry);
    });
    let mut pixels = own.screenshot(Size::new(width, height), scale, Color::TRANSPARENT);
    unpremultiply(&mut pixels);
    Some(Faded {
        handle: image::Handle::from_rgba(width, height, pixels),
        size,
    })
}

/// tiny-skia's pixels carry their colour multiplied by their alpha; a
/// picture's don't.
fn unpremultiply(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            let straight = (u16::from(*channel) * 255 + alpha / 2) / alpha;
            *channel = straight.min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn drawn(gpu: bool) -> Drawn {
        Drawn {
            shaped: 1,
            style: Style::new(14, 3, &[0x00ff99]).expect("a colour"),
            dark: true,
            opacity: 1.0_f32.to_bits(),
            size: (40.0_f32.to_bits(), 16.0_f32.to_bits()),
            scale: 1.0_f32.to_bits(),
            gpu,
            frame: 0,
        }
    }

    /// Whether `kept` drew its outlines anew when asked for `drawn`.
    fn drawn_anew(kept: &Kept, renderer: &Renderer, drawn: Drawn) -> bool {
        kept.keep(drawn);
        let called = Cell::new(false);
        let _ = kept.outlines.borrow().draw_with_bounds(
            renderer,
            Rectangle::with_size(Size::new(40.0, 16.0)),
            |_| called.set(true),
        );
        called.get()
    }

    #[test]
    fn outlines_are_kept_until_the_window_changes_renderer() {
        // Only the processor's renderer is at hand here: what's checked is
        // that a change of kind starts the cache over.
        let renderer = software(Settings::default());
        let kept = Kept::default();
        assert!(drawn_anew(&kept, &renderer, drawn(false)), "the first time");
        assert!(!drawn_anew(&kept, &renderer, drawn(false)), "then kept");
        assert!(drawn_anew(&kept, &renderer, drawn(true)), "on the GPU");
        assert!(!drawn_anew(&kept, &renderer, drawn(true)), "kept there");
        assert!(
            drawn_anew(&kept, &renderer, drawn(false)),
            "back on the CPU"
        );
    }

    #[test]
    fn a_moving_name_is_drawn_again_for_each_frame_and_kept_while_it_holds() {
        let renderer = software(Settings::default());
        let kept = Kept::default();
        let at = |frame| Drawn {
            frame,
            ..drawn(false)
        };
        assert!(drawn_anew(&kept, &renderer, at(0)), "the first time");
        assert!(drawn_anew(&kept, &renderer, at(1)), "a frame on");
        assert!(!drawn_anew(&kept, &renderer, at(1)), "the same frame");
        assert!(drawn_anew(&kept, &renderer, at(2)), "the next");
        assert!(drawn_anew(&kept, &renderer, at(0)), "at rest again");
        assert!(!drawn_anew(&kept, &renderer, at(0)), "and kept");
    }

    #[test]
    fn a_faded_picture_is_drawn_whole_in_straight_colour() {
        let clip = Rectangle::new(Point::new(-2.0, -2.0), Size::new(4.0, 4.0));
        let made = whole(Settings::default(), clip, 2.0, |frame| {
            frame.fill_rectangle(
                Point::new(-2.0, -2.0),
                Size::new(4.0, 4.0),
                Color::from_rgba8(255, 0, 0, 0.5),
            );
        })
        .expect("a picture");
        assert_eq!(made.size, Size::new(4.0, 4.0));
        let image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = &made.handle
        else {
            panic!("a picture of its pixels");
        };
        assert_eq!((*width, *height), (8, 8));
        // The middle pixel: red at half its alpha, not multiplied by it.
        let at = (4 * 8 + 4) * 4;
        let pixel = &pixels[at..at + 4];
        assert!(
            pixel[0] >= 250 && pixel[1] == 0 && pixel[2] == 0,
            "{pixel:?}"
        );
        assert!((126..=129).contains(&pixel[3]), "{pixel:?}");
        assert!(whole(Settings::default(), Rectangle::default(), 2.0, |_| ()).is_none());
    }
}
