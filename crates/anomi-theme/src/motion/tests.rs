//! Where the window stands and what moves there, for each sight and
//! setting; the curves; the setting as saved.

use super::*;

#[test]
fn curves_start_and_end_in_place() {
    for curve in [arrive, glide] {
        assert!(curve(0.0).abs() < 1e-4);
        assert!((curve(1.0) - 1.0).abs() < 1e-4);
    }
    // Arriving is mostly done by half way; gliding is half done.
    assert!(arrive(0.5) > 0.85);
    assert!((glide(0.5) - 0.5).abs() < 1e-3);
}

/// The rule, for each sight and setting: whether what matters is live,
/// whether the member list is, and whether animations play unless they
/// are turned off.
const RULE: [(Sight, Background, bool, bool, bool); 9] = [
    (Sight::Front, Background::Usual, true, true, true),
    (Sight::Front, Background::Important, true, true, true),
    (Sight::Front, Background::Nothing, true, true, true),
    (Sight::Visible, Background::Usual, true, true, true),
    (Sight::Visible, Background::Important, true, false, false),
    (Sight::Visible, Background::Nothing, false, false, false),
    (Sight::Hidden, Background::Usual, false, false, false),
    (Sight::Hidden, Background::Important, false, false, false),
    (Sight::Hidden, Background::Nothing, false, false, false),
];

/// Every motion that stands at a sight: a hidden window with the focus
/// and one without.
fn standing(sight: Sight, background: Background, never: bool, idle: bool) -> Vec<Motion> {
    let focus_and_unseen: &[(bool, bool)] = match sight {
        Sight::Front => &[(true, false)],
        Sight::Visible => &[(false, false)],
        Sight::Hidden => &[(true, true), (false, true)],
    };
    focus_and_unseen
        .iter()
        .map(|&(front, unseen)| Motion {
            front,
            unseen,
            background,
            never,
            idle,
        })
        .collect()
}

const SETTINGS: [Background; 3] = [
    Background::Usual,
    Background::Important,
    Background::Nothing,
];

#[test]
fn each_sight_and_setting_keeps_what_the_rule_says() {
    for sight in [Sight::Front, Sight::Visible, Sight::Hidden] {
        for background in SETTINGS {
            let rows = RULE
                .iter()
                .filter(|row| row.0 == sight && row.1 == background)
                .count();
            assert_eq!(rows, 1, "{sight:?} under {background:?}");
        }
    }
    for (sight, background, live, members, plays) in RULE {
        for never in [false, true] {
            for idle in [false, true] {
                for motion in standing(sight, background, never, idle) {
                    let playing = plays && !never;
                    assert_eq!(motion.sight(), sight, "{motion:?}");
                    assert_eq!(motion.live(), live, "{motion:?}");
                    assert_eq!(motion.members_live(), members, "{motion:?}");
                    assert_eq!(motion.playing(), playing, "{motion:?}");
                    assert_eq!(motion.watched(), playing && !idle, "{motion:?}");
                    assert_eq!(motion.still(), !playing, "{motion:?}");
                    let slide = if playing { SLIDE } else { Duration::ZERO };
                    assert_eq!(motion.duration(SLIDE), slide, "{motion:?}");
                }
            }
        }
    }
}

#[test]
fn the_default_still_holds_still_and_front_still_plays() {
    let behind = Motion::default();
    assert_eq!(behind.sight(), Sight::Visible);
    assert_eq!(behind.background, Background::Nothing);
    assert!(!behind.live());
    assert!(!behind.members_live());
    assert!(behind.still());
    assert!(!behind.watched());
    assert_eq!(behind.duration(SLIDE), Duration::ZERO);
    // What every test and shot builds: in front, as before.
    let front = Motion {
        front: true,
        ..behind
    };
    assert_eq!(front.sight(), Sight::Front);
    assert!(front.live());
    assert!(front.watched());
    assert_eq!(front.duration(SLIDE), SLIDE);
    assert!(
        !Motion {
            never: true,
            ..front
        }
        .playing()
    );
    let idle = Motion {
        idle: true,
        ..front
    };
    assert!(idle.playing());
    assert!(!idle.watched());
}

#[test]
fn awake_wakes_only_what_idleness_held() {
    let front = Motion {
        front: true,
        ..Motion::default()
    };
    let idle = Motion {
        idle: true,
        ..front
    };
    assert!(idle.awake().watched(), "a speaking tile plays on");
    assert_eq!(front.awake(), front);
    for motion in [
        Motion {
            unseen: true,
            idle: true,
            ..front
        },
        Motion {
            never: true,
            idle: true,
            ..front
        },
        Motion {
            idle: true,
            ..Motion::default()
        },
    ] {
        assert!(!motion.awake().watched(), "{motion:?}");
    }
}

#[test]
fn the_setting_is_saved_in_lower_case() {
    assert_eq!(Background::default(), Background::Nothing);
    for (setting, saved) in SETTINGS.into_iter().zip(["usual", "important", "nothing"]) {
        let json = format!("\"{saved}\"");
        assert_eq!(serde_json::to_string(&setting).ok(), Some(json.clone()));
        let read: Option<Background> = serde_json::from_str(&json).ok();
        assert_eq!(read, Some(setting));
    }
}
