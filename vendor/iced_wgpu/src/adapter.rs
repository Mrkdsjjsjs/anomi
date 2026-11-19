//! Anomi: the graphics adapter the window draws on (`PATCHES.md`, "The
//! adapter the app chooses").
//!
//! Stock, iced asks wgpu for any adapter by power preference, which on a
//! desktop with one integrated and one discrete adapter may take the card
//! the games use, and with only a discrete one always does. The owner's
//! rule: draw on the integrated adapter, so the discrete card stays with the
//! game, or on the CPU; the discrete card only when chosen by hand. So the
//! compositor enumerates Vulkan's adapters itself (Vulkan only: its
//! adapters carry their PCI bus id, which ties one to the card the system
//! lists, `/sys/class/drm/cardN/device`) and takes the one [`choose`] asks
//! for; with none, it refuses, and iced's fallback draws with tiny-skia. A
//! CPU adapter (lavapipe, llvmpipe) is never taken: it draws slower than
//! tiny-skia.
//!
//! How the frames reach the window is [`path`]'s: straight into a Vulkan
//! surface only on the adapter the desktop composites on, which the app
//! names ([`set_display_adapter`]); any other adapter's are read back and
//! shown through shared memory (`window/shared.rs`).
//!
//! What the last start saw, what draws now and why the last start failed are
//! kept here for the app's Settings page ([`seen`], [`active`],
//! [`last_error`]).

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

/// The adapter the app asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Choice {
    /// The first integrated adapter, or none: never a discrete one.
    #[default]
    Auto,
    /// The adapter at this PCI bus id, as Vulkan and sysfs write it
    /// (`"0000:73:00.0"`), or none. When no adapter's bus id matches, the one
    /// whose vendor and device ids match (`"1002:13c0"`, [`Info::ids`]): a
    /// driver without `VK_EXT_pci_bus_info` doesn't say its bus.
    Bus(String),
}

/// What kind of adapter it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Built into the processor, sharing the system's memory.
    Integrated,
    /// A card of its own, with its own memory.
    Discrete,
    /// Virtual, or one the driver doesn't say.
    Other,
}

/// What Vulkan says of an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// The driver's name for it ("AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)").
    pub name: String,
    /// Its PCI bus id (`"0000:73:00.0"`), empty when the driver doesn't say.
    pub bus: String,
    /// Its PCI vendor id (`0x1002`).
    pub vendor: u32,
    /// Its PCI device id (`0x13c0`).
    pub device: u32,
    /// Integrated, discrete or other.
    pub kind: Kind,
    /// The driver ("radv", "NVIDIA").
    pub driver: String,
    /// The driver's version, as it writes it.
    pub driver_info: String,
    /// Whether its frames go straight into the window's Vulkan surface
    /// (`true`) or are read back and shown through shared memory (`false`),
    /// as [`path`] decides: known for the adapters a compositor saw at its
    /// start ([`seen`]) and the one drawing ([`active`]), `None` from
    /// [`list`].
    pub direct: Option<bool>,
}

impl Info {
    fn new(info: &wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name.clone(),
            bus: info.device_pci_bus_id.clone(),
            vendor: info.vendor,
            device: info.device,
            kind: match info.device_type {
                wgpu::DeviceType::IntegratedGpu => Kind::Integrated,
                wgpu::DeviceType::DiscreteGpu => Kind::Discrete,
                wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Cpu | wgpu::DeviceType::Other => {
                    Kind::Other
                }
            },
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
            direct: None,
        }
    }

    /// Its vendor and device ids, four hex digits each: `"1002:13c0"`.
    pub fn ids(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor, self.device)
    }
}

/// Asks for an adapter. Read when a compositor or a headless renderer
/// starts: a running one keeps its adapter until the app makes another
/// (`iced::backend::configure`).
pub fn choose(choice: Choice) {
    state().choice = choice;
}

/// The adapter asked for, [`Choice::Auto`] until [`choose`].
pub fn choice() -> Choice {
    state().choice.clone()
}

/// Says which adapter the desktop composites on, by its PCI bus id
/// (`"0000:01:00.0"`), or that it isn't known (`None`, the default). Read
/// when a compositor starts, as [`choose`] is: the app finds it (the
/// Wayland compositor's own word, or the one adapter a monitor is plugged
/// into) and says it before the window opens.
pub fn set_display_adapter(bus: Option<String>) {
    state().display = bus;
}

/// The adapter the desktop composites on, as last said; `None` when not
/// known.
pub fn display_adapter() -> Option<String> {
    state().display.clone()
}

/// How a window shows what an adapter draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Into a Vulkan surface of the window, which hands the desktop's
    /// compositor the adapter's own buffers to import.
    Direct,
    /// Into a texture of the adapter's own, copied back to memory and shown
    /// through shared memory, as the processor's frames are: the compositor
    /// needs nothing from the adapter.
    Shared,
}

