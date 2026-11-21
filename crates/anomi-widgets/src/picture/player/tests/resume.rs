//! What only decorates: a place made from the start begins again at its
//! first frame each time it resumes, and a capped one beyond the slots
//! holds its first frame.

use super::*;
use crate::picture::MOVING;

#[test]
fn from_the_start_it_resumes_at_its_first_frame_after_a_pause() {
    let still = moving(0);
    let mut player = Player::from_start();
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    let (shown, window) = (square(32.0), square(500.0));
    assert_eq!(
        player.update(&still, shown, &window, PLAYING, start),
        Some(at(100))
    );
    player.update(&still, shown, &window, PLAYING, at(150));
    assert_eq!(player.frame(), 1);
    // The pointer leaves its row: its first frame, and nothing asked.
    assert_eq!(
        player.update(&still, shown, &window, Motion::default(), at(160)),
        None
    );
    assert_eq!(player.frame(), 0);
    // Back a minute later: from its first frame, not from frame 1.
    assert_eq!(
        player.update(&still, shown, &window, PLAYING, at(60_000)),
        Some(at(60_100))
    );
    assert_eq!(player.frame(), 0);
    player.update(&still, shown, &window, PLAYING, at(60_100));
    assert_eq!(player.frame(), 1);
    forget(&still);
}

#[test]
fn from_the_start_it_resumes_after_a_gap_it_wasnt_updated_in() {
    let still = moving(0);
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    let (shown, window) = (square(32.0), square(500.0));
    // A hover's top: updated while the pointer is on it, then not at all.
    let mut top = Player::from_start();
    // Something else plays throughout, so the shared clock runs.
    let mut sticker = Player::default();
    top.update(&still, shown, &window, PLAYING, start);
    sticker.update(&still, shown, &window, PLAYING, start);
    top.update(&still, shown, &window, PLAYING, at(150));
    assert_eq!(top.frame(), 1);
    for millis in (200..=1000).step_by(100) {
        sticker.update(&still, shown, &window, PLAYING, at(millis));
    }
    assert_eq!(sticker.frame(), 1, "the clock's frame at a second");
    // Its frame was due at 200; at 1000 it comes back to its first.
    assert_eq!(
        top.update(&still, shown, &window, PLAYING, at(1000)),
        Some(at(1100))
    );
    assert_eq!(top.frame(), 0);
    // Updated on time, it goes on.
    top.update(&still, shown, &window, PLAYING, at(1100));
    assert_eq!(top.frame(), 1);
    forget(&still);
}

#[test]
fn from_the_start_one_holding_its_last_frame_plays_again_after_a_second_away() {
    let still = moving(1);
    let mut player = Player::from_start();
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    let (shown, window) = (square(32.0), square(500.0));
    player.update(&still, shown, &window, PLAYING, start);
    assert_eq!(
        player.update(&still, shown, &window, PLAYING, at(320)),
        None
    );
    assert_eq!(player.frame(), 2, "held on its last frame");
    // A redraw soon after keeps it held.
    player.update(&still, shown, &window, PLAYING, at(900));
    assert_eq!(player.frame(), 2);
    assert_eq!(
        player.update(&still, shown, &window, PLAYING, at(2_500)),
        Some(at(2_600)),
        "away a second and more: round again"
    );
    assert_eq!(player.frame(), 0);
    forget(&still);
}

#[test]
fn a_capped_picture_beyond_the_slots_holds_its_first_frame() {
    let stills: Vec<_> = (0..=MOVING).map(|_| moving(0)).collect();
    let mut players: Vec<_> = stills
        .iter()
        .map(|_| Player::from_start().capped())
        .collect();
    let start = Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    let (shown, window) = (square(32.0), square(500.0));
    let mut frame = |millis| -> Vec<Option<Instant>> {
        players
            .iter_mut()
            .zip(&stills)
            .map(|(player, still)| player.update(still, shown, &window, PLAYING, at(millis)))
            .collect()
    };
    let first = frame(0);
    assert!(first[..MOVING].iter().all(|due| *due == Some(at(100))));
    assert_eq!(first[MOVING], None, "beyond the slots: it asks for nothing");
    let next = frame(150);
    assert!(next[..MOVING].iter().all(|due| *due == Some(at(200))));
    assert_eq!(next[MOVING], None);
    assert!(players[..MOVING].iter().all(|player| player.frame() == 1));
    assert_eq!(players[MOVING].frame(), 0, "its first frame");
    // An uncapped one plays beside them.
    let sticker = moving(0);
    let mut free = Player::from_start();
    assert_eq!(
        free.update(&sticker, shown, &window, PLAYING, at(150)),
        Some(at(250))
    );
    for still in stills.iter().chain([&sticker]) {
        forget(still);
    }
}
