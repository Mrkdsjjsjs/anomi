pub mod entry;

mod allocation;
mod allocator;
mod layer;

pub use allocation::Allocation;
pub use entry::Entry;
pub use layer::Layer;

use allocator::Allocator;

/// Anomi: the side of a layer. Stock, 2048 (16 MiB a layer, made at start).
/// Pictures of `own::BYTES` (256 KiB) or more, and every video frame, take
/// textures of their own, so what's left here are avatars, emoji, icons,
/// SVGs and small copies: a 1024 layer (4 MiB) holds at least sixteen of the
/// largest, and the atlas grows and gives back 4 MiB at a time.
pub const SIZE: u32 = 1024;

/// Anomi: the trims in a row the layers at the top must stay empty before
/// they're given back, so a picture coming and going at a layer's edge
/// doesn't copy the atlas each frame.
pub const SHRINK_AFTER: u32 = 60;

use crate::core::Size;
use crate::graphics::color;

use std::sync::Arc;

#[derive(Debug)]
pub struct Atlas {
    size: u32,
    backend: wgpu::Backend,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    texture_bind_group: Arc<wgpu::BindGroup>,
    texture_layout: wgpu::BindGroupLayout,
    layers: Vec<Layer>,
    /// Anomi: written (a picture uploaded, the atlas grown) since the last
    /// trim, by commands not yet submitted.
    written: bool,
    /// Anomi: trims in a row the last layer lay empty.
    idle: u32,
}

