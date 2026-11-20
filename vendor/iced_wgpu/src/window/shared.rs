//! Anomi: frames drawn on an adapter the desktop doesn't composite on, read
//! back and shown through shared memory (`PATCHES.md`, "Drawn on one
//! adapter, shown through another" and "Only what changed, read back and
//! shown").
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
//! Only what changed is drawn, read back and shown:
//!
//! - The frame's layers are held against the frame presented last's
//!   (`damage.rs`, as tiny-skia damages its frames): where they may lay
//!   other pixels is the frame's change. A frame of another size, scale,
//!   background or set of fonts, the first, and the first after the window
//!   was sized or a frame failed, change the whole window.
//! - softbuffer's buffer holds the frame presented `age` frames ago (two on
//!   Wayland, one on X11; none known after a resize): it lacks the frame's
//!   change and the changes of the frames presented since its own
//!   (`damage::History`). Only those regions, made disjoint, are drawn on
//!   the adapter (each primitive scissored to them, so their pixels are
//!   the frame drawn whole's), copied into the read-back buffer one after
//!   another, and copied into the buffer at their places.
//! - The compositor is told the frame's own change (`present_with_damage`),
//!   against the frame it shows, so it uploads only that.
//! - A frame that changes nothing, into a buffer that lacks nothing, draws
//!   and reads back nothing: the buffer is presented with no damage.
//!
//! The cost, for each frame presented and only then (nothing runs between
//! frames: no thread, no timer, nothing polled):
//!
//! - The frame's layers held against the frame before's on the window's
//!   thread, and what they draw kept for the next ([`Scene`]: quads, text
//!   and live meshes by value, pictures and caches by id), as tiny-skia
//!   keeps its frames' layers. Every layer is still prepared for the
//!   adapter, as for the whole frame.
//! - Two copies of the lacking regions: the texture's into the read-back
//!   buffer, on the adapter, and that buffer's into softbuffer's, a
//!   `memcpy` a row on the window's thread. The texture's format is
//!   softbuffer's byte order ([`FORMAT`]), so no pass touches a pixel.
//! - One wait on the window's thread ([`Frame::read`]): for the adapter to
//!   finish this frame's drawing and copies, nothing else, at most
//!   [`PATIENCE`]; a frame given up counts as one the surface couldn't give
//!   (`compositor::MISSED`). softbuffer's buffer is taken first, as which
//!   regions to draw depends on its age; its wait, for the compositor to
//!   give it back, is a buffer committed two frames before, given back
//!   long since. Measured on the Radeon 610M while scrolling a channel:
//!   3.0 ms a frame at 1400 by 868 and 7.8 ms maximised at 3440 by 1408
//!   (the whole frame drawn and read back, 4.1 and 10.1 ms); a key typed,
//!   1.6 and 2.9 ms (`docs/measurements/README.md`).
//! - Memory: the texture (on an integrated adapter, in system memory),
//!   made at the first frame drawn and again when the size changes, without
//!   waiting for the adapter, freed with the window; the read-back buffer,
//!   made at the size of the largest regions read back (the whole window
//!   at the first frame) and again at the size of what's read back once
//!   that has been a quarter of it or less for 30 frames in a row; and
//!   softbuffer's two buffers, as tiny-skia's.

use crate::Renderer;
use crate::core::{Color, Rectangle, Size};
use crate::damage::{self, Change, History, Scene};
use crate::graphics::compositor::{self, SurfaceError};
use crate::graphics::{self, Viewport};

use std::fmt;
use std::num::NonZeroU32;
use std::sync::{OnceLock, PoisonError};
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

/// Bytes the read-back buffer is made in multiples of.
const STEP: u64 = 64 * 1024;

