//! What draws the interface: iced's fallback renderer, wgpu on a graphics
//! adapter first (`vendor/iced_wgpu`) and tiny-skia on the processor second
//! (`vendor/iced_tiny_skia`), both in one build.
//!
//! The window takes the first that starts. wgpu refuses until the program
//! enables it and picks an adapter (`iced_wgpu::enabled`,
//! `iced_wgpu::adapter`), so by default, and in every test and shot, the
//! processor draws. Widgets never ask which renderer they have: they draw
//! only through iced's renderer traits (quads, text, pictures, SVG,
//! geometry), which both implement, and the window can change from one to
//! the other while it runs (`iced::backend::configure`, or a lost adapter).
//!
//! What must be drawn on the processor whatever draws the window makes a
//! renderer of its own with [`software`]: the shots and the tests, whose
//! pixels are compared byte for byte, and a faded styled name's picture.
//! [`is_gpu`] says which kind a renderer is, for what the program shows of
//! it.

use iced::advanced::renderer::Settings;

use crate::Renderer;

/// A renderer drawing on the processor (tiny-skia), whatever draws the
/// window: ready at once, it starts no device and reads no setting.
pub fn software(settings: Settings) -> Renderer {
    Renderer::Secondary(iced_tiny_skia::Renderer::new(settings))
}

/// Whether `renderer` draws on a graphics adapter (wgpu) rather than on the
/// processor.
pub fn is_gpu(renderer: &Renderer) -> bool {
    matches!(renderer, Renderer::Primary(_))
}

#[cfg(test)]
mod tests {
    use iced::advanced::renderer::{Headless as _, Quad, Renderer as _};
    use iced::{Color, Point, Rectangle, Size};

    use super::*;

    #[test]
    fn a_software_renderer_draws_on_the_processor() {
        let mut renderer = software(Settings::default());
        assert!(!is_gpu(&renderer));
        assert_eq!(renderer.name(), "tiny-skia");
        renderer.reset(Rectangle::with_size(Size::new(4.0, 4.0)));
        renderer.fill_quad(
            Quad {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(2.0, 2.0)),
                ..Quad::default()
            },
            Color::from_rgb8(255, 0, 0),
        );
        let pixels = renderer.screenshot(Size::new(4, 4), 1.0, Color::BLACK);
        assert_eq!(pixels.len(), 4 * 4 * 4);
        // RGBA, the quad at the top left and the ground elsewhere.
        assert_eq!(pixels[..4], [255_u8, 0, 0, 255]);
        assert_eq!(pixels[pixels.len() - 4..], [0_u8, 0, 0, 255]);
    }
}
