//! A participant's menu: what it shows and what it asks for. The Slint side
//! is `VolumeMenu` (`ui/voice.slint:1594`) and the window's `voice-menu`
//! popup (`ui/yapfast.slint:2686`); the rows under it are the window's
//! context menu (`vm.menu`), built by `src/ui/menus.rs` for a target named
//! `in-voice:<room>/<user>` (Profile, Mention, Message, Start call, notes,
//! the mute boxes, Invite, Move to, server mute, Disconnect), whose choices
//! send `menu::Intent::Choose`.
//!
//! This module is the card on top of those rows: the person's name and their
//! volumes, and in a Discord call how strongly their noise and this
//! account's echo are taken out of their voice, while each is (Anomi's own;
//! `docs/discord/voice.md`). A stream watched has the same card over the
//! menu of its tile ([`ParticipantMenu::stream`]), as Discord's menu of a
//! stream starts: under its name "Stop Watching", a line, the box "Mute"
//! ([`ParticipantMenu::rows`]), then its volume, "Stream volume", and how
//! strongly its sound is cleaned. The app opens `Layer::ParticipantMenu` and the
//! context menu
//! `HEAD_OVERLAP` px above the card's bottom edge, so the two read as one: a
//! card [`ParticipantMenu::joined`] to rows has square bottom corners. Where
//! they go is [`ParticipantMenu::place`]: at the pointer, both moving up
//! together when they don't fit below it (`top`, `ui/yapfast.slint:2694`).

use iced::{Point, Size};

use crate::menu::Row;

/// The card's width unless the rows under it are wider: the context menu's
/// narrowest there (`narrowest: 260px`, `ui/yapfast.slint:2713`).
pub const WIDTH: f32 = 260.0;

/// The card's padding and the space between its lines (`padding: 12px;
/// spacing: 12px`).
pub const PADDING: f32 = 12.0;
pub const SPACING: f32 = 12.0;
/// The person's name line (Slint's 15 px line is 19.5, kept whole), and one
/// volume slider with its header (13 px x 1.3 + 6 + 18 = 40.9, kept whole).
pub const NAME_HEIGHT: f32 = 20.0;
pub const SLIDER_HEIGHT: f32 = 41.0;

/// How far the rows start above the card's bottom edge, so the two share
/// one line.
pub const HEAD_OVERLAP: f32 = 1.0;

/// The card's own rows, as a menu's (`MenuRows`, `ui/controls.slint:1287`):
/// each 32 px, 2 px apart, 6 px from the card's sides and from what's above
/// and below them; a group's line takes 9 px with it.
pub const ROW: f32 = 32.0;
pub const ROWS_PADDING: f32 = 6.0;
pub const ROWS_GAP: f32 = 2.0;
pub const RULE: f32 = 9.0;

/// Where a volume runs to, in percent.
pub const MAXIMUM: i32 = 200;
/// Where the strength of noise suppression runs to, in percent, and how
/// far our echo can be turned down, in dB (`yapfast_core::volume`'s
/// `FULL_STRENGTH` and `MOST_ECHO_DEPTH`).
pub const STRENGTH: i32 = 100;
pub const DEPTH: i32 = 40;

/// A participant's menu: what its screen draws. The window holds it as
/// `vm.voice.participant_menu`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticipantMenu {
    /// Who across calls (a Discord or Matrix user id), which their volumes
    /// are remembered by; every [`Intent`] names them.
    pub person: String,
    pub name: String,
    /// They share their screen: a second slider sets their stream's volume.
    pub live: bool,
    /// The card of their stream watched, not of them: its volume alone,
    /// then how its sound is cleaned, each slider about the stream's sound
    /// (`Intent::NoiseStrength` and `EchoDepth` too).
    pub stream: bool,
    /// Their voice's volume and their stream's, in percent.
    pub voice_volume: i32,
    pub stream_volume: i32,
    /// How much of their noise is taken out, in percent, while a suppressor
    /// is chosen for them, and how far they're turned down while only our
    /// echo comes from them, in dB, while that's on: a slider for each.
    pub noise_strength: Option<i32>,
    pub echo_depth: Option<i32>,
    /// The menu's rows follow right under, as one card.
    pub joined: bool,
    /// The card's width: the rows' under it when they are wider than
    /// [`WIDTH`].
    pub width: f32,
    /// A stream's card: its own rows between its name and its volume, each
    /// 32 px tall without a second line, a line before one `separated`.
    /// Someone's card has none.
    pub rows: Vec<Row<Intent>>,
}