/// Frames in a row whose read-back takes a quarter of the read-back buffer
/// or less, before the buffer is made again at their size.
const SHRINK_AFTER: u8 = 30;

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
    /// What the frame presented last was drawn with, and what its layers
    /// draw: the next frame's change is worked out against it.
    shown: Option<(Key, Scene)>,
    /// What the frames presented changed (`damage::History`).
    history: History,
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Window")
            .field("sized", &self.sized)
            .field("frame", &self.frame)
            .field("history", &self.history)
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
                shown: None,
                history: History::default(),
            })
            .map_err(|error| error.to_string())
    }

    /// Sizes it to the window; a frame of another size is let go now. What
    /// softbuffer's buffers hold is no longer known (a resized buffer keeps
    /// its age), so the next frame is drawn and shown whole.
    pub(crate) fn configure(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.forget();

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

    /// Draws on the adapter what the window's buffer lacks of the frame,
    /// reads it back into the window's shared memory and presents it, the
    /// frame's change as its damage. A lost device answers "occluded", as
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

        // What the frame draws, held against what the frame presented last
        // drew.
        renderer.settle();

        let key = Key::of(viewport, background);
        let scene = Scene::of(renderer.layers.as_slice());

        let change = match &self.shown {
            Some((shown, before)) if *shown == key => {
                Change::Regions(damage::regions(Scene::between(before, &scene), viewport))
            }
            _ => Change::Whole,
        };

        let mut buffer = match self.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(_error) => {
                self.shown = None;
                self.history.forget();

                return Err(SurfaceError::Lost);
            }
        };

        let window = Size::new(buffer.width().get(), buffer.height().get());

        let lacking = match self.history.lacking(buffer.age(), &change) {
            Change::Whole => vec![Rectangle::with_size(size)],
            Change::Regions(regions) => regions,
        };

        if !lacking.is_empty() {
            let mut frame = match self.frame.take() {
                Some(frame) if frame.size == size => frame,
                _ => Frame::new(&renderer.engine.device, size),
            };

            // On a failure the frame, whose buffer may still be mapping,
            // goes, and is made anew.
            match frame.read(
                renderer,
                viewport,
                background,
                &lacking,
                &mut buffer,
                window.width as usize,
            ) {
                Ok(read) => {
                    log::trace!(
                        "read back {read} bytes in {} regions, {} damaged",
                        lacking.len(),
                        match &change {
                            Change::Whole => "the whole window",
                            Change::Regions(_) => "regions",
                        }
                    );

                    self.frame = Some(frame);
                }
                Err(failure) => {
                    log::warn!("a frame drawn on the adapter couldn't be read back: {failure}");

                    self.shown = None;
                    self.history.forget();

                    return Err(match failure {
                        Failed::Lost => SurfaceError::Occluded,
                        Failed::Other(_) => SurfaceError::Other,
                    });
                }
            }
        }

        if verifying() {
            verify(
                renderer,
                viewport,
                background,
                &buffer,
                window.width as usize,
            );
        }

        on_pre_present();

        let presented = match &change {
            Change::Whole => buffer.present(),
            Change::Regions(regions) => buffer.present_with_damage(&damaged(regions, window)),
        };

        // A buffer of another size than the frame (a resize under way) holds
        // only part of it, as far as both go.
        if presented.is_err() || window != size {
            self.shown = None;
            self.history.forget();
        } else {
            self.history.presented(change);
            self.shown = Some((key, scene));
        }

        presented.map_err(|_error| SurfaceError::Lost)
    }

    /// What softbuffer's buffers hold is no longer known.
    fn forget(&mut self) {
        self.shown = None;
        self.history.forget();
    }
}

/// What a frame is drawn with beyond its layers: a frame drawn with other
/// changes every pixel, as far as its layers can tell.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    size: Size<u32>,
    scale: f32,
    background: Color,
    /// Fonts loaded since lay other glyphs for the same text.
    fonts: graphics::text::Version,
}

impl Key {
    fn of(viewport: &Viewport, background: Color) -> Self {
        Self {
            size: viewport.physical_size(),
            scale: viewport.scale_factor(),
            background,
            fonts: graphics::text::font_system()
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .version(),
        }
    }
}

