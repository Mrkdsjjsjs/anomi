//! How Discord's animated name effects move, from its client's CSS
//! (`163466.71f5984e9faac7fc.css`: the `.animated_or4Hj-` rules and their
//! `@keyframes`): what each draws a given time into its loop. An effect
//! loops for as long as its name is shown moving, from its look at rest,
//! and moves only within a span of the loop, holding that look outside it:
//!
//! - prism's colours scroll left a cycle every 2 s, all the time;
//! - neon's white letters flicker to a light shade of the colour three
//!   times, from 15% to 50% of 4 s;
//! - toon's bands run down a gradient four times the name's height, from 5%
//!   to 55% of 4 s, and end as they began;
//! - pop's white letters hop up, then press down into their copy, from 0 to
//!   50% of 4 s. The copy is the letters' `:before` in the client, so it
//!   moves with them, and its own keyframes move it back by as much: it
//!   stays where it is.
//!
//! Each step from one keyframe to the next is eased as the client's
//! `cubic-bezier` says. A moving name is drawn a frame at a time, [`FRAME`]
//! apart from the start of the span, so its outlines are made again 30
//! times a second at most within the span and not at all outside it.

use std::time::Duration;

use iced::Color;

use anomi_theme::motion::cubic_bezier;

use super::colours::Colours;
use super::style::Effect;

/// How far apart a moving name's frames are, in milliseconds: 30 a second.
pub(super) const FRAME: u32 = 33;

/// Prism's loop, in milliseconds: `prism-scroll` takes 2 s.
const PRISM: u32 = 2000;
/// Neon's, toon's and pop's loop: 4 s.
const LOOP: u32 = 4000;
/// Prism's shortest cycle, in logical pixels: the client's `max(Nch, 38px)`.
const PRISM_CYCLE: f32 = 38.0;

/// An effect's loop, in milliseconds: how long it is, and the span of it in
/// which the effect moves.
#[derive(Clone, Copy)]
struct Loop {
    period: u32,
    from: u32,
    to: u32,
}

/// The loop of `effect` drawn with `count` colours; none for one that never
/// moves: gradient and glow, as in the client, solid, gummy (not yet), and a
/// prism of one colour, which is drawn solid.
fn looped(effect: Effect, count: usize) -> Option<Loop> {
    let (period, from, to) = match effect {
        Effect::Prism if count > 1 => (PRISM, 0, PRISM),
        Effect::Neon if count > 0 => (LOOP, 600, 2000),
        Effect::Toon if count > 0 => (LOOP, 200, 2200),
        Effect::Pop if count > 0 => (LOOP, 0, 2000),
        _ => return None,
    };
    Some(Loop { period, from, to })
}

/// Whether `effect` with `count` colours moves while it's shown moving.
pub(super) fn moves(effect: Effect, count: usize) -> bool {
    looped(effect, count).is_some()
}

/// `at` in whole milliseconds: where its loop started, and how far into it
/// it is.
fn split(at: Duration, period: u32) -> (u64, u32) {
    let ms = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
    let phase = (ms % u64::from(period)) as u32;
    (ms - u64::from(phase), phase)
}

/// The frame drawn `at` into the effect's animation: 0, its look at rest,
/// outside the span in which it moves, and 1 on within the span.
pub(super) fn frame(effect: Effect, count: usize, at: Duration) -> u32 {
    let Some(looped) = looped(effect, count) else {
        return 0;
    };
    let (_, phase) = split(at, looped.period);
    if (looped.from..looped.to).contains(&phase) {
        1 + (phase - looped.from) / FRAME
    } else {
        0
    }
}

/// How far into its loop `frame` is drawn, in milliseconds; none for frame
/// 0, the look at rest.
pub(super) fn moment(effect: Effect, count: usize, frame: u32) -> Option<u32> {
    let looped = looped(effect, count)?;
    let step = frame.checked_sub(1)?;
    let at = looped.from.saturating_add(step.saturating_mul(FRAME));
    Some(at.min(looped.to))
}

