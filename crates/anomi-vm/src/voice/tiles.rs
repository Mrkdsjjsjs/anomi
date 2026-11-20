//! A call's tiles: what one tile shows, someone's, a stream's or this
//! account's shared screen, and what a click on it asks for. Task V03
//! (docs/iced/tasks/voice.md). The Slint side is `StageTile`, `TileChip`,
//! `LivePill`, `ViewerStack`, `StreamPicture` and `StreamPreview`
//! (`ui/voice.slint:751`, `:819`, `:41`, `:60`, `:840`, `:849`) as the three
//! loops of `ParticipantGrid` fill them (`:988`, `:1008`, `:1051`), and the
//! tiles' identities, `Focus` in `src/ui/tiles.rs`.
//!
//! As in Discord's call, a stream is always a tile of its own, right before
//! its streamer's, who keeps their picture or camera, their speaking ring
//! and their marks: everyone in the call's order, each one who shares a
//! screen, this account included ("Your screen"), with their stream's tile
//! first. The stream's tile carries the red LIVE badge at its top right; its
//! streamer's carries none. The stage (V04) and picture in picture (V06)
//! hold [`Tile`]s in that order. The projection decides what each shows; a
//! tile draws what it's told. A camera's or a stream's frames come through a
//! [`Live`], which the app puts each in as it comes and the tile reads as it
//! draws: a frame builds no view, and is drawn from the core's memory, never
//! copied.

use iced::widget::image;

use crate::kind::{Avatar, Live};
use crate::menu::Spot;

/// Which tile it is, which a click on it shows large: `Focus` in
/// `src/ui/tiles.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// This account's shared screen, right before its own tile.
    OwnScreen,
    /// Someone's stream, in a tile of its own right before theirs, by
    /// participant id.
    Stream(String),
    /// Someone's own tile, by participant id: their camera or their
    /// picture, never their stream.
    Person(String),
}

impl Default for Key {
    fn default() -> Self {
        Self::Person(String::new())
    }
}

impl Key {
    /// The participant it's about, whose volume menu a right click opens;
    /// none for this account's screen.
    pub fn participant(&self) -> Option<&str> {
        match self {
            Self::OwnScreen => None,
            Self::Stream(id) | Self::Person(id) => Some(id),
        }
    }

    /// A stream's tile: someone's or this account's screen.
    pub fn stream(&self) -> bool {
        !matches!(self, Self::Person(_))
    }
}

/// What fills a tile. Their picture lies on the raised ground; everything
/// else on black, the stage's in every theme.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// Their picture in the middle, at half strength until they connect.
    Avatar { avatar: Avatar, connected: bool },
    /// Their camera's newest picture, whole.
    Camera(Live),
    /// A stream this account watches (`StreamPicture`, `:840`): its newest
    /// picture, whole, or until the first a turning ring over what it waits
    /// for.
    Watched { frame: Option<Live>, status: String },
    /// A stream not watched: the screen icon over a "Watch stream" pill, on
    /// its preview, dimmed, when Discord has one (`StreamPreview`, `:849`;
    /// `:1033`).
    Offer { preview: Option<image::Handle> },
    /// This account's shared screen: its newest picture, whole, or until
    /// the first the screen icon over how it's going (`:999`).
    OwnScreen { frame: Option<Live>, status: String },
    /// A stream shown large, in its place among the tiles beneath it, as
    /// Discord's strip keeps it: the screen icon alone, its picture being
    /// on the stage above. Nothing more is drawn or decoded for it.
    Large,
}

impl Default for Fill {
    fn default() -> Self {
        Self::Avatar {
            avatar: Avatar::default(),
            connected: true,
        }
    }
}

impl Fill {
    /// Laid on black rather than on the raised ground.
    pub fn on_black(&self) -> bool {
        !matches!(self, Self::Avatar { .. })
    }
}

/// A tile's outline, over its picture: a participant's has one (`:1091`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outline {
    /// A stream's tile, or this account's screen.
    #[default]
    None,
    /// A thin line.
    Line,
    /// They speak: a thicker ring in the live colour.
    Speaking,
}

