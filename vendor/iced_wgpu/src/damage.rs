//! Anomi: what a frame changed from the frame before, for a window whose
//! frames are read back from the adapter (`window/shared.rs`; `PATCHES.md`,
//! "Only what changed, read back and shown").
//!
//! Two frames' layers are paired as `vendor/iced_tiny_skia` pairs them
//! (`window::compositor::layers`): the layers that end both frames with the
//! same bounds from the end, the rest by place. In a pair with the same
//! bounds each kind of primitive is held against the other's by place, and
//! each that differs damages where it may lay pixels in either frame; a
//! layer unpaired, or paired with other bounds, damages where its
//! primitives reach. A pixel no damage covers is then drawn by the same
//! primitives in the same order in both frames, so it is the same pixel.
//!
//! What is kept of a frame ([`Scene`]) holds nothing strongly that the
//! frame let go: a picture is kept as its handle's id, a cache of meshes or
//! text as its id, version and address, and text as iced's layers hold it
//! (a paragraph weakly). A video frame's slot is its writer's again once
//! the next frame is drawn, as without this.

use crate::core::image as raster;
use crate::core::svg as vector;
use crate::core::{Rectangle, Size, Transformation};
use crate::graphics::text::Text;
use crate::graphics::{self, Mesh, Viewport, damage, mesh};
use crate::layer::Layer;
use crate::{quad, text, triangle};

use std::collections::VecDeque;
use std::sync::Arc;

/// What a frame's layers draw, kept to be held against the next frame's.
#[derive(Debug, Default)]
pub(crate) struct Scene {
    layers: Vec<Drawn>,
}

impl Scene {
    /// What `layers` (merged: `Renderer::settle`) draw. A layer with
    /// nothing in it draws nothing and is left out.
    pub(crate) fn of(layers: &[Layer]) -> Self {
        Self {
            layers: layers
                .iter()
                .filter(|layer| !layer.is_empty())
                .map(Drawn::of)
                .collect(),
        }
    }

    /// Where `current` may lay other pixels than `previous`, in logical
    /// pixels.
    pub(crate) fn between(previous: &Self, current: &Self) -> Vec<Rectangle> {
        let (previous, current) = (previous.layers.as_slice(), current.layers.as_slice());

        let tail = previous
            .iter()
            .rev()
            .zip(current.iter().rev())
            .take_while(|(a, b)| a.bounds == b.bounds)
            .count();

        let (previous, previous_tail) = previous.split_at(previous.len() - tail);
        let (current, current_tail) = current.split_at(current.len() - tail);

        let mut damage = damage::diff(
            previous,
            current,
            |layer| layer.reach().into_iter().collect(),
            Drawn::damage,
        );

        for (a, b) in previous_tail.iter().zip(current_tail) {
            damage.extend(Drawn::damage(a, b));
        }

        damage
    }
}

/// What a layer draws.
#[derive(Debug)]
struct Drawn {
    bounds: Rectangle,
    quads: quad::Batch,
    meshes: Vec<Meshes>,
    /// Custom primitives can't be compared: each damages its bounds in
    /// every frame.
    primitives: Vec<Rectangle>,
    pictures: Vec<Picture>,
    words: Vec<Words>,
}

impl Drawn {
    fn of(layer: &Layer) -> Self {
        Self {
            bounds: layer.bounds,
            quads: layer.quads.clone(),
            meshes: layer.triangles.iter().map(Meshes::of).collect(),
            primitives: layer
                .primitives
                .iter()
                .map(|instance| instance.bounds)
                .collect(),
            pictures: pictures(&layer.images),
            words: layer.text.iter().map(Words::of).collect(),
        }
    }

    /// Where the layer may lay pixels: what its primitives reach, within its
    /// bounds, as one rectangle.
    fn reach(&self) -> Option<Rectangle> {
        let reach = self
            .quads
            .reach()
            .chain(self.meshes.iter().flat_map(Meshes::reach))
            .chain(self.primitives.iter().copied())
            .chain(self.pictures.iter().map(Picture::reach))
            .chain(self.words.iter().flat_map(Words::reach))
            .reduce(|a, b| a.union(&b))?;

        reach.intersection(&self.bounds.expand(1.0))
    }

