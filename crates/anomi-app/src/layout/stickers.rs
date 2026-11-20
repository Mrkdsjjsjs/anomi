//! The stickers `layout.json` keeps for the picker, as the Slint app's
//! `Layout` keeps them (`src/ui.rs`): those this device keeps for
//! Favorites, and those it sent, for Frequently Used.

use serde::{Deserialize, Serialize};

/// A sticker this device kept, as `layout.json` keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeptSticker {
    pub id: String,
    pub name: String,
    /// Discord's `format_type`, which its picture's address takes.
    pub format: u64,
    /// Its picture at the picker's tile size.
    pub picture: String,
}

/// A sticker this device sent, and how often, as `layout.json` keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeptStickerUse {
    pub id: String,
    pub name: String,
    pub format: u64,
    pub picture: String,
    pub uses: u32,
}
