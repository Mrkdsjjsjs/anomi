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
//! for, if it can present to the window; otherwise it refuses, as when no
//! adapter exists, and iced's fallback draws with tiny-skia. A CPU adapter
//! (lavapipe, llvmpipe) is never taken: it draws slower than tiny-skia.
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
    /// The first integrated adapter that can present to the window, or
    /// none: never a discrete one.
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
    /// Whether it can present to the window: known for the adapters a
    /// compositor saw at its start ([`seen`]), `None` from [`list`].
    pub presents: Option<bool>,
}

impl Info {
    fn new(info: &wgpu::AdapterInfo, presents: Option<bool>) -> Self {
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
            presents,
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

/// Vulkan's adapters, enumerated now, without a window, CPU adapters left
/// out. It blocks while the Vulkan loader loads every installed driver into
/// this process, and their libraries stay mapped after it returns: the app
/// lists adapters in a child process (`anomi --list-adapters`); the window's
/// own process reads [`seen`].
pub fn list() -> Vec<Info> {
    let instance = instance_without_window();

    ready(candidates(&instance, wgpu::Backends::VULKAN, None))
        .unwrap_or_default()
        .into_iter()
        .map(|(_adapter, info)| info)
        .collect()
}

/// The adapters the last compositor start enumerated, with whether each can
/// present to the window; empty until one started.
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
    seen: Vec<Info>,
    /// The adapter drawing, with the generation of the compositor drawing
    /// on it: `iced::backend::configure` makes the next compositor before it
    /// drops the one before, whose drop must not clear the new one's.
    active: Option<(u64, Info)>,
    last_error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    choice: Choice::Auto,
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
/// known of it: every one but a CPU's, with whether it can present to
/// `surface` when there is one.
pub(crate) async fn candidates(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
    surface: Option<&wgpu::Surface<'_>>,
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

            let presents = surface.map(|surface| adapter.is_surface_supported(surface));
            let info = Info::new(&info, presents);

            Some((adapter, info))
        })
        .collect()
}

/// Which of `candidates` `choice` takes, or why none. For a window
/// (`window`) the adapter must be able to present to it.
pub(crate) fn pick(candidates: &[Info], choice: &Choice, window: bool) -> Result<usize, String> {
    let presents = |info: &Info| !window || info.presents == Some(true);

    match choice {
        Choice::Auto => {
            let mut integrated = candidates
                .iter()
                .enumerate()
                .filter(|(_index, info)| info.kind == Kind::Integrated)
                .peekable();

            let Some((_index, first)) = integrated.peek() else {
                return Err(String::from("Vulkan offers no integrated graphics adapter"));
            };

            let name = first.name.clone();

            integrated
                .find(|&(_index, info)| presents(info))
                .map(|(index, _info)| index)
                .ok_or_else(|| format!("the integrated adapter {name} can't present to the window"))
        }
        Choice::Bus(wanted) => {
            let wanted = wanted.trim().to_ascii_lowercase();

            let index = candidates
                .iter()
                .position(|info| !info.bus.is_empty() && info.bus.eq_ignore_ascii_case(&wanted))
                .or_else(|| candidates.iter().position(|info| info.ids() == wanted));

            let Some(index) = index else {
                return Err(format!("Vulkan offers no adapter at {wanted}"));
            };

            let info = &candidates[index];

            if presents(info) {
                Ok(index)
            } else {
                Err(format!(
                    "{} ({wanted}) can't present to the window",
                    info.name
                ))
            }
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

    fn adapter(name: &str, bus: &str, kind: Kind, presents: Option<bool>) -> Info {
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
            presents,
        }
    }

    /// The owner's desktop with `vulkan-radeon` installed: the 3080 first,
    /// as NVIDIA's driver lists it, then the Radeon.
    fn desktop(radeon_presents: bool) -> Vec<Info> {
        vec![
            adapter(
                "NVIDIA GeForce RTX 3080",
                "0000:01:00.0",
                Kind::Discrete,
                Some(true),
            ),
            adapter(
                "AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)",
                "0000:73:00.0",
                Kind::Integrated,
                Some(radeon_presents),
            ),
        ]
    }

    #[test]
    fn automatic_takes_the_integrated_adapter() {
        assert_eq!(pick(&desktop(true), &Choice::Auto, true), Ok(1));
    }

    #[test]
    fn automatic_never_takes_a_discrete_adapter() {
        let only_nvidia = vec![desktop(true).remove(0)];

        assert!(pick(&only_nvidia, &Choice::Auto, true).is_err());
        assert!(pick(&only_nvidia, &Choice::Auto, false).is_err());
        assert!(pick(&[], &Choice::Auto, true).is_err());
    }

    #[test]
    fn an_adapter_that_cannot_present_is_refused_for_a_window() {
        assert!(pick(&desktop(false), &Choice::Auto, true).is_err());
        assert!(pick(&desktop(false), &Choice::Bus("0000:73:00.0".into()), true).is_err());

        // A headless renderer presents nothing.
        assert_eq!(pick(&desktop(false), &Choice::Auto, false), Ok(1));
    }

    #[test]
    fn a_bus_id_takes_that_adapter() {
        let adapters = desktop(true);

        assert_eq!(
            pick(&adapters, &Choice::Bus("0000:01:00.0".into()), true),
            Ok(0)
        );
        assert_eq!(
            pick(&adapters, &Choice::Bus(" 0000:73:00.0 ".into()), true),
            Ok(1)
        );
        assert_eq!(
            pick(&adapters, &Choice::Bus("0000:73:00.0".to_uppercase()), true),
            Ok(1)
        );
        assert!(pick(&adapters, &Choice::Bus("0000:02:00.0".into()), true).is_err());
    }

    #[test]
    fn vendor_and_device_stand_in_for_a_bus_id() {
        let mut adapters = desktop(true);

        adapters[1].bus.clear();

        assert_eq!(adapters[1].ids(), "1002:13c0");
        assert_eq!(
            pick(&adapters, &Choice::Bus("1002:13C0".into()), true),
            Ok(1)
        );
        assert!(pick(&adapters, &Choice::Bus("0000:73:00.0".into()), true).is_err());
    }

    #[test]
    fn only_the_compositor_of_the_active_adapter_clears_it() {
        let radeon = desktop(true).remove(1);
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
            assert_eq!(adapter.presents, None);
        }
    }
}