    fn damage(previous: &Self, current: &Self) -> Vec<Rectangle> {
        if previous.bounds != current.bounds {
            return previous
                .reach()
                .into_iter()
                .chain(current.reach())
                .collect();
        }

        let mut damage = quad::Batch::damage(&previous.quads, &current.quads);

        damage.extend(damage::diff(
            &previous.meshes,
            &current.meshes,
            Meshes::reach,
            Meshes::damage,
        ));

        damage.extend(previous.primitives.iter().chain(&current.primitives));

        damage.extend(damage::list(
            &previous.pictures,
            &current.pictures,
            |picture| vec![picture.reach()],
            Picture::eq,
        ));

        damage.extend(damage::diff(
            &previous.words,
            &current.words,
            Words::reach,
            Words::damage,
        ));

        // The layer's scissor is its bounds, rounded to the nearest pixel.
        let clip = current.bounds.expand(1.0);

        damage
            .into_iter()
            .filter_map(|region| region.intersection(&clip))
            .collect()
    }
}

/// A layer's meshes, as an item of its triangle batch.
#[derive(Debug)]
enum Meshes {
    /// Meshes drawn live, held by value: a canvas drawn as it was is no
    /// damage (the sidebar's thread lines, the voice meter's bar).
    Group {
        transformation: Transformation,
        meshes: Vec<Mesh>,
    },
    /// A cache of meshes: the same id, version and batch are the same
    /// meshes.
    Cached {
        id: mesh::Id,
        version: usize,
        batch: usize,
        transformation: Transformation,
        reach: Option<Rectangle>,
    },
}

impl Meshes {
    fn of(item: &triangle::Item) -> Self {
        match item {
            triangle::Item::Group {
                transformation,
                meshes,
            } => Self::Group {
                transformation: *transformation,
                meshes: meshes.clone(),
            },
            triangle::Item::Cached {
                transformation,
                cache,
            } => Self::Cached {
                id: cache.id(),
                version: cache.version(),
                batch: Arc::as_ptr(cache.batch()).cast::<Mesh>().addr(),
                transformation: *transformation,
                reach: cache
                    .batch()
                    .iter()
                    .map(|mesh| mesh_reach(mesh, *transformation))
                    .reduce(|a, b| a.union(&b)),
            },
        }
    }

    fn reach(&self) -> Vec<Rectangle> {
        match self {
            Self::Group {
                transformation,
                meshes,
            } => meshes
                .iter()
                .map(|mesh| mesh_reach(mesh, *transformation))
                .collect(),
            Self::Cached { reach, .. } => reach.iter().copied().collect(),
        }
    }

    fn damage(previous: &Self, current: &Self) -> Vec<Rectangle> {
        match (previous, current) {
            (
                Self::Group {
                    transformation,
                    meshes: before,
                },
                Self::Group {
                    transformation: now,
                    meshes,
                },
            ) if transformation == now => damage::list(
                before,
                meshes,
                |mesh| vec![mesh_reach(mesh, *transformation)],
                Mesh::eq,
            ),
            (
                Self::Cached {
                    id,
                    version,
                    batch,
                    transformation,
                    ..
                },
                Self::Cached {
                    id: id_now,
                    version: version_now,
                    batch: batch_now,
                    transformation: transformation_now,
                    ..
                },
            ) if id == id_now
                && version == version_now
                && batch == batch_now
                && transformation == transformation_now =>
            {
                Vec::new()
            }
            _ => previous
                .reach()
                .into_iter()
                .chain(current.reach())
                .collect(),
        }
    }
}

/// Where a mesh may lay pixels: its scissor (`triangle::Layer::render`),
/// and a pixel round it for the rounding to whole pixels.
fn mesh_reach(mesh: &Mesh, transformation: Transformation) -> Rectangle {
    (mesh.clip_bounds() * transformation).expand(1.0)
}

/// A picture, its handle kept as its id: the same id is the same pixels
/// (`image::Handle`'s ids are unique to the bytes they're made with, or a
/// path's hash; an SVG's, its data's or path's hash).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(any(feature = "image", feature = "svg")), allow(dead_code))]
enum Picture {
    Raster {
        image: raster::Image<raster::Id>,
        bounds: Rectangle,
        clip_bounds: Rectangle,
    },
    Vector {
        svg: vector::Svg<u64>,
        bounds: Rectangle,
        clip_bounds: Rectangle,
    },
}

