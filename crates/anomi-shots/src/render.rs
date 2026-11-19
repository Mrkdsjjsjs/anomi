//! Draws a view headless and writes it as a PNG: iced's `UserInterface` and
//! the processor's renderer, at any scale (`iced_test`'s snapshots are always
//! at 2x and write only when missing).

use std::path::Path;
use std::time::Instant;

use iced::advanced::renderer::{self, Headless as _};
use iced::advanced::{mouse, shell};
use iced::theme::Base;
use iced::{Event, Size, window};
use iced_runtime::{UserInterface, user_interface};

use anomi_theme::{Renderer, Theme, View, fonts, typography};
use anomi_widgets::picture;

/// Always the processor's renderer (tiny-skia), whatever the window would
/// draw with: shots are compared byte for byte, and a graphics adapter's
/// pixels differ from it at edges and between drivers.
fn renderer() -> Renderer {
    anomi_theme::renderer::software(renderer::Settings {
        font: typography::UI,
        ..renderer::Settings::default()
    })
}

/// Lays `view` out at `size` logical pixels, draws it once settled, and
/// writes the RGBA picture at `scale` to `path`. With `time` frames, the
/// renderer then draws the same layers whole that many times more, as a
/// window drawn whole would, and the seconds each took on average come back:
/// what the renderer costs for that screen, without a window.
pub fn png<Message>(
    view: impl View<Message>,
    size: (f32, f32),
    scale: f32,
    theme: &Theme,
    path: &Path,
    time: usize,
) -> Option<f64> {
    // As the program does before its first frame: the fonts with Twemoji
    // and the name faces at every weight, and copies of pictures, names and
    // frames made for the scale drawn at.
    fonts::install();
    picture::set_scale(scale);
    let size = Size::new(size.0, size.1);
    let mut renderer = renderer();
    let mut ui = UserInterface::build(view, size, user_interface::Cache::default(), &mut renderer);
    let mut messages = shell::Bus::new();
    let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
    let cursor = mouse::Cursor::Unavailable;
    let _ = ui.update(
        &window::Headless,
        &shell::Waker::noop(),
        &[redraw],
        cursor,
        &mut renderer,
        &mut messages,
    );
    let base = theme.base();
    ui.draw(
        &mut renderer,
        theme,
        &renderer::Style {
            text_color: base.text_color,
        },
        cursor,
    );
    let physical = Size::new(
        (size.width * scale).round() as u32,
        (size.height * scale).round() as u32,
    );
    let rgba = renderer.screenshot(physical, scale, base.background_color);
    write(path, physical, &rgba);
    (time > 0).then(|| {
        let start = Instant::now();
        for _ in 0..time {
            let _ = renderer.screenshot(physical, scale, base.background_color);
        }
        start.elapsed().as_secs_f64() / time as f64
    })
}

fn write(path: &Path, size: Size<u32>, rgba: &[u8]) {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).expect("the shots folder");
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).expect("the PNG file"));
    let mut encoder = png::Encoder::new(file, size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("a PNG header");
    writer.write_image_data(rgba).expect("the PNG's pixels");
}
