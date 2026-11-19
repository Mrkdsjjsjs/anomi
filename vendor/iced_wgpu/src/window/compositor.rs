//! Connect a window with a renderer.
use crate::adapter;
use crate::core::Color;
use crate::core::backend;
use crate::core::renderer;
use crate::enabled;
use crate::graphics::color;
use crate::graphics::compositor;
use crate::graphics::{self, Antialiasing, Shell, Viewport};
use crate::{Engine, Renderer};

use super::shared;

/// A window graphics backend for iced powered by `wgpu`.
pub struct Compositor {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
    engine: Engine,
    settings: Settings,
    // Anomi: the number `adapter::active` knows this compositor by while it
    // draws (adapter.rs).
    generation: u64,
    // Anomi: how its frames reach the windows (`PATCHES.md`, "Drawn on one
    // adapter, shown through another").
    shows: Shows,
}

/// Anomi: how a compositor's frames reach its windows (`adapter::path`).
enum Shows {
    /// Into a Vulkan surface of each window: the adapter is the one the
    /// desktop composites on.
    Direct,
    /// Drawn offscreen, read back and shown through each window's shared
    /// memory (shared.rs): no Vulkan surface is ever made.
    Shared(shared::Context),
}

/// A window's surface.
///
/// Anomi: wgpu's surface, if it could be made, and what's known of it
/// (`PATCHES.md`, "Surfaces that fail without a panic or a loop"). Stock,
/// the surface was wgpu's own and a failure to make one panicked.
#[derive(Debug)]
pub struct Surface {
    raw: Option<wgpu::Surface<'static>>,
    /// On an adapter the desktop doesn't composite on, the window's shared
    /// memory in its place (shared.rs); `raw` is then empty.
    shared: Option<shared::Window>,
    /// Its last configuration succeeded: a surface never configured must not
    /// be asked for a frame, as wgpu panics on the error then.
    configured: bool,
    /// No frame was presented since it was configured.
    fresh: bool,
    /// Its last frame came suboptimal and was set aside.
    suboptimal: bool,
    /// Frames in a row it couldn't give, none presented between.
    missed: u8,
}

/// Anomi: frames in a row a window's surface may fail to give (lost, or
/// outdated however it's configured), none presented between, before the
/// app is told (lost.rs) as of a surface that couldn't be made: a window
/// that never shows a frame is a GPU that didn't start, and iced only makes
/// the surface again, at most once a second, for as long as it fails.
const MISSED: u8 = 3;

impl Surface {
    fn new(raw: Option<wgpu::Surface<'static>>) -> Self {
        Self {
            raw,
            shared: None,
            configured: false,
            fresh: true,
            suboptimal: false,
            missed: 0,
        }
    }

    fn shared(window: shared::Window) -> Self {
        Self {
            shared: Some(window),
            ..Self::new(None)
        }
    }

    /// Counts a frame it couldn't give: whether that makes [`MISSED`].
    fn miss(&mut self) -> bool {
        self.missed = self.missed.saturating_add(1);

        self.missed == MISSED
    }
}

/// A compositor error.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    /// The surface creation failed.
    #[error("the surface creation failed: {0}")]
    SurfaceCreationFailed(#[from] wgpu::CreateSurfaceError),
    /// The surface is not compatible.
    #[error("the surface is not compatible")]
    IncompatibleSurface,
    /// No adapter was found for the options requested.
    #[error("no adapter was found for the options requested: {0}")]
    NoAdapterFound(String),
    /// No device request succeeded.
    #[error("no device request succeeded: {0:?}")]
    RequestDeviceFailed(Vec<(wgpu::Limits, wgpu::RequestDeviceError)>),
    /// Anomi: the device raised an error while the renderer was set up on
    /// it (its pipelines, its shaders).
    #[error("the renderer could not be set up on the device: {0}")]
    SetupFailed(String),
    /// Anomi: the window system's shared memory couldn't be set up for an
    /// adapter whose frames are read back (shared.rs).
    #[error("the window's shared memory could not be set up: {0}")]
    Shared(String),
}