#[cfg_attr(not(any(feature = "image", feature = "svg")), allow(dead_code))]
impl Picture {
    fn of(image: &graphics::Image) -> Self {
        match image {
            graphics::Image::Raster {
                image,
                bounds,
                clip_bounds,
            } => Self::Raster {
                image: raster::Image {
                    handle: image.handle.id(),
                    filter_method: image.filter_method,
                    rotation: image.rotation,
                    border_radius: image.border_radius,
                    opacity: image.opacity,
                },
                bounds: *bounds,
                clip_bounds: *clip_bounds,
            },
            graphics::Image::Vector {
                svg,
                bounds,
                clip_bounds,
            } => Self::Vector {
                svg: vector::Svg {
                    handle: svg.handle.id(),
                    color: svg.color,
                    rotation: svg.rotation,
                    opacity: svg.opacity,
                },
                bounds: *bounds,
                clip_bounds: *clip_bounds,
            },
        }
    }

    /// Its bounds turned as it's drawn, and a pixel round them.
    fn reach(&self) -> Rectangle {
        let turned = match self {
            Self::Raster { image, bounds, .. } => bounds.rotate(image.rotation),
            Self::Vector { svg, bounds, .. } => bounds.rotate(svg.rotation),
        };

        turned.expand(1.0)
    }
}

#[cfg(any(feature = "image", feature = "svg"))]
fn pictures(batch: &crate::image::Batch) -> Vec<Picture> {
    batch.iter().map(Picture::of).collect()
}

#[cfg(not(any(feature = "image", feature = "svg")))]
fn pictures(_batch: &crate::image::Batch) -> Vec<Picture> {
    Vec::new()
}

/// A layer's text, as an item of its text batch.
#[derive(Debug)]
enum Words {
    Group {
        transformation: Transformation,
        text: Vec<Text>,
    },
    /// A cache of text (`text::Cache::key`).
    Cached {
        key: (text::Id, usize, usize),
        transformation: Transformation,
        reach: Vec<Rectangle>,
    },
}

impl Words {
    fn of(item: &text::Item) -> Self {
        match item {
            text::Item::Group {
                transformation,
                text,
            } => Self::Group {
                transformation: *transformation,
                text: text.clone(),
            },
            text::Item::Cached {
                transformation,
                cache,
            } => Self::Cached {
                key: cache.key(),
                transformation: *transformation,
                reach: cache
                    .text()
                    .iter()
                    .filter_map(|text| text_reach(text, *transformation))
                    .collect(),
            },
        }
    }

    fn reach(&self) -> Vec<Rectangle> {
        match self {
            Self::Group {
                transformation,
                text,
            } => text
                .iter()
                .filter_map(|text| text_reach(text, *transformation))
                .collect(),
            Self::Cached { reach, .. } => reach.clone(),
        }
    }

    fn damage(previous: &Self, current: &Self) -> Vec<Rectangle> {
        match (previous, current) {
            (
                Self::Group {
                    transformation,
                    text: before,
                },
                Self::Group {
                    transformation: now,
                    text,
                },
            ) if transformation == now => damage::list(
                before,
                text,
                |text| text_reach(text, *transformation).into_iter().collect(),
                same_text,
            ),
            (
                Self::Cached {
                    key,
                    transformation,
                    ..
                },
                Self::Cached {
                    key: key_now,
                    transformation: transformation_now,
                    ..
                },
            ) if key == key_now && transformation == transformation_now => Vec::new(),
            _ => previous
                .reach()
                .into_iter()
                .chain(current.reach())
                .collect(),
        }
    }
}

/// Where text may lay its glyphs' pixels, as `vendor/iced_tiny_skia`
/// damages it: its bounds and `REACH` (8 px) round them, within its clip
/// unless it lies within it, where its alignment puts it. This renderer
/// cuts glyphs at the text's clip, as tiny-skia does at the reach: a glyph
/// past its line by more than 8 px (none at the sizes the window draws
/// text at) would be left behind as it is there.
fn text_reach(text: &Text, transformation: Transformation) -> Option<Rectangle> {
    iced_tiny_skia::reach::text_reach(text).map(|reach| reach * transformation)
}

