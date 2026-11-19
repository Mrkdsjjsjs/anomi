# Anomi's iced_wgpu

This is `wgpu/` of iced's master at the commit the workspace pins
(`e5c350f271a8046f875c6662d452fde9e462d486`, `0.15.0-dev`), under iced's MIT
licence (`LICENSE`, from iced's repository). It is the iced interface's GPU
renderer: the workspace builds iced with both renderers (`wgpu-bare`,
`tiny-skia`), so `iced::Renderer` is iced's fallback, this one first and
`vendor/iced_tiny_skia` second. The root `Cargo.toml` patches it in through
`[patch."https://github.com/iced-rs/iced"]`; Cargo ignores the patch unless
the name and version stay the git package's.

It starts only when the app enables it (below), so a start on the CPU, every
test and every shot draw with tiny-skia, byte for byte as before, and never
load a GPU driver.

Beside iced's sources:

- `Cargo.toml` has iced's workspace fields and lints written out, and takes
  `iced_graphics` and `iced_debug` from the same commit, the other
  dependencies at the versions iced's workspace asks for. wgpu is built with
  its Vulkan backend alone, whatever the features (`default` is empty;
  stock, it turned on every backend): Vulkan's adapters carry the PCI bus id
  the app chooses by, and GL's backend would bring EGL in. It depends on
  `vendor/iced_tiny_skia` by path, the package the root patches in, for the
  marks the app puts on pictures and frames there and the video hook (its
  `image` with this crate's `image`; its `wayland` for this crate's own
  tests alone, as a dev-dependency: tiny-skia's window buffers need a window
  system, which iced turns on in the app's build).
- `clippy.toml` is iced's, so the crate lints as iced lints it.
- `Cargo.lock` is the root lock pruned to this crate, for its own tests. The
  crate is outside the workspace (`exclude` in the root `Cargo.toml`), so
  the Slint app's checks don't take iced in. Its tests:

      CARGO_TARGET_DIR=$PWD/target cargo test --locked \
        --manifest-path vendor/iced_wgpu/Cargo.toml --features image,svg,geometry,web-colors

  (`scripts/check-iced --review` runs them, and its formatting.)

  A test that needs a GPU runs only with `ANOMI_GPU_TESTS=1` (and, to take
  another adapter than the integrated one, `ANOMI_GPU_BUS=0000:01:00.0`),
  and passes without doing anything otherwise.

Code changed from iced's says `Anomi:` where it does, and the patch's heading
here.

## Started only when the app says so

`enabled.rs` (new), `window/compositor.rs` `Compositor::new`, `lib.rs`
`Headless::new`. `enabled::set_enabled(true)` lets the renderer start; off
by default. Until then the compositor answers with iced's "the backend did
not match" (`GraphicsAdapterNotFound`, `DidNotMatch`), as for settings that
name another backend, and the headless renderer with `None`, so iced's
fallback takes tiny-skia: iced_test's simulator, `anomi-shots` and the tests'
`Headless::new(.., None)` draw on the CPU with no change of theirs. It is
read when a renderer is made; a running one isn't touched, and the app turns
it on before the window opens or before `iced::backend::configure`. Upstream
Yapfast's `set_enabled` is the same switch. Tests: the headless renderer and
the compositor refuse while it's off.

To carry over: the check at the top of `Compositor::new` and of
`Headless::new`.

## The adapter the app chooses

`adapter.rs` (new), `window/compositor.rs` `Compositor::start`, `Settings`,
`lib.rs` `Headless::new`. Stock asked wgpu for an adapter by power
preference (`WGPU_POWER_PREF` or the settings') among every backend, and
enumerated adapters only for the log; headless, it asked for a "high
performance" one. Here the compositor enumerates Vulkan's adapters, leaves
out CPU ones (lavapipe, llvmpipe: slower than tiny-skia), asks each whether
it can present to the window (`Adapter::is_surface_supported`), and takes the
one `adapter::choose` asks for:

- `Choice::Auto` (the default): the first integrated adapter that can
  present to the window, or none. Never a discrete one, so on a desktop the
  card the games use stays theirs; with no integrated adapter Vulkan can
  use, the compositor refuses and tiny-skia draws.
- `Choice::Bus("0000:73:00.0")`: the adapter at that PCI bus id, as Vulkan
  (`VK_EXT_pci_bus_info`) and sysfs write it, or, when no adapter's bus id
  matches, the one whose vendor and device ids do (`"1002:13c0"`); it too
  must present to the window, or the compositor refuses.

The power preference in the settings is unused. `Backend::Best` and no API
mean Vulkan (stock: every backend); `WGPU_BACKEND` and `ICED_PRESENT_MODE`
still override, as stock. The headless renderer takes the same choice
without a window. What the last start enumerated (`adapter::seen`, with
whether each presents), the adapter drawing now (`adapter::active`: none
while tiny-skia draws, in the tray, where iced drops the compositor, or once
the device is lost) and why the last start failed (`adapter::last_error`,
cleared by a start) are kept for the app's Settings page. A compositor is
known by a number: `iced::backend::configure` makes the next before it
drops the one before, whose drop then leaves the new one active.
`adapter::list` enumerates without a window, loading every Vulkan driver
into the process (the app runs it in a child process). Tests: the choice on
the owner's desktop (a 3080 and a Radeon 610M), by bus, by ids, refused
when it can't present, and the active adapter across two compositors;
`adapter::list` against the real drivers, opt-in.

To carry over: in `Compositor::request`, the surface made first (an error at
once; stock went on without one), `adapter::candidates`, `pick` and the
records in place of `request_adapter`; in `Headless::new`, the same with no
surface; `Settings::from`'s backends.

## A failing GPU reported, never a panic

`lost.rs` (new), `engine.rs` `Engine::new`, `window/compositor.rs`
`Compositor::start`, `lib.rs` `Renderer::screenshot`. Stock, wgpu's default
handler for an uncaught error panicked, ending the whole app, call included,
and nothing listened for a lost device (a driver reset or crash, the GPU
removed, its driver updated under the app, a suspend that didn't keep video
memory): the window froze. `Engine::new` gives each device an error handler
and a device-lost callback before anything is made on it (`lost::Watch`).
An error is logged (the first eight of a device; a broken frame may repeat
each frame) and flagged, and drawing goes on. Once the compositor serves a
window, the first failure of its device, error or loss, calls the app's hook
(`lost::set_hook`), once, with its cause and wgpu's message, keeps it in
`adapter::last_error`, and asks the window for a redraw so the app hears of
it with nothing else happening. The hook may be called on any thread from
inside a wgpu call with wgpu's locks held, so it only passes the news on;
the app then switches to tiny-skia (`iced::backend::configure` with
`Backend::Software`). A lost device draws nothing more: the compositor
answers "occluded" and asks nothing of it, and a screenshot is left blank,
as waiting on a lost device (`Device::poll`) panics in wgpu.
`Engine::is_lost` says so to the rest of the crate. The compositor sets up
its renderer (pipelines, shaders) inside error scopes, so a device that
can't build them refuses to start (`Error::SetupFailed`) and tiny-skia
draws, rather than failing at the first frame. A headless renderer's errors
are logged and go no further. Test: the hook is replaced and called with the
failure.

To carry over: `lost.rs`, the watch in `Engine`, `caught` around
`Engine::new` and `serve` in `Compositor::start`, and the two early returns
of `Renderer::screenshot`.

## Surfaces that fail without a panic or a loop

`window/compositor.rs` `Surface`, `create_surface`, `configure_surface`,
`present`. The compositor's surface is its own: wgpu's, if it could be made,
and whether it's configured. Stock, a surface that couldn't be made panicked
(`expect`); here it's reported to the app's hook (cause `Surface`) and kept
empty, and presenting to it answers "lost", which iced's loop retries by
making the surface again at most once a second, and only when a redraw comes.
The configuration runs inside error scopes: a surface never configured isn't
asked for a frame (wgpu panics on the error then), and a failure is
reported. And the answers to iced keep its loop still:

- A timeout (no image for a second: a window the compositor hides keeps its
  images, with FIFO) answers "occluded", on which iced waits for the next
  redraw asked for. Stock answered "timeout", on which iced asked another
  redraw at once, blocking the loop a second at a time while hidden.
- A suboptimal frame is set aside once and the surface configured again; if
  the next frame comes suboptimal too, it's drawn. Stock set every one
  aside, so a driver that calls every frame suboptimal configured and
  redrew without end.
- "Outdated" right after a configure answers "other", which iced retries at
  most once a second; stock's "outdated" had it configure and redraw at
  once, each time.
- Nothing answers "out of memory", on which iced panics.
- A surface that gives no frame three times in a row (lost, or outdated
  however it's configured), none presented between, is reported to the
  app's hook as a surface that couldn't be made (`MISSED`): the window
  would otherwise stay empty while iced made the surface again once a
  second, and the app would never fall back. Test: reported once, and a
  frame presented starts the count over.

FIFO stays the present mode (vsync on): frames are drawn only when iced asks,
and an animation's are paced by the display. Test: the settings ask for
Vulkan and vsync.

To carry over: `Surface` as the compositor's surface type (`window.rs`
re-exports it), `draw`, and `present`'s answers.

## Geometry caches cleared as the GPU starts and goes

`window/compositor.rs` `start`, `Drop`. A geometry one kind of renderer
cached can't be drawn by the other: iced's fallback reaches
`unreachable!()`. iced clears every cache (`graphics::cache::invalidate_all`)
when the app switches renderer at run time, but not when the last window
closes to the tray and the next opens with a renderer of the other kind (the
GPU failed to start then, or the app chose the processor meanwhile); the
window's widgets are made anew then, but a cache the app keeps beside them,
such as a QR code's `Data`, isn't. So a compositor clears every cache as it
starts and as it's dropped.

To carry over: the two `invalidate_all` calls.

## Not done here

- Damage. Each frame wgpu draws the whole window (every layer prepared,
  every paragraph's glyphs), where tiny-skia draws only what changed and
  presents only that (its patches 9 and 23): a blinking caret costs a whole
  frame's preparation on the CPU, though not the GPU's time to speak of.
- Upstream Yapfast's other changes to its copy (the image shader's corners
  and edge, pictures up to 4K uploaded in the frame) belong to the image
  pipeline, below.

## Pictures and video

The image pipeline (`src/image/*`, `src/shader/image.wgsl`). Nothing in
`crates/anomi-*` changes for pictures: the app marks its buffers and sets
its video hook through `iced_tiny_skia` as before, and this renderer reads
the same marks. The hook isn't asked here: the GPU scales a frame as it
samples it.

### Marked pictures drawn as they are

`image/mark.rs` (new), `image/mod.rs` `Instance`, `add_instances`,
`shader/image.wgsl` `fs_main`. The app hands most of its pictures and every
video frame over in tiny-skia's own format, premultiplied BGRA, marked in
`iced_tiny_skia::premultiplied`'s registry (tiny-skia's patches 1 and 13).
Stock, this renderer took every `Rgba` handle for straight RGBA: a marked
picture came out with red and blue swapped and premultiplied twice, its
see-through edges darker. When a picture is uploaded, `mark::of` reads its
mark (while its bytes live, as the registry knows a buffer only then); the
bytes go to the GPU as they are, and the picture's instances carry flags,
`BGRA` and `PREMULTIPLIED` (vertex attribute 9), on which the shader swaps
the texel back (`.bgra`) and skips its `premultiply`. Nothing is converted
on either renderer, and a switch between them at run time needs no picture
made again. With `web-colors` off the atlas is sRGB, and a see-through
marked texel is linearized as if it were straight, a little off at the
edges; the workspace builds with `web-colors`. Tests: `mark.rs`, the marks
are tiny-skia's; `tests/pictures.rs` (GPU, opt-in), a marked picture, in the
atlas and in a texture of its own, draws as its stock copy within 2 levels
(only its opaque pixels compared without `web-colors`), and keeps red red
and blue blue.

Needs, in `vendor/iced_tiny_skia/src/premultiplied.rs`, a public
`is_frame` beside `is_premultiplied` (a video frame takes a texture of its
own at any size, below):

    /// Whether `handle` draws from a live video frame of [`frame`] or
    /// [`frame_of`].
    pub fn is_frame(handle: &Handle) -> bool {
        match handle {
            Handle::Rgba { pixels, .. } => entry(pixels).is_some_and(|entry| entry.frame),
            Handle::Path(..) | Handle::Bytes(..) => false,
        }
    }

To carry over: `mark.rs`, the instance's `flags` and their attribute, and
the shader's two branches after `textureSample`.

### Corners as tiny-skia cuts them

`shader/image.wgsl` `fs_main`. Upstream Yapfast's two fixes, still needed at
the pin. The offset passed to `rounded_box_sdf` is negated, as the quad
shader has it: stock, each corner took its opposite's radius, and a banner
round at the top came out round at the bottom. The edge is at `0.5 - d`:
stock's `1.0 - d` put it half a pixel outside, which the clip then cut, and
round avatars came out fuller, flat at their sides. A pixel's coverage is
now half a pixel past its centre's distance, as tiny-skia's blit has it
(its patches 2, 6, 7). Corners cut the picture's clip, on both renderers;
a turned picture turns its corners here, where tiny-skia keeps them square
(no widget turns pictures). Tests (`tests/pictures.rs`, GPU): a banner
round at the top only, in the atlas and in a texture of its own, marked or
not; a round avatar full at the middle of each side and nearly empty where
its circle barely reaches.

To carry over: the two lines in `fs_main`.

### Every picture uploaded in the frame that draws it

`image/cache.rs`, `image/raster.rs`. Stock decoded encoded pictures, and
uploaded those of 2 MiB or more, on a worker thread, ready a frame or more
later: a video frame, a new handle each frame, could be replaced before it
ever showed (iced #3381), and the worker read a stream's frame after the
window had let the stream worker have its slot back (the contract of
`crates/anomi-app/src/media/frames.rs`). The worker is gone. A picture is
loaded when it's first measured or drawn (an `Rgba` handle's own bytes,
shared; an encoded one decoded on the window's thread, as tiny-skia does)
and uploaded in `prepare` of the frame that draws it: its bytes are read
then, on the window's thread, as tiny-skia's blit reads them, and let go.
`allocate_image` and `load_image` load the picture and answer at once
(stock's `load_image` uploaded through a submission of its own and waited
for the GPU). A thread fewer, with its 4 MiB staging belt, and the cache's
own 2 MiB belt. The cache drops what a frame didn't draw at the end of every
frame (unless the app holds an allocation of it): stock trimmed only after a
frame that loaded a new picture, so a large picture scrolled away kept its
memory until another came.

To carry over: `Cache` without its `Worker`; `Memory::Host` and
`Memory::Device` with a `Picture`; `raster::Cache::trim` every frame.

### Large pictures and video frames in textures of their own

`image/own.rs` (new), `image/cache.rs` `upload`, `image/mod.rs`
`add_instances`. A video frame, a picture of 256 KiB or more (`own::BYTES`)
or one wider than an atlas layer takes a texture of exactly its size,
written with `Queue::write_texture` (wgpu copies the bytes into a staging
buffer during the call and frees it once the frame's submission is done),
drawn as one instance over the whole texture. A texture whose picture isn't
drawn in a frame is given back at its end: kept as a spare if a picture of
its size was uploaded in this one (the next frame of a video, replacing
it), freed at once otherwise; a spare no picture takes within 16 frames
(`SPARE_FRAMES`: a source slower than the window's other animations has
frames between its own) is freed then. A source playing holds two textures,
the frame shown and the one before, and none is made after its second
frame. Pictures under the threshold (avatars, emoji, icons, SVGs, small
copies) share the atlas. Tests: `own.rs`, a source playing thirty frames
makes two textures, one with a frame every third frame drawn too, a picture
no longer drawn is freed at once and a stopped source's spare 16 frames
later, spares are bounded by the textures of their size wanted, and which
pictures take their own; `tests/frames.rs` (GPU, opt-in, after
`vendor/iced_tiny_skia`'s), thirty
frames of 1280 by 720, each a new handle over one of four buffers, each
shows with red and blue in place, and once the next has been drawn the
renderer holds nothing of it (its buffer's only owner is the test's); frames
scaled down, and a source changing size.

To carry over: `own.rs`, `upload` and the `Placed::Whole` instance.

### The atlas smaller, and giving back its empty layers

`image/atlas.rs` `SIZE`, `trim`, `resize`, `upload_allocation`. Layers of
1024 pixels a side (4 MiB) where stock's were 2048 (16 MiB): large pictures
no longer enter the atlas, and a layer holds at least sixteen of the largest
that do. The layers at its top that have lain empty for 60 frames
(`SHRINK_AFTER`) are given back: the ones kept are copied into a texture as
deep as they are, submitted at the end of the frame ahead of its commands,
and never in a frame that wrote the atlas, whose writes are in those
commands. Stock only grew. An upload larger than the renderer's belt's
100 KiB chunks goes through a buffer of its own; through the belt, stock,
it made a chunk of its size that the belt then kept, mapped in RAM, for
good. Test: `atlas.rs`, only empty layers at the top go, never below the
backend's floor.

To carry over: `SIZE`, `trim` called from `Cache::trim`, `grow` through
`resize`, and the one-off buffer in `upload_allocation`.

### GPU memory, per window

- The atlas: one 4 MiB layer made with the renderer; a view of avatars,
  emoji and icons takes one, two at most; given back from the top 60 frames
  after it empties.
- Textures of their own: each picture of 256 KiB or more while it's drawn,
  at its size; the app's copies are made at the size they show, so this is
  bounded by what's on screen (a 1400 by 860 window holds 4.8 MB of pixels).
- Video: two textures per source playing: 7.4 MB at 720p, 16.6 MB at 1080p,
  66 MB at 4K. A source that stops (out of view, or replaced) gives its last
  back at the end of the next frame drawn, its spare 16 frames later; a
  window behind a game draws no frames, so what it held stays until it
  draws again (the tray drops the whole device).
- Staging, in RAM: the renderer's belt's 100 KiB chunks; anything larger is
  a buffer freed with its frame. After an upload this renderer keeps no copy
  of a picture's bytes.

Not measured yet: the numbers above are worked out from the code.
