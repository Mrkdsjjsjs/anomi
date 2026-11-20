//! iced's settings for the window's renderer, and switching it while the
//! window is open: `iced::backend::configure` makes the renderer again on
//! the window and swaps it in, no restart and no call dropped; if it can't
//! make one, what drew before goes on.

use std::sync::atomic::Ordering;

use iced::Task;
use iced::backend::{self, Backend, PowerPreference};
use iced_wgpu::adapter::{Choice as Wanted, Info, Kind};

use super::plan::{self, Why};
use super::{Choice, Due, FAILED, FIRST_FRAME, drivers, live, mark, screen, state};
use crate::app::App;
use crate::message::Message;

/// iced's settings for the window: the GPU if enabled, else the processor
/// (`Backend::Best`, which the vendored wgpu refuses until enabled), the
/// display's pace (FIFO: frames only when asked, none faster than shown),
/// and paths antialiased (MSAA x4, on the GPU only).
pub fn settings() -> iced::Settings {
    let backend = backend_settings(Backend::Best);
    iced::Settings {
        backend: backend.backend,
        power_preference: backend.power_preference,
        antialiasing: backend.antialiasing,
        vsync: backend.vsync,
        ..iced::Settings::default()
    }
}

fn backend_settings(backend: Backend) -> backend::Settings {
    backend::Settings {
        backend,
        power_preference: PowerPreference::LowPower,
        antialiasing: true,
        vsync: true,
    }
}

/// Has iced make the window's renderer again on `backend`: the window's
/// watchdog judges nothing before its first frame.
pub(super) fn remake(backend: Backend) -> Task<Message> {
    crate::window::seen::anew();
    backend::configure(backend_settings(backend)).map(|made: Result<(), backend::Error>| {
        Message::Gpu(Due::Switched(made.err().map(|error| error.to_string())))
    })
}

/// Keeps `choice` and draws with it at once, failures and a start that
/// died forgotten. A driver the start didn't let load waits for the next
/// start; without a window (in the tray) the next window takes it.
pub fn choose(app: &mut App, choice: Choice) -> Task<Message> {
    let saved = choice.saved();
    app.layout.edit(|layout| {
        layout.interface_renderer = saved;
        layout.renderer_failures = 0;
        layout.renderer_trying = None;
    });
    if !live() {
        return Task::none();
    }
    let cards = yapfast_core::adapters::list();
    let mut plan = plan::decide(&choice, &cards, drivers::installed, 0, true);
    if let Some(target) = plan.target.take_if(|_| !drivers::loadable(&plan.drivers)) {
        plan.why = Some(Why::NextStart(target.name));
    }
    FAILED.store(false, Ordering::Release);
    FIRST_FRAME.store(false, Ordering::Relaxed);
    // Asked of the compositor only for a GPU: a round trip on Wayland.
    let display = plan.target.as_ref().and_then(|_| screen::adapter(&cards));
    let drawing = iced_wgpu::adapter::active();
    let mut state = state();
    state.cards = cards;
    state.asked = plan.choice;
    state.trying = false;
    state.settled = false;
    let Some(target) = plan.target else {
        iced_wgpu::enabled::set_enabled(false);
        state.target = None;
        // A driver that drew is unloaded with the device: no restart
        // gives anything more back.
        state.why = plan.why;
        drop(state);
        tracing::info!("the window is to draw on the processor");
        let gpu = drawing.is_some() && app.window.is_some();
        return if gpu {
            remake(Backend::Software)
        } else {
            Task::none()
        };
    };
    state.why = None;
    state.target = Some((target.id.clone(), target.name.clone()));
    drop(state);
    let same = drawing.is_some_and(|info| meets(&info, &target.wanted));
    iced_wgpu::adapter::choose(target.wanted);
    iced_wgpu::adapter::set_display_adapter(display);
    iced_wgpu::enabled::set_enabled(true);
    if same || app.window.is_none() {
        return Task::none();
    }
    tracing::info!(adapter = %target.name, "the window is to draw on the graphics adapter");
    // On disk before the new renderer is made, as a window opening is.
    mark(&mut app.layout, &target.id);
    remake(Backend::Best)
}

/// Whether the adapter drawing is the one asked for.
fn meets(info: &Info, wanted: &Wanted) -> bool {
    match wanted {
        Wanted::Auto => info.kind == Kind::Integrated,
        Wanted::Bus(bus) => info.bus.eq_ignore_ascii_case(bus) || info.ids() == *bus,
    }
}