/// The way the frames of `info` reach the window, given the adapter the
/// desktop composites on (`display`, [`set_display_adapter`]): straight
/// only on that one. Any other adapter, or every one when that isn't
/// known, is read back. Vulkan's own answer can't decide this:
/// `Adapter::is_surface_supported` says whether the driver can speak to
/// the window system, not whether the compositor can import what it
/// makes, and a Wayland compositor that can't (mutter on an NVIDIA card,
/// a Radeon's buffers) ends the client's connection with a protocol error,
/// which nothing in the process survives.
pub fn path(info: &Info, display: Option<&str>) -> Path {
    match display {
        Some(display)
            if !info.bus.is_empty() && info.bus.trim().eq_ignore_ascii_case(display.trim()) =>
        {
            Path::Direct
        }
        _ => Path::Shared,
    }
}

/// Vulkan's adapters, enumerated now, without a window, CPU adapters left
/// out. It blocks while the Vulkan loader loads every installed driver into
/// this process, and their libraries stay mapped after it returns: the app
/// lists adapters in a child process (`anomi --list-adapters`); the window's
/// own process reads [`seen`].
pub fn list() -> Vec<Info> {
    let instance = instance_without_window();

    ready(candidates(&instance, wgpu::Backends::VULKAN))
        .unwrap_or_default()
        .into_iter()
        .map(|(_adapter, info)| info)
        .collect()
}

/// The adapters the last compositor start enumerated, with the way each
/// would show its frames ([`Info::direct`]); empty until one started.
pub fn seen() -> Vec<Info> {
    state().seen.clone()
}

/// The adapter the window draws on now; `None` while tiny-skia draws, while
/// no window is open (in the tray iced drops the compositor, and with it the
/// device) and once the device was lost.
pub fn active() -> Option<Info> {
    state()
        .active
        .as_ref()
        .map(|(_generation, info)| info.clone())
}

/// Why the last start failed or the device was lost, in English for the
/// log; cleared when a compositor starts.
pub fn last_error() -> Option<String> {
    state().last_error.clone()
}