impl From<Error> for backend::Error {
    fn from(error: Error) -> Self {
        Self::GraphicsAdapterNotFound {
            backend: "wgpu",
            reason: backend::Reason::RequestFailed(error.to_string()),
        }
    }
}

impl Compositor {
    /// Requests a new [`Compositor`] with the given [`Settings`].
    ///
    /// Returns `None` if no compatible graphics adapter could be found.
    pub async fn request(
        settings: Settings,
        display: impl compositor::Display + Clone,
        compatible_window: impl compositor::Window,
        shell: Shell,
    ) -> Result<Self, Error> {
        let result = Self::start(settings, display, compatible_window, shell).await;

        // Anomi: why the GPU didn't start, for the log and the app's Settings
        // page (adapter.rs); iced's fallback then draws with tiny-skia.
        if let Err(error) = &result {
            log::warn!("wgpu didn't start: {error}");

            adapter::failed(error.to_string());
        }

        result
    }

    async fn start(
        settings: Settings,
        display: impl compositor::Display + Clone,
        compatible_window: impl compositor::Window,
        shell: Shell,
    ) -> Result<Self, Error> {
        // Anomi: the window system's connection again, for the shared memory
        // of an adapter whose frames are read back (shared.rs).
        let shared_display: Box<dyn compositor::Display> = Box::new(display.clone());

        let instance = wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor {
            backends: settings.backends,
            flags: if cfg!(feature = "strict-assertions") {
                wgpu::InstanceFlags::debugging()
            } else {
                wgpu::InstanceFlags::empty()
            },
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display))
        })
        .await;

        log::info!("{settings:#?}");

        // Anomi: the adapter the app chose among Vulkan's (adapter.rs),
        // enumerated without asking any of them about the window; stock took
        // wgpu's pick by power preference, the discrete card whenever it was
        // the only one, and made a surface first to ask each about it.
        let mut candidates = adapter::candidates(&instance, settings.backends).await;
        let display_adapter = adapter::display_adapter();

        let seen: Vec<adapter::Info> = candidates
            .iter()
            .map(|(_adapter, info)| adapter::Info {
                direct: Some(
                    adapter::path(info, display_adapter.as_deref()) == adapter::Path::Direct,
                ),
                ..info.clone()
            })
            .collect();

        log::info!("Available adapters: {seen:#?}");

        let picked = adapter::pick(&seen, &adapter::choice());

        adapter::record_seen(seen);

        let (chosen, mut info) = candidates.swap_remove(picked.map_err(Error::NoAdapterFound)?);

        log::info!("Selected: {:#?}", chosen.get_info());

        // Anomi: a Vulkan surface of the window only on the adapter the
        // desktop composites on, which imports its buffers as its own; any
        // other adapter's frames are read back and shown through shared
        // memory, and it never gets a surface (`PATCHES.md`, "Drawn on one
        // adapter, shown through another").
        let direct = match adapter::path(&info, display_adapter.as_deref()) {
            adapter::Path::Direct => surface_format(&instance, &chosen, compatible_window)?,
            adapter::Path::Shared => None,
        };

        let (format, alpha_mode, shows) = match direct {
            Some((format, alpha_mode)) => (format, alpha_mode, Shows::Direct),
            None => (
                shared::FORMAT,
                wgpu::CompositeAlphaMode::Opaque,
                Shows::Shared(shared::Context::new(shared_display).map_err(Error::Shared)?),
            ),
        };

        info.direct = Some(matches!(shows, Shows::Direct));

        log::info!(
            "draws on {}, {}",
            info.name,
            if matches!(shows, Shows::Direct) {
                "presented to the window directly"
            } else {
                "shown through shared memory"
            }
        );

        log::info!("Selected format: {format:?} with alpha mode: {alpha_mode:?}");

        #[cfg(target_arch = "wasm32")]
        let limits = [wgpu::Limits::downlevel_webgl2_defaults().using_resolution(chosen.limits())];

        #[cfg(not(target_arch = "wasm32"))]
        let limits = [
            wgpu::Limits::default().using_resolution(chosen.limits()),
            wgpu::Limits::downlevel_defaults().using_resolution(chosen.limits()),
        ];

        let limits = limits.into_iter().map(|limits| wgpu::Limits {
            max_non_sampler_bindings: 2048,
            ..limits
        });

        // Request SHADER_F16 only if the adapter supports it (e.g., not available in WebGL2)
        let required_features = if chosen.features().contains(wgpu::Features::SHADER_F16) {
            wgpu::Features::SHADER_F16
        } else {
            wgpu::Features::empty()
        };

        let mut errors = Vec::new();

        for required_limits in limits {
            let result = chosen
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("iced_wgpu::window::compositor device descriptor"),
                    required_features,
                    required_limits: required_limits.clone(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                })
                .await;

            match result {
                Ok((device, queue)) => {
                    // Anomi: the renderer is set up inside error scopes, so a
                    // device that can't build its pipelines (a driver's
                    // shader compiler failing) refuses here and tiny-skia
                    // draws, rather than failing at the first frame.
                    let scoped = device.clone();

                    let (engine, error) = caught(&scoped, || {
                        Engine::new(&chosen, device, queue, format, settings.antialiasing, shell)
                    });

                    if let Some(error) = error {
                        return Err(Error::SetupFailed(error.to_string()));
                    }

                    // Anomi: from here the device's failures reach the app
                    // (lost.rs), and the adapter is the one drawing.
                    let generation = adapter::next_generation();

                    engine.watch.serve(generation);
                    adapter::started(generation, info);

                    // Anomi: geometry the processor's renderer cached can't
                    // be drawn by this one (iced's fallback reaches
                    // `unreachable!()`). iced clears every cache when the app
                    // switches renderer, but not when a window opens from
                    // the tray with a renderer of the other kind, and a
                    // cache the app keeps outside the window's widgets (a
                    // QR code's) outlives that window.
                    graphics::cache::invalidate_all();

                    return Ok(Compositor {
                        instance,
                        adapter: chosen,
                        format,
                        alpha_mode,
                        engine,
                        settings,
                        generation,
                        shows,
                    });
                }
                Err(error) => {
                    errors.push((required_limits, error));
                }
            }
        }

        Err(Error::RequestDeviceFailed(errors))
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        // Anomi: iced drops the compositor when the last window closes (the
        // tray) or when the app switches renderer; the device goes with it,
        // and what was cached for it can't be drawn by tiny-skia (see
        // `start`).
        adapter::stopped(self.generation);
        graphics::cache::invalidate_all();
    }
}

