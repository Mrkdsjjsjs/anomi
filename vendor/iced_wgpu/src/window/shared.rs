//! Anomi: frames drawn on an adapter the desktop doesn't composite on, read
//! back and shown through shared memory (`PATCHES.md`, "Drawn on one
//! adapter, shown through another").
//!
//! A Vulkan surface hands the desktop's compositor the adapter's own
//! buffers (dmabufs) to import. A compositor on another adapter may not be
//! able to: mutter on an NVIDIA card fails to import a Radeon's, and a
//! Wayland compositor answers that with a protocol error, which ends the
//! client's connection and the process with it (the owner's desktop:
//! "failed to import supplied dmabufs: Could not bind the given EGLImage to
//! a CoglTexture2D", 150 ms after the window opened). Vulkan said the
//! Radeon could present to the window: it asks whether the driver can speak
//! to the window system, not whether the compositor can import what it
//! makes. So on any adapter but the desktop's (`adapter::path`) the window
//! gets no Vulkan surface at all. Each frame iced presents is drawn into a
//! texture of the window's size on the adapter ([`Frame`]), copied there
//! into a buffer the processor can read, and its rows copied once into
//! softbuffer's buffer: the shared memory tiny-skia's frames go through
//! (`wl_shm` on Wayland, MIT-SHM on X11), which a compositor takes whatever
//! adapter it draws on.
//!
//! The cost, for each frame presented and only then (nothing runs between
//! frames: no thread, no timer, nothing polled):
//!
//! - Two copies: the texture into the read-back buffer, on the adapter, and
//!   that buffer into softbuffer's, a `memcpy` a row on the window's thread.
//!   The texture's format is softbuffer's byte order ([`FORMAT`]), so no
//!   pass touches a pixel.
//! - One wait on the window's thread ([`Frame::read`]): for the adapter to
//!   finish this frame's submission, its drawing and the copy, nothing
//!   else. Measured on the Radeon 610M while scrolling a channel, the wait
//!   and the `memcpy`: 4.4 ms a frame at 1400 by 868, 10.4 ms maximised at
//!   3440 by 1408, of 5.0 and 11.3 ms on the window's thread in all
//!   (`docs/measurements/README.md`). Softbuffer's
//!   own wait, for the compositor to give its buffer back, runs meanwhile:
//!   the frame is submitted before the buffer is asked for. Past
//!   [`PATIENCE`] the frame is given up and counted as one the surface
//!   couldn't give (`compositor::MISSED`).
//! - Memory: the texture (on an integrated adapter, in system memory) and
//!   the read-back buffer, each the window's size, made at the first frame
//!   and again when the size changes, without waiting for the adapter (the
//!   old ones go once it's done with them), freed with the window; and
//!   softbuffer's two buffers, as tiny-skia's.
//!
//! Damage could cut both copies, and the compositor's upload, to what
//! changed: the union of the damage of the frames since the buffer
//! softbuffer hands back was last shown (its age). This renderer has no
//! damage yet, it draws the whole window each frame (`PATCHES.md`, "Not
//! done here"), so the whole frame is copied and presented.

use crate::Renderer;
use crate::core::{Color, Size};
use crate::graphics::compositor::{self, SurfaceError};
use crate::graphics::{self, Viewport};

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

/// The format frames are drawn in. Softbuffer's pixel is a `u32` of
/// `0x00RRGGBB`: in memory, on a little-endian machine, B, G, R and a byte
/// the window system ignores (`wl_shm`'s XRGB8888, X11's depth 24). That is
/// `Bgra8Unorm`'s byte order, its alpha in the ignored byte. sRGB or not as
/// the window's own surface is chosen (`Compositor::start`): with
/// `web-colors`, the workspace's, colours blend in sRGB as tiny-skia's do
/// and are written as they are, so a frame here is the one the direct path
/// would draw, byte for byte. Premultiplied there and here; the window's
/// background is opaque, so nothing is left see-through.
pub(crate) const FORMAT: wgpu::TextureFormat = if graphics::color::GAMMA_CORRECTION {
    wgpu::TextureFormat::Bgra8UnormSrgb
} else {
    wgpu::TextureFormat::Bgra8Unorm
};

/// How long a frame's drawing and copy may take before it's given up.
const PATIENCE: Duration = Duration::from_secs(1);

