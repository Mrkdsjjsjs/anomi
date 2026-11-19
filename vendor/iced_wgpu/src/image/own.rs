//! Anomi: pictures in textures of their own (`PATCHES.md`, "Large pictures
//! and video frames").
//!
//! A video frame is a new handle at every frame, and a large picture fills
//! much of an atlas layer, which the atlas could give back only once
//! everything above it went too. Stock iced uploaded a picture of 2 MiB or
//! more on a worker thread into an atlas of its own, ready a frame or more
//! later: a frame could be replaced before it ever showed (iced #3381), and
//! the worker read the app's frame after the window had let the frame's
//! writer have it back. Here every such picture is uploaded in the frame
//! that draws it, through [`wgpu::Queue::write_texture`], into a texture of
//! exactly its size: wgpu copies the bytes into a staging buffer during the
//! call, on the window's thread, as tiny-skia's blit reads them, and frees
//! that buffer once the frame's submission is done.
//!
//! A texture whose picture is no longer drawn is given back at the end of
//! the frame. If a picture of its size was uploaded in that frame (the next
//! frame of a video, or of a camera, replacing it), it's kept as a spare,
//! which the source's next frame then takes instead of a new texture: a
//! source playing holds two textures, the frame drawn and the one before.
//! Anything else is freed at once, and a spare no picture takes within
//! [`SPARE_FRAMES`] frames is freed then.

use crate::core::Size;
use crate::image::atlas;

use std::sync::Arc;

/// Pictures of this many bytes or more take a texture of their own; every
/// video frame does, at any size. Below it (avatars, emoji, icons, small
/// copies) they share the atlas.
pub const BYTES: usize = 256 * 1024;

/// The frames a spare texture waits for a picture of its size. The window
/// draws a frame for anything that moves, so a source slower than the
/// window's other animations (a 24 fps video beside a 180 Hz scroll) has
/// several frames between two of its own.
pub const SPARE_FRAMES: u32 = 16;

/// Whether a picture of `size` takes a texture of its own: a video frame,
/// a picture of [`BYTES`] or more, or one larger than an atlas layer (`atlas`
/// pixels a side); never one larger than the device's textures (`max`
/// pixels a side), which the atlas splits across its layers instead.
pub fn wants(size: Size<u32>, frame: bool, atlas: u32, max: u32) -> bool {
    let fits = size.width <= max && size.height <= max;
    let bytes = size.width as usize * size.height as usize * 4;

    fits && (frame || bytes >= BYTES || size.width > atlas || size.height > atlas)
}

/// What the pool keeps track of: a texture's size.
pub trait Sized2d {
    /// The size, in pixels.
    fn size(&self) -> Size<u32>;
}

/// A picture's texture, exactly its size, and the bind group the image
/// pipeline draws it with.
#[derive(Debug)]
pub struct Texture {
    texture: wgpu::Texture,
    bind_group: Arc<wgpu::BindGroup>,
    size: Size<u32>,
}

impl Texture {
    /// A texture of `size`, in the atlas's format, viewed as an array of one
    /// layer so the image pipeline's bind group layout takes it.
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: Size<u32>) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("iced_wgpu::image texture of its own"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: atlas::format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu::image texture of its own bind group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        Self {
            texture,
            bind_group: Arc::new(bind_group),
            size,
        }
    }

    /// Writes the whole picture, `pixels` rows of `width * 4` bytes, as they
    /// are. wgpu copies them during the call and writes the texture before
    /// the commands of the next submission, this frame's.
    pub fn write(&self, queue: &wgpu::Queue, pixels: &[u8]) {
        let Size { width, height } = self.size;

        queue.write_texture(
            self.texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The bind group the image pipeline draws it with.
    pub fn bind_group(&self) -> &Arc<wgpu::BindGroup> {
        &self.bind_group
    }
}

impl Sized2d for Texture {
    fn size(&self) -> Size<u32> {
        self.size
    }
}

/// Textures given back, kept a few frames for a picture of the same size.
#[derive(Debug)]
pub struct Pool<T = Texture> {
    /// Kept for a picture to take, each with the trims it has waited.
    spare: Vec<(T, u32)>,
    /// Given back during this trim.
    given: Vec<T>,
    /// The size of each texture taken or made this frame.
    wanted: Vec<Size<u32>>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self {
            spare: Vec::new(),
            given: Vec::new(),
            wanted: Vec::new(),
        }
    }
}

impl<T: Sized2d> Pool<T> {
    /// A spare texture of `size`, if one is kept; none, and the caller makes
    /// one. Either way a texture of `size` is wanted this frame.
    pub fn take(&mut self, size: Size<u32>) -> Option<T> {
        self.wanted.push(size);

        let index = self
            .spare
            .iter()
            .position(|(spare, _)| spare.size() == size)?;

        Some(self.spare.swap_remove(index).0)
    }

    /// Gives back the texture of a picture no longer drawn.
    pub fn give_back(&mut self, texture: T) {
        self.given.push(texture);
    }

