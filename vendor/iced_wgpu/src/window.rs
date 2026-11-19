//! Display rendering results on windows.
pub mod compositor;

// Anomi: frames drawn on an adapter the desktop doesn't composite on, read
// back and shown through shared memory (PATCHES.md).
mod shared;

pub use compositor::Compositor;
// Anomi: the compositor's own surface, which a failure leaves empty rather
// than panicking (PATCHES.md); stock re-exported `wgpu::Surface`.
pub use compositor::Surface;
