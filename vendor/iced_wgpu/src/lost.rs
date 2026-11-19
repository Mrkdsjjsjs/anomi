//! Anomi: what the renderer does when the GPU fails under it (`PATCHES.md`,
//! "A failing GPU reported, never a panic").
//!
//! Stock, wgpu's default handler for an error nobody catches panics, so a
//! validation error ended the whole app, a call included, and a lost device
//! (a driver reset or crash, the GPU gone, its driver updated under the app,
//! a suspend that didn't keep video memory) froze the window: iced made its
//! surface again once a second, forever. Here every device this renderer
//! makes has an error handler that logs and flags, and a device-lost
//! callback; the first failure of the window's device calls the hook the
//! app sets ([`set_hook`]), once, and asks the window for a redraw, so the
//! app can switch to tiny-skia (`iced::backend::configure` with
//! `Backend::Software`). While the device is lost the compositor draws
//! nothing and asks nothing of it.

use crate::graphics::Shell;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Why the window's renderer failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The device was lost: nothing more can be drawn with it.
    DeviceLost,
    /// wgpu caught an error in what was asked of it (validation, an
    /// internal error, out of memory); drawing goes on, maybe wrong.
    Error,
    /// The window's surface couldn't be made or set up.
    Surface,
}

/// A failure of the window's renderer, as the hook is told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lost {
    /// What failed.
    pub cause: Cause,
    /// What wgpu or the driver said, in English, for the log.
    pub message: String,
}

type Hook = Arc<dyn Fn(&Lost) + Send + Sync>;

static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

/// Sets what is called when the window's renderer fails: once for each
/// compositor, on the first failure. It may be called on any thread, from
/// inside a wgpu call (a submission, a present), with wgpu's locks held: it
/// must only pass the news on (a message to the app) and return, never
/// call wgpu. A second call replaces the hook.
pub fn set_hook(hook: impl Fn(&Lost) + Send + Sync + 'static) {
    *HOOK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
}

fn call_hook(lost: &Lost) {
    let hook = HOOK.lock().unwrap_or_else(PoisonError::into_inner).clone();

    if let Some(hook) = hook {
        hook(lost);
    }
}

/// How many of a device's errors are written to the log: a broken frame
/// may repeat each frame until the app switches.
const LOGGED: u32 = 8;

/// One device's health, shared by its error handler, its device-lost
/// callback and the renderer drawing with it.
pub(crate) struct Watch {
    shell: Shell,
    /// The compositor's generation once it serves a window
    /// (`adapter::next_generation`); 0 for a headless renderer, whose
    /// failures are logged and not reported.
    generation: AtomicU64,
    lost: AtomicBool,
    reported: AtomicBool,
    errors: AtomicU32,
}

impl Watch {
    /// Watches `device`: its uncaptured errors and its loss are handled
    /// here from now on, never by wgpu's panicking default.
    pub(crate) fn new(device: &wgpu::Device, shell: Shell) -> Arc<Self> {
        let watch = Arc::new(Self {
            shell,
            generation: AtomicU64::new(0),
            lost: AtomicBool::new(false),
            reported: AtomicBool::new(false),
            errors: AtomicU32::new(0),
        });

        // The device keeps these two, and they keep the watch (never the
        // device), so there's no cycle: all go with the device.
        let handler: Arc<dyn wgpu::UncapturedErrorHandler> = {
            let watch = Arc::clone(&watch);

            Arc::new(move |error: wgpu::Error| watch.error(&error))
        };

        device.on_uncaptured_error(handler);

        let on_lost = {
            let watch = Arc::clone(&watch);

            move |reason: wgpu::DeviceLostReason, message: String| {
                watch.device_lost(reason, message);
            }
        };

        device.set_device_lost_callback(on_lost);

        watch
    }

    /// The window's compositor of `generation` draws with this device: its
    /// failures go to the app's hook from now on.
    pub(crate) fn serve(&self, generation: u64) {
        self.generation.store(generation, Ordering::Relaxed);
    }

    /// Whether the device was lost: nothing more is drawn with it.
    pub(crate) fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    fn error(&self, error: &wgpu::Error) {
        let count = self.errors.fetch_add(1, Ordering::Relaxed);

        if count < LOGGED {
            log::error!("wgpu error, caught: {error}");
        } else if count == LOGGED {
            log::error!("wgpu keeps failing; its further errors aren't logged");
        }

        self.report(Cause::Error, || error.to_string());
    }

    fn device_lost(&self, reason: wgpu::DeviceLostReason, message: String) {
        // `Destroyed` is our own `Device::destroy`, which nothing here
        // calls; dropping a device calls nothing.
        if matches!(reason, wgpu::DeviceLostReason::Destroyed) {
            return;
        }

        self.lost.store(true, Ordering::Release);

        log::error!("the GPU device was lost: {message}");

        let generation = self.generation.load(Ordering::Relaxed);

        if generation != 0 {
            crate::adapter::stopped(generation);
        }

        self.report(Cause::DeviceLost, || {
            format!("the GPU device was lost: {message}")
        });
    }

    /// The window's surface failed (`window::compositor`).
    pub(crate) fn surface(&self, message: String) {
        log::error!("{message}");

        self.report(Cause::Surface, move || message);
    }

    /// Tells the app of a failure of the window's device: its first, and a
    /// loss after an error, the worse news, which `adapter::last_error`
    /// keeps. The message is made only then: an error may repeat each
    /// frame.
    fn report(&self, cause: Cause, message: impl FnOnce() -> String) {
        let generation = self.generation.load(Ordering::Relaxed);

        if generation == 0 {
            return;
        }

        let first = !self.reported.swap(true, Ordering::AcqRel);

        if !first && !matches!(cause, Cause::DeviceLost) {
            return;
        }

        let message = message();

        crate::adapter::failed(message.clone());

        if !first {
            return;
        }

        call_hook(&Lost { cause, message });

        // Wakes the window's loop, so the app hears of it now even with
        // nothing else happening.
        self.shell.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_is_replaced_and_called_with_the_failure() {
        use std::sync::atomic::AtomicUsize;

        static CALLS: AtomicUsize = AtomicUsize::new(0);

        set_hook(|_lost| panic!("replaced"));
        set_hook(|lost| {
            assert_eq!(lost.cause, Cause::Surface);

            let _ = CALLS.fetch_add(1, Ordering::Relaxed);
        });

        call_hook(&Lost {
            cause: Cause::Surface,
            message: String::from("test"),
        });

        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    }
}