/// Anomi: the window's Vulkan surface on `adapter`, the one the desktop
/// composites on: the format and alpha mode to configure it with, or `None`
/// when Vulkan can't present to the window there, whose frames are then
/// read back too. Only this adapter is ever asked.
fn surface_format(
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
    window: impl compositor::Window,
) -> Result<Option<(wgpu::TextureFormat, wgpu::CompositeAlphaMode)>, Error> {
    // An error at once; stock went on without a surface and failed later
    // with `IncompatibleSurface`.
    let surface = instance.create_surface(wgpu::SurfaceTarget::Window(Box::new(window)))?;

    if !adapter.is_surface_supported(&surface) {
        log::info!("Vulkan can't present to the window on the desktop's own adapter");

        return Ok(None);
    }

    let capabilities = surface.get_capabilities(adapter);

    let formats = capabilities.formats.iter().copied();

    log::info!("Available formats: {formats:#?}");

    const BLACKLIST: &[wgpu::TextureFormat] = &[
        wgpu::TextureFormat::Rgb10a2Unorm,
        wgpu::TextureFormat::Rgb10a2Uint,
        wgpu::TextureFormat::Rgba16Float,
    ];

    let mut formats = formats.filter(|format| {
        format.required_features() == wgpu::Features::empty() && !BLACKLIST.contains(format)
    });

    let format = if color::GAMMA_CORRECTION {
        formats.find(wgpu::TextureFormat::is_srgb)
    } else {
        formats.find(|format| !wgpu::TextureFormat::is_srgb(format))
    };

    let format = format.or_else(|| {
        log::warn!("No format found!");

        capabilities.formats.first().copied()
    });

    let alpha_modes = capabilities.alpha_modes;

    log::info!("Available alpha modes: {alpha_modes:#?}");

    let preferred_alpha = if alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        wgpu::CompositeAlphaMode::Auto
    };

    format
        .zip(Some(preferred_alpha))
        .map(Some)
        .ok_or(Error::IncompatibleSurface)
}

