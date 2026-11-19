//! Anomi: whether this renderer may start at all (`PATCHES.md`, "Started
//! only when the app says so").
//!
//! Anomi's build has both of iced's renderers, and iced's fallback tries
//! this one first wherever a renderer is made: the window's compositor, and
//! every headless renderer (`iced_test`'s simulator, `anomi-shots`, the
//! tests' renderers). Until the app turns it on, the compositor answers
//! with iced's ordinary "this backend didn't match" and the headless
//! renderer with `None`, so the fallback takes tiny-skia: a start on the
//! CPU, the tests and the shots never load a GPU driver, and the shots stay
//! byte for byte those of tiny-skia.
//!
//! Upstream Yapfast's `set_enabled` (their `vendor/iced_wgpu`), the same
//! switch.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Lets this renderer start (`true`) or keeps it from starting (`false`,
/// the default). It's read when a compositor or a headless renderer is
/// made, so a running one isn't touched: the app turns it on before the
/// window opens, or before `iced::backend::configure`.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether the app let this renderer start.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The switch is global: the tests that turn it or count on it hold this.
#[cfg(test)]
pub(crate) static SWITCH: std::sync::Mutex<()> = std::sync::Mutex::new(());