/// The icon before the name on a tile's chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Muted and not deafened: the struck microphone, in red.
    Muted,
    /// Deafened: the struck headphones, in red.
    Deafened,
    /// A stream's own tile: the screen, in white.
    Screen,
}

/// The name at a tile's bottom left (`TileChip`, `:819`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chip {
    pub name: String,
    pub mark: Option<Mark>,
    /// The LIVE tag after the name; a stream's tile says it with its
    /// [`Tile::badge`] instead.
    pub live: bool,
    /// Why the tile may look wrong, such as a lost connection, in the
    /// accent after the name; empty for none.
    pub note: String,
}

/// Who watches the stream a tile shows (`ViewerStack`, `:60`): an eye and
/// how many, the first three's pictures, and how many more.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Viewers {
    /// The first [`Viewers::FACES`] viewers' pictures.
    pub faces: Vec<Avatar>,
    /// How many watch; none hides the stack.
    pub count: usize,
    /// "Watching: " and their names, for the tooltip.
    pub names: String,
}

impl Viewers {
    /// How many pictures the stack shows.
    pub const FACES: usize = 3;

    /// How many watch beyond the pictures shown.
    pub fn more(&self) -> usize {
        self.count.saturating_sub(Self::FACES)
    }
}

/// A call's tile: what its screen draws. The stage and picture in picture
/// hold these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub key: Key,
    pub fill: Fill,
    pub outline: Outline,
    pub chip: Chip,
    /// The red LIVE badge at its top right, as Discord's stream tile has
    /// it: a stream's tile, once it's live.
    pub badge: bool,
    /// Who watches the stream it shows: this account's screen's viewers,
    /// or a watched stream's.
    pub viewers: Viewers,
    /// A click watches the stream it offers, not watched yet, which its
    /// middle says ([`Fill::Offer`]).
    pub watch: bool,
    /// This account's camera is on its way: a pill at its top left, beating
    /// while the window plays.
    pub starting: bool,
    /// Shown large: its stop button stays, and it has the full screen
    /// button and a double click.
    pub focused: bool,
    /// Over the whole window, alone and square.
    pub full: bool,
    /// Large, with other tiles beneath it, which its button hides.
    pub can_hide_others: bool,
    pub others_hidden: bool,
}

impl Tile {
    /// It shows a stream this account watches, which its top right stops.
    pub fn watched(&self) -> bool {
        matches!(self.fill, Fill::Watched { .. })
    }

    /// What stands for this stream's tile among those beneath it while it's
    /// large ([`Fill::Large`]): its name, its mark and its badge, nothing
    /// that moves.
    pub fn stand_in(&self) -> Self {
        Self {
            key: self.key.clone(),
            fill: Fill::Large,
            chip: self.chip.clone(),
            badge: self.badge,
            ..Self::default()
        }
    }

    /// What a click on it asks: watching the stream it offers, else showing
    /// it large; nothing once it is, nor on what stands for it beneath
    /// (`:1022`, `:1071`).
    pub fn click(&self) -> Option<Intent> {
        match self.key.participant() {
            _ if matches!(self.fill, Fill::Large) => None,
            Some(id) if self.watch => Some(Intent::Watch(id.to_owned())),
            _ if !self.focused => Some(Intent::Focus(self.key.clone())),
            _ => None,
        }
    }

    /// What a right click on it at `spot` asks: someone's menu, their
    /// stream's, or this account's stream's.
    pub fn menu(&self, spot: Spot) -> Intent {
        match &self.key {
            Key::Person(id) => Intent::Menu(id.clone(), spot),
            Key::Stream(id) => Intent::StreamMenu(id.clone(), spot),
            Key::OwnScreen => Intent::OwnMenu(spot),
        }
    }
}