/// When, from the animation's start, the frame after the one drawn `at` is
/// due: the next one within the span, or the span's end, drawn at rest, or
/// else the start of the next span. None for an effect that never moves.
pub(super) fn next(effect: Effect, count: usize, at: Duration) -> Option<Duration> {
    let looped = looped(effect, count)?;
    let (start, phase) = split(at, looped.period);
    let next = if phase < looped.from {
        looped.from
    } else if phase < looped.to {
        let step = (phase - looped.from) / FRAME + 1;
        (looped.from + step * FRAME).min(looped.to)
    } else {
        looped.period + looped.from
    };
    Some(Duration::from_millis(start.saturating_add(u64::from(next))))
}

/// A CSS animation's value `at` milliseconds in: its keyframes, (time,
/// value) in order of time, each step to the next eased by `ease`. It holds
/// its last value past the last keyframe.
fn through(keyframes: &[(u32, f32)], at: u32, ease: fn(f32) -> f32) -> f32 {
    let mut value = keyframes.first().map_or(0.0, |&(_, value)| value);
    for pair in keyframes.windows(2) {
        let &[(from, a), (to, b)] = pair else {
            continue;
        };
        if at < to {
            let length = to.saturating_sub(from).max(1);
            let progress = at.saturating_sub(from) as f32 / length as f32;
            return a + (b - a) * ease(progress);
        }
        value = b;
    }
    value
}

/// Neon's steps: `neon-flicker-animation 4s cubic-bezier(.24,.31,.36,.93)`.
fn flickering(x: f32) -> f32 {
    cubic_bezier(0.24, 0.31, 0.36, 0.93, x)
}

/// Toon's and pop's steps: `4s cubic-bezier(.44,.29,.48,1)`.
fn settle(x: f32) -> f32 {
    cubic_bezier(0.44, 0.29, 0.48, 1.0, x)
}

/// `a` turned toward `b` by `amount`, channel by channel, as CSS mixes the
/// colours of a gradient and of an animation.
fn mix(a: Color, b: Color, amount: f32) -> Color {
    let channel = |a: f32, b: f32| a + (b - a) * amount;
    Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: channel(a.a, b.a),
    }
}

/// A gradient's stops, offset and colour in order, as many as iced's
/// gradient holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Stops {
    stops: [(f32, Color); 8],
    len: usize,
}