/// Whether two texts lay the same glyphs: equal as iced holds text equal,
/// and a paragraph or an editor the same one. iced's equality takes two
/// live paragraphs of the same size, font and bounds for equal whatever
/// they say; one changed since is another allocation (iced makes a new
/// one, as the frame before holds the old one weakly), so the same
/// allocation, alive, is the same text.
fn same_text(a: &Text, b: &Text) -> bool {
    if a != b {
        return false;
    }

    match (a, b) {
        (Text::Paragraph { paragraph: a, .. }, Text::Paragraph { paragraph: b, .. }) => {
            match (a.upgrade(), b.upgrade()) {
                (Some(a), Some(b)) => std::ptr::eq(a.buffer(), b.buffer()),
                _ => false,
            }
        }
        (Text::Editor { editor: a, .. }, Text::Editor { editor: b, .. }) => {
            match (a.upgrade(), b.upgrade()) {
                (Some(a), Some(b)) => std::ptr::eq(a.buffer(), b.buffer()),
                _ => false,
            }
        }
        _ => true,
    }
}

/// What a frame changed from the frame before, in the window's pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// Not known: the whole window.
    Whole,
    /// These regions; none when nothing changed.
    Regions(Vec<Rectangle<u32>>),
}

/// `damage`, in logical pixels, as regions of the window's pixels: grouped
/// as tiny-skia groups a frame's damage (iced's `damage::group` within the
/// window, then neighbours merged, `iced_tiny_skia::window::compositor::
/// merged`), each rounded out with a pixel more round it, within the
/// window.
pub(crate) fn regions(damage: Vec<Rectangle>, viewport: &Viewport) -> Vec<Rectangle<u32>> {
    let window = Rectangle::with_size(viewport.logical_size());
    let grouped = iced_tiny_skia::window::compositor::merged(damage::group(damage, window));

    physical(&grouped, viewport.scale_factor(), viewport.physical_size())
}

/// `regions`, in logical pixels, as the pixels of a window of `size` they
/// touch at `scale`, and one more round each (an edge anti-aliased or
/// snapped to the grid lays half a pixel past where it lies), within the
/// window; none of no size.
pub(crate) fn physical(regions: &[Rectangle], scale: f32, size: Size<u32>) -> Vec<Rectangle<u32>> {
    regions
        .iter()
        .filter_map(|region| {
            let left = ((region.x * scale).floor() - 1.0).max(0.0) as u32;
            let top = ((region.y * scale).floor() - 1.0).max(0.0) as u32;
            let right = (((region.x + region.width) * scale).ceil() + 1.0).max(0.0) as u32;
            let bottom = (((region.y + region.height) * scale).ceil() + 1.0).max(0.0) as u32;

            let (left, right) = (left.min(size.width), right.min(size.width));
            let (top, bottom) = (top.min(size.height), bottom.min(size.height));

            (right > left && bottom > top).then(|| Rectangle {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            })
        })
        .collect()
}

/// Regions drawn and read back in a frame at most: more cost a pass over
/// every layer each, so the two whose union adds least are merged.
const MOST: usize = 8;

/// `regions` made disjoint, each two that overlap taken as their union,
/// and at most [`MOST`]: a frame is drawn once in each, and a pixel drawn
/// twice would blend twice.
pub(crate) fn settled(mut regions: Vec<Rectangle<u32>>) -> Vec<Rectangle<u32>> {
    loop {
        let pair =
            overlapping(&regions).or_else(|| (regions.len() > MOST).then(|| cheapest(&regions)));

        let Some((a, b)) = pair else {
            return regions;
        };

        // `a < b`: `a` stays where it is.
        let other = regions.swap_remove(b);
        regions[a] = union(regions[a], other);
    }
}

/// The first two regions that overlap, the first first.
fn overlapping(regions: &[Rectangle<u32>]) -> Option<(usize, usize)> {
    regions.iter().enumerate().find_map(|(a, region)| {
        regions[a + 1..]
            .iter()
            .position(|other| within(*region, *other).is_some())
            .map(|b| (a, a + 1 + b))
    })
}

/// The two regions whose union adds least to what they cover, the first
/// first; there are at least two.
fn cheapest(regions: &[Rectangle<u32>]) -> (usize, usize) {
    let area = |region: Rectangle<u32>| u64::from(region.width) * u64::from(region.height);

    let mut best = (0, 1, u64::MAX);

    for (a, region) in regions.iter().enumerate() {
        for (b, other) in regions.iter().enumerate().skip(a + 1) {
            let added = area(union(*region, *other)).saturating_sub(area(*region) + area(*other));

            if added < best.2 {
                best = (a, b, added);
            }
        }
    }

    (best.0, best.1)
}

