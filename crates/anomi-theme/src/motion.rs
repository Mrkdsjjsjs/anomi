//! How the interface moves: the durations and easings of `Theme` and `Popup`
//! in `ui/theme.slint`, and the switch that holds everything still.
//!
//! [`Motion`] knows where the window stands, its [`Sight`]:
//!
//! - **Front**: it has the keyboard focus and the desktop paints it.
//! - **Visible**: painted but not focused, such as on a second monitor
//!   beside a game.
//! - **Hidden**: not painted, focused or not: in the tray, minimised,
//!   covered, on another workspace, or with the screen off.
//!
//! What asks for attention follows the focus alone, `Motion::front`: marking
//! read, notifications, the caret, popups kept open. What keeps the window
//! alive follows its sight and the [`Background`] setting:
//!
//! - [`Motion::live`]: clocks, video, cameras, the typing line and previews;
//!   in front, or visible unless the setting is `Nothing`.
//! - [`Motion::members_live`]: the member list's presence; in front, or
//!   visible under `Usual`.
//! - [`Motion::playing`]: transitions and loops; as the member list, unless
//!   animations are turned off. [`Motion::watched`] also stops the loops
//!   15 seconds after the last input.
//!
//! Hidden, nothing is live and nothing moves, whatever the setting: a window
//! behind a game draws a change once rather than for every frame of its
//! transition. Every animation takes its duration from [`Motion`], so a
//! still interface gets zero-length ones.
//!
//! What opens over the window arrives and leaves by [`Popup`]
//! (`PopupMotion` and `PopupEntry` in `ui/theme.slint:216`, `:234`).

mod popup;

pub use popup::{Pace, Popup};

use std::time::Duration;

use iced::animation::{Animation, Easing};

/// A hover fill, a pressed button, a small toggle, a colour that tells a state.
pub const QUICK: Duration = Duration::from_millis(120);
/// What stays on screen and moves or turns: a tab's underline, a selection
/// pill, a chevron, a small bar changing size. With [`GLIDE`].
pub const SLIDE: Duration = Duration::from_millis(180);
/// A highlight that fades once seen, such as the message a jump lands on.
pub const LINGER: Duration = Duration::from_millis(400);
/// A menu, a popover, a picker or a tooltip arriving.
pub const POPUP: Duration = Duration::from_millis(150);
/// A dialog, a sheet, a viewer or the settings arriving.
pub const DIALOG: Duration = Duration::from_millis(220);
/// Any of them leaving.
pub const LEAVE: Duration = Duration::from_millis(120);

/// Arriving, and answering a click: quick at first, then settling;
/// `cubic-bezier(0.22, 1, 0.36, 1)`.
pub const ARRIVE: Easing = Easing::Custom(arrive);
/// Moving while staying on screen: starts and stops gently; CSS `ease-in-out`.
pub const GLIDE: Easing = Easing::Custom(glide);

/// What a window on screen but not focused keeps doing: the setting, saved
/// in layout.json as `"usual"`, `"important"` or `"nothing"`. A hidden window
/// does nothing under any of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    /// Everything, as in front.
    Usual,
    /// What matters: clocks, video, cameras, the typing line and previews.
    /// Transitions land at once and the member list waits for the focus.
    Important,
    /// Nothing: the window rests as one behind a game.
    #[default]
    Nothing,
}

/// Where the window stands, from [`Motion::sight`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// Focused and painted.
    Front,
    /// Painted, not focused.
    Visible,
    /// Not painted, focused or not.
    Hidden,
}

/// Whether things may move and the window stays live, from its focus,
/// whether the desktop paints it, the settings and how lately it was used.
/// Copy it into what animates or ticks.
///
/// `Motion::default()` is a window visible but not focused, under
/// [`Background::Nothing`]: nothing is live and nothing moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motion {
    /// The window has the keyboard focus: what asks attention follows it.
    pub front: bool,
    /// The desktop doesn't paint the window: in the tray, minimised,
    /// covered, on another workspace, the screen off.
    pub unseen: bool,
    /// The setting as it applies here (`Nothing` where the desktop can't
    /// say whether a game covers the window and nobody chose).
    pub background: Background,
    /// Animations are turned off in the settings.
    pub never: bool,
    /// Nobody used the window in the last 15 seconds.
    pub idle: bool,
}