/// `regions` as softbuffer's damage, within a buffer of `window`'s size.
fn damaged(regions: &[Rectangle<u32>], window: Size<u32>) -> Vec<softbuffer::Rect> {
    regions
        .iter()
        .filter_map(|region| {
            let region = damage::within(*region, Rectangle::with_size(window))?;

            Some(softbuffer::Rect {
                x: region.x,
                y: region.y,
                width: NonZeroU32::new(region.width)?,
                height: NonZeroU32::new(region.height)?,
            })
        })
        .collect()
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
/// its regions are copied into for the processor to read; reused while the
/// window keeps its size.
#[derive(Debug)]
pub(crate) struct Frame {
    size: Size<u32>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Made at the first frame read back, at the size it needs
    /// ([`Frame::reserve`]).
    read_back: Option<wgpu::Buffer>,
    /// Frames in a row that read back a quarter of `read_back` or less.
    oversized: u8,
}

/// Where a region's rows go in the read-back buffer: from `offset`,
/// `stride` bytes apart, its row's `width * 4` rounded up to the 256 a copy
/// from a texture takes.
#[derive(Debug, Clone, Copy)]
struct Placed {
    region: Rectangle<u32>,
    offset: u64,
    stride: u32,
}

impl Frame {
    pub(crate) fn new(device: &wgpu::Device, size: Size<u32>) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("iced_wgpu.shared.frame"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            size,
            texture,
            view,
            read_back: None,
            oversized: 0,
        }
    }

    /// Draws the renderer's frame on the adapter within `regions` alone
    /// (physical, disjoint, within the frame), has each region copied into
    /// the read-back buffer in the same submission, waits for that, and
    /// copies each region's rows into `window`, `width` pixels a row, at
    /// the region's place: there the window holds the frame, byte for byte.
    /// The wait is for this frame's drawing and copies on the adapter,
    /// nothing else, and at most [`PATIENCE`]. Answers the bytes read back.
    /// On a failure the frame must be dropped: its buffer may still be
    /// mapping, and a buffer whose mapping failed can't be unmapped without
    /// an error.
    pub(crate) fn read(
        &mut self,
        renderer: &mut Renderer,
        viewport: &Viewport,
        background: Color,
        regions: &[Rectangle<u32>],
        window: &mut [u32],
        width: usize,
    ) -> Result<u64, Failed> {
        // A wait on a lost device panics in wgpu.
        if renderer.engine.is_lost() {
            return Err(Failed::Lost);
        }

        let mut places = Vec::with_capacity(regions.len());
        let mut used = 0;

        for region in regions {
            let stride = (region.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

            places.push(Placed {
                region: *region,
                offset: used,
                stride,
            });

            used += u64::from(stride) * u64::from(region.height);
        }

        if used == 0 {
            return Ok(0);
        }

        let device = renderer.engine.device.clone();
        self.reserve(&device, used);

        let Some(read_back) = &self.read_back else {
            return Err(Failed::Other(String::from("no read-back buffer")));
        };

        let mut encoder = renderer.draw_within(Some(background), &self.view, viewport, regions);

        for place in &places {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: place.region.x,
                        y: place.region.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: read_back,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: place.offset,
                        bytes_per_row: Some(place.stride),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: place.region.width,
                    height: place.region.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        renderer.finish();
        let submission = renderer.engine.queue.submit([encoder.finish()]);
        renderer.recall();

        // Whether it mapped is read below: `get_mapped_range` fails if not.
        read_back
            .slice(..used)
            .map_async(wgpu::MapMode::Read, |_mapped| {});

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

        let bytes = read_back
            .slice(..used)
            .get_mapped_range()
            .map_err(|error| Failed::Other(error.to_string()))?;

        for place in &places {
            copy_region(&bytes, place, window, width);
        }

        drop(bytes);
        read_back.unmap();

        Ok(used)
    }

    /// Has a read-back buffer of at least `used` bytes: made larger when it
    /// isn't (twice as large, as far as the whole frame, so a list scrolled
    /// grows it once or twice), and smaller once frames in a row have read
    /// back a quarter of it or less. A buffer let go here is in no
    /// submission: each read waits for its own.
    fn reserve(&mut self, device: &wgpu::Device, used: u64) {
        let capacity = self.read_back.as_ref().map_or(0, wgpu::Buffer::size);
        let fitted = used.next_multiple_of(STEP);

        if used > capacity {
            let row = (self.size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let whole = u64::from(row) * u64::from(self.size.height);
            let size = fitted.max(capacity.saturating_mul(2).min(whole).next_multiple_of(STEP));

            self.read_back = Some(new_read_back(device, size));
            self.oversized = 0;
        } else if fitted.saturating_mul(4) <= capacity {
            self.oversized = self.oversized.saturating_add(1);

            if self.oversized >= SHRINK_AFTER {
                self.read_back = Some(new_read_back(device, fitted));
                self.oversized = 0;
            }
        } else {
            self.oversized = 0;
        }
    }
}

fn new_read_back(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("iced_wgpu.shared.read_back"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Copies a region's rows from the read-back buffer into a window's,
/// `width` pixels a row, at the region's place, as far as the window goes:
/// a `memcpy` a row, the bytes as they are.
fn copy_region(bytes: &[u8], place: &Placed, window: &mut [u32], width: usize) {
    let Placed {
        region,
        offset,
        stride,
    } = *place;

    let (x, y) = (region.x as usize, region.y as usize);
    let columns = (region.width as usize).min(width.saturating_sub(x));

    if columns == 0 {
        return;
    }

    for row in 0..region.height as usize {
        let from = offset as usize + row * stride as usize;
        let to = (y + row) * width + x;

        let (Some(source), Some(target)) = (
            bytes.get(from..from + columns * 4),
            window.get_mut(to..to + columns),
        ) else {
            break;
        };

        bytemuck::cast_slice_mut::<u32, u8>(target).copy_from_slice(source);
    }
}

/// Whether each frame presented is held against the frame drawn whole, for
/// a review: `ANOMI_SHARED_VERIFY` set. Each frame then draws the whole
/// frame again and reads it back (`Renderer::screenshot`), and any pixel of
/// the window's buffer that differs is logged.
fn verifying() -> bool {
    static VERIFYING: OnceLock<bool> = OnceLock::new();

    *VERIFYING.get_or_init(|| std::env::var_os("ANOMI_SHARED_VERIFY").is_some())
}

/// Logs the pixels of `window`, `width` pixels a row, that differ from the
/// frame drawn whole.
fn verify(
    renderer: &mut Renderer,
    viewport: &Viewport,
    background: Color,
    window: &[u32],
    width: usize,
) {
    let size = viewport.physical_size();
    let whole = renderer.screenshot(viewport, background);
    let columns = size.width as usize;

    let mut differ = 0_usize;
    let mut within: Option<(usize, usize, usize, usize)> = None;

    for y in 0..size.height as usize {
        for x in 0..columns {
            let at = (y * columns + x) * 4;

            let (Some(pixel), Some(drawn)) = (window.get(y * width + x), whole.get(at..at + 3))
            else {
                continue;
            };

            let [blue, green, red, _ignored] = pixel.to_ne_bytes();

            if drawn != [red, green, blue].as_slice() {
                differ += 1;

                within = Some(match within {
                    None => (x, y, x, y),
                    Some((left, top, right, bottom)) => {
                        (left.min(x), top.min(y), right.max(x), bottom.max(y))
                    }
                });
            }
        }
    }

    if let Some((left, top, right, bottom)) = within {
        log::warn!(
            "a frame shown from its damage differs from the frame drawn whole \
             in {differ} pixels, from ({left}, {top}) to ({right}, {bottom})"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Engine;
    use crate::adapter;
    use crate::core::renderer::{self, Quad, Renderer as _};
    use crate::core::{Border, Point, Transformation};
    use crate::graphics::mesh::{self, Renderer as _};
    use crate::graphics::{Antialiasing, Mesh, Shell};

    #[test]
    fn rows_are_copied_without_their_padding_as_far_as_the_window_goes() {
        // Two rows of three pixels, 16 bytes apart: 4 bytes of padding.
        let bytes: Vec<u8> = (0..48).collect();
        let pixel = |at: u8| u32::from_ne_bytes([at, at + 1, at + 2, at + 3]);
        let place = |x, y, width, height, offset| Placed {
            region: Rectangle {
                x,
                y,
                width,
                height,
            },
            offset,
            stride: 16,
        };

        let mut window = vec![0; 6];
        copy_region(&bytes, &place(0, 0, 3, 2, 0), &mut window, 3);
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

        // A window narrower than the region, and one wider.
        let mut narrow = vec![0; 4];
        copy_region(&bytes, &place(0, 0, 3, 2, 0), &mut narrow, 2);
        assert_eq!(narrow, [pixel(0), pixel(4), pixel(16), pixel(20)]);

        let mut wide = vec![0; 8];
        copy_region(&bytes, &place(0, 0, 3, 2, 0), &mut wide, 4);
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

        // A region at its place, its rows from its offset; rows past the
        // window's bottom are left.
        let mut placed = vec![0; 8];
        copy_region(&bytes, &place(2, 1, 2, 3, 32), &mut placed, 4);
        assert_eq!(placed, [0, 0, 0, 0, 0, 0, pixel(32), pixel(36)]);

        copy_region(&bytes, &place(0, 0, 3, 2, 0), &mut wide, 0);
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

    #[test]
    fn damage_is_told_within_the_buffer() {
        let shown = damaged(
            &[
                Rectangle {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
                Rectangle {
                    x: 90,
                    y: 50,
                    width: 30,
                    height: 40,
                },
                Rectangle {
                    x: 200,
                    y: 0,
                    width: 10,
                    height: 10,
                },
            ],
            Size::new(100, 80),
        );

        let shown: Vec<_> = shown
            .iter()
            .map(|rect| (rect.x, rect.y, rect.width.get(), rect.height.get()))
            .collect();

        assert_eq!(shown, [(10, 20, 30, 40), (90, 50, 10, 30)]);
    }

    /// Two quads, the second see-through over the first.
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

    /// The scene, a small rounded quad of `color` at (200, 150), and a
    /// see-through triangle drawn over it and across the window, antialiased.
    fn scene_with(gpu: &mut Renderer, size: Size<u32>, color: Color) {
        scene(gpu, size);

        gpu.fill_quad(
            Quad {
                bounds: Rectangle::new(Point::new(200.0, 150.0), Size::new(10.0, 8.0)),
                border: Border {
                    radius: 3.0.into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            color,
        );

        let vertex = |x, y| mesh::SolidVertex2D {
            position: [x, y],
            color: graphics::color::pack(Color::from_rgba8(0x00, 0x40, 0xff, 0.6)),
        };

        gpu.draw_mesh(Mesh::Solid {
            buffers: mesh::Indexed {
                vertices: vec![
                    vertex(120.0, 90.0),
                    vertex(330.0, 120.0),
                    vertex(180.0, 215.0),
                ],
                indices: vec![0, 1, 2],
            },
            transformation: Transformation::IDENTITY,
            clip_bounds: Rectangle::with_size(Size::new(size.width as f32, size.height as f32)),
        });
    }

    /// Opt-in, as it starts a GPU: `ANOMI_GPU_TESTS=1`, on the integrated
    /// adapter or the one at `ANOMI_GPU_BUS` ("0000:73:00.0"). A renderer
    /// drawing as a read-back window's does.
    fn gpu() -> Option<Renderer> {
        if std::env::var_os("ANOMI_GPU_TESTS").is_none() {
            return None;
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

        Some(Renderer::new(engine, renderer::Settings::default()))
    }

    fn viewport(size: Size<u32>) -> Viewport {
        Viewport::with_physical_size(
            size,
            renderer::Scale {
                window: 1.0,
                application: 1.0,
            },
        )
    }

    /// Asserts that `window`, as softbuffer's pixels, is `rgba`, the frame
    /// drawn in place and read out by `Renderer::screenshot`, byte for byte
    /// (measured on the Radeon 610M and the RTX 3080).
    fn assert_shows(window: &[u32], rgba: &[u8], width: u32) {
        assert_eq!(rgba.len(), window.len() * 4);

        for (index, (pixel, theirs)) in window.iter().zip(rgba.chunks(4)).enumerate() {
            let [b, g, r, _alpha] = pixel.to_ne_bytes();

            assert!(
                [r, g, b] == theirs[..3],
                "pixel {index} ({}, {}): {:?}, in place {theirs:?}",
                index % width as usize,
                index / width as usize,
                [r, g, b],
            );
        }
    }

    /// A frame drawn offscreen and read back whole, as a window's shared
    /// memory takes it, equals the same frame read out by
    /// `Renderer::screenshot`, which draws it in place into a texture as the
    /// direct path draws into the window's.
    #[test]
    fn a_frame_read_back_equals_the_one_drawn_in_place() {
        let Some(mut gpu) = gpu() else {
            return;
        };

        // Not a multiple of 64 pixels wide: the buffer's rows are padded.
        let size = Size::new(333, 217);
        let viewport = viewport(size);
        let background = Color::from_rgb8(0x20, 0x40, 0x60);

        let mut window = vec![0_u32; (size.width * size.height) as usize];

        scene(&mut gpu, size);
        let mut frame = Frame::new(&gpu.engine.device, size);

        if let Err(failure) = frame.read(
            &mut gpu,
            &viewport,
            background,
            &[Rectangle::with_size(size)],
            &mut window,
            size.width as usize,
        ) {
            panic!("{failure}");
        }

        scene(&mut gpu, size);
        let rgba = gpu.screenshot(&viewport, background);

        assert_shows(&window, &rgba, size.width);

        // Red stays red: the quad's middle, at (60, 20).
        let [b, g, r, _alpha] = window[(20 * size.width + 60) as usize].to_ne_bytes();

        assert_eq!([r, g, b], [0xff, 0x00, 0x00]);
    }

    /// Opt-in, as above. A small quad changes colour under a see-through
    /// triangle: only the pixels round it are drawn and read back, into a
    /// window that holds the frame before, which then holds the frame drawn
    /// whole, byte for byte.
    #[test]
    fn a_change_alone_is_read_back_and_the_window_shows_the_frame_drawn_whole() {
        let Some(mut gpu) = gpu() else {
            return;
        };

        let size = Size::new(333, 217);
        let viewport = viewport(size);
        let background = Color::from_rgb8(0x20, 0x40, 0x60);
        let width = size.width as usize;

        let mut window = vec![0_u32; width * size.height as usize];
        let mut frame = Frame::new(&gpu.engine.device, size);

        // The frame before, read back whole.
        scene_with(&mut gpu, size, Color::from_rgb8(0xff, 0xff, 0x00));
        gpu.settle();
        let before = Scene::of(gpu.layers.as_slice());

        if let Err(failure) = frame.read(
            &mut gpu,
            &viewport,
            background,
            &[Rectangle::with_size(size)],
            &mut window,
            width,
        ) {
            panic!("{failure}");
        }

        // The next, the small quad blue.
        scene_with(&mut gpu, size, Color::from_rgb8(0x00, 0x00, 0xff));
        gpu.settle();
        let after = Scene::of(gpu.layers.as_slice());

        let regions = damage::settled(damage::regions(Scene::between(&before, &after), &viewport));

        // The quad, a pixel round it for its edge, and one more.
        let round = Rectangle {
            x: 198,
            y: 148,
            width: 14,
            height: 12,
        };

        assert!(!regions.is_empty());
        assert!(
            regions
                .iter()
                .all(|region| damage::within(*region, round) == Some(*region)),
            "{regions:?}"
        );

        let read = frame
            .read(
                &mut gpu,
                &viewport,
                background,
                &regions,
                &mut window,
                width,
            )
            .unwrap_or_else(|failure| panic!("{failure}"));

        // Twelve rows of 256 bytes at most, where the whole frame is 217
        // rows of 1,536.
        assert!(read <= 12 * 256, "{read} bytes read back");

        let rgba = gpu.screenshot(&viewport, background);

        assert_shows(&window, &rgba, size.width);
    }
}
