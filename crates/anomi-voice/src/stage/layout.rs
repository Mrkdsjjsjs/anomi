//! Where the grid puts each tile: `ParticipantGrid`'s properties and pure
//! functions (`ui/voice.slint:910`-`:982`), from the room it has and
//! [`Arrangement`], which tile is large and what shows.
//!
//! A few people take the columns, and the size, that make their tiles
//! largest within both the width and the height, so a short call above a
//! DM's messages shows everyone without scrolling: side by side wins unless
//! stacking makes tiles a tenth larger, and four sit two by two or in one
//! row, never three over one. More people take tiles at least 240 px wide,
//! and scroll. With a tile large, the others are small, 120 to 192 px
//! wide, in rows beneath it; full screen shows it alone. A stream large
//! keeps its place in those rows with its stand-in ([`STAND_IN`]), as
//! Discord's strip does; a person large leaves theirs. A short last row is
//! centred under the full ones, as Discord's is.

use iced::{Point, Rectangle, Size};

use anomi_vm::voice::stage::{Placement, Stage};

/// Every tile is 16:9 (`:926`).
pub(super) const RATIO: f32 = 0.5625;
/// Between tiles, and round them (`:910`, `:911`); none full screen.
const GAP: f32 = 12.0;
const INSET: f32 = 24.0;
/// A tile is at most this wide (`:925`), and a few at least the other
/// (`:951`), as tall as a large one is at least (`:945`).
const WIDEST: f32 = 420.0;
const NARROWEST: f32 = 160.0;
/// More than four take columns of at least this (`:928`).
const CROWDED: f32 = 240.0;
/// The tiles beneath a large one (`:937`).
const STRIP_LEAST: f32 = 120.0;
const STRIP_MOST: f32 = 192.0;
/// How much larger stacking must make tiles to win (`:929`).
const LARGER: f32 = 1.1;
/// What a row of the strip may run over and still hold a tile, against
/// rounding: five of 185.6 px fill 976 exactly.
const FIT_SLACK: f32 = 1e-3;

/// The slot the grid lays the large stream's stand-in out at: its own
/// place in the rows beneath it ([`Stage::stand_in`]).
pub const STAND_IN: usize = usize::MAX;

/// Which tile is large and what shows, from the stage's tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    pub count: usize,
    pub focused: Option<usize>,
    pub full: bool,
    pub others_hidden: bool,
    /// The large one is a stream whose stand-in keeps its place beneath.
    pub stand_in: bool,
}

impl Arrangement {
    pub fn of(stage: &Stage) -> Self {
        let focused = stage.focused();
        Self {
            count: stage.tiles.len(),
            focused,
            full: stage.full(),
            others_hidden: stage.others_hidden(),
            stand_in: stage.stand_in.is_some() && focused.is_some(),
        }
    }

    /// The rows beneath the large tile hold its stand-in too.
    fn holds_stand_in(&self) -> bool {
        self.stand_in && self.focused.is_some() && !self.full && !self.others_hidden
    }

    /// The tile at `slot` shows large (`large`, `:965`).
    pub fn large(&self, slot: usize) -> bool {
        self.focused == Some(slot)
    }

    /// The tile at `slot` shows: full screen and hidden others leave the
    /// large tile alone (`shown`, `:964`).
    pub fn shown(&self, slot: usize) -> bool {
        self.large(slot) || !(self.full || (self.focused.is_some() && self.others_hidden))
    }
}

/// The grid laid out in a room of `size`: every tile's place in the
/// content, which scrolls when taller than the room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    arrangement: Arrangement,
    size: Size,
    gap: f32,
    columns: usize,
    tile: Size,
    focus: Rectangle,
    first_row: f32,
    placed: usize,
    rows: usize,
    /// The content's height, at least the room's.
    pub content: f32,
}

