//! Which graphics adapter the desktop composites on, by its PCI address:
//! the one whose buffers the window system imports as its own. Only that
//! adapter draws straight into the window's Vulkan surface; any other's
//! frames are read back and shown through shared memory
//! (`iced_wgpu::adapter::Path`, `vendor/iced_wgpu/PATCHES.md`, "Drawn on one
//! adapter, shown through another"). Taken wrongly for it, an adapter hands
//! the compositor buffers it may not import, and a Wayland compositor then
//! ends the connection with a protocol error, which nothing in the process
//! survives: so where it isn't sure, it says nothing, and every adapter's
//! frames are read back.
//!
//! - On Wayland, the compositor's own word: the main device of linux-dmabuf's
//!   default feedback (versions 4 and 5: mutter 42, KWin 5.27, wlroots 0.16,
//!   Weston 11 and later), the device Mesa's Vulkan drivers check their
//!   buffers against. Asked on a connection of its own, one round trip,
//!   before the window opens.
//! - Without it (X11, an older compositor, one that draws on the
//!   processor): the computer's only adapter, else the only one a monitor
//!   is plugged into (sysfs, `yapfast_core::adapters`). On X11 a wrong guess
//!   costs no connection: Mesa's drivers check the X server's device
//!   themselves (DRI3), and an X error isn't fatal.

use yapfast_core::adapters::Adapter;

/// The PCI address of the adapter the desktop composites on, or `None`
/// where that isn't known. On Wayland it asks the compositor, about a
/// millisecond.
pub(super) fn adapter(cards: &[Adapter]) -> Option<String> {
    let feedback = wayland::main_device();
    let found = decide(feedback.as_deref(), cards);
    tracing::info!(
        compositor = ?feedback,
        adapter = ?found,
        "the adapter the desktop composites on"
    );
    found
}

/// From the compositor's word where it gave one, else from the system's
/// adapters: the only one, else the only one driving a monitor.
pub(super) fn decide(feedback: Option<&str>, cards: &[Adapter]) -> Option<String> {
    if let Some(address) = feedback {
        return Some(address.to_ascii_lowercase());
    }
    only(cards.iter()).or_else(|| only(cards.iter().filter(|card| card.drives_display)))
}

/// The address of the one adapter of `cards`; none for none or several.
fn only<'a>(mut cards: impl Iterator<Item = &'a Adapter>) -> Option<String> {
    match (cards.next(), cards.next()) {
        (Some(card), None) => card.pci_address().map(str::to_ascii_lowercase),
        _ => None,
    }
}

/// A `dev_t` as linux-dmabuf sends it: the machine's own, 8 bytes (4 with
/// a 32-bit libc).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn device_number(bytes: &[u8]) -> Option<u64> {
    match bytes.len() {
        8 => Some(u64::from_ne_bytes(bytes.try_into().ok()?)),
        4 => Some(u64::from(u32::from_ne_bytes(bytes.try_into().ok()?))),
        _ => None,
    }
}

/// Whether `text` is a PCI address as sysfs writes one: `0000:01:00.0`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn is_pci_address(text: &str) -> bool {
    let bytes = text.as_bytes();
    let hex = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
    };
    bytes.len() == 12
        && (bytes[4], bytes[7], bytes[10]) == (b':', b':', b'.')
        && hex(0..4)
        && hex(5..7)
        && hex(8..10)
        && hex(11..12)
}

#[cfg(target_os = "linux")]
mod wayland {
    use std::path::Path;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
    use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_feedback_v1::{
        self, ZwpLinuxDmabufFeedbackV1,
    };
    use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

    /// Round trips waited for the feedback: it comes in the first, as the
    /// compositor sends it when the object is made.
    const ROUND_TRIPS: usize = 2;

    #[derive(Default)]
    struct Feedback {
        /// The main device's `dev_t`, in the machine's byte order.
        main: Option<Vec<u8>>,
        done: bool,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Feedback {
        fn event(
            _state: &mut Self,
            _registry: &WlRegistry,
            _event: wl_registry::Event,
            _data: &GlobalListContents,
            _connection: &Connection,
            _handle: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for Feedback {
        fn event(
            state: &mut Self,
            _feedback: &ZwpLinuxDmabufFeedbackV1,
            event: zwp_linux_dmabuf_feedback_v1::Event,
            _data: &(),
            _connection: &Connection,
            _handle: &QueueHandle<Self>,
        ) {
            match event {
                zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } => {
                    state.main = Some(device);
                }
                zwp_linux_dmabuf_feedback_v1::Event::Done => state.done = true,
                // The format table's file closes as the event goes.
                _ => {}
            }
        }
    }

    delegate_noop!(Feedback: ignore ZwpLinuxDmabufV1);

    /// The PCI address of the device the Wayland compositor names its main
    /// one; none off Wayland, without the feedback, or for a device that
    /// isn't a PCI adapter.
    pub(super) fn main_device() -> Option<String> {
        // A socket handed down is good for one connection, winit's: taking
        // it here would leave the window none.
        if std::env::var_os("WAYLAND_SOCKET").is_some()
            || std::env::var_os("WAYLAND_DISPLAY").is_none()
        {
            return None;
        }
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<Feedback>(&connection).ok()?;
        let handle = queue.handle();
        // Version 6 no longer names a main device.
        let dmabuf: ZwpLinuxDmabufV1 = globals.bind(&handle, 4..=5, ()).ok()?;
        let _feedback = dmabuf.get_default_feedback(&handle, ());
        let mut feedback = Feedback::default();
        for _ in 0..ROUND_TRIPS {
            if feedback.done {
                break;
            }
            queue.roundtrip(&mut feedback).ok()?;
        }
        // The connection closes as it goes, and the compositor drops its
        // objects with it.
        let device = super::device_number(&feedback.main?)?;
        address_of(device, Path::new("/dev/dri"), Path::new("/sys/class/drm"))
    }

    /// The PCI address of the DRM node whose device number is `device`: the
    /// node in `nodes` (`/dev/dri`) with that number, and the PCI device
    /// sysfs puts it under (`drm`, `/sys/class/drm`). Primary and render
    /// nodes of one adapter give the same address, as the protocol warns a
    /// compositor may name either.
    fn address_of(device: u64, nodes: &Path, drm: &Path) -> Option<String> {
        use std::os::unix::fs::MetadataExt as _;
        let node = std::fs::read_dir(nodes).ok()?.flatten().find(|entry| {
            std::fs::metadata(entry.path()).is_ok_and(|meta| meta.rdev() == device)
        })?;
        let parent = std::fs::canonicalize(drm.join(node.file_name()).join("device")).ok()?;
        let address = parent.file_name()?.to_str()?.to_ascii_lowercase();
        super::is_pci_address(&address).then_some(address)
    }
}

#[cfg(not(target_os = "linux"))]
mod wayland {
    pub(super) fn main_device() -> Option<String> {
        None
    }
}
