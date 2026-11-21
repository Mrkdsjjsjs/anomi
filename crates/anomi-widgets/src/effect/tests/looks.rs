//! What the effect draws, read back from its pixels: at rest its still,
//! cut to the card's top corners; hovered, its layers in the still's place;
//! opened, nothing until it plays, then its layers over the card's
//! contents, or its still once a layer has kept it waiting too long.

use std::time::Instant;

use iced::widget::image::Handle;
use iced::widget::{container, space, stack};
use iced::{Color, Fill, Point, Size};

use anomi_theme::{Motion, Theme};

use super::super::{Effect, Play, effect, sample};
use super::support::{Shot, ground, near};
use super::{front, ms};

const CARD: Size = Size::new(300.0, 515.0);
/// The card's own radius, as the popout's (`ui/people.slint:1446`).
const RADIUS: f32 = 12.0;
/// What Bob's still lies on here, which no theme's ground is.
const BLUE: [u8; 4] = [30, 80, 200, 255];

/// Bob's effect with its still laid on [`BLUE`]: the fake's still is clear
/// but for its petals, which would hide both the radius's cut and the
/// still's going.
fn on_blue() -> Effect {
    let mut petals = sample::petals();
    let Some(Handle::Rgba {
        width,
        height,
        pixels,
        ..
    }) = &petals.still
    else {
        panic!("a still of pixels");
    };
    let mut opaque = pixels.to_vec();
    for pixel in opaque.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for (channel, blue) in pixel.iter_mut().zip(BLUE).take(3) {
            *channel =
                ((u16::from(*channel) * alpha + u16::from(blue) * (255 - alpha)) / 255) as u8;
        }
        pixel[3] = 255;
    }
    petals.still = Some(Handle::from_rgba(*width, *height, opaque));
    petals
}

/// Some of a petal's pink over `under`: far redder than it.
fn petal(pixel: [u8; 4], under: [u8; 4]) -> bool {
    pixel[0] >= under[0].saturating_add(60)
}

fn count(shot: &image::RgbaImage, which: impl Fn([u8; 4]) -> bool) -> usize {
    shot.pixels().filter(|pixel| which(pixel.0)).count()
}

#[test]
fn at_rest_its_still_shows_cut_to_the_cards_corners() {
    let theme = Theme::default();
    let ground = ground(&theme);
    let still = on_blue();
    let mut shot = Shot::new(effect(&still, front()).radius(RADIUS), CARD);
    shot.frame(Instant::now());
    let rest = shot.pixels(&theme, 2.0);
    let right = rest.width() - 1;
    assert!(near(rest.get_pixel(0, 0).0, ground), "cut by the radius");
    assert!(
        near(rest.get_pixel(right, 0).0, ground),
        "cut by the radius"
    );
    // Past the curve, along the top and down the side, the still.
    for (x, y) in [(0, 30), (30, 0), (right, 30), (right - 30, 0)] {
        assert!(
            !near(rest.get_pixel(x, y).0, ground),
            "the still at ({x}, {y})"
        );
    }
    // The still is taller than the card: cut at its foot, from its top,
    // and rounded there with the card.
    let foot = rest.height() - 1;
    for x in [0, right] {
        assert!(
            near(rest.get_pixel(x, foot).0, ground),
            "rounded at the foot"
        );
    }
    for (x, y) in [(0, foot - 30), (30, foot), (right, foot - 30)] {
        assert!(
            !near(rest.get_pixel(x, y).0, ground),
            "the still at ({x}, {y})"
        );
    }
    let blue = count(&rest, |pixel| near(pixel, BLUE));
    assert!(blue > rest.len() / 4 / 2, "mostly the still's ground");
    // Its petals, a pixel or two each, scaled by 4/3: how strong each shows
    // depends on where the scale's samples fall, so they're looked for
    // faint.
    let faint = |pixel: [u8; 4]| pixel[0] >= BLUE[0].saturating_add(40);
    assert!(count(&rest, faint) > 1_000, "and its petals");
}