/// Anomi: runs `f`, catching what wgpu raises on `device` meanwhile instead
/// of handing it to the device's error handler; the first error caught, if
/// any.
fn caught<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> (T, Option<wgpu::Error>) {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

    let value = f();

    // Innermost first, as wgpu requires; each is ready at once natively.
    let internal = adapter::ready(internal.pop()).flatten();
    let validation = adapter::ready(validation.pop()).flatten();
    let out_of_memory = adapter::ready(out_of_memory.pop()).flatten();

    (value, validation.or(internal).or(out_of_memory))
}

/// Creates a [`Compositor`] with the given [`Settings`] and window.
pub async fn new(
    settings: Settings,
    display: impl compositor::Display + Clone,
    compatible_window: impl compositor::Window,
    shell: Shell,
) -> Result<Compositor, Error> {
    Compositor::request(settings, display, compatible_window, shell).await
}

/// Presents the given primitives with the given [`Compositor`].
///
/// Anomi: no failure of the GPU or the surface panics or makes iced loop
/// (`PATCHES.md`, "Surfaces that fail without a panic or a loop"), and it
/// never answers `OutOfMemory`, on which iced panics. A surface that keeps
/// giving no frame is reported to the app ([`MISSED`]).
pub fn present(
    renderer: &mut Renderer,
    surface: &mut Surface,
    viewport: &Viewport,
    background_color: Color,
    on_pre_present: impl FnOnce(),
) -> Result<(), compositor::SurfaceError> {
    let answer = give(
        renderer,
        surface,
        viewport,
        background_color,
        on_pre_present,
    );

    match &answer {
        Ok(()) => surface.missed = 0,
        Err(compositor::SurfaceError::Lost | compositor::SurfaceError::Other) => {
            if surface.miss() {
                renderer.engine.watch.surface(format!(
                    "the window's surface gave no frame {MISSED} times in a row"
                ));
            }
        }
        // Hidden, or to be configured again: not a failure.
        Err(_) => {}
    }

    answer
}