impl Motion {
    /// Where the window stands: hidden while unpainted, focused or not;
    /// else in front with the focus, and visible without it.
    pub fn sight(self) -> Sight {
        if self.unseen {
            Sight::Hidden
        } else if self.front {
            Sight::Front
        } else {
            Sight::Visible
        }
    }

    /// What matters keeps going: clocks, video, cameras, the typing line,
    /// previews. In front, or visible unless the setting is `Nothing`.
    pub fn live(self) -> bool {
        match self.sight() {
            Sight::Front => true,
            Sight::Visible => self.background != Background::Nothing,
            Sight::Hidden => false,
        }
    }

    /// The member list's presence follows as it changes: in front, or
    /// visible under `Usual`. Elsewhere its changes wait, and show at once
    /// when it follows again.
    pub fn members_live(self) -> bool {
        self.as_front()
    }

    /// Animations play: in front, or visible under `Usual`, and they aren't
    /// turned off.
    pub fn playing(self) -> bool {
        !self.never && self.as_front()
    }

    /// Someone may be looking: what only decorates, such as a skeleton's
    /// sweep, moves only then.
    pub fn watched(self) -> bool {
        self.playing() && !self.idle
    }

    /// Nothing moves: every duration is zero.
    pub fn still(self) -> bool {
        !self.playing()
    }

    /// A token's duration, or zero while still.
    pub fn duration(self, token: Duration) -> Duration {
        if self.still() { Duration::ZERO } else { token }
    }

    /// An animation of a token's length with an easing, which lands at once
    /// while still.
    pub fn animation<T>(self, initial: T, token: Duration, easing: Easing) -> Animation<T>
    where
        T: Clone + Copy + PartialEq + iced::animation::Float,
    {
        Animation::new(initial)
            .duration(self.duration(token))
            .easing(easing)
    }

    /// A popup's way in or out as an iced [`Animation`], for one kept in the
    /// app's state: [`POPUP`] or [`DIALOG`] in, [`LEAVE`] out. Its duration
    /// is the one the move starts with; a popup that may turn round half way
    /// keeps a [`Popup`], which changes pace as it turns.
    pub fn popup(self, open: bool, dialog: bool) -> Animation<bool> {
        let token = match (open, dialog) {
            (false, _) => LEAVE,
            (true, true) => DIALOG,
            (true, false) => POPUP,
        };
        Animation::new(!open)
            .duration(self.duration(token))
            .easing(ARRIVE)
    }

    /// The whole window runs as in front: it is, or it is visible under
    /// `Usual`.
    fn as_front(self) -> bool {
        match self.sight() {
            Sight::Front => true,
            Sight::Visible => self.background == Background::Usual,
            Sight::Hidden => false,
        }
    }
}

/// A popup's card scale for how far it's open, from 0 to 1: it grows from 96%.
pub fn popup_scale(amount: f32) -> f32 {
    0.96 + 0.04 * amount
}

/// How far from its place what slides in starts, in logical pixels.
pub fn popup_shift(amount: f32) -> f32 {
    (1.0 - amount) * 6.0
}

fn arrive(x: f32) -> f32 {
    cubic_bezier(0.22, 1.0, 0.36, 1.0, x)
}

fn glide(x: f32) -> f32 {
    cubic_bezier(0.42, 0.0, 0.58, 1.0, x)
}

/// CSS `cubic-bezier`: solves x(t) = x by Newton's method and returns y(t).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let curve = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let slope = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
    };
    let mut t = x;
    for _ in 0..8 {
        let error = curve(x1, x2, t) - x;
        let rate = slope(x1, x2, t);
        if error.abs() < 1e-5 || rate.abs() < 1e-6 {
            break;
        }
        t = (t - error / rate).clamp(0.0, 1.0);
    }
    curve(y1, y2, t)
}

#[cfg(test)]
mod tests {
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
    fn the_setting_is_saved_in_lower_case() {
        assert_eq!(Background::default(), Background::Nothing);
        for (setting, saved) in SETTINGS.into_iter().zip(["usual", "important", "nothing"]) {
            let json = format!("\"{saved}\"");
            assert_eq!(serde_json::to_string(&setting).ok(), Some(json.clone()));
            let read: Option<Background> = serde_json::from_str(&json).ok();
            assert_eq!(read, Some(setting));
        }
    }
}