/// The window system's side of the shared memory, one for a compositor.
pub(crate) struct Context(softbuffer::Context<Box<dyn compositor::Display>>);

impl Context {
    pub(crate) fn new(display: Box<dyn compositor::Display>) -> Result<Self, String> {
        // No wgpu format has softbuffer's byte order on a big-endian machine.
        if cfg!(target_endian = "big") {
            return Err(String::from(
                "shared memory takes a little-endian machine's pixels",
            ));
        }

        softbuffer::Context::new(display)
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

/// A window's shared memory, and the frame drawn for it.
pub(crate) struct Window {
    surface: softbuffer::Surface<Box<dyn compositor::Display>, Box<dyn compositor::Window>>,
    /// Its size was set: softbuffer hands out no buffer before.
    sized: bool,
    frame: Option<Frame>,
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Window")
            .field("sized", &self.sized)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl Window {
    pub(crate) fn new(
        context: &Context,
        window: Box<dyn compositor::Window>,
    ) -> Result<Self, String> {
        softbuffer::Surface::new(&context.0, window)
            .map(|surface| Self {
                surface,
                sized: false,
                frame: None,
            })
            .map_err(|error| error.to_string())
    }

    /// Sizes it to the window; a frame of another size is let go now.
    pub(crate) fn configure(&mut self, width: u32, height: u32) -> Result<(), String> {
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            self.sized = false;

            return Ok(());
        };

        self.surface
            .resize(width, height)
            .map_err(|error| error.to_string())?;

        self.sized = true;

        let size = Size::new(width.get(), height.get());
        let _other = self.frame.take_if(|frame| frame.size != size);

        Ok(())
    }

    /// Draws the frame on the adapter, reads it back into the window's
    /// shared memory and presents it. A lost device answers "occluded", as
    /// the direct path does; a frame that couldn't be read back "other",
    /// and one softbuffer couldn't take "lost": both count toward
    /// `compositor::MISSED`.
    pub(crate) fn present(
        &mut self,
        renderer: &mut Renderer,
        viewport: &Viewport,
        background: Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), SurfaceError> {
        let size = viewport.physical_size();

        if !self.sized || size.width == 0 || size.height == 0 {
            return Err(SurfaceError::Occluded);
        }

        let frame = match self.frame.take() {
            Some(frame) if frame.size == size => frame,
            _ => Frame::new(&renderer.engine.device, size),
        };

        let submission = frame.submit(renderer, viewport, background);

        // While the adapter draws: softbuffer's buffer, which on Wayland may
        // wait for the compositor to give it back. On a failure the frame,
        // whose buffer may still be mapping, goes, and is made anew.
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return Err(SurfaceError::Lost);
        };

        let width = buffer.width().get() as usize;

        if let Err(failure) = frame.read(renderer, submission, &mut buffer, width) {
            log::warn!("a frame drawn on the adapter couldn't be read back: {failure}");

            return Err(match failure {
                Failed::Lost => SurfaceError::Occluded,
                Failed::Other(_) => SurfaceError::Other,
            });
        }

        self.frame = Some(frame);

        on_pre_present();

        buffer.present().map_err(|_error| SurfaceError::Lost)
    }
}

/// Why a frame wasn't read back.
#[derive(Debug)]
pub(crate) enum Failed {
    /// The device was lost: nothing more is drawn with it.
    Lost,
    /// The wait or the mapping failed.
    Other(String),
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lost => f.write_str("the device was lost"),
            Self::Other(reason) => f.write_str(reason),
        }
    }
}

/// The texture a window's frame is drawn into on the adapter, and the buffer
/// it's copied into for the processor to read; reused while the window
/// keeps its size.
#[derive(Debug)]
pub(crate) struct Frame {
    size: Size<u32>,
    /// Bytes from one row of `buffer` to the next: a row's `width * 4`,
    /// rounded up to the 256 a copy from a texture takes.
    stride: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl Frame {
    pub(crate) fn new(device: &wgpu::Device, size: Size<u32>) -> Self {
        let stride = (size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("iced_wgpu.shared.frame"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("iced_wgpu.shared.read_back"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            size,
            stride,
            texture,
            view,
            buffer,
        }
    }

    /// Draws the renderer's frame into the texture, has it copied into the
    /// buffer in the same submission, and asks for the buffer to be mapped
    /// once that's done; returns at once.
    pub(crate) fn submit(
        &self,
        renderer: &mut Renderer,
        viewport: &Viewport,
        background: Color,
    ) -> wgpu::SubmissionIndex {
        let mut encoder = renderer.draw(Some(background), &self.view, viewport);

        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: None,
                },
            },
            extent(self.size),
        );

        renderer.finish();
        let submission = renderer.engine.queue.submit([encoder.finish()]);
        renderer.recall();

        // Whether it mapped is read below: `get_mapped_range` fails if not.
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_mapped| {});

