//! Anomi (`PATCHES.md`, "Large pictures and video frames"): video frames
//! drawn through the whole renderer on a GPU, as the window draws them: a
//! new handle each frame over the memory the app wrote, marked premultiplied
//! BGRA (`iced_tiny_skia::premultiplied::frame`), each uploaded in the frame
//! that draws it into a texture of its own. The GPU's side of
//! `vendor/iced_tiny_skia/tests/frames.rs`.
//!
//! Opt-in, as it starts a GPU (`gpu/mod.rs`):
//!
//!     ANOMI_GPU_TESTS=1 ANOMI_GPU_BUS=0000:01:00.0 cargo test \
//!       --manifest-path vendor/iced_wgpu/Cargo.toml --features image --test frames

#![cfg(feature = "image")]

mod gpu;

use std::sync::Arc;

use iced_tiny_skia::premultiplied;
use iced_wgpu::core::image;
use iced_wgpu::core::{Color, Rectangle, Size};

const WINDOW: Size<u32> = Size::new(1280, 720);

/// Four frame buffers, as the core's feed keeps, each opaque and of its own
/// colour, `width` by `height`, premultiplied BGRA.
fn buffers(width: u32, height: u32) -> Vec<Arc<Vec<u8>>> {
    (0..4u8)
        .map(|n| {
            let pixel = [40 + n * 50, 200 - n * 40, 90 + n * 30, 255];

            Arc::new(pixel.repeat(width as usize * height as usize))
        })
        .collect()
}

/// The colour a buffer's pixels show: its BGRA read as RGBA.
fn shown(buffer: &[u8]) -> [u8; 4] {
    [buffer[2], buffer[1], buffer[0], buffer[3]]
}

/// Whether every pixel of `window` is `pixel`, within a level.
fn all(window: &[u8], pixel: [u8; 4]) -> bool {
    window
        .chunks_exact(4)
        .all(|p| gpu::near([p[0], p[1], p[2], p[3]], pixel, 1))
}

/// Thirty frames of 1280 by 720, each a new handle on one of the feed's
/// buffers, drawn one to one: each shows, red and blue in place, and once
/// the next has been drawn the renderer holds nothing of it, so its buffer
/// (a stream's slot) is its writer's again.
#[test]
fn thirty_frames_of_720p_each_show_and_none_is_kept() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    let buffers = buffers(WINDOW.width, WINDOW.height);
    let whole = Rectangle::with_size(Size::new(WINDOW.width as f32, WINDOW.height as f32));

    for at in 0..30 {
        let frame =
            premultiplied::frame(WINDOW.width, WINDOW.height, buffers[at % 4].clone(), true);

        let window = gpu::draw(
            &mut renderer,
            image::Image::new(frame),
            whole,
            WINDOW,
            Color::BLACK,
        );

        assert!(all(&window, shown(&buffers[at % 4])), "frame {at} shows");

        if at > 0 {
            assert_eq!(
                Arc::strong_count(&buffers[(at - 1) % 4]),
                1,
                "frame {} let go once the next was drawn",
                at - 1
            );
        }
    }
}

/// Frames scaled into a smaller rectangle (the GPU samples them; tiny-skia's
/// hook isn't asked), and a source changing size: each shows.
#[test]
fn frames_drawn_scaled_and_frames_of_a_new_size_show() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    let large = buffers(1280, 720);
    let shrunk = Rectangle::new([160.0, 90.0].into(), Size::new(960.0, 540.0));

    for at in 0..6 {
        let frame = premultiplied::frame(1280, 720, large[at % 4].clone(), true);
        let window = gpu::draw(
            &mut renderer,
            image::Image::new(frame),
            shrunk,
            WINDOW,
            Color::BLACK,
        );

        let centre = gpu::pixel(&window, WINDOW.width, 640, 360);
        let outside = gpu::pixel(&window, WINDOW.width, 80, 45);

        assert!(
            gpu::near(centre, shown(&large[at % 4]), 1),
            "frame {at} scaled"
        );
        assert_eq!(outside, [0, 0, 0, 255], "the ground around it");
    }

    let small = buffers(640, 360);
    let whole = Rectangle::with_size(Size::new(WINDOW.width as f32, WINDOW.height as f32));

    for at in 0..6 {
        let frame = premultiplied::frame(640, 360, small[at % 4].clone(), true);
        let window = gpu::draw(
            &mut renderer,
            image::Image::new(frame),
            whole,
            WINDOW,
            Color::BLACK,
        );

        assert!(
            all(&window, shown(&small[at % 4])),
            "frame {at} of the new size, scaled up"
        );
    }
}
