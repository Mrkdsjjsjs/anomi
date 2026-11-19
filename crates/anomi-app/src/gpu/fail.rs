//! The window's renderer as it starts, switches and fails. A GPU that
//! doesn't start (iced's fallback has the processor draw then), or that
//! fails under the window (a lost device, an error wgpu caught, a surface
//! it couldn't make: `iced_wgpu::lost`, whose hook a renderer's thread
//! calls), gives way to the processor from the next frame. The window says
//! why, `renderer_failures` counts it, and the next start tries again;
//! after [`LIMIT`] in a row the processor draws until a GPU is chosen again,
//! which forgets them, as does a window that drew on one for [`SETTLE`].
//! A death before then, which no code of the app's survives to count, is
//! known at the next start by the adapter written ahead (`super::opening`),
//! and that one is enough.

use std::sync::atomic::Ordering;
use std::time::Duration;

use iced::Task;
use iced::backend::Backend;
use iced_wgpu::adapter::{Info, Kind};
use iced_wgpu::lost::{Cause, Lost};
use yapfast_lang::tr;

use super::plan::Why;
use super::switch::remake;
use super::{Choice, Due, FAILED, FIRST_FRAME, live, state};
use crate::app::App;
use crate::clocks::later;
use crate::message::Message;
use crate::project::Reach;

/// Failures in a row after which the processor draws until a GPU is chosen
/// again.
pub(super) const LIMIT: u8 = 3;

/// How long a window draws on the GPU from its first frame, or on the
/// processor after giving the GPU up, before the try is settled: the
/// adapter written ahead is cleared, and the failures forgotten if the GPU
/// still draws. Not at the first frame: a compositor that can't import the
/// frames ends a Wayland connection after it (the owner's, 150 ms after
/// the window opened), and an app that cleared it then would try again at
/// every start.
pub(super) const SETTLE: Duration = Duration::from_secs(5);

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
    said(&info);
    // A try written ahead, or failures counted, wait for a frame drawn on it.
    let unsettled = state().trying || app.layout.renderer_failures != 0;
    FIRST_FRAME.store(unsettled, Ordering::Relaxed);
    Task::none()
}

/// Says in the log what draws the window and how its frames reach it.
fn said(info: &Info) {
    let how = if info.direct == Some(true) {
        "presented to the window directly"
    } else {
        "shown through shared memory"
    };
    tracing::info!(bus = %info.bus, driver = %info.driver, "the window draws on {}, {how}", info.name);
}

/// A frame was drawn on the GPU since the window opened: unless it fails,
/// or the app dies, within [`SETTLE`], the try is settled then.
pub(super) fn drew() -> Task<Message> {
    let first = FIRST_FRAME.swap(false, Ordering::AcqRel);
    let failed = FAILED.load(Ordering::Acquire);
    if !first || failed || iced_wgpu::adapter::active().is_none() {
        return Task::none();
    }
    tracing::debug!(settle = ?SETTLE, "a first frame drawn on the graphics adapter");
    settle()
}

/// Settles the try in progress after [`SETTLE`]: one timer, which fires
/// once.
fn settle() -> Task<Message> {
    let epoch = state().epoch;
    later(SETTLE, Message::Gpu(Due::Settled(epoch)))
}

/// [`SETTLE`] after a first frame on the GPU, or after giving it up: the
/// app didn't die on the adapter tried, which is cleared; if the GPU still
/// draws, its failures in a row are over too. A settling of an earlier try
/// (`epoch`) is ignored.
pub(super) fn settled(app: &mut App, epoch: u32) -> Task<Message> {
    let drawing = !FAILED.load(Ordering::Acquire) && iced_wgpu::adapter::active().is_some();
    let mut state = state();
    if state.epoch != epoch {
        return Task::none();
    }
    state.trying = false;
    state.settled |= drawing;
    drop(state);
    let failures = if drawing {
        0
    } else {
        app.layout.renderer_failures
    };
    if app.layout.renderer_trying.is_some() || app.layout.renderer_failures != failures {
        app.layout.edit(|saved| {
            saved.renderer_trying = None;
            saved.renderer_failures = failures;
        });
    }
    tracing::debug!(drawing, "the graphics adapter's try is settled");
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
    said(&info);
    let mut state = state();
    state.why = None;
    // A switch writes its adapter ahead: a frame drawn on it settles it.
    FIRST_FRAME.store(state.trying, Ordering::Relaxed);
    drop(state);
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
    state.trying = false;
    drop(state);
    // Nothing was drawn on: no adapter stays written ahead.
    if app.layout.renderer_trying.is_some() {
        app.layout.edit(|saved| saved.renderer_trying = None);
    }
    app.show(Reach::SETTINGS);
    Task::none()
}

/// The GPU failed: the processor draws from the next frame, the failure is
/// counted and the window says why.
fn failed(app: &mut App, failure: Failure) -> Task<Message> {
    iced_wgpu::enabled::set_enabled(false);
    FIRST_FRAME.store(false, Ordering::Relaxed);
    let failures = app.layout.renderer_failures.saturating_add(1);
    app.layout.edit(|saved| saved.renderer_failures = failures);
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
    // The adapter written ahead stays until the processor has drawn a
    // while: a death meanwhile still counts (`plan::died`).
    let settle = settle();
    match failure {
        // At a start, iced's fallback already took the processor.
        Failure::Lost(_) if app.window.is_some() => {
            Task::batch([remake(Backend::Software), settle])
        }
        Failure::Start | Failure::Lost(_) => settle,
    }
}
