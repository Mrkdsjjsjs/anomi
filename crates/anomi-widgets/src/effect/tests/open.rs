//! As a card opens: it waits for every layer's picture and half a second
//! more, drawing nothing, then plays its intro and its loops for as long as
//! the card shows; unwatched or idle it rests, and goes on from where it
//! stood; another effect starts it over.

use super::*;

#[test]
fn opened_it_waits_for_its_layers_then_half_a_second() {
    let mut waiting = sample::petals();
    let picture = waiting.layers[1].picture.take();
    let mut widget = effect(&waiting, front())
        .play(Play::Open)
        .on_wanted("wanted");
    let mut rig = Rig::new(&widget);
    let now = Instant::now();
    // The loop's picture is on its way: nothing, not even the intro whose
    // picture came, and a frame when the wait would run out.
    assert_eq!(
        rig.frame(&mut widget, now),
        RedrawRequest::At(now + ms(5000))
    );
    assert_eq!(rig.look(), Look::Waiting);
    assert_eq!(rig.showing(), [false, false]);
    assert_eq!(rig.sent, ["wanted"]);
    rig.frame(&mut widget, now + ms(500));
    assert_eq!(rig.sent, ["wanted"], "once a play");
    waiting.layers[1].picture = picture;
    let mut widget = effect(&waiting, front()).play(Play::Open);
    assert_eq!(
        rig.frame(&mut widget, now + ms(1000)),
        RedrawRequest::At(now + ms(1500))
    );
    assert_eq!(rig.look(), Look::Waiting, "half a second more");
    let due = rig.frame(&mut widget, now + ms(1500));
    assert!(rig.playing());
    assert_eq!(rig.showing(), [true, false], "the intro from its start");
    assert_eq!(due, RedrawRequest::At(now + ms(1700)), "its next frame");
    rig.frame(&mut widget, now + ms(3100));
    assert_eq!(rig.showing(), [true, true], "and the loop 1.5 s on");
}

#[test]
fn unwatched_it_rests_and_wants_nothing() {
    let petals = sample::petals();
    let idle = Motion {
        idle: true,
        ..front()
    };
    let never = Motion {
        never: true,
        ..front()
    };
    for motion in [Motion::default(), idle, never] {
        let mut widget = effect(&petals, motion)
            .play(Play::Open)
            .on_wanted("wanted");
        let mut rig = Rig::new(&widget);
        let asked = rig.frame(&mut widget, Instant::now());
        assert_eq!(asked, RedrawRequest::Wait, "{motion:?}");
        assert_eq!(rig.look(), Look::Rest, "its still");
        assert!(rig.sent.is_empty());
    }
}

#[test]
fn idle_it_stops_and_used_again_goes_on() {
    let petals = sample::petals();
    let mut widget = effect(&petals, front()).play(Play::Open);
    let mut rig = Rig::new(&widget);
    let now = Instant::now();
    rig.frame(&mut widget, now);
    rig.frame(&mut widget, now + ms(500));
    assert!(rig.playing());
    let idle = Motion {
        idle: true,
        ..front()
    };
    let mut resting = effect(&petals, idle).play(Play::Open);
    assert_eq!(
        rig.frame(&mut resting, now + ms(16_000)),
        RedrawRequest::Wait
    );
    assert_eq!(rig.look(), Look::Rest);
    assert_eq!(rig.showing(), [false, false]);
    // A key: on from where it stood, with no new wait.
    rig.frame(&mut widget, now + ms(20_000));
    assert!(rig.playing());
    assert_eq!(rig.showing(), [true, false]);
}

#[test]
fn another_effect_starts_it_over() {
    let petals = sample::petals();
    let mut widget = effect(&petals, front()).play(Play::Open);
    let mut rig = Rig::new(&widget);
    let now = Instant::now();
    rig.frame(&mut widget, now);
    rig.frame(&mut widget, now + ms(2000));
    assert_eq!(rig.showing(), [true, true]);
    let mut other = sample::petals();
    other.layers[1].start = 500;
    let mut widget = effect(&other, front()).play(Play::Open);
    rig.frame(&mut widget, now + ms(2100));
    assert_eq!(rig.look(), Look::Waiting, "its own half second");
    assert_eq!(rig.showing(), [false, false]);
}
