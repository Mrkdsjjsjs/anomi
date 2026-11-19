//! Anomi: pictures the app marked as premultiplied BGRA (`PATCHES.md`,
//! "Marked pictures").
//!
//! The app hands most of its pictures and every video frame over in
//! tiny-skia's own format, premultiplied BGRA, and marks their buffers in
//! `iced_tiny_skia::premultiplied`'s registry, so the CPU renderer draws
//! them from their own bytes. Stock, this renderer took every `Rgba` handle
//! for straight RGBA: a marked picture came out with red and blue swapped,
//! premultiplied twice (its see-through edges darker). Here the same marks
//! are read when a picture is uploaded, the bytes go to the GPU as they
//! are, and the instance's flags have the shader swap red and blue back and
//! skip its premultiply. Nothing is converted on either renderer, and a
//! switch between them at run time needs no picture made again.

use crate::core::image::Handle;

/// The texel's red and blue are swapped: the shader reads `.bgra`.
pub const BGRA: u32 = 1;
/// The texel is premultiplied already: the shader doesn't premultiply it.
pub const PREMULTIPLIED: u32 = 2;

/// How a picture's bytes are to be read, and whether it's a video frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mark {
    /// [`BGRA`] and [`PREMULTIPLIED`], for the shader; none for straight
    /// RGBA.
    pub flags: u32,
    /// A video frame (`premultiplied::frame`, `frame_of`): a new handle each
    /// frame, which takes a texture of its own at any size (`own.rs`).
    pub frame: bool,
}

/// The mark of `handle`, read while its bytes are alive (the registry knows
/// a buffer only while it lives). An encoded or unmarked handle is straight
/// RGBA, as stock.
pub fn of(handle: &Handle) -> Mark {
    if !iced_tiny_skia::premultiplied::is_premultiplied(handle) {
        return Mark::default();
    }

    Mark {
        flags: BGRA | PREMULTIPLIED,
        frame: iced_tiny_skia::premultiplied::is_frame(handle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    #[test]
    fn marks_are_tiny_skias() {
        let rgba = vec![200, 100, 50, 255, 200, 100, 50, 128];

        let stock = Handle::from_rgba(2, 1, rgba.clone());
        assert_eq!(of(&stock), Mark::default(), "straight RGBA");

        let picture = iced_tiny_skia::premultiplied::from_rgba(2, 1, rgba);
        assert_eq!(
            of(&picture),
            Mark {
                flags: BGRA | PREMULTIPLIED,
                frame: false,
            }
        );

        let pixels = Arc::new(vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let frame = iced_tiny_skia::premultiplied::frame(2, 1, pixels, true);
        assert_eq!(
            of(&frame),
            Mark {
                flags: BGRA | PREMULTIPLIED,
                frame: true,
            }
        );

        // A copy of a marked picture's bytes isn't marked.
        let Handle::Rgba { pixels, .. } = &picture else {
            panic!("pixels");
        };
        let copy = Handle::from_rgba(2, 1, pixels.to_vec());
        assert_eq!(of(&copy), Mark::default());
    }
}