    /// Ends a frame: a spare that has waited [`SPARE_FRAMES`] frames is
    /// freed, and of the textures given back, one is kept for each texture
    /// of its size taken or made this frame; the rest are freed.
    pub fn trim(&mut self) {
        self.spare.retain_mut(|(_, waited)| {
            *waited += 1;

            *waited < SPARE_FRAMES
        });

        for texture in self.given.drain(..) {
            if let Some(index) = self.wanted.iter().position(|&size| size == texture.size()) {
                let _ = self.wanted.swap_remove(index);

                self.spare.push((texture, 0));
            }
        }

        self.wanted.clear();
    }

    /// How many textures are kept for the next frame.
    #[cfg(test)]
    pub fn spares(&self) -> usize {
        self.spare.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A texture's stand-in: its size, and which one it is.
    #[derive(Debug)]
    struct Fake(Size<u32>, usize);

    impl Sized2d for Fake {
        fn size(&self) -> Size<u32> {
            self.0
        }
    }

    const HD: Size<u32> = Size::new(1280, 720);
    const SMALL: Size<u32> = Size::new(320, 180);

    /// A source playing: each frame a new texture is wanted, the frame
    /// before's given back at the trim.
    #[test]
    fn a_playing_source_holds_two_textures_and_makes_none_after_its_second_frame() {
        let mut pool = Pool::default();
        let mut made = 0;
        let mut make = |size| {
            made += 1;
            Fake(size, made)
        };

        // Frame 0: nothing to take.
        let first = pool.take(HD).unwrap_or_else(|| make(HD));
        pool.trim();
        assert_eq!(pool.spares(), 0);

        // Frame 1: a second texture; the first is given back and kept.
        let second = pool.take(HD).unwrap_or_else(|| make(HD));
        pool.give_back(first);
        pool.trim();
        assert_eq!(pool.spares(), 1);

        // Frames 2..: each takes the one given back the frame before.
        let mut shown = second;
        for _ in 2..30 {
            let next = pool.take(HD).expect("the frame before's texture");
            pool.give_back(shown);
            pool.trim();
            assert_eq!(pool.spares(), 1);
            shown = next;
        }

        assert_eq!(made, 2, "two textures in all");
    }

    /// A source with a frame every third frame the window draws: its
    /// textures are reused all the same.
    #[test]
    fn a_source_slower_than_the_window_reuses_its_textures_too() {
        let mut pool = Pool::default();
        let mut made = 0;
        let mut shown: Option<Fake> = None;

        for drawn in 0..90 {
            if drawn % 3 == 0 {
                let next = pool.take(HD).unwrap_or_else(|| {
                    made += 1;
                    Fake(HD, made)
                });

                if let Some(before) = shown.replace(next) {
                    pool.give_back(before);
                }
            }

            pool.trim();
        }

        assert_eq!(made, 2, "two textures in all");
    }

    #[test]
    fn a_picture_no_longer_drawn_is_freed_at_once_and_a_spare_in_time() {
        let mut pool = Pool::default();

        // A picture scrolled away: no picture of its size this frame.
        pool.give_back(Fake(SMALL, 1));
        pool.trim();
        assert_eq!(pool.spares(), 0, "nothing of its size wanted");

        // A source that stopped: its last texture is kept a while, then freed.
        let _ = pool.take(HD);
        pool.give_back(Fake(HD, 2));
        pool.trim();
        for _ in 0..SPARE_FRAMES {
            assert_eq!(pool.spares(), 1);
            pool.trim();
        }
        assert_eq!(pool.spares(), 0, "not taken: freed");
    }

    #[test]
    fn no_more_spares_are_kept_than_textures_of_their_size_were_wanted() {
        let mut pool = Pool::default();
        let _ = pool.take(HD);
        let _ = pool.take(SMALL);
        for n in 0..4 {
            pool.give_back(Fake(HD, n));
        }
        pool.give_back(Fake(SMALL, 9));
        pool.trim();
        assert_eq!(pool.spares(), 2, "one of each size");
        assert_eq!(pool.take(SMALL).map(|spare| spare.1), Some(9));
        assert!(pool.take(SMALL).is_none());
    }

    #[test]
    fn frames_large_pictures_and_pictures_wider_than_a_layer_take_their_own() {
        let (atlas, max) = (1024, 8192);
        assert!(wants(Size::new(64, 36), true, atlas, max), "a small frame");
        assert!(
            !wants(Size::new(64, 36), false, atlas, max),
            "a small picture"
        );
        assert!(!wants(Size::new(255, 256), false, atlas, max), "just under");
        assert!(wants(Size::new(256, 256), false, atlas, max), "256 KiB");
        assert!(
            wants(Size::new(1100, 40), false, atlas, max),
            "wider than a layer"
        );
        assert!(
            !wants(Size::new(9000, 10), true, atlas, max),
            "past the device"
        );
    }
}