/// The format of the atlas and of the textures of their own (`own.rs`).
pub fn format() -> wgpu::TextureFormat {
    if color::GAMMA_CORRECTION {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

impl Atlas {
    pub fn new(
        device: &wgpu::Device,
        backend: wgpu::Backend,
        texture_layout: wgpu::BindGroupLayout,
    ) -> Self {
        let size = SIZE;

        let layers = match backend {
            // On the GL backend we start with 2 layers, to help wgpu figure
            // out that this texture is `GL_TEXTURE_2D_ARRAY` rather than `GL_TEXTURE_2D`
            // https://github.com/gfx-rs/wgpu/blob/004e3efe84a320d9331371ed31fa50baa2414911/wgpu-hal/src/gles/mod.rs#L371
            wgpu::Backend::Gl => vec![Layer::Empty, Layer::Empty],
            _ => vec![Layer::Empty],
        };

        let texture = texture(device, size, layers.len() as u32);

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu::image texture atlas bind group"),
            layout: &texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            }],
        });

        Atlas {
            size,
            backend,
            texture,
            texture_view,
            texture_bind_group: Arc::new(texture_bind_group),
            texture_layout,
            layers,
            written: false,
            idle: 0,
        }
    }

    pub fn bind_group(&self) -> &Arc<wgpu::BindGroup> {
        &self.texture_bind_group
    }

    /// The side of a layer, in pixels.
    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Option<Entry> {
        // Anomi: an empty picture has nothing to upload, and a staging
        // buffer of no bytes can't be made.
        if width == 0 || height == 0 {
            return None;
        }

        let entry = {
            let current_size = self.layers.len();
            let entry = self.allocate(width, height)?;

            // We grow the internal texture after allocating if necessary
            let new_layers = self.layers.len() - current_size;
            self.grow(new_layers, device, encoder);

            entry
        };

        self.written = true;

        log::debug!("Allocated atlas entry: {entry:?}");

        match &entry {
            Entry::Contiguous(allocation) => {
                self.upload_allocation(device, pixels, width, 0, allocation, encoder, belt);
            }
            Entry::Fragmented { fragments, .. } => {
                for fragment in fragments {
                    let (x, y) = fragment.position;
                    let offset = 4 * (y * width + x) as usize;

                    self.upload_allocation(
                        device,
                        pixels,
                        width,
                        offset,
                        &fragment.allocation,
                        encoder,
                        belt,
                    );
                }
            }
        }

        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "Atlas layers: {} (busy: {}, allocations: {})",
                self.layers.len(),
                self.layers.iter().filter(|layer| !layer.is_empty()).count(),
                self.layers.iter().map(Layer::allocations).sum::<usize>(),
            );
        }

        Some(entry)
    }

    pub fn remove(&mut self, entry: &Entry) {
        log::debug!("Removing atlas entry: {entry:?}");

        match entry {
            Entry::Contiguous(allocation) => {
                self.deallocate(allocation);
            }
            Entry::Fragmented { fragments, .. } => {
                for fragment in fragments {
                    self.deallocate(&fragment.allocation);
                }
            }
        }
    }

    /// Anomi: gives back the layers at the top that held no picture for
    /// [`SHRINK_AFTER`] trims in a row. Stock, the atlas only grew: a channel
    /// of pictures scrolled through once kept every layer it ever took.
    /// Only layers at the top go (an allocation knows its layer by index),
    /// and the ones kept are copied into a texture as deep as they are.
    ///
    /// Called at the end of a frame, before that frame's commands are
    /// submitted: the copy is submitted here, ahead of them (they still
    /// sample the old texture, which they keep alive). Never in a frame
    /// that wrote the atlas, whose writes lie in those commands and would
    /// miss the copy.
    pub fn trim(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let written = std::mem::take(&mut self.written);
        let keep = kept(&self.layers, self.floor());

        if keep >= self.layers.len() {
            self.idle = 0;
            return;
        }

        self.idle = self.idle.saturating_add(1);

        if written || self.idle < SHRINK_AFTER {
            return;
        }

        self.idle = 0;

        let before = self.layers.len();
        self.layers.truncate(keep);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("iced_wgpu::image atlas shrink"),
        });

        self.resize(device, &mut encoder, keep);

        let _ = queue.submit([encoder.finish()]);

        log::debug!("Atlas layers given back: {before} to {keep}");
    }

    /// The fewest layers the backend takes.
    fn floor(&self) -> usize {
        match self.backend {
            // See `new`.
            wgpu::Backend::Gl => 2,
            _ => 1,
        }
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<Entry> {
        // Allocate one layer if texture fits perfectly
        if width == self.size && height == self.size {
            let mut empty_layers = self
                .layers
                .iter_mut()
                .enumerate()
                .filter(|(_, layer)| layer.is_empty());

            if let Some((i, layer)) = empty_layers.next() {
                *layer = Layer::Full;

                return Some(Entry::Contiguous(Allocation::Full {
                    layer: i,
                    size: self.size,
                }));
            }

            self.layers.push(Layer::Full);

            return Some(Entry::Contiguous(Allocation::Full {
                layer: self.layers.len() - 1,
                size: self.size,
            }));
        }

        // Split big textures across multiple layers
        if width > self.size || height > self.size {
            let mut fragments = Vec::new();
            let mut y = 0;

            while y < height {
                let height = std::cmp::min(height - y, self.size);
                let mut x = 0;

                while x < width {
                    let width = std::cmp::min(width - x, self.size);

                    let allocation = self.allocate(width, height)?;

                    if let Entry::Contiguous(allocation) = allocation {
                        fragments.push(entry::Fragment {
                            position: (x, y),
                            allocation,
                        });
                    }

                    x += width;
                }

                y += height;
            }

            return Some(Entry::Fragmented {
                size: Size::new(width, height),
                fragments,
            });
        }

        // Try allocating on an existing layer
        for (i, layer) in self.layers.iter_mut().enumerate() {
            match layer {
                Layer::Empty => {
                    let mut allocator = Allocator::new(self.size);

                    if let Some(region) = allocator.allocate(width, height) {
                        *layer = Layer::Busy(allocator);

                        return Some(Entry::Contiguous(Allocation::Partial {
                            region,
                            layer: i,
                            atlas_size: self.size,
                        }));
                    }
                }
                Layer::Busy(allocator) => {
                    if let Some(region) = allocator.allocate(width, height) {
                        return Some(Entry::Contiguous(Allocation::Partial {
                            region,
                            layer: i,
                            atlas_size: self.size,
                        }));
                    }
                }
                Layer::Full => {}
            }
        }

        // Create new layer with atlas allocator
        let mut allocator = Allocator::new(self.size);

        if let Some(region) = allocator.allocate(width, height) {
            self.layers.push(Layer::Busy(allocator));

            return Some(Entry::Contiguous(Allocation::Partial {
                region,
                layer: self.layers.len() - 1,
                atlas_size: self.size,
            }));
        }

        // We ran out of memory (?)
        None
    }

    fn deallocate(&mut self, allocation: &Allocation) {
        log::debug!("Deallocating atlas: {allocation:?}");

        match allocation {
            Allocation::Full { layer, .. } => {
                if let Some(layer) = self.layers.get_mut(*layer) {
                    *layer = Layer::Empty;
                }
            }
            Allocation::Partial { layer, region, .. } => {
                if let Some(layer) = self.layers.get_mut(*layer)
                    && let Layer::Busy(allocator) = layer
                {
                    allocator.deallocate(region);

                    if allocator.is_empty() {
                        *layer = Layer::Empty;
                    }
                }
            }
        }
    }

    fn upload_allocation(
        &self,
        device: &wgpu::Device,
        pixels: &[u8],
        image_width: u32,
        offset: usize,
        allocation: &Allocation,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
    ) {
        // What stock asked of the belt.
        const ALIGNMENT: wgpu::BufferSize = wgpu::BufferSize::new(8 * 4).expect("not zero");

        let (x, y) = allocation.position();
        let Size { width, height } = allocation.size();
        let layer = allocation.layer();
        let padding = allocation.padding();

        // It is a webgpu requirement that:
        //   BufferCopyView.layout.bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT == 0
        // So we calculate bytes_per_row by rounding width up to the next
        // multiple of wgpu::COPY_BYTES_PER_ROW_ALIGNMENT.
        let bytes_per_row = (4 * (width + padding.width * 2))
            .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            as usize;
        let total_bytes = bytes_per_row * (height + padding.height * 2) as usize;

        let Some(size) = wgpu::BufferSize::new(total_bytes as u64) else {
            return;
        };

        // Anomi: an upload larger than the renderer's belt's chunks goes
        // through a buffer of its own, which wgpu frees once the frame's
        // submission is done. Through the belt, stock, it made a chunk of
        // its size that the belt then kept, mapped in RAM, for good: the
        // belt grew to the most a frame ever uploaded.
        let one_off = (total_bytes > crate::buffer::MAX_WRITE_SIZE).then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("iced_wgpu::image atlas upload"),
                size: size.get(),
                usage: wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            })
        });

        let buffer_slice = match &one_off {
            Some(buffer) => buffer.slice(..),
            None => belt.allocate(size, ALIGNMENT),
        };

        let Ok(mut fragment) = buffer_slice.get_mapped_range_mut() else {
            log::warn!("Atlas upload: the staging buffer isn't mapped");
            return;
        };

        const PIXEL: usize = 4;

        let w = width as usize;
        let h = height as usize;
        let pad_w = padding.width as usize;
        let pad_h = padding.height as usize;
        let stride = PIXEL * w;

        // Copy image rows
        for row in 0..h {
            let src = offset + row * PIXEL * image_width as usize;
            let dst = (row + pad_h) * bytes_per_row;

            fragment
                .slice(dst + PIXEL * pad_w..dst + PIXEL * pad_w + stride)
                .copy_from_slice(&pixels[src..src + stride]);

            // Add padding to the sides, if needed
            for i in 0..pad_w {
                fragment
                    .slice(dst + PIXEL * i..dst + PIXEL * (i + 1))
                    .copy_from_slice(&pixels[src..src + PIXEL]);

                fragment
                    .slice(
                        dst + stride + PIXEL * (pad_w + i)..dst + stride + PIXEL * (pad_w + i + 1),
                    )
                    .copy_from_slice(&pixels[src + stride - PIXEL..src + stride]);
            }
        }

        // Add padding on top and bottom
        for row in 0..pad_h {
            let dst_top = row * bytes_per_row;
            let dst_bottom = (pad_h + h + row) * bytes_per_row;
            let src_top = offset;
            let src_bottom = offset + (h - 1) * PIXEL * image_width as usize;

            // Top
            fragment
                .slice(dst_top + PIXEL * pad_w..dst_top + PIXEL * (pad_w + w))
                .copy_from_slice(&pixels[src_top..src_top + PIXEL * w]);

            // Bottom
            fragment
                .slice(dst_bottom + PIXEL * pad_w..dst_bottom + PIXEL * (pad_w + w))
                .copy_from_slice(&pixels[src_bottom..src_bottom + PIXEL * w]);

            // Corners
            for i in 0..pad_w {
                // Top left
                fragment
                    .slice(dst_top + PIXEL * i..dst_top + PIXEL * (i + 1))
                    .copy_from_slice(&pixels[offset..offset + PIXEL]);

                // Top right
                fragment
                    .slice(dst_top + PIXEL * (w + pad_w + i)..dst_top + PIXEL * (w + pad_w + i + 1))
                    .copy_from_slice(&pixels[offset + PIXEL * (w - 1)..offset + PIXEL * w]);

                // Bottom left
                fragment
                    .slice(dst_bottom + PIXEL * i..dst_bottom + PIXEL * (i + 1))
                    .copy_from_slice(&pixels[src_bottom..src_bottom + PIXEL]);

                // Bottom right
                fragment
                    .slice(
                        dst_bottom + PIXEL * (w + pad_w + i)
                            ..dst_bottom + PIXEL * (w + pad_w + i + 1),
                    )
                    .copy_from_slice(&pixels[src_bottom + PIXEL * (w - 1)..src_bottom + PIXEL * w]);
            }
        }

        // A mapped buffer is written only through its views, all dropped
        // before it's unmapped; the belt unmaps its own at `finish`.
        drop(fragment);

        if let Some(buffer) = &one_off {
            buffer.unmap();
        }

        // Copy actual image
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: buffer_slice.buffer(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: buffer_slice.offset(),
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(height + padding.height * 2),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x - padding.width,
                    y: y - padding.height,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::default(),
            },
            wgpu::Extent3d {
                width: width + padding.width * 2,
                height: height + padding.height * 2,
                depth_or_array_layers: 1,
            },
        );
    }

    fn grow(&mut self, amount: usize, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if amount == 0 {
            return;
        }

        let copied = self.layers.len() - amount;

        self.resize(device, encoder, copied);
    }

    /// Anomi: stock `grow`'s body, for growing and shrinking alike. Makes a
    /// texture as deep as `self.layers`, copies the first `copied` layers
    /// that hold pictures from the old one, and binds it.
    fn resize(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, copied: usize) {
        // On the GL backend if layers.len() is a multiple of 6 we need to help wgpu figure out that this texture
        // is still a `GL_TEXTURE_2D_ARRAY` rather than `GL_TEXTURE_CUBE_MAP` or `GL_TEXTURE_CUBE_ARRAY`.
        // This will over-allocate some unused memory on GL, but it's better than not being able to
        // grow the atlas past multiples of 6!
        // https://github.com/gfx-rs/wgpu/blob/004e3efe84a320d9331371ed31fa50baa2414911/wgpu-hal/src/gles/mod.rs#L371
        let depth_or_array_layers = match self.backend {
            wgpu::Backend::Gl if self.layers.len().is_multiple_of(6) => {
                self.layers.len() as u32 + 1
            }
            _ => self.layers.len() as u32,
        };

        let new_texture = texture(device, self.size, depth_or_array_layers);

        for (i, layer) in self.layers.iter().take(copied).enumerate() {
            if layer.is_empty() {
                continue;
            }

            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &new_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::Extent3d {
                    width: self.size,
                    height: self.size,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.texture = new_texture;
        self.texture_view = self.texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        self.texture_bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu::image texture atlas bind group"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.texture_view),
            }],
        }));
    }
}

/// The atlas texture: `layers` layers of `size` pixels a side.
fn texture(device: &wgpu::Device, size: u32, layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("iced_wgpu::image texture atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format(),
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// Anomi: how many layers the atlas keeps: up to the last that holds a
/// picture, and at least `floor`.
fn kept(layers: &[Layer], floor: usize) -> usize {
    layers
        .iter()
        .rposition(|layer| !layer.is_empty())
        .map_or(0, |last| last + 1)
        .max(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_empty_layers_at_the_top_are_given_back() {
        use Layer::{Empty, Full};

        assert_eq!(kept(&[Full, Empty, Empty], 1), 1);
        assert_eq!(kept(&[Full, Empty, Full, Empty], 1), 3, "a hole stays");
        assert_eq!(kept(&[Empty, Empty], 1), 1, "never below the floor");
        assert_eq!(kept(&[Empty, Empty, Empty], 2), 2, "GL's two");
        assert_eq!(kept(&[Full, Full], 1), 2, "nothing to give back");
    }
}