/// The pixels two regions share, if any.
pub(crate) fn within(a: Rectangle<u32>, b: Rectangle<u32>) -> Option<Rectangle<u32>> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.x.saturating_add(a.width).min(b.x.saturating_add(b.width));
    let bottom =
        a.y.saturating_add(a.height)
            .min(b.y.saturating_add(b.height));

    (right > x && bottom > y).then(|| Rectangle {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

fn union(a: Rectangle<u32>, b: Rectangle<u32>) -> Rectangle<u32> {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let bottom =
        a.y.saturating_add(a.height)
            .max(b.y.saturating_add(b.height));

    Rectangle {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

/// Frames whose changes a window keeps: a buffer older than one more is
/// redrawn whole (softbuffer's are one frame old on X11, two on Wayland).
const KEPT: usize = 3;

/// What each frame presented in a window changed from the one before, the
/// newest first, since the window was last sized or a frame failed.
#[derive(Debug, Default)]
pub(crate) struct History {
    changes: VecDeque<Change>,
}

impl History {
    /// What a buffer lacks of the frame being presented, which changes
    /// `now` from the frame presented last, when the buffer holds the frame
    /// presented `age` frames ago (softbuffer's `Buffer::age`; 0 is a
    /// buffer of unknown pixels): `now` and the changes of the `age - 1`
    /// frames presented after the buffer's own, made disjoint ([`settled`]);
    /// the whole window when any of them is unknown or older than kept.
    pub(crate) fn lacking(&self, age: u8, now: &Change) -> Change {
        let Change::Regions(regions) = now else {
            return Change::Whole;
        };

        let Some(after) = usize::from(age).checked_sub(1) else {
            return Change::Whole;
        };

        if after > self.changes.len() {
            return Change::Whole;
        }

        let mut lacking = regions.clone();

        for change in self.changes.iter().take(after) {
            match change {
                Change::Whole => return Change::Whole,
                Change::Regions(regions) => lacking.extend_from_slice(regions),
            }
        }

        Change::Regions(settled(lacking))
    }

    /// A frame that changed `change` was presented.
    pub(crate) fn presented(&mut self, change: Change) {
        self.changes.push_front(change);
        self.changes.truncate(KEPT);
    }

    /// The buffers' pixels are no longer known: the window was sized, or a
    /// frame failed between drawing and presenting.
    pub(crate) fn forget(&mut self) {
        self.changes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::renderer::Quad;
    use crate::core::{Background, Color, Point, Shadow, Vector};
    use crate::graphics::Layer as _;

    fn region(x: u32, y: u32, width: u32, height: u32) -> Rectangle<u32> {
        Rectangle {
            x,
            y,
            width,
            height,
        }
    }

    fn rectangle(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    fn quad(bounds: Rectangle) -> Quad {
        Quad {
            bounds,
            ..Quad::default()
        }
    }

    /// A layer of `bounds` with these quads, each of `color`.
    fn layer(bounds: Rectangle, quads: &[(Rectangle, Color)]) -> Layer {
        let mut layer = Layer::with_bounds(bounds);

        for (bounds, color) in quads {
            layer.draw_quad(
                quad(*bounds),
                Background::Color(*color),
                Transformation::IDENTITY,
            );
        }

        layer
    }

    fn covers(damage: &[Rectangle], area: Rectangle) -> bool {
        damage.iter().any(|region| area.is_within(region))
    }

    const RED: Color = Color::from_rgb(1.0, 0.0, 0.0);
    const BLUE: Color = Color::from_rgb(0.0, 0.0, 1.0);

    #[test]
    fn the_same_frame_changes_nothing() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let frame = || {
            [
                layer(window, &[(rectangle(10.0, 10.0, 50.0, 20.0), RED)]),
                layer(
                    rectangle(100.0, 100.0, 200.0, 100.0),
                    &[(rectangle(120.0, 120.0, 30.0, 30.0), BLUE)],
                ),
            ]
        };

        let damage = Scene::between(&Scene::of(&frame()), &Scene::of(&frame()));

        assert!(damage.is_empty(), "{damage:?}");
    }

    #[test]
    fn a_quad_changed_damages_where_it_was_and_is_and_nothing_else() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let still = (rectangle(10.0, 10.0, 50.0, 20.0), RED);
        let before = [layer(
            window,
            &[still, (rectangle(200.0, 150.0, 10.0, 8.0), BLUE)],
        )];
        let after = [layer(
            window,
            &[still, (rectangle(204.0, 150.0, 10.0, 8.0), BLUE)],
        )];

        let damage = Scene::between(&Scene::of(&before), &Scene::of(&after));

        assert!(
            covers(&damage, rectangle(200.0, 150.0, 10.0, 8.0)),
            "{damage:?}"
        );
        assert!(
            covers(&damage, rectangle(204.0, 150.0, 10.0, 8.0)),
            "{damage:?}"
        );
        assert!(
            damage
                .iter()
                .all(|region| region.intersection(&still.0).is_none()),
            "{damage:?}"
        );
    }

    #[test]
    fn a_quad_damages_its_shadow_and_anti_aliased_edge() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let shadowed = |color| {
            let mut layer = Layer::with_bounds(window);

            layer.draw_quad(
                Quad {
                    bounds: rectangle(100.0, 100.0, 40.0, 20.0),
                    shadow: Shadow {
                        color: Color::BLACK,
                        offset: Vector::new(3.0, -2.0),
                        blur_radius: 6.0,
                    },
                    ..Quad::default()
                },
                Background::Color(color),
                Transformation::IDENTITY,
            );

            [layer]
        };

        let damage = Scene::between(&Scene::of(&shadowed(RED)), &Scene::of(&shadowed(BLUE)));

        // From 6 left of it to 3 + 6 right of it, 2 + 6 above and 6 below,
        // and a pixel round that.
        assert!(
            covers(&damage, rectangle(93.0, 91.0, 57.0, 36.0)),
            "{damage:?}"
        );
    }

    #[test]
    fn a_layer_more_damages_what_it_draws_not_its_bounds() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let ground = layer(window, &[(window, RED)]);
        let tooltip = layer(
            rectangle(290.0, 10.0, 100.0, 60.0),
            &[(rectangle(300.0, 20.0, 60.0, 24.0), BLUE)],
        );

        let before = [layer(window, &[(window, RED)])];
        let after = [ground, tooltip];

        let damage = Scene::between(&Scene::of(&before), &Scene::of(&after));

        assert!(
            covers(&damage, rectangle(300.0, 20.0, 60.0, 24.0)),
            "{damage:?}"
        );
        assert!(
            damage.iter().all(|region| region.area() < 70.0 * 30.0),
            "{damage:?}"
        );
    }

    #[test]
    fn layers_after_one_more_are_paired_from_the_end() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let list = rectangle(0.0, 50.0, 400.0, 250.0);
        let row = (rectangle(0.0, 60.0, 400.0, 40.0), RED);
        let picture = rectangle(10.0, 10.0, 30.0, 30.0);

        let before = [layer(window, &[(window, BLUE)]), layer(list, &[row])];
        let after = [
            layer(window, &[(window, BLUE)]),
            layer(rectangle(5.0, 5.0, 40.0, 40.0), &[(picture, RED)]),
            layer(list, &[row]),
        ];

        let damage = Scene::between(&Scene::of(&before), &Scene::of(&after));

        assert!(covers(&damage, picture), "{damage:?}");
        assert!(
            damage
                .iter()
                .all(|region| region.intersection(&row.0).is_none()),
            "the list paired with its own: {damage:?}"
        );
    }

    #[test]
    fn empty_layers_are_left_out() {
        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let before = [
            layer(window, &[(window, BLUE)]),
            Layer::with_bounds(rectangle(0.0, 0.0, 10.0, 10.0)),
        ];
        let after = [
            layer(window, &[(window, BLUE)]),
            Layer::with_bounds(rectangle(50.0, 50.0, 10.0, 10.0)),
        ];

        let damage = Scene::between(&Scene::of(&before), &Scene::of(&after));

        assert!(damage.is_empty(), "{damage:?}");
    }

    #[cfg(feature = "image")]
    #[test]
    fn a_picture_with_another_handle_damages_where_it_lies() {
        use crate::core::image::{Handle, Image};

        let window = rectangle(0.0, 0.0, 400.0, 300.0);
        let bounds = rectangle(50.0, 60.0, 64.0, 36.0);
        let frame = |handle: &Handle| {
            let mut layer = Layer::with_bounds(window);

            layer.draw_raster(
                Image::new(handle.clone()),
                bounds,
                window,
                Transformation::IDENTITY,
            );

            [layer]
        };

        let first = Handle::from_rgba(1, 1, vec![0_u8; 4]);
        let next = Handle::from_rgba(1, 1, vec![0_u8; 4]);

        let same = Scene::between(&Scene::of(&frame(&first)), &Scene::of(&frame(&first)));
        let other = Scene::between(&Scene::of(&frame(&first)), &Scene::of(&frame(&next)));

        assert!(same.is_empty(), "{same:?}");
        assert!(covers(&other, bounds), "{other:?}");
    }

    #[test]
    fn damage_is_the_pixels_it_touches_and_one_more_within_the_window() {
        let size = Size::new(300, 200);
        let regions = [
            // Fractional, at 1.5: 15.3..30.3 by 30..36.9.
            rectangle(10.2, 20.0, 10.0, 4.6),
            // Past the window's edge.
            rectangle(-5.0, 120.0, 50.0, 50.0),
            // Outside.
            rectangle(400.0, 10.0, 10.0, 10.0),
        ];

        assert_eq!(
            physical(&regions, 1.5, size),
            [region(14, 29, 18, 9), region(0, 179, 69, 21)]
        );
    }

    #[test]
    fn regions_overlapping_are_merged_and_apart_kept() {
        let apart = region(300, 300, 10, 10);
        let settled = settled(vec![
            region(0, 0, 10, 10),
            apart,
            region(5, 5, 10, 10),
            // Overlaps only the union of the first and third.
            region(12, 0, 4, 4),
        ]);

        assert_eq!(settled, [region(0, 0, 16, 15), apart]);
    }

    #[test]
    fn at_most_so_many_regions_are_drawn_and_none_overlap() {
        let many: Vec<_> = (0..20)
            .map(|n| region(n * 50, (n % 3) * 7, 10, 10))
            .collect();
        let settled = settled(many.clone());

        assert!(settled.len() <= MOST, "{settled:?}");

        for (a, first) in settled.iter().enumerate() {
            for second in &settled[a + 1..] {
                assert_eq!(within(*first, *second), None, "{settled:?}");
            }
        }

        for region in many {
            assert!(
                settled.iter().any(|it| within(*it, region) == Some(region)),
                "{region:?} in {settled:?}"
            );
        }
    }

    #[test]
    fn a_buffer_lacks_the_changes_since_its_own_frame() {
        let caret = region(100, 400, 2, 20);
        let hover = region(0, 200, 800, 40);
        let mut history = History::default();

        // Unknown buffers, and a frame of unknown change, are redrawn whole.
        assert_eq!(
            history.lacking(0, &Change::Regions(vec![caret])),
            Change::Whole
        );
        assert_eq!(history.lacking(1, &Change::Whole), Change::Whole);

        // A buffer one frame old lacks the frame's own change; two old, the
        // change of the frame between too, which isn't known yet.
        assert_eq!(
            history.lacking(1, &Change::Regions(vec![caret])),
            Change::Regions(vec![caret])
        );
        assert_eq!(
            history.lacking(2, &Change::Regions(vec![caret])),
            Change::Whole
        );

        history.presented(Change::Whole);
        assert_eq!(
            history.lacking(2, &Change::Regions(vec![caret])),
            Change::Whole
        );

        history.presented(Change::Regions(vec![hover]));
        assert_eq!(
            history.lacking(2, &Change::Regions(vec![caret])),
            Change::Regions(vec![caret, hover])
        );
        assert_eq!(
            history.lacking(3, &Change::Regions(vec![caret])),
            Change::Whole
        );

        // Nothing changed, and nothing since the buffer's frame either.
        history.presented(Change::Regions(Vec::new()));
        assert_eq!(
            history.lacking(2, &Change::Regions(Vec::new())),
            Change::Regions(Vec::new())
        );
        assert_eq!(
            history.lacking(3, &Change::Regions(Vec::new())),
            Change::Regions(vec![hover])
        );

        // Older than kept.
        history.presented(Change::Regions(Vec::new()));
        history.presented(Change::Regions(Vec::new()));
        assert_eq!(
            history.lacking(4, &Change::Regions(Vec::new())),
            Change::Regions(Vec::new())
        );
        assert_eq!(
            history.lacking(5, &Change::Regions(Vec::new())),
            Change::Whole
        );

        history.forget();
        assert_eq!(
            history.lacking(2, &Change::Regions(vec![caret])),
            Change::Whole
        );
    }
}