/// The frame drawn and presented, or why not.
fn give(
    renderer: &mut Renderer,
    surface: &mut Surface,
    viewport: &Viewport,
    background_color: Color,
    on_pre_present: impl FnOnce(),
) -> Result<(), compositor::SurfaceError> {
    // Anomi: a lost device draws nothing and is asked nothing until the app
    // switches renderer. "Occluded" has iced wait for the next redraw it's
    // asked for; stock's "lost" had it make the surface again once a second,
    // forever.
    if renderer.engine.is_lost() {
        return Err(compositor::SurfaceError::Occluded);
    }

    // Anomi: an adapter the desktop doesn't composite on: the frame is read
    // back into the window's shared memory (shared.rs).
    if let Some(shared) = &mut surface.shared {
        return shared.present(renderer, viewport, background_color, on_pre_present);
    }

    // Anomi: a surface that couldn't be made or configured is made again by
    // iced, at most once a second.
    let Some(raw) = surface.raw.as_ref().filter(|_raw| surface.configured) else {
        return Err(compositor::SurfaceError::Lost);
    };

    match raw.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => {
            draw(renderer, frame, viewport, background_color, on_pre_present);

            surface.fresh = false;
            surface.suboptimal = false;

            Ok(())
        }
        // Anomi: a suboptimal frame is set aside once and the surface
        // configured again, as stock did each time; one that comes
        // suboptimal again right after is drawn. A driver may call every
        // frame suboptimal, and stock then configured and redrew without end.
        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            if surface.suboptimal {
                draw(renderer, frame, viewport, background_color, on_pre_present);

                surface.fresh = false;

                Ok(())
            } else {
                surface.suboptimal = true;

                Err(compositor::SurfaceError::Outdated)
            }
        }
        // Anomi: outdated again right after a configure, and configuring
        // doesn't help: "other" has iced try again at most once a second,
        // where "outdated" has it configure and redraw at once, each time.
        wgpu::CurrentSurfaceTexture::Outdated => Err(if surface.fresh {
            compositor::SurfaceError::Other
        } else {
            compositor::SurfaceError::Outdated
        }),
        // Anomi: no image for a second (a hidden window whose compositor
        // keeps its images, with FIFO) is taken as occluded: iced waits for
        // the next redraw asked for, where stock asked one at once and
        // blocked the loop a second at a time while the window was hidden.
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            Err(compositor::SurfaceError::Occluded)
        }
        wgpu::CurrentSurfaceTexture::Lost => Err(compositor::SurfaceError::Lost),
        wgpu::CurrentSurfaceTexture::Validation => Err(compositor::SurfaceError::Other),
    }
}

fn draw(
    renderer: &mut Renderer,
    frame: wgpu::SurfaceTexture,
    viewport: &Viewport,
    background_color: Color,
    on_pre_present: impl FnOnce(),
) {
    let view = &frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let _submission = renderer.present(
        Some(background_color),
        frame.texture.format(),
        view,
        viewport,
    );

    // Present the frame
    on_pre_present();
    renderer.engine.queue.present(frame);
}

impl graphics::Compositor for Compositor {
    type Renderer = Renderer;
    type Surface = Surface;

    async fn new(
        settings: backend::Settings,
        display: impl compositor::Display + Clone,
        compatible_window: impl compositor::Window,
        shell: Shell,
    ) -> Result<Self, backend::Error> {
        // Anomi: nothing starts until the app enables this renderer
        // (enabled.rs); until then iced's fallback takes tiny-skia, as for a
        // backend the settings don't name.
        if !enabled::is_enabled()
            || (settings.backend.hardware().is_none() && !settings.backend.matches("wgpu"))
        {
            return Err(backend::Error::GraphicsAdapterNotFound {
                backend: "wgpu",
                reason: backend::Reason::DidNotMatch {
                    preferred_backend: settings.backend,
                },
            });
        }

        let mut settings = Settings::from(settings);

        if let Some(backends) = wgpu::Backends::from_env() {
            settings.backends = backends;
        }

        if let Some(present_mode) = present_mode_from_env() {
            settings.present_mode = present_mode;
        }

        Ok(new(settings, display, compatible_window, shell).await?)
    }

    fn create_renderer(&self, settings: renderer::Settings) -> Self::Renderer {
        Renderer::new(self.engine.clone(), settings)
    }

