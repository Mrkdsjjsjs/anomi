//! What the GPU tests share: a renderer on a GPU, opt-in, and a picture
//! drawn and read back.
//!
//! A test starts a GPU only with `ANOMI_GPU_TESTS=1`, on an integrated
//! Vulkan adapter or the one at `ANOMI_GPU_BUS` ("0000:01:00.0"); without it,
//! it passes having done nothing.

#![allow(dead_code)]

use std::future::Future;
use std::pin::pin;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use iced_wgpu::core::image::{self, Renderer as _};
use iced_wgpu::core::renderer::{self, Renderer as _};
use iced_wgpu::core::{Color, Rectangle, Size};
use iced_wgpu::{Renderer, adapter, enabled};

/// Held while a test draws: one GPU renderer at a time.
static DRAWING: Mutex<()> = Mutex::new(());

/// A renderer on the GPU, with the lock held while it draws; none when the
/// tests weren't asked to start a GPU.
pub fn start() -> Option<(MutexGuard<'static, ()>, Renderer)> {
    if std::env::var_os("ANOMI_GPU_TESTS").is_none() {
        eprintln!("skipped: ANOMI_GPU_TESTS=1 draws on a GPU");
        return None;
    }

    let drawing = DRAWING.lock().unwrap_or_else(PoisonError::into_inner);

    if let Ok(bus) = std::env::var("ANOMI_GPU_BUS") {
        adapter::choose(adapter::Choice::Bus(bus));
    }

    enabled::set_enabled(true);

    let started = block_on(<Renderer as renderer::Headless>::new(
        renderer::Settings::default(),
        Some("wgpu"),
    ));

    let Some(gpu) = started else {
        panic!(
            "no adapter for {:?} among {:?}",
            adapter::choice(),
            adapter::list()
        );
    };

    Some((drawing, gpu))
}

/// Runs `future` to its end on this thread; wgpu's native futures are ready
/// at once.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());

    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }

        std::thread::yield_now();
    }
}

/// Draws `picture` at `bounds`, its clip, on a window of `size` and
/// `background`, as a frame of its own, and reads the window back as RGBA.
pub fn draw(
    gpu: &mut Renderer,
    picture: image::Image,
    bounds: Rectangle,
    size: Size<u32>,
    background: Color,
) -> Vec<u8> {
    gpu.reset(Rectangle::with_size(Size::new(
        size.width as f32,
        size.height as f32,
    )));
    gpu.draw_image(picture, bounds, bounds);

    renderer::Headless::screenshot(gpu, size, 1.0, background)
}

/// The pixel at `x`, `y` of a window `width` pixels wide.
pub fn pixel(window: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;

    [window[at], window[at + 1], window[at + 2], window[at + 3]]
}

/// Whether `a` and `b` are within `levels` of each other in each channel.
pub fn near(a: [u8; 4], b: [u8; 4], levels: u8) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= levels)
}
