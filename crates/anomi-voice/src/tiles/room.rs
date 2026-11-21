//! What a tile has room for at the size it's laid out at, so its parts never
//! cover one another however small the strip beneath a large tile (120 px
//! wide at the least) or the grid makes it: the name's chip at the bottom
//! left, the LIVE badge at the top right, before it who watches and the
//! stop button or the gear, a camera's pill at the top left, and what the
//! middle says of a stream on its way, offered, or this account's screen
//! before its picture.
//!
//! Each lies in a band of its own: the top row's under the top inset, the
//! chip's over the bottom one, the middle centred in the tile, clear of
//! both by a pixel. What has no room goes, the least needed first: the
//! words under the middle's icon or ring (the note, the status, the pill),
//! then the icon or ring shrinks, rises into the band between the badge and
//! the chip where it isn't clear of both centred, then goes; the viewers and the button go
//! before the middle does (the button before the viewers), and the badge
//! only where even the chip leaves it none. A stream on its way with no room
//! in the middle turns its ring in the chip, in its mark's place.
//!
//! Widths of words are bounded, not shaped (`live::width`, `viewers::width`,
//! `marks::watch_width`), so laying a tile out shapes nothing more.

use iced::widget::text::LineHeight;
use iced::{Padding, Size};

use anomi_vm::voice::tiles::{Fill, Key, Tile};

use super::{INSET, PILL, chip, controls, fill, live, marks, viewers, waiting};

/// Kept between two parts, against rounding.
const CLEAR: f32 = 1.0;

/// What a tile at one size shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// The name's chip at the bottom left.
    pub chip: bool,
    /// The LIVE badge at the top right.
    pub badge: bool,
    /// Who watches, at rest and under the pointer.
    pub viewers: [bool; 2],
    /// The stop button of a watched stream, the gear of this account's.
    pub button: bool,
    /// "Starting camera..." at the top left.
    pub starting: bool,
    pub middle: Middle,
    /// How far the middle sits above the tile's centre: nothing but where
    /// only the band between the badge and the chip holds its icon or ring
    /// (Discord's strip has its screen in the middle of a tile with no
    /// chip), so the strip's smallest tiles with a chip keep it.
    pub lift: f32,
}

/// What the middle of a stream's tile shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Middle {
    /// Nothing: a picture fills the tile, or there's no room.
    Empty,
    /// The icon or the ring with its words under it, this tall.
    Full(f32),
    /// The icon or the ring alone, this many px square.
    Glyph(f32),
    /// The ring turning in the chip, in its mark's place.
    InChip,
}

impl Middle {
    /// How tall it is in the middle; none for nothing there.
    pub fn height(self) -> Option<f32> {
        match self {
            Self::Full(height) | Self::Glyph(height) => Some(height),
            Self::Empty | Self::InChip => None,
        }
    }
}

impl Room {
    /// What `tile` has room for at `size`.
    pub fn of(tile: &Tile, size: Size) -> Self {
        let inner = Size::new(size.width - 2.0 * INSET, size.height - 2.0 * INSET);
        let chip = inner.height >= chip::HEIGHT && inner.width > 0.0;
        // How low what's above the chip may reach.
        let floor = size.height - INSET - if chip { chip::HEIGHT + CLEAR } else { 0.0 };
        let badge_width = live::width();
        let badge = tile.badge && INSET + live::HEIGHT <= floor && badge_width <= inner.width;
        let top = INSET + if badge { live::HEIGHT + CLEAR } else { 0.0 };
        // The tallest a part centred in the tile may be, clear of both.
        let centre = size.height / 2.0;
        let tallest = 2.0 * (centre - top).min(floor - centre);
        let (middle, banded) = middle(tile, tallest, floor - top, inner.width, chip);
        let lift = if banded {
            centre - (top + floor) / 2.0
        } else {
            0.0
        };
        let below = middle
            .height()
            .map_or(floor, |height| centre - lift - height / 2.0 - CLEAR);
        // The top row, viewers and buttons, as tall as a button.
        let row = INSET + controls::BUTTON <= floor.min(below);
        let each = |width: f32| width + controls::GAP;
        let badged = if badge { each(badge_width) } else { 0.0 };
        let may_button = tile.watched() || tile.key == Key::OwnScreen;
        let button = row && may_button && badged + each(controls::BUTTON) <= inner.width;
        let resting = button && tile.watched() && tile.focused;
        let watchers = row && tile.viewers.count > 0;
        let fits = |with_button: bool| {
            let buttons = if with_button {
                each(controls::BUTTON)
            } else {
                0.0
            };
            watchers && badged + buttons + each(viewers::width(&tile.viewers)) <= inner.width
        };
        Self {
            chip,
            badge,
            viewers: [fits(resting), fits(button)],
            button,
            starting: tile.starting && INSET + PILL <= floor,
            middle,
            lift,
        }
    }

    /// Room round a part centred in the tile that puts it [`Self::lift`]
    /// above the centre.
    pub fn raised(self) -> Padding {
        if self.lift >= 0.0 {
            Padding::ZERO.bottom(2.0 * self.lift)
        } else {
            Padding::ZERO.top(-2.0 * self.lift)
        }
    }
}

/// The largest middle `tile` has room for, at most `tallest` high centred
/// and `width` wide; past that its icon or ring alone in the `band` between
/// the badge and the chip (`true` with it), then the ring in the `chip`.
fn middle(tile: &Tile, tallest: f32, band: f32, width: f32, chip: bool) -> (Middle, bool) {
    let line = |size: f32| LineHeight::default().to_absolute(iced::Pixels(size)).0;
    // The ring or the icon alone, largest first.
    const RINGS: &[f32] = &[waiting::RING, fill::SMALL_ICON];
    const SHARED: &[f32] = &[fill::SHARE_ICON, fill::SMALL_ICON];
    const SCREEN: &[f32] = &[fill::SMALL_ICON];
    let (full, glyphs) = match &tile.fill {
        Fill::Watched { frame: None, .. } => (
            Some(waiting::RING + waiting::GAP + line(waiting::NOTE)),
            RINGS,
        ),
        Fill::OwnScreen { frame: None, .. } => (
            Some(fill::SHARE_ICON + fill::GAP + line(fill::STATUS)),
            SHARED,
        ),
        Fill::Offer { .. } => (
            (marks::watch_width() <= width).then_some(fill::OFFER_ICON + fill::GAP + PILL),
            SCREEN,
        ),
        Fill::Large => (None, SCREEN),
        _ => return (Middle::Empty, false),
    };
    if let Some(full) = full.filter(|&full| full <= tallest) {
        return (Middle::Full(full), false);
    }
    let fits = |room: f32| (glyphs.iter().copied()).find(|&side| side <= room && side <= width);
    if let Some(side) = fits(tallest) {
        return (Middle::Glyph(side), false);
    }
    if let Some(side) = fits(band) {
        return (Middle::Glyph(side), true);
    }
    if chip && matches!(tile.fill, Fill::Watched { .. }) {
        (Middle::InChip, false)
    } else {
        (Middle::Empty, false)
    }
}

#[cfg(test)]
mod steady;
#[cfg(test)]
mod tests;