    fn create_surface(&mut self, window: impl compositor::Window) -> Self::Surface {
        // Anomi: on an adapter the desktop doesn't composite on, the
        // window's shared memory, and no Vulkan surface (shared.rs).
        if let Shows::Shared(context) = &self.shows {
            return match shared::Window::new(context, Box::new(window)) {
                Ok(window) => Surface::shared(window),
                Err(error) => {
                    self.engine.watch.surface(format!(
                        "the window's shared memory could not be set up: {error}"
                    ));

                    Surface::new(None)
                }
            };
        }

        // Anomi: a surface that can't be made is reported to the app
        // (lost.rs) and kept empty; stock panicked.
        match self
            .instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window)))
        {
            Ok(raw) => Surface::new(Some(raw)),
            Err(error) => {
                self.engine
                    .watch
                    .surface(format!("the window's surface could not be made: {error}"));

                Surface::new(None)
            }
        }
    }

    fn configure_surface(&mut self, surface: &mut Self::Surface, width: u32, height: u32) {
        // Anomi: shared memory is sized to the window (shared.rs).
        if let Some(shared) = &mut surface.shared {
            if let Err(error) = shared.configure(width, height) {
                self.engine.watch.surface(format!(
                    "the window's shared memory could not be sized: {error}"
                ));
            }

            return;
        }

        let Some(raw) = &surface.raw else {
            return;
        };

        // Anomi: configured inside error scopes, so whether it worked is
        // known here, and a failure reported to the app (lost.rs).
        let ((), error) = caught(&self.engine.device, || {
            raw.configure(
                &self.engine.device,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: self.format,
                    present_mode: self.settings.present_mode,
                    width,
                    height,
                    alpha_mode: self.alpha_mode,
                    view_formats: vec![],
                    desired_maximum_frame_latency: 1,
                    color_space: wgpu::SurfaceColorSpace::Auto,
                },
            );
        });

        surface.configured = error.is_none();
        surface.fresh = true;

        if let Some(error) = error {
            self.engine
                .watch
                .surface(format!("the window's surface could not be set up: {error}"));
        }
    }

    fn information(&self) -> compositor::Information {
        let information = self.adapter.get_info();

        compositor::Information {
            adapter: information.name,
            backend: format!("{:?}", information.backend),
        }
    }

    fn present(
        &mut self,
        renderer: &mut Self::Renderer,
        surface: &mut Self::Surface,
        viewport: &Viewport,
        background_color: Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), compositor::SurfaceError> {
        present(
            renderer,
            surface,
            viewport,
            background_color,
            on_pre_present,
        )
    }

    fn screenshot(
        &mut self,
        renderer: &mut Self::Renderer,
        viewport: &Viewport,
        background_color: Color,
    ) -> Vec<u8> {
        renderer.screenshot(viewport, background_color)
    }
}

/// The settings of a [`Compositor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// The present mode of the [`Renderer`].
    ///
    /// [`Renderer`]: crate::Renderer
    pub present_mode: wgpu::PresentMode,

    /// The graphics backends to use.
    ///
    /// Anomi: Vulkan, the only one built in (`PATCHES.md`, "The adapter the
    /// app chooses").
    pub backends: wgpu::Backends,

    /// The power-usage preference for graphics adapters.
    ///
    /// By default, it is [`backend::PowerPreference::None`].
    ///
    /// Anomi: unused; the adapter is the one [`adapter::choose`] asks for.
    pub power_preference: backend::PowerPreference,

    /// The antialiasing strategy that will be used for triangle primitives.
    ///
    /// By default, it is `None`.
    pub antialiasing: Option<Antialiasing>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            present_mode: wgpu::PresentMode::AutoVsync,
            backends: wgpu::Backends::VULKAN,
            power_preference: backend::PowerPreference::None,
            antialiasing: None,
        }
    }
}

impl From<backend::Settings> for Settings {
    fn from(settings: backend::Settings) -> Self {
        // Anomi: `Best` and no API at all mean Vulkan; stock, every backend.
        let backends = settings
            .backend
            .hardware()
            .map(|api| match api {
                backend::Api::Best | backend::Api::Vulkan => wgpu::Backends::VULKAN,
                backend::Api::Metal => wgpu::Backends::METAL,
                backend::Api::DirectX12 => wgpu::Backends::DX12,
                backend::Api::OpenGL => wgpu::Backends::GL,
                backend::Api::WebGPU => wgpu::Backends::BROWSER_WEBGPU,
            })
            .unwrap_or(wgpu::Backends::VULKAN);

        Self {
            // Anomi: with vsync, FIFO, which every surface offers: frames are
            // drawn only when iced asks, and an animation's are paced by the
            // display rather than drawn as fast as the GPU goes.
            present_mode: if settings.vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            antialiasing: settings.antialiasing.then_some(Antialiasing::MSAAx4),
            backends,
            power_preference: settings.power_preference,
        }
    }
}

