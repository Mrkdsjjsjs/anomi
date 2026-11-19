use crate::core::Size;
use crate::core::image;
use crate::graphics;
use crate::image::atlas::{self, Atlas};
use crate::image::own;

use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Weak;

pub type Image = graphics::image::Buffer;

/// Entry in cache corresponding to an image handle
#[derive(Debug)]
pub enum Memory {
    /// Image data on host: an `Rgba` handle's own bytes (shared, not
    /// copied) or a decoded picture. Anomi: drawn, it's uploaded in that
    /// frame and these bytes are let go.
    Host {
        image: Image,
        allocation: Option<Weak<image::Memory>>,
    },
    /// On the GPU.
    Device {
        picture: Picture,
        allocation: Option<Weak<image::Memory>>,
    },
    Error(image::Error),
}

/// Anomi: a picture on the GPU, where it lies and how its texels read.
#[derive(Debug)]
pub struct Picture {
    pub place: Place,
    pub size: Size<u32>,
    /// `mark::BGRA` and `mark::PREMULTIPLIED`, for the shader.
    pub flags: u32,
}

/// Anomi: where a picture lies on the GPU.
#[derive(Debug)]
pub enum Place {
    /// In the atlas, with the pictures under `own::BYTES`.
    Atlas(atlas::Entry),
    /// In a texture of its own (`own.rs`).
    Own(own::Texture),
}

impl Memory {
    /// Loads `handle`: an `Rgba` handle's bytes as they are (a clone of its
    /// `Bytes`), or an encoded one decoded, here, on the window's thread, as
    /// tiny-skia does.
    pub fn load(handle: &image::Handle) -> Self {
        match graphics::image::load(handle) {
            Ok(image) => Self::Host {
                image,
                allocation: None,
            },
            Err(error) => Self::Error(error),
        }
    }

    pub fn dimensions(&self) -> Size<u32> {
        match self {
            Memory::Host { image, .. } => {
                let (width, height) = image.dimensions();

                Size::new(width, height)
            }
            Memory::Device { picture, .. } => picture.size,
            Memory::Error(_) => Size::new(1, 1),
        }
    }

    /// An allocation of `handle`, this memory's: the one the app still holds,
    /// or a new one. Every picture drawn is uploaded in the frame that draws
    /// it, so one loaded on the host is as ready to draw as one on the GPU.
    pub fn allocation(
        &mut self,
        handle: &image::Handle,
    ) -> Result<image::Allocation, image::Error> {
        let (weak, size) = match self {
            Memory::Host { image, allocation } => {
                (allocation, Size::new(image.width(), image.height()))
            }
            Memory::Device {
                picture,
                allocation,
            } => (allocation, picture.size),
            Memory::Error(error) => return Err(error.clone()),
        };

        if let Some(allocation) = weak.as_ref().and_then(image::Allocation::upgrade) {
            return Ok(allocation);
        }

        // iced's contract for `allocate`: the picture is in memory, on the
        // host or the GPU, and draws in the frame that next draws it.
        #[allow(unsafe_code)]
        let allocation = unsafe { image::allocate(handle, size) };

        *weak = Some(allocation.downgrade());

        Ok(allocation)
    }

    /// Whether the app holds an allocation of it, which keeps it.
    fn allocated(&self) -> bool {
        let (Memory::Host { allocation, .. } | Memory::Device { allocation, .. }) = self else {
            return false;
        };

        allocation
            .as_ref()
            .is_some_and(|allocation| allocation.strong_count() > 0)
    }
}

#[derive(Debug, Default)]
pub struct Cache {
    map: FxHashMap<image::Id, Memory>,
    hits: FxHashSet<image::Id>,
}

impl Cache {
    /// The memory of `handle`, loaded now if it isn't yet; a hit this frame.
    pub fn load(&mut self, handle: &image::Handle) -> &mut Memory {
        let _ = self.hits.insert(handle.id());

        self.map
            .entry(handle.id())
            .or_insert_with(|| Memory::load(handle))
    }

    /// Anomi: drops every picture this frame didn't draw (nor measure), and
    /// that the app holds no allocation of, at the end of every frame. Stock
    /// trimmed only after a frame that loaded a new picture, so a large
    /// picture scrolled out of view kept its GPU memory until another came.
    /// This renderer draws every layer in view each frame, so a picture
    /// still showing is always a hit. An atlas entry is freed; a texture of
    /// its own goes back to `pool`.
    pub fn trim(&mut self, atlas: &mut Atlas, pool: &mut own::Pool) {
        let hits = &self.hits;

        self.map.retain(|id, memory| {
            if hits.contains(id) || memory.allocated() {
                return true;
            }

            log::debug!("Dropping image allocation: {id:?}");

            // The entry goes with this `false`; its picture is taken out of
            // it first, to be freed or kept.
            let gone = std::mem::replace(memory, Memory::Error(image::Error::Empty));

            if let Memory::Device { picture, .. } = gone {
                match picture.place {
                    Place::Atlas(entry) => atlas.remove(&entry),
                    Place::Own(texture) => pool.give_back(texture),
                }
            }

            false
        });

        self.hits.clear();
    }
}
