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
}

/// A window's surface.
///
/// Anomi: wgpu's surface, if it could be made, and what's known of it
/// (`PATCHES.md`, "Surfaces that fail without a panic or a loop"). Stock,
/// the surface was wgpu's own and a failure to make one panicked.
#[derive(Debug)]
pub struct Surface {
    raw: Option<wgpu::Surface<'static>>,
    /// Its last configuration succeeded: a surface never configured must not
    /// be asked for a frame, as wgpu panics on the error then.
    configured: bool,
    /// No frame was presented since it was configured.
    fresh: bool,
    /// Its last frame came suboptimal and was set aside.
    suboptimal: bool,
}

impl Surface {
    fn new(raw: Option<wgpu::Surface<'static>>) -> Self {
        Self {
            raw,
            configured: false,
            fresh: true,
            suboptimal: false,
        }
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
        display: impl compositor::Display,
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
        display: impl compositor::Display,
        compatible_window: impl compositor::Window,
        shell: Shell,
    ) -> Result<Self, Error> {
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

        // Anomi: an error at once; stock went on without a surface and
        // failed later with `IncompatibleSurface`.
        let compatible_surface =
            instance.create_surface(wgpu::SurfaceTarget::Window(Box::new(compatible_window)))?;

        // Anomi: the adapter the app chose among Vulkan's, if it can present
        // to the window (adapter.rs); stock took wgpu's pick by power
        // preference, the discrete card whenever it was the only one.
        let mut candidates =
            adapter::candidates(&instance, settings.backends, Some(&compatible_surface)).await;

        let seen: Vec<adapter::Info> = candidates
            .iter()
            .map(|(_adapter, info)| info.clone())
            .collect();

        log::info!("Available adapters: {seen:#?}");

        let picked = adapter::pick(&seen, &adapter::choice(), true);

        adapter::record_seen(seen);

        let (chosen, info) = candidates.swap_remove(picked.map_err(Error::NoAdapterFound)?);

        log::info!("Selected: {:#?}", chosen.get_info());

        let (format, alpha_mode) = {
            let capabilities = compatible_surface.get_capabilities(&chosen);

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

            let preferred_alpha = if alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied)
            {
                wgpu::CompositeAlphaMode::PreMultiplied
            } else {
                wgpu::CompositeAlphaMode::Auto
            };

            format
                .zip(Some(preferred_alpha))
                .ok_or(Error::IncompatibleSurface)?
        };

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

                    return Ok(Compositor {
                        instance,
                        adapter: chosen,
                        format,
                        alpha_mode,
                        engine,
                        settings,
                        generation,
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
        // tray) or when the app switches renderer; the device goes with it.
        adapter::stopped(self.generation);
    }
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
    display: impl compositor::Display,
    compatible_window: impl compositor::Window,
    shell: Shell,
) -> Result<Compositor, Error> {
    Compositor::request(settings, display, compatible_window, shell).await
}

/// Presents the given primitives with the given [`Compositor`].
///
/// Anomi: no failure of the GPU or the surface panics or makes iced loop
/// (`PATCHES.md`, "Surfaces that fail without a panic or a loop"), and it
/// never answers `OutOfMemory`, on which iced panics.
pub fn present(
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
        display: impl compositor::Display,
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
    fn the_settings_ask_for_vulkan_and_vsync() {
        let settings = Settings::from(backend::Settings::default());

        assert_eq!(settings.backends, wgpu::Backends::VULKAN);
        assert_eq!(settings.present_mode, wgpu::PresentMode::AutoVsync);
        assert_eq!(settings.antialiasing, Some(Antialiasing::MSAAx4));
        assert_eq!(Settings::default().backends, wgpu::Backends::VULKAN);
    }
}
