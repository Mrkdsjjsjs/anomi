//! What each moving effect draws at chosen moments of its loop, and when
//! its frames are due: Discord's keyframes, read off `keyframes`.

use std::time::Duration;

use iced::Color;

use super::super::colours::Colours;
use super::super::keyframes::{self, FRAME};
use super::super::{Effect, Style};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn colours(effect: u8, colors: &[u32]) -> Colours {
    let style = Style::new(11, effect, colors).expect("a colour");
    Colours::of(&style, true).expect("colours")
}

fn prism_stops(given: &[Color], width: f32, at: Option<u32>) -> Vec<(f32, Color)> {
    keyframes::prism(given, width, at).iter().collect()
}

fn toon_stops(colours: &Colours, at: Option<u32>) -> Vec<(f32, Color)> {
    keyframes::toon(colours, at).iter().collect()
}

/// Whether two colours are the same but for rounding.
fn close(a: Color, b: Color) -> bool {
    [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
        .iter()
        .all(|(a, b)| (a - b).abs() < 1e-3)
}

/// Whether `stops` go from 0 to 1 in order, as many as a gradient holds.
fn in_order(stops: &[(f32, Color)]) -> bool {
    stops.len() >= 2
        && stops.len() <= 8
        && stops.first().is_some_and(|stop| stop.0 == 0.0)
        && stops.last().is_some_and(|stop| stop.0 == 1.0)
        && stops.windows(2).all(|pair| pair[0].0 < pair[1].0)
}

#[test]
fn each_effect_moves_within_its_span_and_rests_outside_it() {
    let frame = |effect, at| keyframes::frame(effect, 1, ms(at));
    // Neon flickers from 15% to 50% of 4 s.
    assert_eq!(frame(Effect::Neon, 0), 0);
    assert_eq!(frame(Effect::Neon, 599), 0);
    assert_eq!(frame(Effect::Neon, 600), 1);
    assert_eq!(frame(Effect::Neon, 633), 2);
    assert_eq!(frame(Effect::Neon, 2000), 0);
    assert_eq!(frame(Effect::Neon, 4600), 1, "the next loop");
    // Toon's bands run from 5% to 55%; pop hops from 0 to 50%.
    assert_eq!(frame(Effect::Toon, 199), 0);
    assert_eq!(frame(Effect::Toon, 200), 1);
    assert_eq!(frame(Effect::Toon, 2200), 0);
    assert_eq!(frame(Effect::Pop, 0), 1);
    assert_eq!(frame(Effect::Pop, 2100), 0);
    // A prism scrolls all the time, a loop every 2 s.
    let prism = |at| keyframes::frame(Effect::Prism, 2, ms(at));
    assert_eq!(prism(0), 1);
    assert_eq!(prism(1999), 1 + 1999 / FRAME);
    assert_eq!(prism(2000), 1);
    // What a frame draws.
    assert_eq!(keyframes::moment(Effect::Neon, 1, 0), None, "at rest");
    assert_eq!(keyframes::moment(Effect::Neon, 1, 1), Some(600));
    assert_eq!(keyframes::moment(Effect::Neon, 1, 2), Some(633));
    assert_eq!(keyframes::moment(Effect::Prism, 2, 1), Some(0));
}

#[test]
fn gradient_glow_solid_gummy_and_a_prism_of_one_colour_never_move() {
    for (effect, count) in [
        (Effect::Gradient, 2),
        (Effect::Glow, 2),
        (Effect::Solid, 1),
        (Effect::Gummy, 2),
        (Effect::Prism, 1),
        (Effect::Neon, 0),
    ] {
        assert!(!keyframes::moves(effect, count), "{effect:?}");
        assert_eq!(keyframes::frame(effect, count, ms(700)), 0);
        assert_eq!(keyframes::next(effect, count, ms(700)), None);
        assert_eq!(keyframes::moment(effect, count, 3), None);
    }
    for effect in [Effect::Prism, Effect::Neon, Effect::Toon, Effect::Pop] {
        assert!(keyframes::moves(effect, 2), "{effect:?}");
    }
}

#[test]
fn a_frame_asks_for_the_next_within_its_span_and_for_the_next_span_outside() {
    // A prism, at most a frame ahead, all the way round.
    let prism = |at| keyframes::next(Effect::Prism, 3, ms(at));
    for at in [0, 10, 990, 1990, 1999, 2000, 2050, 61_234] {
        let next = prism(at).expect("a prism moves");
        assert!(next > ms(at), "{at}: {next:?}");
        assert!(next - ms(at) <= ms(u64::from(FRAME)), "{at}: {next:?}");
    }
    let next = |effect, at| keyframes::next(effect, 1, ms(at));
    // Neon at 60% waits for the next loop's 15%; before 15%, for 15%.
    assert_eq!(next(Effect::Neon, 2400), Some(ms(4600)));
    assert_eq!(next(Effect::Neon, 100), Some(ms(600)));
    assert_eq!(next(Effect::Neon, 600), Some(ms(633)));
    // The span's end comes as a frame of its own, drawn at rest.
    assert_eq!(next(Effect::Neon, 1990), Some(ms(2000)));
    assert_eq!(next(Effect::Neon, 2000), Some(ms(4600)));
    assert_eq!(next(Effect::Toon, 3000), Some(ms(4200)));
    assert_eq!(next(Effect::Pop, 2100), Some(ms(4000)));
    assert_eq!(next(Effect::Pop, 8010), Some(ms(8033)));
}

#[test]
fn neon_flickers_to_its_light_shade_at_16_22_and_28_percent() {
    let colours = colours(3, &[0x00ff99]);
    assert_eq!(keyframes::neon(&colours, None), Color::WHITE, "at rest");
    for at in [0, 599, 720, 800, 920, 1000, 2000, 2040, 3999] {
        assert_eq!(keyframes::neon(&colours, Some(at)), Color::WHITE, "{at}");
    }
    for at in [640, 880, 1120] {
        let lit = keyframes::neon(&colours, Some(at));
        assert!(close(lit, colours.flicker), "{at}: {lit:?}");
    }
    // Between, on its way.
    let between = keyframes::neon(&colours, Some(660));
    assert!(
        !close(between, Color::WHITE) && !close(between, colours.flicker),
        "{between:?}"
    );
    // Back to white by 50%, eased: nearly there just before.
    let late = keyframes::neon(&colours, Some(1990));
    assert!(late.r > 0.97 && late.r < 1.0, "{late:?}");
}

#[test]
fn toon_shows_its_gradients_top_quarter_at_rest_and_ends_its_loop_so() {
    let colours = colours(4, &[0xffaa33]);
    let rest = toon_stops(&colours, None);
    let offsets = [0.0, 0.32, 0.6, 1.0];
    let shades = [Color::WHITE, colours.light2, colours.light1, colours.main];
    assert_eq!(rest.len(), 4, "{rest:?}");
    for (index, (offset, colour)) in rest.iter().enumerate() {
        assert!((offset - offsets[index]).abs() < 1e-4, "{rest:?}");
        assert_eq!(*colour, shades[index], "{rest:?}");
    }
    // Before 5% and after 55% it shows the same.
    for at in [0, 199, 2200, 3000, 3999] {
        let shown = toon_stops(&colours, Some(at));
        assert_eq!(shown.len(), 4, "{at}: {shown:?}");
        for ((offset, colour), (want, shade)) in shown.iter().zip(&rest) {
            assert!((offset - want).abs() < 1e-4, "{at}: {shown:?}");
            assert!(close(*colour, *shade), "{at}: {shown:?}");
        }
    }
    // Half way it shows the middle of the gradient, in at most eight stops.
    assert_ne!(toon_stops(&colours, Some(1200)), rest);
    for at in (0..4000).step_by(7) {
        let shown = toon_stops(&colours, Some(at));
        assert!(in_order(&shown), "{at}: {shown:?}");
    }
}

#[test]
fn pop_hops_up_then_presses_down_and_settles_by_half_way() {
    assert_eq!(keyframes::pop(None), 0.0);
    assert_eq!(keyframes::pop(Some(0)), 0.0);
    let up = keyframes::pop(Some(720));
    let down = keyframes::pop(Some(1400));
    assert!((up + 0.05).abs() < 1e-6, "up 0.05em: {up}");
    assert!((down - 0.08).abs() < 1e-6, "down 0.08em: {down}");
    assert_eq!(keyframes::pop(Some(2000)), 0.0);
    assert_eq!(keyframes::pop(Some(3000)), 0.0);
    let rising = keyframes::pop(Some(360));
    assert!(rising < 0.0 && rising > -0.05, "{rising}");
    assert!(keyframes::pop(Some(1300)) > 0.0);
}

#[test]
fn a_prism_scrolls_its_colours_left_a_cycle_every_two_seconds() {
    let red = Color::from_rgb(1.0, 0.0, 0.0);
    let blue = Color::from_rgb(0.0, 0.0, 1.0);
    let given = [red, blue];
    // At rest, and as its loop starts, round once across the name.
    let rest = vec![(0.0, red), (0.5, blue), (1.0, red)];
    assert_eq!(prism_stops(&given, 100.0, None), rest);
    assert_eq!(prism_stops(&given, 100.0, Some(0)), rest);
    // Half way, half a cycle on: blue at the left, red half way along.
    let half = vec![(0.0, blue), (0.5, red), (1.0, blue)];
    assert_eq!(prism_stops(&given, 100.0, Some(1000)), half);
    // A quarter on, the colours between stand at the ends: purple, blue a
    // quarter along, red three quarters, purple.
    let quarter = prism_stops(&given, 100.0, Some(500));
    let purple = Color::from_rgb(0.5, 0.0, 0.5);
    assert_eq!(quarter.len(), 4, "{quarter:?}");
    assert!(close(quarter[0].1, purple));
    assert!((quarter[1].0 - 0.25).abs() < 1e-6 && quarter[1].1 == blue);
    assert!((quarter[2].0 - 0.75).abs() < 1e-6 && quarter[2].1 == red);
    assert!(quarter[3].0 == 1.0 && close(quarter[3].1, purple));
    // A name narrower than 38 px shows only that much of a cycle.
    let narrow = vec![(0.0, red), (1.0, blue)];
    assert_eq!(prism_stops(&given, 19.0, None), narrow);
    // Five colours at any moment fit a gradient.
    let five = [red, blue, Color::WHITE, Color::BLACK, red];
    for width in [10.0, 38.0, 120.0] {
        for at in (0..2000).step_by(13) {
            let shown = prism_stops(&five, width, Some(at));
            assert!(in_order(&shown), "{width} {at}: {shown:?}");
        }
    }
}