/// Obtains a [`wgpu::PresentMode`] from the current environment
/// configuration, if set.
///
/// The value returned by this function can be changed by setting
/// the `ICED_PRESENT_MODE` env variable. The possible values are:
///
/// - `vsync` → [`wgpu::PresentMode::AutoVsync`]
/// - `no_vsync` → [`wgpu::PresentMode::AutoNoVsync`]
/// - `immediate` → [`wgpu::PresentMode::Immediate`]
/// - `fifo` → [`wgpu::PresentMode::Fifo`]
/// - `fifo_relaxed` → [`wgpu::PresentMode::FifoRelaxed`]
/// - `mailbox` → [`wgpu::PresentMode::Mailbox`]
pub fn present_mode_from_env() -> Option<wgpu::PresentMode> {
    let present_mode = std::env::var("ICED_PRESENT_MODE").ok()?;

    match present_mode.to_lowercase().as_str() {
        "vsync" => Some(wgpu::PresentMode::AutoVsync),
        "no_vsync" => Some(wgpu::PresentMode::AutoNoVsync),
        "immediate" => Some(wgpu::PresentMode::Immediate),
        "fifo" => Some(wgpu::PresentMode::Fifo),
        "fifo_relaxed" => Some(wgpu::PresentMode::FifoRelaxed),
        "mailbox" => Some(wgpu::PresentMode::Mailbox),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::enabled::SWITCH;

    use std::sync::PoisonError;
    use wgpu::rwh;

    /// A display and a window that have no handle: the compositor must
    /// refuse before it asks for one.
    #[derive(Debug, Clone)]
    struct Nothing;

    impl rwh::HasDisplayHandle for Nothing {
        fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
            Err(rwh::HandleError::Unavailable)
        }
    }

    impl rwh::HasWindowHandle for Nothing {
        fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
            Err(rwh::HandleError::Unavailable)
        }
    }

    #[test]
    fn the_compositor_refuses_until_the_app_enables_it() {
        let _switch = SWITCH.lock().unwrap_or_else(PoisonError::into_inner);

        enabled::set_enabled(false);

        let new = <Compositor as graphics::Compositor>::new(
            backend::Settings::default(),
            Nothing,
            Nothing,
            Shell::headless(),
        );

        let Some(Err(error)) = adapter::ready(new) else {
            panic!("a compositor started while the switch was off");
        };

        assert!(matches!(
            error,
            backend::Error::GraphicsAdapterNotFound {
                backend: "wgpu",
                reason: backend::Reason::DidNotMatch { .. },
            }
        ));
    }

    #[test]
    fn a_surface_that_keeps_giving_no_frame_is_reported_once() {
        let mut surface = Surface::new(None);

        assert!(!surface.miss());
        assert!(!surface.miss());
        assert!(surface.miss(), "the third in a row");
        assert!(!surface.miss(), "told once");

        surface.missed = 0;

        assert!(!surface.miss(), "a frame presented starts the count over");
    }

    #[test]
    fn the_settings_ask_for_vulkan_and_vsync() {
        let settings = Settings::from(backend::Settings::default());

        assert_eq!(settings.backends, wgpu::Backends::VULKAN);
        assert_eq!(settings.present_mode, wgpu::PresentMode::AutoVsync);
        assert_eq!(settings.antialiasing, Some(Antialiasing::MSAAx4));
        assert_eq!(Settings::default().backends, wgpu::Backends::VULKAN);
    }
}
