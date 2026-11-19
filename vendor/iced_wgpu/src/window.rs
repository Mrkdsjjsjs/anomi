//! Display rendering results on windows.
pub mod compositor;

pub use compositor::Compositor;
// Anomi: the compositor's own surface, which a failure leaves empty rather
// than panicking (PATCHES.md); stock re-exported `wgpu::Surface`.
pub use compositor::Surface;
