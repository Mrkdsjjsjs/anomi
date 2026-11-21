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
  system, which iced turns on in the app's build), and softbuffer, at
  tiny-skia's version, for the window's shared memory (below).
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
out CPU ones (lavapipe, llvmpipe: slower than tiny-skia), asks none of them
about the window, and takes the one `adapter::choose` asks for:

- `Choice::Auto` (the default): the first integrated adapter, or none.
  Never a discrete one, so on a desktop the card the games use stays
  theirs; with no integrated adapter Vulkan can use, the compositor refuses
  and tiny-skia draws.
- `Choice::Bus("0000:73:00.0")`: the adapter at that PCI bus id, as Vulkan
  (`VK_EXT_pci_bus_info`) and sysfs write it, or, when no adapter's bus id
  matches, the one whose vendor and device ids do (`"1002:13c0"`); with
  none there, the compositor refuses.

How its frames reach the window is the next section's. The power
preference in the settings is unused. `Backend::Best` and no API mean
Vulkan (stock: every backend); `WGPU_BACKEND` and `ICED_PRESENT_MODE` still
override, as stock. The headless renderer takes the same choice without a
window. What the last start enumerated (`adapter::seen`, with the way each
would show its frames), the adapter drawing now (`adapter::active`: none
while tiny-skia draws, in the tray, where iced drops the compositor, or once
the device is lost) and why the last start failed (`adapter::last_error`,
cleared by a start) are kept for the app's Settings page. A compositor is
known by a number: `iced::backend::configure` makes the next before it
drops the one before, whose drop then leaves the new one active.
`adapter::list` enumerates without a window, loading every Vulkan driver
into the process (the app runs it in a child process). Tests: the choice on
the owner's desktop (a 3080 and a Radeon 610M), by bus, by ids, and the
active adapter across two compositors; `adapter::list` against the real
drivers, opt-in.

To carry over: in `Compositor::request`, `adapter::candidates`, `pick` and
the records in place of `request_adapter`; in `Headless::new`, the same;
`Settings::from`'s backends.

## Drawn on one adapter, shown through another

`window/shared.rs` (new), `adapter.rs` `Path`, `path`,
`set_display_adapter`, `window/compositor.rs` `Shows`, `start`,
`surface_format`, `create_surface`, `configure_surface`, `give`;
`Cargo.toml`'s softbuffer. A Vulkan surface hands the desktop's compositor
the adapter's own buffers to import. On the owner's desktop (mutter on the
RTX 3080, the monitor on it) Automatic took the Radeon 610M, Vulkan said it
could present to the window, and 150 ms after the window opened mutter
failed to import its first buffers and ended the connection:

    gnome-shell: WL: error in client communication (pid …)
    [destroyed object]: error 7: failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D
    ERROR iced_wgpu::lost: the window's surface could not be set up: Validation Error … In Surface::configure: Surface does not support the adapter's queue family
    WARN iced_winit: Error Lost when presenting surface. Recovering it.
    ERROR iced_wgpu::lost: … Surface does not support the adapter's queue family
    Protocol error 7 on object @0

The swapchain the first configure made had sent its buffers; mutter's
refusal came back asynchronously, and the next configure (a resize) found
the connection dead, which Mesa's driver and wgpu report as a surface the
adapter's queue family can't present to. A Wayland protocol error ends the
process: nothing can fall back. Stock iced makes a surface before choosing
an adapter and asks each `is_surface_supported`, which only says whether
the driver can speak to the window system, not whether the compositor can
import what it makes.