/// A call's tile: what a click or a key on it asks for. Keep
/// `#[non_exhaustive]`: the app matches it with a wildcard until its I task
/// handles every variant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// Show this tile large (`focus-tile`).
    Focus(Key),
    /// Watch someone's stream, by participant id (`watch-stream`).
    Watch(String),
    /// Stop watching the stream watched (`stop-watching`).
    StopWatching,
    /// The large tile over the whole window, or back (`full-screen`).
    FullScreen(bool),
    /// Hide the tiles beneath the large one, or show them again
    /// (`toggle-others`).
    HideOthers(bool),
    /// Someone's volume menu at the pointer, by participant id
    /// (`open-volume`).
    Menu(String, Spot),
    /// A right click on someone's stream's tile, by participant id: while
    /// it's watched, the menu of its sound at the pointer (its volume, and
    /// how its noise and this account's echo are taken out of it, Anomi's
    /// own); else their menu, as [`Intent::Menu`].
    StreamMenu(String, Spot),
    /// This account's stream's menu (its quality, its sound, what's shared,
    /// stopping), opened under the gear its tile shows under the pointer,
    /// as Discord's.
    StreamSettings(Spot),
    /// A right click on this account's stream's tile: Discord's menu of it
    /// at the pointer (`discord-own-stream-menu.png`).
    OwnMenu(Spot),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(key: Key) -> Tile {
        Tile {
            key,
            ..Tile::default()
        }
    }

    #[test]
    fn a_click_watches_what_it_offers_else_shows_it_large() {
        let bob = Key::Stream("bob".into());
        let offered = Tile {
            watch: true,
            ..tile(bob.clone())
        };
        assert_eq!(offered.click(), Some(Intent::Watch("bob".into())));
        assert_eq!(tile(bob.clone()).click(), Some(Intent::Focus(bob.clone())));
        let large = Tile {
            focused: true,
            ..tile(bob)
        };
        assert_eq!(large.click(), None);
        assert_eq!(
            large.stand_in().click(),
            None,
            "it stands for the large one"
        );
        assert_eq!(
            tile(Key::OwnScreen).click(),
            Some(Intent::Focus(Key::OwnScreen))
        );
    }

    #[test]
    fn a_right_click_asks_for_the_menu_of_what_the_tile_is() {
        let at = Spot::default();
        let menu = |key: Key| tile(key).menu(at);
        assert_eq!(
            menu(Key::Person("bob".into())),
            Intent::Menu("bob".into(), at)
        );
        assert_eq!(
            menu(Key::Stream("bob".into())),
            Intent::StreamMenu("bob".into(), at)
        );
        assert_eq!(menu(Key::OwnScreen), Intent::OwnMenu(at));
    }

    #[test]
    fn the_stand_in_keeps_the_streams_name_and_badge_and_nothing_that_moves() {
        let stream = Tile {
            key: Key::Stream("bob".into()),
            fill: Fill::Watched {
                frame: None,
                status: String::new(),
            },
            chip: Chip {
                name: "Bob".into(),
                mark: Some(Mark::Screen),
                ..Chip::default()
            },
            badge: true,
            focused: true,
            ..Tile::default()
        };
        let stand_in = stream.stand_in();
        assert_eq!(stand_in.fill, Fill::Large);
        assert_eq!((&stand_in.chip, stand_in.badge), (&stream.chip, true));
        assert!(!stand_in.focused && !stand_in.watched());
    }

    #[test]
    fn the_stack_shows_three_and_counts_the_rest() {
        let viewers = Viewers {
            count: 5,
            ..Viewers::default()
        };
        assert_eq!(viewers.more(), 2);
        assert_eq!(Viewers::default().more(), 0);
    }

    #[test]
    fn only_a_picture_lies_on_the_raised_ground() {
        assert!(!Fill::default().on_black());
        assert!(Fill::Offer { preview: None }.on_black());
        assert!(Fill::Large.on_black());
        assert_eq!(Key::OwnScreen.participant(), None);
        assert_eq!(Key::Stream("ana".into()).participant(), Some("ana"));
        assert!(Key::OwnScreen.stream() && !Key::Person("ana".into()).stream());
    }
}