impl Default for ParticipantMenu {
    fn default() -> Self {
        Self {
            person: String::new(),
            name: String::new(),
            live: false,
            stream: false,
            voice_volume: 100,
            stream_volume: 100,
            noise_strength: None,
            echo_depth: None,
            joined: false,
            width: WIDTH,
            rows: Vec::new(),
        }
    }
}

impl ParticipantMenu {
    /// The card's height: the app places the rows under it with this. Its
    /// own rows take the space before the first slider.
    pub fn height(&self) -> f32 {
        let sliders = 1
            + u8::from(self.live && !self.stream)
            + u8::from(self.noise_strength.is_some())
            + u8::from(self.echo_depth.is_some());
        let rows = self.rows_height();
        let before = if rows > 0.0 { SPACING } else { 0.0 };
        2.0 * PADDING + NAME_HEIGHT + rows - before + f32::from(sliders) * (SPACING + SLIDER_HEIGHT)
    }

    /// How tall its own rows are with the room round them: none without.
    pub fn rows_height(&self) -> f32 {
        if self.rows.is_empty() {
            return 0.0;
        }
        let lines = (self.rows.iter().skip(1))
            .filter(|row| row.separated)
            .count();
        let rows = self.rows.len() as f32;
        2.0 * ROWS_PADDING + rows * ROW + (rows - 1.0) * ROWS_GAP + lines as f32 * (RULE + ROWS_GAP)
    }

    /// Where the card and the rows under it go, as their top left corners,
    /// for a right click at `pointer` in a window of `window`, the rows being
    /// `rows` big: as Slint's `voice-menu` places them. They sit at the
    /// pointer when both fit under it with 8 px to spare, else both move up
    /// until the rows end at the pointer (never above the window's 8 px
    /// edge); sideways the card keeps 8 px from either edge.
    pub fn place(&self, pointer: Point, rows: Size, window: Size) -> (Point, Point) {
        const EDGE: f32 = 8.0;
        let head = if self.joined {
            self.height() - HEAD_OVERLAP
        } else {
            self.height()
        };
        let x = (window.width - self.width - EDGE).min(pointer.x).max(EDGE);
        let top = if pointer.y + head + rows.height <= window.height - EDGE {
            pointer.y
        } else {
            (pointer.y - head - rows.height).max(EDGE)
        };
        (Point::new(x, top), Point::new(x, top + head))
    }
}