So the window gets a Vulkan surface only on the adapter the desktop
composites on: the app says which (`adapter::set_display_adapter`, from the
Wayland compositor's linux-dmabuf feedback, else the one adapter driving a
monitor), and `adapter::path` compares bus ids. Any other adapter, or every
one when the app says nothing, is `Path::Shared`: the compositor makes no
surface at all (`Shows::Shared`, a softbuffer context on the window
system's connection), and each window gets softbuffer's shared memory, the
path tiny-skia's frames take, which a compositor takes from any client.
Each frame iced presents is drawn into a texture of the window's size on
the adapter, copied there into a buffer the processor maps, and its rows
copied once into softbuffer's buffer, where they changed (next section):
`Bgra8Unorm` (sRGB where iced corrects gamma, as the surface's format
would be) is softbuffer's 0RGB pixel byte for byte on a little-endian
machine, so nothing converts a pixel. The window's thread waits once a
frame, for that frame's drawing and copies and nothing else, at most a
second. While a channel scrolls on the Radeon 610M under GNOME Shell on the
3080, that is 3.0 ms a frame at 1400 by 868 and 7.8 ms maximised at 3440 by
1408, read back where it changed (next section); drawn and read back whole,
4.1 and 10.1 ms
(`docs/measurements/README.md`, "Drawn on the Radeon, shown through shared
memory"). The texture is made at the first frame drawn and again when the
size changes (no wait: the old one goes when the adapter is done with it),
and freed with the window. Nothing runs between frames. A frame that can't
be read back counts toward `MISSED`; a lost device answers "occluded", as on
the direct path. On the desktop's own adapter, a window Vulkan can't
present to is read back too. Tests: the path on the owner's desktop and a
laptop's, when unknown, a driver that doesn't say its bus; rows copied
without their padding, at their place;
`a_frame_read_back_equals_the_one_drawn_in_place` (GPU, opt-in): a frame
read back whole equals, byte for byte (on the Radeon 610M and the RTX
3080), the same frame read out by `Renderer::screenshot`, red staying red.

To carry over: `shared.rs`, `Shows` and the surface's `shared`, the path
chosen in `start` before any surface is made, `surface_format`.

## Only what changed, read back and shown

`damage.rs` (new), `window/shared.rs` `Window::present`, `Frame::read`,
`lib.rs` `Renderer::draw_within`, `settle`, `render`, `triangle.rs`
`State::render`, `Layer::render`, `triangle/msaa.rs` `State::render`,
`quad.rs` `Batch::damage`, `reach`, `text.rs` `Cache::key`, `text`; in
`vendor/iced_tiny_skia`, `reach` and `window::compositor::merged` made
public. A frame read back (previous section) was drawn, read back and
uploaded by the compositor whole: 19.8 MB a frame maximised on the owner's
monitor, for a blinking caret as for a scrolled channel, and gnome-shell
copied it all into its texture on each commit. Now:

- **What changed.** `damage::Scene` keeps what a frame's merged layers
  draw: quads by value (byte for byte), live meshes and text by value as
  iced holds them (a paragraph weakly, compared as the same allocation as
  well as equal: iced's equality takes two live paragraphs of one font,
  size and bounds for equal whatever they say), pictures by their handle's
  id, caches of meshes and text by id, version and address, custom
  primitives by their bounds alone. Nothing it holds keeps a picture's or
  a cache's memory alive (a video frame's slot is its writer's again once
  the window read the frame after it, as before). The next frame's layers are held
  against it as tiny-skia holds its frames' (`Scene::between`): layers that
  end both frames with the same bounds paired from the end, the rest by
  place; in a pair of the same bounds each kind of primitive by place, and
  what differs damages where it may lay pixels in either frame (a quad its
  shadow-grown bounds and a pixel, text tiny-skia's `text_reach`, 8 px round
  its lines, a picture its turned bounds and a pixel, a mesh its scissor);
  a layer unpaired, or of other bounds, where its primitives reach, as one
  rectangle; custom primitives always. Empty layers draw nothing and are
  left out. A pixel no damage covers is drawn by the same primitives in the
  same order in both frames. The damage is grouped as tiny-skia's
  (`damage::group`, then its `merged`) and taken as the window's pixels it
  touches, a pixel more round each (`damage::regions`). A frame of another
  size, scale, background or font set (`Key`), the first, and the first
  after the window was sized or a frame failed, change the whole window.
- **What the buffer lacks.** softbuffer's buffer holds the frame presented
  `age` frames ago (two on Wayland, one on X11, unknown after a resize,
  whose buffers keep their age): it lacks the frame's change and those of
  the `age - 1` frames presented since its own (`damage::History`, three
  kept), made disjoint and at most eight (`damage::settled`). None of them
  known, the whole window.
- **Drawn there alone.** `Renderer::draw_within` prepares every layer as
  for the whole frame (the caches see the frame as drawn) and draws each
  layer once in each region its scissor reaches, the scissor cut to it:
  quads, pictures, text and custom primitives in the pass, meshes in theirs
  and the anti-aliased meshes' blit, which stock spread over the whole
  target, within the same clips. A layer outside every region is passed over
  as drawn. Scissoring keeps or drops each fragment, so within a region the
  pixels are the whole frame's; the regions being disjoint, none blends
  twice. Drawn whole (the direct path, screenshots), the one region is the
  viewport and every scissor stock's; the blit, now cut to its layer's
  scissor, left every pixel outside it as it was before as well.
- **Read back there alone.** Each region is copied from the texture into
  the read-back buffer one after another, its rows padded to 256 bytes,
  mapped as far as used, and copied into softbuffer's buffer at its place.
  The read-back buffer is made at the size of the largest regions read
  back (twice as large as before when it grows, as far as the whole frame)
  and smaller again after 30 frames in a row that read back a quarter of it
  or less. softbuffer's buffer is taken before drawing, as the regions
  depend on its age; the compositor gives it back long before (it was
  committed two frames before).
- **Shown there alone.** The buffer is presented with the frame's own
  change as its damage (`present_with_damage`), against the frame the
  compositor shows: gnome-shell uploads those rectangles alone, as from
  tiny-skia.
- **Nothing changed.** A frame that changes nothing into a buffer that lacks
  nothing draws, waits for and reads back nothing; the buffer is presented
  with no damage, as tiny-skia's is.
- **Measured** (`docs/measurements/README.md`, "Drawn on the Radeon, shown
  through shared memory"): a frame of a channel scrolling reads back 2.1 of
  4.7 MiB at 1400 by 868 and 13.5 of 18.6 MiB maximised at 3440 by 1408, a
  key typed 139 and 573 KiB, the caret blinking 74 and 330 KiB; with
  gnome-shell's upload, scrolling now takes less CPU than the processor's
  frames, and a small change about as much.

`Renderer::settle` merges the layers once a frame (`reset` begins one), as
the window holds them against the frame before's before drawing them.
`ANOMI_SHARED_VERIFY=1` holds each frame presented against the frame drawn
whole and read back (`Renderer::screenshot`, a second draw and wait a
frame: for a review only) and logs the pixels that differ. It can't see a
video frame its writer wrote while it was uploaded, as both draws read the
same texture: `ANOMI_FRAMES_VERIFY=1` (`iced_tiny_skia::verify`, its
`PATCHES.md`) hashes each video frame around its `write_texture` in
`image/cache.rs` `upload` and warns when its bytes changed. Tests:
`damage.rs`, the same frame changes nothing, a quad changed damages where
it was and is and not its neighbour, its shadow and edge; a layer more
damages what it draws, layers after it paired from the end, empty layers
left out, a picture with another handle; damage in the window's pixels; a
buffer lacks the changes since its own frame (ages 0 to 5, a frame of
unknown change, history forgotten); regions overlapping merged, apart kept,
at most eight and disjoint; a live picture leaves no buffer stale (a new
handle every one, two or three presents, a hover quad over its edge in a
layer that comes and goes, text above, the window sized midway: 40
presents into buffers held per pixel, two taken in turn at age 2 and one
at age 1, scales 1 and 1.5, each equal to the frame once given what it
lacks); `shared.rs`, damage told within the buffer;
`a_change_alone_is_read_back_and_the_window_shows_the_frame_drawn_whole`
(GPU, opt-in): a small quad under a see-through anti-aliased triangle
changes colour, only its 14 by 12 pixels are drawn and read back (3 KiB,
where the whole frame reads back 325 KiB) into the window holding the frame
before, which then equals the frame drawn whole byte for byte.

To carry over: `damage.rs`; in `shared.rs`, `present` from the scene to
the history and `Frame::read` with its regions; `draw_within` and the
clips in `render`, `triangle::State::render` and the blit; the quad and
text accessors; tiny-skia's two exports.

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

- Damage on the direct path, and in the preparation. A frame read back is
  drawn, read back and shown where it changed ("Only what changed"); one
  drawn into a Vulkan surface is drawn whole, and every frame on either
  path prepares every layer (every paragraph's glyphs) on the CPU, where
  tiny-skia draws only what changed: a blinking caret costs a whole frame's
  preparation. Preparing only the layers in the damage would change what
  the atlases and the picture cache see as drawn (a picture outside it,
  trimmed, uploaded again when next drawn).
- Pixels that only moved. A channel scrolled damages its whole list, read
  back and uploaded whole. Keeping the frame in memory and moving its rows
  would read back only the strip scrolled in, but it isn't sound simply:
  the layers don't say a region scrolled (a scrollable's offset is in every
  primitive's place, beside what stays: the date over the list, the bar of
  new messages, the scrollbar), a row's content lies on half pixels and f32
  layout sums round the same line a row apart from one frame to the next,
  so a row moved isn't the row drawn there (tiny-skia tried it and dropped
  it: 5 frames of 200 wrong, and moved on wrong; its `PATCHES.md`,
  "Measuring"), and softbuffer's buffer is two frames old on Wayland. And
  it would save only the adapter's copy: moving the rows in memory costs
  the processor what copying them from the read-back buffer does, and the
  compositor uploads every moved pixel (`wl_shm` has no "move"), as for
  tiny-skia.
- Not waiting for the adapter. The window's thread waits for the frame's
  drawing and copies on the adapter (a kernel wait on its fence, no CPU),
  now only for the regions that changed. Presenting each frame one late
  (collecting it at the next `present`) would take that off the thread
  while frames come at the display's pace, but the last frame of a change
  must still be shown with nothing else coming: iced calls the compositor
  only to draw, so only a redraw request (`Shell::request_redraw`, every
  window's whole UI drawn again) or a thread waiting on the fence could;
  the buffer's age, which the regions depend on, is known only when the
  buffer is taken, so a late frame drawn for one age must be thrown away
  and drawn whole when it comes back another; and the processor's use and
  the compositor's upload stay what they are. Measure the wait while
  scrolling first.
- Handing the compositor the adapter's frame without the processor's
  memory. A dmabuf it can import is safe to try only with
  `zwp_linux_dmabuf_v1` version 4's surface feedback (the tranches of
  formats and modifiers, per target device, it can take for the window)
  and `zwp_linux_buffer_params_v1.create`, which answers `created` or
  `failed`; `create_immed`, which Mesa's Vulkan swapchain uses, answers a
  failure with the protocol error that ended the connection here. A frame
  would be drawn into a linear buffer in system memory exported as a dmabuf
  (Vulkan's external memory, beneath wgpu), offered as XRGB8888 with the
  linear modifier only when a tranche of the compositor's main device
  lists it, tried once with `create` and shown only once `created` came,
  else read back as now; the window would still wait for the adapter
  before each commit, as NVIDIA's driver may not wait on the buffer's
  implicit fence.
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
