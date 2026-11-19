use crate::core::Size;
use crate::image::atlas;

#[derive(Debug)]
pub enum Entry {
    Contiguous(atlas::Allocation),
    Fragmented {
        size: Size<u32>,
        fragments: Vec<Fragment>,
    },
}

// Anomi: stock's `Entry::size` went with the worker that read it; a
// picture's size is kept beside its entry (`raster::Picture`).

#[derive(Debug)]
pub struct Fragment {
    pub position: (u32, u32),
    pub allocation: atlas::Allocation,
}