#[test]
fn hovered_its_layers_draw_in_the_stills_place() {
    let theme = Theme::default();
    let ground = ground(&theme);
    let still = on_blue();
    let mut shot = Shot::new(effect(&still, front()).radius(RADIUS), CARD);
    let now = Instant::now();
    shot.frame(now);
    let rest = shot.pixels(&theme, 2.0);
    shot.point_at(Point::new(150.0, 250.0));
    shot.frame(now);
    let playing = shot.pixels(&theme, 2.0);
    assert_ne!(rest, playing);
    assert_eq!(
        count(&playing, |pixel| near(pixel, BLUE)),
        0,
        "the still is gone"
    );
    let petals = count(&playing, |pixel| petal(pixel, ground));
    assert!(petals > 100, "the intro's petals");
}

#[test]
fn opened_it_draws_nothing_while_it_waits_and_its_still_unwatched() {
    let theme = Theme::default();
    let ground = ground(&theme);
    let still = on_blue();
    let opened = |motion| effect(&still, motion).radius(RADIUS).play(Play::Open);
    let mut waiting = Shot::new(opened(front()), CARD);
    waiting.frame(Instant::now());
    let waiting = waiting.pixels(&theme, 1.0);
    assert!(
        waiting.pixels().all(|pixel| near(pixel.0, ground)),
        "nothing"
    );
    let mut behind = Shot::new(opened(Motion::default()), CARD);
    behind.frame(Instant::now());
    let behind = behind.pixels(&theme, 1.0);
    assert!(count(&behind, |pixel| near(pixel, BLUE)) > behind.len() / 4 / 2);
}

#[test]
fn opened_its_still_shows_once_a_layer_has_kept_it_waiting() {
    // A layer that never comes, as one over the download limit or one that
    // doesn't decode: nothing at first, as Discord's card, then the still,
    // never an empty card.
    let theme = Theme::default();
    let ground = ground(&theme);
    let mut stuck = on_blue();
    stuck.layers[1].picture = None;
    let mut shot = Shot::new(effect(&stuck, front()).play(Play::Open), CARD);
    let now = Instant::now();
    shot.frame(now);
    let waiting = shot.pixels(&theme, 1.0);
    assert!(
        waiting.pixels().all(|pixel| near(pixel.0, ground)),
        "nothing"
    );
    shot.frame(now + ms(5000));
    let waited = shot.pixels(&theme, 1.0);
    assert!(count(&waited, |pixel| near(pixel, BLUE)) > waited.len() / 4 / 2);
}

/// What a card's contents are here: one opaque colour, which neither the
/// still nor a petal is.
const CONTENT: [u8; 4] = [20, 120, 60, 255];

#[test]
fn opened_it_plays_over_the_cards_contents() {
    // As the card lays it, over its contents: the intro's petals over them
    // half a second after the card opened, and the loop's too a moment on.
    let theme = Theme::default();
    let petals = sample::petals();
    let [r, g, b, _] = CONTENT;
    let contents = container(space())
        .width(Fill)
        .height(Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Color::from_rgb8(r, g, b).into()),
            ..container::Style::default()
        });
    let card = stack![contents, effect(&petals, front()).play(Play::Open)];
    let mut shot = Shot::new(card, CARD);
    let now = Instant::now();
    shot.frame(now);
    let opened = shot.pixels(&theme, 1.0);
    assert!(opened.pixels().all(|pixel| near(pixel.0, CONTENT)), "waits");
    shot.frame(now + ms(600));
    let intro = shot.pixels(&theme, 1.0);
    assert!(
        count(&intro, |pixel| petal(pixel, CONTENT)) > 100,
        "over them"
    );
    shot.frame(now + ms(2100));
    let looping = shot.pixels(&theme, 1.0);
    assert_ne!(intro, looping, "a moment on, it moved");
    let pixels = looping.len() / 4;
    assert!(
        count(&looping, |pixel| near(pixel, CONTENT)) > pixels / 2,
        "and they under"
    );
}