impl Geometry {
    pub fn new(arrangement: Arrangement, size: Size) -> Self {
        let Arrangement {
            count,
            focused,
            full,
            others_hidden,
            ..
        } = arrangement;
        let (width, height) = (size.width, size.height);
        let gap = if full { 0.0 } else { GAP };
        let inset = if full { 0.0 } else { INSET };
        let span = width - 2.0 * inset;
        let fitted = |columns: usize| {
            let rows = count.div_ceil(columns) as f32;
            let columns = columns as f32;
            WIDEST
                .min((span - (columns - 1.0) * gap) / columns)
                .min((height - 2.0 * inset - (rows - 1.0) * gap) / rows / RATIO)
        };
        let fewest = (((span + gap) / (CROWDED + gap)).floor() as usize)
            .min(count)
            .max(1);
        let best_fit = if count >= 2 && fitted(2) * LARGER > fitted(1) {
            match count {
                3 if fitted(3) * LARGER > fitted(2) => 3,
                4 if fitted(4) * LARGER > fitted(2) => 4,
                _ => 2,
            }
        } else {
            1
        };
        let focusing = focused.is_some();
        let others = count.saturating_sub(1) + usize::from(arrangement.holds_stand_in());
        let strip_width = if others == 0 {
            0.0
        } else {
            let others = others as f32;
            STRIP_LEAST.max(STRIP_MOST.min((span - (others - 1.0) * gap) / others))
        };
        // A hair over, so a row that fits exactly isn't lost to rounding.
        let fit = ((span + gap) / (strip_width + gap) + FIT_SLACK).floor();
        let strip_columns = (others as f32).min(fit).max(1.0) as usize;
        let strip_rows = if full || others_hidden || others == 0 {
            0
        } else {
            others.div_ceil(strip_columns)
        };
        let strip_height =
            strip_rows as f32 * strip_width * RATIO + strip_rows.saturating_sub(1) as f32 * gap;
        let under_focus = if strip_rows > 0 {
            gap + strip_height
        } else {
            0.0
        };
        let focus_width = if full {
            width
        } else {
            span.min(NARROWEST.max(height - 2.0 * inset - under_focus) / RATIO)
        };
        let focus_height = if full { height } else { focus_width * RATIO };
        let focus_top = if full {
            0.0
        } else {
            inset.max((height - focus_height - under_focus) / 2.0)
        };
        let columns = match () {
            () if focusing && (full || others_hidden) => 0,
            () if focusing => strip_columns,
            () if count <= 4 => best_fit,
            () => fewest,
        };
        let tile_width = if focusing {
            strip_width
        } else if count <= 4 {
            NARROWEST.max(fitted(columns))
        } else {
            let columns = columns as f32;
            WIDEST.min((span - (columns - 1.0) * gap) / columns)
        };
        let tile = Size::new(tile_width, tile_width * RATIO);
        let placed = if focusing { others } else { count };
        let rows = placed.div_ceil(columns.max(1));
        let grid_height = rows as f32 * tile.height + rows.saturating_sub(1) as f32 * gap;
        let (first_row, content) = if focusing {
            (
                focus_top + focus_height + gap,
                height.max(focus_top + focus_height + under_focus + inset),
            )
        } else {
            (
                inset.max((height - grid_height) / 2.0),
                height.max(grid_height + 2.0 * inset),
            )
        };
        Self {
            arrangement,
            size,
            gap,
            columns,
            tile,
            focus: Rectangle::new(
                Point::new((width - focus_width) / 2.0, focus_top),
                Size::new(focus_width, focus_height),
            ),
            first_row,
            placed,
            rows,
            content,
        }
    }

    /// Where the tile at `slot` lies in the content (`tile-x`, `tile-y`,
    /// `:975`-`:982`); at [`STAND_IN`], the large stream's stand-in.
    pub fn rect(&self, slot: usize) -> Rectangle {
        let held = self.arrangement.holds_stand_in();
        let place = match self.arrangement.focused {
            // The stand-in lies where the large one would in the rows.
            Some(focused) if slot == STAND_IN && held => focused,
            _ if slot == STAND_IN => return Rectangle::default(),
            _ if self.arrangement.large(slot) => return self.focus,
            // The rows skip the large tile (`place`, `:967`), unless its
            // stand-in holds its place.
            Some(focused) if slot > focused && !held => slot - 1,
            _ => slot,
        };
        let columns = self.columns.max(1);
        let (row, column) = (place / columns, place % columns);
        // A short last row is centred under the full ones (`row-start`).
        let in_row = if row + 1 == self.rows {
            self.placed - (self.rows - 1) * columns
        } else {
            columns
        };
        let step = Size::new(self.tile.width + self.gap, self.tile.height + self.gap);
        let start = (self.size.width - in_row as f32 * step.width + self.gap) / 2.0;
        Rectangle::new(
            Point::new(
                start + column as f32 * step.width,
                self.first_row + row as f32 * step.height,
            ),
            self.tile,
        )
    }

    /// Where the tiles are, scrolled down by `scrolled`, for the app
    /// (`report`, `:896`).
    pub fn placement(&self, scrolled: f32) -> Placement {
        let focused = self.arrangement.focused;
        let focus = focused.map_or(Size::ZERO, |_| self.focus.size());
        Placement {
            columns: self.columns as u32,
            width: self.tile.width,
            height: self.tile.height,
            gap: self.gap,
            top: self.first_row,
            scrolled,
            shown: self.size.height,
            focused,
            focus_width: focus.width,
            focus_height: focus.height,
            stand_in: self.arrangement.holds_stand_in(),
        }
    }
}

#[cfg(test)]
mod tests;
