//! The window's renderer as it starts, switches and fails. A GPU that
//! doesn't start (iced's fallback has the processor draw then), or that
//! fails under the window (a lost device, an error wgpu caught, a surface
//! it couldn't make: `iced_wgpu::lost`, whose hook a renderer's thread
//! calls), gives way to the processor from the next frame. The window says
//! why, `renderer_failures` counts it, and the next start tries again;
//! after [`LIMIT`] in a row the processor draws until a GPU is chosen again,
//! which forgets them, as does a first frame drawn on one.

use std::sync::atomic::Ordering;

use iced::Task;
use iced::backend::Backend;
use iced_wgpu::adapter::Kind;
use iced_wgpu::lost::{Cause, Lost};
use yapfast_lang::tr;

use super::plan::Why;
use super::switch::remake;
use super::{Choice, FAILED, FIRST_FRAME, live, state};
use crate::app::App;
use crate::message::Message;
use crate::project::Reach;

/// Failures in a row after which the processor draws until a GPU is chosen
/// again.
pub(super) const LIMIT: u8 = 3;

/// How the GPU failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// It didn't start.
    Start,
    /// It failed under the window.
    Lost(Cause),
}

impl Failure {
    pub(super) fn text(self, name: &str) -> String {
        match self {
            Self::Start => tr!("{} couldn't start.", name),
            Self::Lost(Cause::DeviceLost) => tr!("{} stopped working.", name),
            Self::Lost(Cause::Error) => tr!("{} reported an error while drawing.", name),
            Self::Lost(Cause::Surface) => {
                tr!("The window couldn't be shown with the graphics adapter.").to_owned()
            }
        }
    }
}

/// What comes next: the GPU tried again at the next start, or, after the
/// last failure allowed, only once chosen again.
pub(super) fn next(last: bool) -> &'static str {
    if last {
        tr!(
            "After three failures in a row it's tried again only once it's chosen again in Settings."
        )
    } else {
        tr!("It's tried again at the next start.")
    }
}

/// The notice over the window: why, that the processor draws, and what
/// comes next.
pub(super) fn notice(why: &str, next: Option<&str>) -> String {
    let now = tr!("The processor draws the window now.");
    match next {
        Some(next) => format!("{why} {now} {next}"),
        None => format!("{why} {now}"),
    }
}

/// The window opened, its renderer made: whether the GPU asked for started.
pub(super) fn opened(app: &mut App) -> Task<Message> {
    if !live() || !iced_wgpu::enabled::is_enabled() {
        return Task::none();
    }
    let Some(info) = iced_wgpu::adapter::active() else {
        return refused(app);
    };
    tracing::info!(adapter = %info.name, bus = %info.bus, driver = %info.driver, "the window draws on the graphics adapter");
    // Failures counted wait for a frame drawn on it to be forgotten.
    let unsettled = state().ahead.is_some() || app.layout.renderer_failures != 0;
    FIRST_FRAME.store(unsettled, Ordering::Relaxed);
    Task::none()
}

/// A frame was drawn on the GPU since the window opened: unless it failed
/// meanwhile, its failures in a row are over.
pub(super) fn drew(app: &mut App) -> Task<Message> {
    let first = FIRST_FRAME.swap(false, Ordering::AcqRel);
    let failed = FAILED.load(Ordering::Acquire);
    if !first || failed || iced_wgpu::adapter::active().is_none() {
        return Task::none();
    }
    state().ahead = None;
    if app.layout.renderer_failures != 0 {
        app.layout.edit(|saved| saved.renderer_failures = 0);
    }
    tracing::debug!("a first frame drawn on the graphics adapter; its failures are forgotten");
    Task::none()
}

/// The window's GPU failed under it, as its renderer's thread told.
pub(super) fn lost(app: &mut App, lost: &Lost) -> Task<Message> {
    tracing::error!(cause = ?lost.cause, message = %lost.message, "the graphics adapter failed under the window");
    // A late word from a GPU already given up.
    if !iced_wgpu::enabled::is_enabled() {
        return Task::none();
    }
    failed(app, Failure::Lost(lost.cause))
}

/// iced made the renderer [`super::choose`] asked for, or couldn't make any
/// (then what drew before goes on).
pub(super) fn switched(app: &mut App, error: Option<String>) -> Task<Message> {
    if let Some(error) = error {
        tracing::warn!(%error, "the window's renderer couldn't be made again");
    }
    if !iced_wgpu::enabled::is_enabled() {
        app.show(Reach::SETTINGS);
        return Task::none();
    }
    let Some(info) = iced_wgpu::adapter::active() else {
        return refused(app);
    };
    tracing::info!(adapter = %info.name, bus = %info.bus, "the window draws on the graphics adapter");
    state().why = None;
    app.show(Reach::SETTINGS);
    Task::none()
}

/// The GPU asked for didn't start, and iced's fallback has the processor
/// draw. Automatic with no integrated graphics Vulkan offers is the rule,
/// not a failure: no notice, nothing counted.
fn refused(app: &mut App) -> Task<Message> {
    let reason = iced_wgpu::adapter::last_error().unwrap_or_default();
    tracing::warn!(%reason, "the graphics adapter didn't start; the processor draws");
    let offered = (iced_wgpu::adapter::seen().iter()).any(|info| info.kind == Kind::Integrated);
    let mut state = state();
    if state.asked != Choice::Auto || offered {
        drop(state);
        return failed(app, Failure::Start);
    }
    iced_wgpu::enabled::set_enabled(false);
    state.target = None;
    state.why = Some(Why::NoIntegrated);
    let ahead = state.ahead.take();
    drop(state);
    if let Some(before) = ahead {
        app.layout.edit(|saved| saved.renderer_failures = before);
    }
    app.show(Reach::SETTINGS);
    Task::none()
}

/// The GPU failed: the processor draws from the next frame, the failure is
/// counted (unless the start counted it ahead) and the window says why.
fn failed(app: &mut App, failure: Failure) -> Task<Message> {
    iced_wgpu::enabled::set_enabled(false);
    FIRST_FRAME.store(false, Ordering::Relaxed);
    let ahead = state().ahead.take();
    let before = app.layout.renderer_failures;
    let failures = if ahead.is_some() {
        before
    } else {
        before.saturating_add(1)
    };
    if failures != before {
        app.layout.edit(|saved| saved.renderer_failures = failures);
    }
    let last = failures >= LIMIT;
    let mut state = state();
    // Always set while a GPU may draw; "GPU" reads in either language.
    let name = (state.target.take()).map_or_else(|| String::from("GPU"), |(_id, name)| name);
    let what = failure.text(&name);
    state.why = Some(Why::Failed {
        failure,
        name,
        last,
    });
    drop(state);
    tracing::warn!(?failure, failures, "the processor draws the window now");
    app.vm.sign_in.graphics_notice = Some(notice(&what, Some(next(last))));
    app.show(Reach::SETTINGS);
    match failure {
        // At a start, iced's fallback already took the processor.
        Failure::Lost(_) if app.window.is_some() => remake(Backend::Software),
        Failure::Start | Failure::Lost(_) => Task::none(),
    }
}
