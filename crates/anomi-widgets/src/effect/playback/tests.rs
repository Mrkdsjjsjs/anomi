//! A play's clock: which layers show when, how it starts over, waits and
//! stands still.

use super::*;

/// The fake Discord's "Petals": an intro once, and a loop that waits a
/// second between rounds.
fn petals() -> [EffectLayer; 2] {
    [
        EffectLayer {
            duration: 2000,
            ..EffectLayer::default()
        },
        EffectLayer {
            y: 20.0,
            start: 1500,
            duration: 3000,
            looped: true,
            loop_delay: 1000,
            ..EffectLayer::default()
        },
    ]
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A play over at `end` unless it `loops`, starting at once.
fn course(end: u64, loops: bool) -> Course {
    Course {
        end,
        loops,
        delay: Duration::ZERO,
    }
}

/// Petals' play as a card opens: after its delay.
const OPENED: Course = Course {
    end: 4500,
    loops: true,
    delay: DELAY,
};

#[test]
fn a_layer_shows_from_its_start_for_its_duration() {
    let [intro, _] = petals();
    assert!(shown(&intro, 0));
    assert!(shown(&intro, 2000));
    assert!(!shown(&intro, 2001));
    assert_eq!(next_change(&intro, 0), Some(2001));
    assert_eq!(next_change(&intro, 2001), None);
}

#[test]
fn a_looping_layer_waits_its_delay_between_rounds() {
    let [_, round] = petals();
    assert!(!shown(&round, 1499));
    assert_eq!(next_change(&round, 0), Some(1500));
    assert!(shown(&round, 1500));
    assert!(shown(&round, 4500));
    assert!(!shown(&round, 4501), "the delay");
    assert_eq!(next_change(&round, 4000), Some(4501));
    assert_eq!(next_change(&round, 4600), Some(5500));
    assert!(shown(&round, 5500), "round again");
}

#[test]
fn a_loop_without_a_delay_shows_for_good() {
    let layer = EffectLayer {
        duration: 500,
        looped: true,
        ..EffectLayer::default()
    };
    assert!(shown(&layer, 10_000));
    assert_eq!(next_change(&layer, 10), None);
}

#[test]
fn an_effect_plays_on_hover_once_then_holds_still() {
    let layers = [petals()[0].clone()];
    let once = course(2000, false);
    let start = Instant::now();
    let mut play = Playback::default();
    assert!(!play.frame(false, true, true, once, start));
    assert!(play.frame(true, true, true, once, start));
    assert_eq!(play.next(&layers, once, start), Some(start + ms(2001)));
    assert!(play.frame(true, true, true, once, start + ms(1200)));
    assert_eq!(play.at(), 1200);
    assert!(!play.frame(true, true, true, once, start + ms(2100)), "over");
    assert_eq!(play.look(), Look::Rest);
    assert_eq!(play.next(&layers, once, start + ms(2100)), None);
    assert!(
        !play.frame(true, true, true, once, start + ms(9000)),
        "held still"
    );
    // The pointer leaves and comes back: it plays from the start again.
    assert!(!play.frame(false, true, true, once, start + ms(9100)));
    assert!(play.frame(true, true, true, once, start + ms(9200)));
    assert_eq!(play.at(), 0);
}

#[test]
fn a_looping_effect_plays_while_hovered() {
    let layers = petals();
    let looping = course(4500, true);
    let start = Instant::now();
    let mut play = Playback::default();
    assert!(play.frame(true, true, true, looping, start));
    assert_eq!(play.next(&layers, looping, start), Some(start + ms(1500)));
    assert!(play.frame(true, true, true, looping, start + ms(60_000)));
    assert_eq!(play.at(), 60_000);
}

#[test]
fn behind_another_window_the_play_stands_still() {
    let once = course(2000, false);
    let start = Instant::now();
    let mut play = Playback::default();
    assert!(play.frame(true, true, true, once, start));
    assert!(play.frame(true, true, true, once, start + ms(500)));
    assert!(!play.frame(true, false, true, once, start + ms(600)));
    assert_eq!(play.look(), Look::Rest);
    assert_eq!(play.next(&petals(), once, start + ms(600)), None);
    // In front again a minute later: it goes on from where it stopped.
    assert!(play.frame(true, true, true, once, start + ms(60_600)));
    assert!(play.frame(true, true, true, once, start + ms(60_700)));
    assert_eq!(play.at(), 600);
}

#[test]
fn the_next_frame_lands_on_the_millisecond_a_layer_changes() {
    let layers = petals();
    let looping = course(4500, true);
    let start = Instant::now();
    let later = start + Duration::from_micros(1_499_600);
    let mut play = Playback::default();
    play.frame(true, true, true, looping, start);
    play.frame(true, true, true, looping, later);
    assert_eq!(play.next(&layers, looping, later), Some(start + ms(1500)));
}

#[test]
fn opened_it_waits_for_its_pictures_then_half_a_second() {
    let layers = petals();
    let start = Instant::now();
    let at = |millis| start + ms(millis);
    let mut play = Playback::default();
    // Its pictures are on their way: nothing shows, and a frame is due
    // when its wait would run out.
    assert!(!play.frame(true, true, false, OPENED, start));
    assert_eq!(play.look(), Look::Waiting);
    assert!(play.pending());
    assert_eq!(play.next(&layers, OPENED, start), Some(at(5000)));
    assert!(!play.frame(true, true, false, OPENED, at(1000)));
    assert_eq!(play.next(&layers, OPENED, at(1000)), Some(at(5000)));
    // They came: half a second more of nothing.
    assert!(!play.frame(true, true, true, OPENED, at(2000)));
    assert_eq!(play.look(), Look::Waiting);
    assert_eq!(play.at(), 0);
    assert_eq!(play.next(&layers, OPENED, at(2000)), Some(at(2500)));
    assert!(!play.frame(true, true, true, OPENED, at(2300)));
    // Then the intro, from its start, and the loop 1.5 s on.
    assert!(play.frame(true, true, true, OPENED, at(2500)));
    assert_eq!(play.at(), 0);
    assert_eq!(play.next(&layers, OPENED, at(2500)), Some(at(4000)));
    assert!(play.frame(true, true, true, OPENED, at(4100)));
    assert_eq!(play.at(), 1600);
}

#[test]
fn pictures_that_dont_come_leave_it_at_rest_until_they_do() {
    let layers = petals();
    let start = Instant::now();
    let at = |millis| start + ms(millis);
    let mut play = Playback::default();
    play.frame(true, true, false, OPENED, start);
    assert!(!play.frame(true, true, false, OPENED, at(5000)));
    assert_eq!(play.look(), Look::Rest, "its still, after five seconds");
    assert_eq!(play.next(&layers, OPENED, at(5000)), None);
    // They come at last: still at rest through its delay, then it plays.
    assert!(!play.frame(true, true, true, OPENED, at(6000)));
    assert_eq!(play.look(), Look::Rest);
    assert_eq!(play.next(&layers, OPENED, at(6000)), Some(at(6500)));
    assert!(play.frame(true, true, true, OPENED, at(6500)));
}

#[test]
fn idle_its_play_stops_and_goes_on_after() {
    let start = Instant::now();
    let at = |millis| start + ms(millis);
    let mut play = Playback::default();
    play.frame(true, true, true, OPENED, start);
    assert!(play.frame(true, true, true, OPENED, at(1000)));
    assert_eq!(play.at(), 500);
    // Nobody used the window for a while: it may not, and rests.
    assert!(!play.frame(true, false, true, OPENED, at(1200)));
    assert_eq!(play.look(), Look::Rest);
    assert!(!play.pending());
    // Used again: no new delay, on from where it stood.
    assert!(play.frame(true, true, true, OPENED, at(30_000)));
    assert!(play.frame(true, true, true, OPENED, at(30_100)));
    assert_eq!(play.at(), 600);
}

#[test]
fn a_restart_waits_again() {
    let start = Instant::now();
    let mut play = Playback::default();
    play.frame(true, true, true, OPENED, start);
    assert!(play.frame(true, true, true, OPENED, start + ms(800)));
    play.restart();
    assert!(!play.frame(true, true, true, OPENED, start + ms(900)));
    assert_eq!(play.look(), Look::Waiting);
    assert_eq!(play.at(), 0);
}