/// A participant's menu: what a click or a key on it asks for. Keep
/// `#[non_exhaustive]`: the app matches it with a wildcard until its I task
/// handles every variant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// A slider moved (`volume-changed(person, stream, volume, last)`,
    /// `src/ui/callbacks.rs:2438`): `last` is the end of a drag or a key,
    /// which keeps the value; every step is heard at once, only the last is
    /// remembered.
    Volume {
        person: String,
        /// Their stream's volume, not their voice's.
        stream: bool,
        volume: i32,
        last: bool,
    },
    /// The strength of their noise suppression moved, in percent, or their
    /// stream's on its card; heard at once, kept once `last`.
    NoiseStrength {
        person: String,
        strength: i32,
        last: bool,
    },
    /// How far they're turned down while only our echo comes from them
    /// moved, in dB, or their stream on its card; heard at once, kept once
    /// `last`.
    EchoDepth {
        person: String,
        depth: i32,
        last: bool,
    },
    /// A stream's card: stop watching it (Discord's "Stop Watching"); the
    /// menu closes.
    StopWatching,
    /// A stream's card: its sound muted here, or back at the volume it had,
    /// by its streamer (Discord's "Mute" box); the menu stays.
    MuteStream(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(joined: bool) -> ParticipantMenu {
        ParticipantMenu {
            joined,
            width: 274.0,
            ..ParticipantMenu::default()
        }
    }

    const WINDOW: Size = Size::new(1400.0, 860.0);
    const ROWS: Size = Size::new(274.0, 644.0);

    #[test]
    fn rows_that_do_not_fit_below_move_up_with_the_card() {
        let (card, rows) = card(true).place(Point::new(670.0, 550.0), ROWS, WINDOW);
        assert_eq!(card, Point::new(670.0, 8.0));
        assert_eq!(rows, Point::new(670.0, 104.0));
    }

    #[test]
    fn a_pointer_near_the_right_edge_keeps_the_card_inside() {
        let (card, rows) =
            card(true).place(Point::new(1390.0, 20.0), Size::new(274.0, 100.0), WINDOW);
        assert_eq!(card, Point::new(1400.0 - 274.0 - 8.0, 20.0));
        assert_eq!(rows.x, card.x);
        assert_eq!(rows.y, 20.0 + 96.0);
    }

    #[test]
    fn each_cleaning_on_adds_a_slider_and_pushes_the_rows_down() {
        let plain = card(true);
        let cleaned = ParticipantMenu {
            noise_strength: Some(80),
            echo_depth: Some(20),
            ..plain.clone()
        };
        assert_eq!(
            cleaned.height(),
            plain.height() + 2.0 * (SPACING + SLIDER_HEIGHT)
        );
        let at = Point::new(100.0, 100.0);
        let rows = Size::new(274.0, 200.0);
        let (_, below) = plain.place(at, rows, WINDOW);
        let (_, lower) = cleaned.place(at, rows, WINDOW);
        assert_eq!(lower.y, below.y + 2.0 * (SPACING + SLIDER_HEIGHT));
    }

    #[test]
    fn a_streams_card_has_its_volume_alone_then_its_cleaning() {
        let stream = ParticipantMenu {
            stream: true,
            live: true,
            ..card(true)
        };
        assert_eq!(stream.height(), card(true).height(), "one slider");
        let cleaned = ParticipantMenu {
            noise_strength: Some(0),
            ..stream.clone()
        };
        assert_eq!(cleaned.height(), stream.height() + SPACING + SLIDER_HEIGHT);
    }

    #[test]
    fn a_streams_rows_stand_between_its_name_and_its_volume() {
        let plain = ParticipantMenu {
            stream: true,
            live: true,
            ..card(true)
        };
        let rows = ParticipantMenu {
            rows: vec![
                Row::new("Stop Watching", Intent::StopWatching),
                Row::new("Mute", Intent::MuteStream("bob".into()))
                    .checkbox(false)
                    .separated(),
            ],
            ..plain.clone()
        };
        // 6 + 32 + 2 + 9 + 2 + 32 + 6, in place of the 12 before the slider.
        assert_eq!(rows.rows_height(), 89.0);
        assert_eq!(rows.height(), plain.height() + 89.0 - SPACING);
        let at = Point::new(100.0, 100.0);
        let (_, below) = rows.place(at, Size::new(274.0, 100.0), WINDOW);
        assert_eq!(below.y, 100.0 + rows.height() - HEAD_OVERLAP);
    }

    #[test]
    fn what_fits_below_opens_at_the_pointer() {
        let (card, rows) =
            card(true).place(Point::new(100.0, 100.0), Size::new(274.0, 200.0), WINDOW);
        assert_eq!(card, Point::new(100.0, 100.0));
        assert_eq!(rows, Point::new(100.0, 196.0));
        let (card, rows) = self::card(false).place(Point::new(2.0, 100.0), Size::ZERO, WINDOW);
        assert_eq!(card, Point::new(8.0, 100.0));
        assert_eq!(rows.y, 100.0 + 97.0);
    }
}
