//! What a window's renderer holds of its pictures: the atlas, the textures
//! of their own, and the SVGs rasterized.
//!
//! Anomi (`PATCHES.md`, "Large pictures and video frames"): no worker
//! thread. Stock iced decoded encoded pictures and uploaded those of 2 MiB
//! or more on a thread of its own, a frame or more after they were first
//! drawn (iced #3381: flicker, and a video frame replaced before it ever
//! showed), reading the app's bytes after the window had let their writer
//! have them back. Here a picture is loaded when first measured or drawn
//! (an `Rgba` handle's own bytes, shared; an encoded one decoded on the
//! window's thread, as tiny-skia does) and uploaded in the frame that draws
//! it, during `prepare`, after which its bytes are let go.

use crate::core::{self, Size};
use crate::graphics::Shell;
use crate::image::atlas::{self, Atlas};

#[cfg(feature = "image")]
use crate::image::{mark, own};

use std::sync::Arc;

/// Where a picture drawn this frame lies, for its instances.
#[derive(Debug, Clone, Copy)]
pub enum Placed<'a> {
    /// In the atlas.
    Atlas(&'a atlas::Entry),
    /// The whole of a texture of its own.
    #[cfg(feature = "image")]
    Whole,
}

pub struct Cache {
    device: wgpu::Device,
    queue: wgpu::Queue,
    atlas: Atlas,
    #[cfg(feature = "image")]
    raster: crate::image::raster::Cache,
    /// Anomi: textures of their own given back, kept a frame for the next
    /// frame of a video (`own.rs`).
    #[cfg(feature = "image")]
    own: own::Pool,
    /// The image pipeline's texture layout, for textures of their own.
    #[cfg(feature = "image")]
    layout: wgpu::BindGroupLayout,
    #[cfg(feature = "svg")]
    vector: crate::image::vector::Cache,
}