impl Stops {
    fn push(&mut self, offset: f32, color: Color) {
        if let Some(slot) = self.stops.get_mut(self.len) {
            *slot = (offset, color);
            self.len += 1;
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (f32, Color)> + '_ {
        self.stops[..self.len].iter().copied()
    }
}

/// Prism's stops across a name `width` wide, `at` milliseconds into its
/// scroll, or at rest. Its colours go round once a cycle, back to the
/// first, the cycle being the name's width or [`PRISM_CYCLE`] if that's
/// more; they repeat, and scroll left a cycle every 2 s. The name shows a
/// cycle at most, the end of one and the start of the next, so one
/// gradient of at most seven stops draws it, as iced's gradient doesn't
/// repeat and its frame's clip doesn't clip on the processor. At rest, and
/// at the start of the loop, a name at least that wide shows one cycle
/// across its width.
pub(super) fn prism(given: &[Color], width: f32, at: Option<u32>) -> Stops {
    let mut stops = Stops::default();
    let Some(&first) = given.first() else {
        return stops;
    };
    let count = given.len();
    let cycle = width.max(PRISM_CYCLE);
    // How much of a cycle the name shows, and where it starts.
    let shown = width.max(0.0) / cycle;
    let start = at.map_or(0.0, |at| (at % PRISM) as f32 / PRISM as f32);
    let colour = |point: f32| {
        let place = point.rem_euclid(1.0) * count as f32;
        let index = (place.floor() as usize).min(count - 1);
        let next = given.get((index + 1) % count).copied().unwrap_or(first);
        mix(given[index], next, place - place.floor())
    };
    stops.push(0.0, colour(start));
    if shown > 0.0 {
        // Each colour's place past the start, until the end: a stop
        // within a hair of the end is the end's own colour.
        let mut stop = (start * count as f32).floor() as usize + 1;
        loop {
            let offset = (stop as f32 / count as f32 - start) / shown;
            if offset.is_nan() || offset >= 1.0 - 1e-4 {
                break;
            }
            stops.push(offset, given[stop % count]);
            stop += 1;
        }
    }
    stops.push(1.0, colour(start + shown));
    stops
}

/// Neon's letters `at` milliseconds into its loop, or at rest:
/// `neon-flicker-animation`, white but for its light shade
/// (`Colours::flicker`) at 16%, 22% and 28% of the loop.
pub(super) fn neon(colours: &Colours, at: Option<u32>) -> Color {
    /// How much of the shade, from white: (time, amount).
    const SHADE: [(u32, f32); 12] = [
        (0, 0.0),
        (600, 0.0),
        (640, 1.0),
        (720, 0.0),
        (800, 0.0),
        (880, 1.0),
        (920, 0.0),
        (1000, 0.0),
        (1120, 1.0),
        (2000, 0.0),
        (2040, 0.0),
        (LOOP, 0.0),
    ];
    let Some(at) = at else {
        return Color::WHITE;
    };
    let shade = through(&SHADE, at, flickering);
    mix(Color::WHITE, colours.flicker, shade)
}

/// A colour of toon's gradient.
#[derive(Clone, Copy)]
enum Shade {
    White,
    Light2,
    Light1,
    Main,
}

/// Toon's gradient, sized four times the name's height
/// (`.toon_or4Hj-:before`): where each colour stands, from its top.
const TOON: [(f32, Shade); 10] = [
    (0.0, Shade::White),
    (0.08, Shade::Light2),
    (0.15, Shade::Light1),
    (0.25, Shade::Main),
    (0.45, Shade::Light2),
    (0.55, Shade::Main),
    (0.75, Shade::White),
    (0.83, Shade::Light2),
    (0.90, Shade::Light1),
    (1.0, Shade::Main),
];

/// The share of toon's gradient a name shows.
const QUARTER: f32 = 0.25;

/// Toon's stops down the name `at` milliseconds into its loop, or at rest.
/// The name shows a quarter of the gradient (`TOON`), which `toon-animation`
/// moves from its top quarter to its bottom one between 5% and 55% of the
/// loop; the two show the same colours, white to the main colour, so the
/// loop ends as it began.
pub(super) fn toon(colours: &Colours, at: Option<u32>) -> Stops {
    /// The gradient's `background-position-y`, from 0 to 1: (time, place).
    const DOWN: [(u32, f32); 4] = [(0, 0.0), (200, 0.0), (2200, 1.0), (LOOP, 1.0)];
    let shade = |shade: Shade| match shade {
        Shade::White => Color::WHITE,
        Shade::Light2 => colours.light2,
        Shade::Light1 => colours.light1,
        Shade::Main => colours.main,
    };
    // The gradient's colour at `point`, from its top: at a stop, that
    // stop's own.
    let along = |point: f32| {
        let index = TOON
            .iter()
            .rposition(|&(offset, _)| offset <= point)
            .unwrap_or(0);
        let (offset, from) = TOON[index];
        match TOON.get(index + 1) {
            Some(&(next, to)) if point > offset => {
                mix(shade(from), shade(to), (point - offset) / (next - offset))
            }
            _ => shade(from),
        }
    };
    let down = at.map_or(0.0, |at| through(&DOWN, at, settle));
    let top = (1.0 - QUARTER) * down;
    let mut stops = Stops::default();
    stops.push(0.0, along(top));
    for &(offset, kind) in &TOON {
        if offset > top && offset < top + QUARTER {
            stops.push((offset - top) / QUARTER, shade(kind));
        }
    }
    stops.push(1.0, along(top + QUARTER));
    stops
}

/// How far down pop's white letters stand from their place `at`
/// milliseconds into its loop, in ems: `pop-animation-main`, up 0.05em at
/// 18%, down 0.08em into their copy at 35%, back in place by 50%. In place
/// at rest.
pub(super) fn pop(at: Option<u32>) -> f32 {
    const LIFT: [(u32, f32); 5] = [
        (0, 0.0),
        (720, -0.05),
        (1400, 0.08),
        (2000, 0.0),
        (LOOP, 0.0),
    ];
    at.map_or(0.0, |at| through(&LIFT, at, settle))
}