struct State {
    choice: Choice,
    /// The PCI bus id of the adapter the desktop composites on.
    display: Option<String>,
    seen: Vec<Info>,
    /// The adapter drawing, with the generation of the compositor drawing
    /// on it: `iced::backend::configure` makes the next compositor before it
    /// drops the one before, whose drop must not clear the new one's.
    active: Option<(u64, Info)>,
    last_error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    choice: Choice::Auto,
    display: None,
    seen: Vec::new(),
    active: None,
    last_error: None,
});

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A number no other compositor of this process has; 0 is none (a headless
/// renderer).
pub(crate) fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn record_seen(seen: Vec<Info>) {
    state().seen = seen;
}

pub(crate) fn started(generation: u64, info: Info) {
    let mut state = state();

    state.active = Some((generation, info));
    state.last_error = None;
}

/// The compositor of `generation` stopped drawing (dropped, or its device
/// lost).
pub(crate) fn stopped(generation: u64) {
    let mut state = state();

    if state
        .active
        .as_ref()
        .is_some_and(|(active, _info)| *active == generation)
    {
        state.active = None;
    }
}

pub(crate) fn failed(reason: String) {
    state().last_error = Some(reason);
}

/// An instance for Vulkan alone, without a window: for [`list`] and the
/// headless renderer.
pub(crate) fn instance_without_window() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// The adapters of `instance` this renderer may draw on, each with what's
/// known of it: every one but a CPU's. No window's surface is asked: one is
/// made only for the adapter [`path`] lets draw into it.
pub(crate) async fn candidates(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
) -> Vec<(wgpu::Adapter, Info)> {
    instance
        .enumerate_adapters(backends)
        .await
        .into_iter()
        .filter_map(|adapter| {
            let info = adapter.get_info();

            if matches!(info.device_type, wgpu::DeviceType::Cpu) {
                return None;
            }

            let info = Info::new(&info);

            Some((adapter, info))
        })
        .collect()
}

/// Which of `candidates` `choice` takes, or why none. Any adapter can draw
/// for a window: [`path`] decides how its frames reach it.
pub(crate) fn pick(candidates: &[Info], choice: &Choice) -> Result<usize, String> {
    match choice {
        Choice::Auto => candidates
            .iter()
            .position(|info| info.kind == Kind::Integrated)
            .ok_or_else(|| String::from("Vulkan offers no integrated graphics adapter")),
        Choice::Bus(wanted) => {
            let wanted = wanted.trim().to_ascii_lowercase();

            let index = candidates
                .iter()
                .position(|info| !info.bus.is_empty() && info.bus.eq_ignore_ascii_case(&wanted))
                .or_else(|| candidates.iter().position(|info| info.ids() == wanted));

            index.ok_or_else(|| format!("Vulkan offers no adapter at {wanted}"))
        }
    }
}

/// The output of a future that is ready when first polled, as wgpu's
/// native futures are (enumerating adapters, requesting an adapter or a
/// device, popping an error scope); `None` if it wasn't.
pub(crate) fn ready<T>(future: impl Future<Output = T>) -> Option<T> {
    let mut future = pin!(future);

    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, bus: &str, kind: Kind) -> Info {
        Info {
            name: name.to_owned(),
            bus: bus.to_owned(),
            vendor: if name.starts_with("NVIDIA") {
                0x10de
            } else {
                0x1002
            },
            device: if name.starts_with("NVIDIA") {
                0x2216
            } else {
                0x13c0
            },
            kind,
            driver: String::new(),
            driver_info: String::new(),
            direct: None,
        }
    }

    /// The owner's desktop with `vulkan-radeon` installed: the 3080 first,
    /// as NVIDIA's driver lists it, then the Radeon. The monitor, and
    /// mutter with it, are on the 3080.
    fn desktop() -> Vec<Info> {
        vec![
            adapter("NVIDIA GeForce RTX 3080", "0000:01:00.0", Kind::Discrete),
            adapter(
                "AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)",
                "0000:73:00.0",
                Kind::Integrated,
            ),
        ]
    }

    const DISPLAY: Option<&str> = Some("0000:01:00.0");

    #[test]
    fn automatic_takes_the_integrated_adapter() {
        assert_eq!(pick(&desktop(), &Choice::Auto), Ok(1));
    }

    #[test]
    fn automatic_never_takes_a_discrete_adapter() {
        let only_nvidia = vec![desktop().remove(0)];

        assert!(pick(&only_nvidia, &Choice::Auto).is_err());
        assert!(pick(&[], &Choice::Auto).is_err());
    }

    #[test]
    fn on_this_desktop_the_radeon_is_read_back_and_shown_through_shared_memory() {
        let adapters = desktop();

        assert_eq!(
            pick(&adapters, &Choice::Auto),
            Ok(1),
            "Automatic: the Radeon"
        );
        assert_eq!(path(&adapters[1], DISPLAY), Path::Shared);
    }

    #[test]
    fn only_the_adapter_the_desktop_composites_on_draws_into_the_window() {
        let adapters = desktop();

        assert_eq!(path(&adapters[0], DISPLAY), Path::Direct);
        assert_eq!(path(&adapters[0], Some(" 0000:01:00.0 ")), Path::Direct);
        assert_eq!(
            path(&adapters[1], Some("0000:73:00.0")),
            Path::Direct,
            "a laptop's integrated graphics, which composite its desktop"
        );
    }

    #[test]
    fn where_the_desktops_adapter_isnt_known_every_adapter_is_read_back() {
        let mut adapters = desktop();

        assert!(adapters.iter().all(|info| path(info, None) == Path::Shared));

        // A driver that doesn't say its bus is never taken for the desktop's.
        adapters[0].bus.clear();

        assert_eq!(path(&adapters[0], Some("")), Path::Shared);
        assert_eq!(path(&adapters[0], DISPLAY), Path::Shared);
    }

    #[test]
    fn a_bus_id_takes_that_adapter() {
        let adapters = desktop();

        assert_eq!(pick(&adapters, &Choice::Bus("0000:01:00.0".into())), Ok(0));
        assert_eq!(
            pick(&adapters, &Choice::Bus(" 0000:73:00.0 ".into())),
            Ok(1)
        );
        assert_eq!(
            pick(&adapters, &Choice::Bus("0000:73:00.0".to_uppercase())),
            Ok(1)
        );
        assert!(pick(&adapters, &Choice::Bus("0000:02:00.0".into())).is_err());
    }

    #[test]
    fn vendor_and_device_stand_in_for_a_bus_id() {
        let mut adapters = desktop();

        adapters[1].bus.clear();

        assert_eq!(adapters[1].ids(), "1002:13c0");
        assert_eq!(pick(&adapters, &Choice::Bus("1002:13C0".into())), Ok(1));
        assert!(pick(&adapters, &Choice::Bus("0000:73:00.0".into())).is_err());
    }

    #[test]
    fn only_the_compositor_of_the_active_adapter_clears_it() {
        let radeon = desktop().remove(1);
        let first = next_generation();
        let second = next_generation();

        started(first, radeon.clone());
        started(second, radeon.clone());
        stopped(first);

        assert_eq!(active(), Some(radeon));

        stopped(second);

        assert_eq!(active(), None);
    }

    /// Opt-in, as it loads the system's Vulkan drivers:
    /// `ANOMI_GPU_TESTS=1 cargo test -- --nocapture adapters_are_listed`.
    #[test]
    fn adapters_are_listed() {
        if std::env::var_os("ANOMI_GPU_TESTS").is_none() {
            return;
        }

        let adapters = list();

        for adapter in &adapters {
            println!("{adapter:?} ({})", adapter.ids());

            assert!(!adapter.name.is_empty());
            assert_eq!(adapter.direct, None);
        }
    }
}