        submission
    }

    /// Waits for `submission` and copies the frame's rows into `window`,
    /// `width` pixels a row. The wait is for this frame's drawing and copy
    /// on the adapter, nothing else, and at most [`PATIENCE`]. On a failure
    /// the frame must be dropped: its buffer may still be mapping, and a
    /// buffer whose mapping failed can't be unmapped without an error.
    pub(crate) fn read(
        &self,
        renderer: &Renderer,
        submission: wgpu::SubmissionIndex,
        window: &mut [u32],
        width: usize,
    ) -> Result<(), Failed> {
        // A wait on a lost device panics in wgpu.
        if renderer.engine.is_lost() {
            return Err(Failed::Lost);
        }

        let _status = renderer
            .engine
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(PATIENCE),
            })
            .map_err(|error| Failed::Other(error.to_string()))?;

        let bytes = self
            .buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| Failed::Other(error.to_string()))?;

        copy_rows(
            &bytes,
            self.stride as usize,
            self.size.width as usize,
            window,
            width,
        );

        drop(bytes);
        self.buffer.unmap();

        Ok(())
    }
}

fn extent(size: Size<u32>) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.width,
        height: size.height,
        depth_or_array_layers: 1,
    }
}

/// Copies the rows of a read-back buffer, `stride` bytes apart and `width`
/// pixels each, into a window's, `window_width` pixels a row, as far as both
/// go: a `memcpy` a row, the bytes as they are.
fn copy_rows(bytes: &[u8], stride: usize, width: usize, window: &mut [u32], window_width: usize) {
    let width = width.min(window_width);

    if width == 0 || stride == 0 {
        return;
    }

    for (row, out) in bytes.chunks(stride).zip(window.chunks_mut(window_width)) {
        let (Some(row), Some(out)) = (row.get(..width * 4), out.get_mut(..width)) else {
            break;
        };

        bytemuck::cast_slice_mut::<u32, u8>(out).copy_from_slice(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Engine;
    use crate::adapter;
    use crate::core::renderer::{self, Quad, Renderer as _};
    use crate::core::{Border, Point, Rectangle};
    use crate::graphics::{Antialiasing, Shell};

    #[test]
    fn rows_are_copied_without_their_padding_as_far_as_both_go() {
        // Two rows of three pixels, 16 bytes apart: 4 bytes of padding.
        let bytes: Vec<u8> = (0..32).collect();
        let pixel = |at: u8| u32::from_ne_bytes([at, at + 1, at + 2, at + 3]);

        let mut window = vec![0; 6];
        copy_rows(&bytes, 16, 3, &mut window, 3);
        assert_eq!(
            window,
            [
                pixel(0),
                pixel(4),
                pixel(8),
                pixel(16),
                pixel(20),
                pixel(24)
            ]
        );

        // A window narrower than the frame, and one wider.
        let mut narrow = vec![0; 4];
        copy_rows(&bytes, 16, 3, &mut narrow, 2);
        assert_eq!(narrow, [pixel(0), pixel(4), pixel(16), pixel(20)]);

        let mut wide = vec![0; 8];
        copy_rows(&bytes, 16, 3, &mut wide, 4);
        assert_eq!(
            wide,
            [
                pixel(0),
                pixel(4),
                pixel(8),
                0,
                pixel(16),
                pixel(20),
                pixel(24),
                0
            ]
        );

        copy_rows(&bytes, 0, 3, &mut wide, 0);
    }

    #[test]
    fn the_formats_bytes_are_softbuffers_pixel() {
        if cfg!(target_endian = "big") {
            return;
        }

        // B, G, R and alpha, as `Bgra8Unorm` lays a texel out.
        let texel = [0x10, 0x20, 0x30, 0xff];

        assert!(matches!(
            FORMAT,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ));
        assert_eq!(u32::from_ne_bytes(texel) & 0x00ff_ffff, 0x0030_2010);
    }

    fn scene(gpu: &mut Renderer, size: Size<u32>) {
        gpu.reset(Rectangle::with_size(Size::new(
            size.width as f32,
            size.height as f32,
        )));

        gpu.fill_quad(
            Quad {
                bounds: Rectangle::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0)),
                border: Border {
                    radius: 12.0.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            Color::from_rgb8(0xff, 0x00, 0x00),
        );

        gpu.fill_quad(
            Quad {
                bounds: Rectangle::new(Point::new(60.5, 40.25), Size::new(200.0, 120.0)),
                ..Quad::default()
            },
            Color::from_rgba8(0x00, 0xff, 0x00, 0.5),
        );
    }

    /// Opt-in, as it starts a GPU: `ANOMI_GPU_TESTS=1`, on the integrated
    /// adapter or the one at `ANOMI_GPU_BUS` ("0000:73:00.0"). A frame drawn
    /// offscreen and read back as a window's shared memory takes it equals
    /// the same frame read out by `Renderer::screenshot`, which draws it in
    /// place into a texture as the direct path draws into the window's.
    #[test]
    fn a_frame_read_back_equals_the_one_drawn_in_place() {
        if std::env::var_os("ANOMI_GPU_TESTS").is_none() {
            return;
        }

        let wanted =
            std::env::var("ANOMI_GPU_BUS").map_or(adapter::Choice::Auto, adapter::Choice::Bus);
        let instance = adapter::instance_without_window();
        let mut candidates = adapter::ready(adapter::candidates(&instance, wgpu::Backends::VULKAN))
            .unwrap_or_default();
        let infos: Vec<_> = candidates
            .iter()
            .map(|(_adapter, info)| info.clone())
            .collect();
        let index = adapter::pick(&infos, &wanted)
            .unwrap_or_else(|reason| panic!("{reason}, among {infos:?}"));
        let (chosen, info) = candidates.swap_remove(index);

        let (device, queue) = adapter::ready(chosen.request_device(&wgpu::DeviceDescriptor {
            label: Some("iced_wgpu.shared test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_bind_groups: 2,
                ..wgpu::Limits::default()
            },
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("no device on {}", info.name));

        let engine = Engine::new(
            &chosen,
            device,
            queue,
            FORMAT,
            Some(Antialiasing::MSAAx4),
            Shell::headless(),
        );
        let mut gpu = Renderer::new(engine, renderer::Settings::default());

        // Not a multiple of 64 pixels wide: the buffer's rows are padded.
        let size = Size::new(333, 217);
        let viewport = Viewport::with_physical_size(
            size,
            renderer::Scale {
                window: 1.0,
                application: 1.0,
            },
        );
        let background = Color::from_rgb8(0x20, 0x40, 0x60);

        let mut window = vec![0_u32; (size.width * size.height) as usize];

        scene(&mut gpu, size);
        let frame = Frame::new(&gpu.engine.device, size);
        let submission = frame.submit(&mut gpu, &viewport, background);

        if let Err(failure) = frame.read(&gpu, submission, &mut window, size.width as usize) {
            panic!("{failure}");
        }

        scene(&mut gpu, size);
        let rgba = gpu.screenshot(&viewport, background);

        assert_eq!(rgba.len(), window.len() * 4);

        for (index, (pixel, theirs)) in window.iter().zip(rgba.chunks(4)).enumerate() {
            let [b, g, r, _alpha] = pixel.to_ne_bytes();

            // Byte for byte: measured on the Radeon 610M and the RTX 3080.
            assert!(
                [r, g, b] == theirs[..3],
                "pixel {index} ({}, {}): {:?}, in place {theirs:?}",
                index % size.width as usize,
                index / size.width as usize,
                [r, g, b],
            );
        }

        // Red stays red: the quad's middle, at (60, 20).
        let [b, g, r, _alpha] = window[(20 * size.width + 60) as usize].to_ne_bytes();

        assert_eq!([r, g, b], [0xff, 0x00, 0x00]);
    }
}
