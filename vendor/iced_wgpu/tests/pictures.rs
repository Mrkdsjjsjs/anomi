//! Anomi (`PATCHES.md`, "Marked pictures", "Corners as tiny-skia cuts
//! them"): pictures drawn on a GPU, in the atlas and in textures of their
//! own: a picture the app marked as premultiplied BGRA draws as its stock
//! copy, and each corner of a rounded one takes its own radius, its edge
//! where tiny-skia puts it.
//!
//! Opt-in, as it starts a GPU (`gpu/mod.rs`):
//!
//!     ANOMI_GPU_TESTS=1 ANOMI_GPU_BUS=0000:01:00.0 cargo test \
//!       --manifest-path vendor/iced_wgpu/Cargo.toml --features image,web-colors --test pictures

#![cfg(feature = "image")]

mod gpu;

use iced_tiny_skia::premultiplied;
use iced_wgpu::core::image;
use iced_wgpu::core::{Color, Rectangle, Size, border};

const WINDOW: Size<u32> = Size::new(800, 600);

/// Where the pictures are drawn.
const AT: (u32, u32) = (100, 100);

/// Small enough for the atlas, and large enough for a texture of its own.
const SIZES: [(u32, u32); 2] = [(33, 17), (600, 500)];

/// A `width` by `height` picture of straight RGBA, each pixel its own
/// colour; of every five, two see-through to a degree and the rest opaque.
fn picture(width: u32, height: u32) -> Vec<u8> {
    (0..width * height)
        .flat_map(|i| {
            let v = i.wrapping_add(7).wrapping_mul(2_654_435_761);
            let alpha = [255, 255, 255, 128, 37][(i % 5) as usize];

            [v as u8, (v >> 8) as u8, (v >> 16) as u8, alpha]
        })
        .collect()
}

/// A picture of one colour.
fn plain(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat((width * height) as usize)
}

/// The rectangle a `width` by `height` picture is drawn in, at [`AT`].
fn placed(width: u32, height: u32) -> Rectangle {
    Rectangle::new(
        [AT.0 as f32, AT.1 as f32].into(),
        Size::new(width as f32, height as f32),
    )
}

#[test]
fn a_marked_picture_draws_as_its_stock_copy() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    let ground = Color::from_rgb8(30, 60, 90);

    for (width, height) in SIZES {
        let rgba = picture(width, height);
        let stock = image::Handle::from_rgba(width, height, rgba.clone());
        let marked = premultiplied::from_rgba(width, height, rgba.clone());
        let bounds = placed(width, height);

        let ours = gpu::draw(
            &mut renderer,
            image::Image::new(marked),
            bounds,
            WINDOW,
            ground,
        );
        let theirs = gpu::draw(
            &mut renderer,
            image::Image::new(stock),
            bounds,
            WINDOW,
            ground,
        );

        for y in 0..height {
            for x in 0..width {
                let alpha = rgba[((y * width + x) * 4 + 3) as usize];

                // Without `web-colors` the atlas is sRGB, and a see-through
                // marked texel is linearized as if straight (image.wgsl):
                // only opaque pixels are compared then.
                if alpha != 255 && !cfg!(feature = "web-colors") {
                    continue;
                }

                let a = gpu::pixel(&ours, WINDOW.width, AT.0 + x, AT.1 + y);
                let b = gpu::pixel(&theirs, WINDOW.width, AT.0 + x, AT.1 + y);

                assert!(
                    gpu::near(a, b, 2),
                    "{width}x{height} at {x}, {y}: {a:?}, stock {b:?}"
                );
            }
        }
    }
}

#[test]
fn a_marked_picture_keeps_red_red_and_blue_blue() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    for (width, height) in SIZES {
        // Red on its left half, blue on its right.
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                if i % width < width / 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect();

        let marked = premultiplied::from_rgba(width, height, rgba);
        let window = gpu::draw(
            &mut renderer,
            image::Image::new(marked),
            placed(width, height),
            WINDOW,
            Color::BLACK,
        );

        let left = gpu::pixel(&window, WINDOW.width, AT.0 + width / 4, AT.1 + height / 2);
        let right = gpu::pixel(
            &window,
            WINDOW.width,
            AT.0 + width * 3 / 4,
            AT.1 + height / 2,
        );

        assert!(gpu::near(left, [255, 0, 0, 255], 1), "{width}x{height}: {left:?}");
        assert!(gpu::near(right, [0, 0, 255, 255], 1), "{width}x{height}: {right:?}");
    }
}

/// A banner round at the top only. Stock, each corner took its opposite's
/// radius, and the banner came out round at the bottom.
#[test]
fn each_corner_takes_its_own_radius() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    for (width, height) in [(64, 40), (640, 400)] {
        for marked in [false, true] {
            let white = plain(width, height, [255; 4]);
            let handle = if marked {
                premultiplied::from_rgba(width, height, white)
            } else {
                image::Handle::from_rgba(width, height, white)
            };

            let mut banner = image::Image::new(handle);
            banner.border_radius = border::Radius {
                top_left: 12.0,
                top_right: 12.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            };

            let window = gpu::draw(
                &mut renderer,
                banner,
                placed(width, height),
                WINDOW,
                Color::BLACK,
            );
            let at = |x: u32, y: u32| gpu::pixel(&window, WINDOW.width, AT.0 + x, AT.1 + y)[0];

            assert!(
                at(0, 0) < 30 && at(width - 1, 0) < 30,
                "{width}x{height}, marked {marked}: round at the top"
            );
            assert!(
                at(0, height - 1) > 225 && at(width - 1, height - 1) > 225,
                "{width}x{height}, marked {marked}: square at the bottom"
            );
        }
    }
}

/// A round avatar, 32 pixels across: full at the middle of each side, and a
/// pixel its circle barely reaches nearly empty, a pixel's coverage being
/// half a pixel past its centre's distance, as tiny-skia's blit has it.
/// Stock put the edge half a pixel out (`1.0 - d`): that pixel came out more
/// than half covered, the avatar flat at its sides.
#[test]
fn a_round_picture_is_round_at_its_sides() {
    let Some((_drawing, mut renderer)) = gpu::start() else {
        return;
    };

    let size = 32;
    let white = premultiplied::from_rgba(size, size, plain(size, size, [255; 4]));

    let mut avatar = image::Image::new(white);
    avatar.border_radius = border::Radius::from(16.0);

    let window = gpu::draw(
        &mut renderer,
        avatar,
        placed(size, size),
        WINDOW,
        Color::BLACK,
    );
    let at = |x: u32, y: u32| gpu::pixel(&window, WINDOW.width, AT.0 + x, AT.1 + y)[0];

    for (x, y) in [(0, 16), (31, 16), (16, 0), (16, 31)] {
        assert!(at(x, y) > 200, "{x}, {y}: {}", at(x, y));
    }

    // Each 16.45 px from the centre: covered 0.05.
    for (x, y) in [(0, 10), (31, 10), (10, 0), (10, 31)] {
        assert!(at(x, y) < 100, "{x}, {y}: {}", at(x, y));
    }
}