impl Cache {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        backend: wgpu::Backend,
        layout: wgpu::BindGroupLayout,
        _shell: &Shell,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            #[cfg(feature = "image")]
            raster: crate::image::raster::Cache::default(),
            #[cfg(feature = "image")]
            own: own::Pool::default(),
            #[cfg(feature = "image")]
            layout: layout.clone(),
            atlas: Atlas::new(device, backend, layout),
            #[cfg(feature = "svg")]
            vector: crate::image::vector::Cache::default(),
        }
    }

    /// Anomi: loads the picture now and calls back at once, as tiny-skia
    /// does; it's uploaded in the frame that draws it.
    #[cfg(feature = "image")]
    pub fn allocate_image(
        &mut self,
        handle: &core::image::Handle,
        callback: impl FnOnce(Result<core::image::Allocation, core::image::Error>) + Send + 'static,
    ) {
        callback(self.raster.load(handle).allocation(handle));
    }

    /// Anomi: loads the picture now; it's uploaded in the frame that draws
    /// it. Stock uploaded it here, through a submission of its own, and
    /// waited for the GPU.
    #[cfg(feature = "image")]
    pub fn load_image(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        handle: &core::image::Handle,
    ) -> Result<core::image::Allocation, core::image::Error> {
        self.raster.load(handle).allocation(handle)
    }

    #[cfg(feature = "image")]
    pub fn measure_image(&mut self, handle: &core::image::Handle) -> Option<Size<u32>> {
        Some(self.raster.load(handle).dimensions())
    }

    #[cfg(feature = "svg")]
    pub fn measure_svg(&mut self, handle: &core::svg::Handle) -> Size<u32> {
        // TODO: Concurrency
        self.vector.load(handle).viewport_dimensions()
    }

    /// The picture of `handle`, uploaded now if it isn't on the GPU yet:
    /// where it lies, the bind group it's drawn with, and its flags for the
    /// shader (`mark.rs`). None for a picture that failed to load or found
    /// no room, drawn as nothing (stock).
    #[cfg(feature = "image")]
    pub fn upload_raster(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        handle: &core::image::Handle,
    ) -> Option<(Placed<'_>, &Arc<wgpu::BindGroup>, u32)> {
        use crate::image::raster::{Memory, Place};

        let memory = self.raster.load(handle);

        if let Memory::Host { image, allocation } = memory {
            let picture = upload(
                device,
                &self.queue,
                encoder,
                belt,
                &mut self.atlas,
                &mut self.own,
                &self.layout,
                handle,
                image,
            )?;

            let allocation = allocation.take();

            // The host's bytes go: an `Rgba` handle's are the app's, shared,
            // and a video frame's slot is the writer's again once the window
            // read its next frame.
            *memory = Memory::Device {
                picture,
                allocation,
            };
        }

        let Memory::Device { picture, .. } = &*memory else {
            return None;
        };

        Some(match &picture.place {
            Place::Atlas(entry) => (Placed::Atlas(entry), self.atlas.bind_group(), picture.flags),
            Place::Own(texture) => (Placed::Whole, texture.bind_group(), picture.flags),
        })
    }

    #[cfg(feature = "svg")]
    pub fn upload_vector(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        handle: &core::svg::Handle,
        color: Option<core::Color>,
        size: Size<u32>,
    ) -> Option<(&atlas::Entry, &Arc<wgpu::BindGroup>)> {
        // TODO: Concurrency
        self.vector
            .upload(device, encoder, belt, handle, color, size, &mut self.atlas)
            .map(|entry| (entry, self.atlas.bind_group()))
    }

    /// Ends a frame: what it didn't draw is let go (`raster::Cache::trim`),
    /// textures of their own are freed or kept a frame (`own::Pool::trim`),
    /// and the atlas gives back its empty layers at the top
    /// (`Atlas::trim`).
    pub fn trim(&mut self) {
        #[cfg(feature = "image")]
        {
            self.raster.trim(&mut self.atlas, &mut self.own);
            self.own.trim();
        }

        #[cfg(feature = "svg")]
        self.vector.trim(&mut self.atlas); // TODO: Concurrency

        self.atlas.trim(&self.device, &self.queue);
    }

    /// Anomi: nothing to receive, with no worker; kept for the renderer's
    /// `tick`.
    #[cfg(feature = "image")]
    pub fn receive(&mut self) {}
}

/// Anomi: uploads the picture of `handle` in this frame: a video frame, a
/// picture of `own::BYTES` or more, or one wider than an atlas layer into a
/// texture of its own (one given back last frame at its size, or a new
/// one), written with `Queue::write_texture`; anything else into the atlas
/// through the frame's encoder. Its bytes go as they are, marked or not.
#[cfg(feature = "image")]
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    belt: &mut wgpu::util::StagingBelt,
    atlas: &mut Atlas,
    pool: &mut own::Pool,
    layout: &wgpu::BindGroupLayout,
    handle: &core::image::Handle,
    image: &crate::image::raster::Image,
) -> Option<crate::image::raster::Picture> {
    use crate::image::raster::{Picture, Place};

    let (width, height) = image.dimensions();

    if width == 0 || height == 0 {
        return None;
    }

    let size = Size::new(width, height);
    let mark = mark::of(handle);
    let max = device.limits().max_texture_dimension_2d;

    let place = if own::wants(size, mark.frame, atlas.size(), max) {
        let texture = pool
            .take(size)
            .unwrap_or_else(|| own::Texture::new(device, layout, size));

        // Anomi: a video frame's bytes held the same while they're copied,
        // with `ANOMI_FRAMES_VERIFY` set (`iced_tiny_skia::verify`).
        let before = (mark.frame && iced_tiny_skia::verify::on())
            .then(|| iced_tiny_skia::verify::hash(image));

        texture.write(queue, image);

        iced_tiny_skia::verify::after(before, image, &handle.id());

        Place::Own(texture)
    } else {
        Place::Atlas(atlas.upload(device, encoder, belt, width, height, image)?)
    };

    Some(Picture {
        place,
        size,
        flags: mark.flags,
    })
}
